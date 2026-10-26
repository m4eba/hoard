//! URL imports: a queue of yt-dlp downloads, each landing as library media in
//! the folder its card points at when it finishes.

mod download;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use rusqlite::{OptionalExtension, Row};
use serde::Serialize;
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    db::{self, params},
    events::Event,
    media,
    ops::Place,
    state::App,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: i64,
    pub batch_id: String,
    pub url: String,
    /// queued | extracting | downloading | processing | done | failed | cancelled
    pub state: String,
    pub title: String,
    pub bytes_total: i64,
    pub bytes_done: i64,
    pub percent: i64,
    pub tags: Vec<String>,
    pub root_id: Option<i64>,
    pub folder: String,
    pub media_id: Option<i64>,
    pub thumb_url: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
}

const JOB_COLUMNS: &str = "id, batch_id, url, state, title, bytes_total, bytes_done, tags, root_id,
    folder, media_id, thumb_url, error, created_at";

fn job_row(r: &Row) -> rusqlite::Result<Job> {
    let (total, done): (i64, i64) = (r.get(5)?, r.get(6)?);
    let tags: String = r.get(7)?;
    let thumb: String = r.get(11)?;
    let error: String = r.get(12)?;
    Ok(Job {
        id: r.get(0)?,
        batch_id: r.get(1)?,
        url: r.get(2)?,
        state: r.get(3)?,
        title: r.get(4)?,
        bytes_total: total,
        bytes_done: done,
        percent: if total > 0 { (done * 100 / total).clamp(0, 100) } else { 0 },
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        root_id: r.get(8)?,
        folder: r.get(9)?,
        media_id: r.get(10)?,
        thumb_url: (!thumb.is_empty()).then_some(thumb),
        error: (!error.is_empty()).then_some(error),
        created_at: media::iso(r.get(13)?),
    })
}

pub fn get_job(conn: &rusqlite::Connection, id: i64) -> Result<Option<Job>> {
    Ok(conn
        .query_row(&format!("SELECT {JOB_COLUMNS} FROM import_jobs WHERE id = ?"), [id], job_row)
        .optional()?)
}

pub fn batch_jobs(conn: &rusqlite::Connection, batch: &str) -> Result<Vec<Job>> {
    let mut stmt = conn.prepare(&format!("SELECT {JOB_COLUMNS} FROM import_jobs WHERE batch_id = ? ORDER BY id"))?;
    let jobs = stmt.query_map([batch], job_row)?.collect::<Result<_, _>>()?;
    Ok(jobs)
}

pub fn is_terminal(state: &str) -> bool {
    matches!(state, "done" | "failed" | "cancelled")
}

pub struct Imports {
    wake: Notify,
    running: Mutex<HashMap<i64, CancellationToken>>,
    slots: Arc<Semaphore>,
}

/// What the format and post-processing default to. Prefers H.264 and AAC in
/// MP4, which every browser plays, over higher-resolution codecs some do not.
pub const DEFAULT_ARGS: &str = r#"-S "vcodec:h264,res,acodec:aac" --merge-output-format mp4"#;

