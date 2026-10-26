//! URL import endpoints: starting batches, refiling cards, cancel and retry,
//! and the live progress stream.

use std::{collections::HashMap, convert::Infallible};

use axum::{
    Json,
    extract::{Path, State},
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    error::{ApiResult, bad_request, not_found},
    events::Event,
    imports::{self, Job},
    media,
    ops::{self, Place},
    state::App,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Start {
    urls: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    root_id: Option<i64>,
    #[serde(default)]
    folder: String,
}

pub async fn start(State(app): State<App>, Json(body): Json<Start>) -> ApiResult<Json<Value>> {
    let mut urls: Vec<String> = body
        .urls
        .iter()
        .map(|u| u.trim().to_string())
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .collect();
    urls.dedup();
    if urls.is_empty() {
        return Err(bad_request("no http(s) links found").into());
    }
    if urls.len() > 500 {
        return Err(bad_request("at most 500 links per batch").into());
    }
    let place = body.root_id.map(|root_id| Place { root_id, folder: body.folder });
    if let Some(p) = &place {
        ops::prepare_place(&app, p).await?;
    }
    let (batch, jobs) = imports::create_batch(&app, urls, body.tags, place).await?;
    Ok(Json(json!({ "batchId": batch, "jobs": jobs })))
}

pub async fn recent(State(app): State<App>) -> ApiResult<Json<Value>> {
    let batches = app.db.run(|c| imports::recent_batches(c, 30)).await?;
    Ok(Json(json!({ "batches": batches })))
}

pub async fn batch(State(app): State<App>, Path(batch): Path<String>) -> ApiResult<Json<Value>> {
    let jobs = app.db.run(move |c| imports::batch_jobs(c, &batch)).await?;
    Ok(Json(json!({ "jobs": jobs })))
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Refile {
    tags: Option<Vec<String>>,
    root_id: Option<i64>,
    folder: Option<String>,
}

/// Applies a card's tags and folder. Before the download lands they are
/// staged on the job; afterwards they are written to the media, and a folder
/// change moves the file.
async fn refile(app: &App, job: Job, body: Refile) -> ApiResult<()> {
    let place = body.root_id.map(|root_id| Place { root_id, folder: body.folder.clone().unwrap_or_default() });
    if let Some(p) = &place {
        ops::prepare_place(app, p).await?;
    }
    let tags = body.tags.as_ref().map(|t| media::normalize_tags(t));
    let (id, media_id) = (job.id, job.media_id);
    {
        let (tags, place) = (tags.clone(), place.clone());
        app.db
            .run(move |c| {
                if let Some(tags) = &tags {
                    c.execute("UPDATE import_jobs SET tags = ? WHERE id = ?", crate::db::params![serde_json::to_string(tags)?, id])?;
                    if let Some(m) = media_id {
                        media::set_tags(c, m, tags)?;
                    }
                }
                if let Some(p) = &place {
                    c.execute(
                        "UPDATE import_jobs SET root_id = ?, folder = ? WHERE id = ?",
                        crate::db::params![p.root_id, p.folder, id],
                    )?;
                }
                Ok(())
            })
            .await?;
    }
    if let (Some(m), Some(p)) = (media_id, place) {
        ops::start_move(app, vec![m], p).await?;
    }
    imports::emit(app, id).await;
    Ok(())
}

pub async fn patch_job(
    State(app): State<App>,
    Path((batch, id)): Path<(String, i64)>,
    Json(body): Json<Refile>,
) -> ApiResult<Json<Value>> {
    let job = app
        .db
        .run(move |c| imports::get_job(c, id))
        .await?
        .filter(|j| j.batch_id == batch)
        .ok_or_else(|| not_found("job"))?;
    refile(&app, job, body).await?;
    Ok(Json(json!({ "updated": 1 })))
}

/// Applies tags and a folder to every card in the batch.
pub async fn patch_batch(State(app): State<App>, Path(batch): Path<String>, Json(body): Json<Refile>) -> ApiResult<Json<Value>> {
    let jobs = app.db.run(move |c| imports::batch_jobs(c, &batch)).await?;
    let n = jobs.len();
    for job in jobs {
        refile(&app, job, body.clone()).await?;
    }
    Ok(Json(json!({ "updated": n })))
}

pub async fn cancel_job(State(app): State<App>, Path((batch, id)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    if !app.imports.cancel(id) {
        let changed = app
            .db
            .run(move |c| {
                Ok(c.execute(
                    "UPDATE import_jobs SET state = 'cancelled' WHERE id = ? AND batch_id = ? AND state = 'queued'",
                    crate::db::params![id, batch],
                )?)
            })
            .await?;
        if changed > 0 {
            imports::emit(&app, id).await;
        }
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn retry_job(State(app): State<App>, Path((batch, id)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let changed = app
        .db
        .run(move |c| {
            Ok(c.execute(
                "UPDATE import_jobs SET state = 'queued', error = '', bytes_done = 0, bytes_total = 0
                 WHERE id = ? AND batch_id = ? AND state IN ('failed', 'cancelled')",
                crate::db::params![id, batch],
            )?)
        })
        .await?;
    if changed == 0 {
        return Err(bad_request("only failed or cancelled downloads can be retried").into());
    }
    imports::emit(&app, id).await;
    app.imports.wake();
    Ok(Json(json!({ "ok": true })))
}

fn frame(job: &Job) -> SseEvent {
    SseEvent::default().data(serde_json::to_string(job).unwrap_or_default())
}

/// Live progress for one batch. Opens with a snapshot of every job, so a page
/// opened late shows the truth at once, and closes with a `done` event when
/// nothing is left running.
pub async fn batch_events(
    State(app): State<App>,
    Path(batch): Path<String>,
) -> ApiResult<Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>>> {
    let mut sub = app.events.subscribe();
    let b = batch.clone();
    let snapshot = app.db.run(move |c| imports::batch_jobs(c, &b)).await?;
    if snapshot.is_empty() {
        return Err(not_found("batch").into());
    }
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut states: HashMap<i64, String> = HashMap::new();
        for job in &snapshot {
            states.insert(job.id, job.state.clone());
            if tx.send(Ok(frame(job))).await.is_err() {
                return;
            }
        }
        loop {
            if states.values().all(|s| imports::is_terminal(s)) {
                let _ = tx.send(Ok(SseEvent::default().event("done").data("{}"))).await;
                return;
            }
            let ev = tokio::select! {
                ev = sub.recv() => ev,
                _ = tx.closed() => return,
            };
            match ev {
                Ok(Event::Job { job }) if job.batch_id == batch => {
                    states.insert(job.id, job.state.clone());
                    if tx.send(Ok(frame(&job))).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    // Frames were dropped; a fresh snapshot is the simplest
                    // way back to the truth.
                    let b = batch.clone();
                    let Ok(jobs) = app.db.run(move |c| imports::batch_jobs(c, &b)).await else { return };
                    for job in jobs {
                        states.insert(job.id, job.state.clone());
                        if tx.send(Ok(frame(&job))).await.is_err() {
                            return;
                        }
                    }
                }
                Err(RecvError::Closed) => return,
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}