impl Imports {
    pub fn new(downloads: usize) -> Self {
        Self { wake: Notify::new(), running: Default::default(), slots: Arc::new(Semaphore::new(downloads)) }
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Stops a running download. Returns false when it was not running.
    pub fn cancel(&self, id: i64) -> bool {
        match self.running.lock().unwrap().get(&id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

pub async fn emit(app: &App, id: i64) {
    if let Ok(Some(job)) = app.db.run(move |c| get_job(c, id)).await {
        app.events.send(Event::Job { job });
    }
}

/// Claims queued jobs as download slots free up.
pub async fn dispatcher(app: App) {
    // Whatever was in flight when the server stopped starts over.
    let _ = app
        .db
        .run(|c| {
            Ok(c.execute(
                "UPDATE import_jobs SET state = 'queued', bytes_done = 0
                 WHERE state IN ('extracting', 'downloading', 'processing')",
                [],
            )?)
        })
        .await;
    loop {
        let permit = app.imports.slots.clone().acquire_owned().await.expect("semaphore open");
        let next: Option<i64> = app
            .db
            .run(|c| {
                Ok(c.query_row(
                    "UPDATE import_jobs SET state = 'extracting'
                     WHERE id = (SELECT id FROM import_jobs WHERE state = 'queued' ORDER BY id LIMIT 1)
                     RETURNING id",
                    [],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .await
            .unwrap_or(None);
        let Some(id) = next else {
            drop(permit);
            app.imports.wake.notified().await;
            continue;
        };
        let app = app.clone();
        tokio::spawn(async move {
            emit(&app, id).await;
            let token = CancellationToken::new();
            app.imports.running.lock().unwrap().insert(id, token.clone());
            let result = download::run_job(&app, id, token).await;
            app.imports.running.lock().unwrap().remove(&id);
            let (state, error) = match result {
                Ok(true) => ("done", String::new()),
                Ok(false) => ("cancelled", String::new()),
                Err(e) => ("failed", format!("{e:#}")),
            };
            let _ = app
                .db
                .run(move |c| {
                    Ok(c.execute("UPDATE import_jobs SET state = ?, error = ? WHERE id = ?", params![state, error, id])?)
                })
                .await;
            emit(&app, id).await;
            drop(permit);
        });
    }
}

/// The configured download destination, or the first storage folder.
pub fn default_place(c: &rusqlite::Connection) -> Result<Option<Place>> {
    let configured = db::get_setting(c, "download_place")?.and_then(|v| serde_json::from_str::<Place>(&v).ok());
    if let Some(place) = configured {
        let exists: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM roots WHERE id = ?)", [place.root_id], |r| r.get(0))?;
        if exists {
            return Ok(Some(place));
        }
    }
    Ok(c.query_row("SELECT id FROM roots ORDER BY id LIMIT 1", [], |r| r.get(0))
        .optional()?
        .map(|root_id| Place { root_id, folder: String::new() }))
}

/// Starts a batch: one job per URL.
pub async fn create_batch(app: &App, urls: Vec<String>, tags: Vec<String>, place: Option<Place>) -> Result<(String, Vec<Job>)> {
    let batch: String = {
        use rand::Rng;
        let mut rng = rand::rng();
        (0..12).map(|_| char::from(b"abcdefghijkmnpqrstuvwxyz23456789"[rng.random_range(0..32)])).collect()
    };
    let tags = serde_json::to_string(&media::normalize_tags(&tags))?;
    let b = batch.clone();
    let jobs = app
        .db
        .run(move |c| {
            let tx = c.transaction()?;
            let now = db::now();
            for url in &urls {
                tx.execute(
                    "INSERT INTO import_jobs (batch_id, url, state, tags, root_id, folder, created_at)
                     VALUES (?, ?, 'queued', ?, ?, ?, ?)",
                    params![b, url, tags, place.as_ref().map(|p| p.root_id), place.as_ref().map(|p| p.folder.clone()).unwrap_or_default(), now],
                )?;
            }
            tx.commit()?;
            batch_jobs(c, &b)
        })
        .await?;
    app.imports.wake();
    Ok((batch, jobs))
}

/// Recently started batches, newest first, for the import page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub batch_id: String,
    pub total: i64,
    pub done: i64,
    pub failed: i64,
    pub running: i64,
    pub first_title: String,
    pub created_at: String,
}

pub fn recent_batches(conn: &rusqlite::Connection, limit: i64) -> Result<Vec<BatchSummary>> {
    let mut stmt = conn.prepare(
        "SELECT batch_id, count(*), sum(state = 'done'), sum(state = 'failed'),
            sum(state NOT IN ('done', 'failed', 'cancelled')),
            (SELECT coalesce(nullif(j2.title, ''), j2.url) FROM import_jobs j2 WHERE j2.batch_id = j.batch_id ORDER BY id LIMIT 1),
            min(created_at)
         FROM import_jobs j GROUP BY batch_id ORDER BY max(id) DESC LIMIT ?",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok(BatchSummary {
            batch_id: r.get(0)?,
            total: r.get(1)?,
            done: r.get(2)?,
            failed: r.get(3)?,
            running: r.get(4)?,
            first_title: r.get(5)?,
            created_at: media::iso(r.get(6)?),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
