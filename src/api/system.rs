//! The event stream, background tasks, settings, the yt-dlp installation and
//! the trash.

use std::convert::Infallible;

use axum::{
    Json,
    extract::State,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    db,
    error::{ApiResult, bad_request},
    imports,
    ops::{self, Place},
    paths, scan,
    state::App,
};

/// Everything the server announces, for any open tab: task progress, new
/// thumbnails, library changes.
pub async fn events(State(app): State<App>) -> Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>> {
    let mut sub = app.events.subscribe();
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        loop {
            let ev = tokio::select! {
                ev = sub.recv() => ev,
                _ = tx.closed() => return,
            };
            let frame = match ev {
                Ok(ev) => SseEvent::default().data(serde_json::to_string(&ev).unwrap_or_default()),
                // A slow tab missed frames; telling it to refetch everything
                // is cheaper than replaying them.
                Err(RecvError::Lagged(_)) => SseEvent::default().data(r#"{"type":"library"}"#),
                Err(RecvError::Closed) => return,
            };
            if tx.send(Ok(frame)).await.is_err() {
                return;
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

pub async fn tasks(State(app): State<App>) -> Json<Value> {
    Json(json!({ "tasks": app.tasks.list() }))
}

pub async fn settings(State(app): State<App>) -> ApiResult<Json<Value>> {
    let (place, args) = app
        .db
        .run(|c| {
            let place = db::get_setting(c, "download_place")?.and_then(|v| serde_json::from_str::<Place>(&v).ok());
            let args = db::get_setting(c, "ytdlp_args")?.unwrap_or_else(|| imports::DEFAULT_ARGS.to_string());
            Ok((place, args))
        })
        .await?;
    Ok(Json(json!({
        "downloadPlace": place,
        "ytdlpArgs": args,
        "defaultYtdlpArgs": imports::DEFAULT_ARGS,
        "trashDays": app.cfg.trash_days,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSettings {
    download_place: Option<Option<Place>>,
    ytdlp_args: Option<String>,
}

pub async fn save_settings(State(app): State<App>, Json(body): Json<SaveSettings>) -> ApiResult<Json<Value>> {
    if let Some(args) = &body.ytdlp_args {
        if shlex::split(args).is_none() {
            return Err(bad_request("the yt-dlp arguments have unbalanced quotes").into());
        }
    }
    if let Some(Some(place)) = &body.download_place {
        ops::prepare_place(&app, place).await?;
    }
    app.db
        .run(move |c| {
            match body.download_place {
                Some(Some(place)) => db::set_setting(c, "download_place", &serde_json::to_string(&place)?)?,
                Some(None) => {
                    c.execute("DELETE FROM settings WHERE key = 'download_place'", [])?;
                }
                None => {}
            }
            if let Some(args) = body.ytdlp_args {
                db::set_setting(c, "ytdlp_args", args.trim())?;
            }
            Ok(())
        })
        .await?;
    settings(State(app)).await
}

pub async fn ytdlp_status(State(app): State<App>) -> Json<Value> {
    Json(serde_json::to_value(app.ytdlp.status()).unwrap_or_default())
}

/// Installs or updates yt-dlp. Runs in the background; progress shows as a
/// task and the status endpoint reflects the outcome.
pub async fn ytdlp_install(State(app): State<App>) -> ApiResult<Json<Value>> {
    if !app.ytdlp.status().available {
        return Err(bad_request("no Python 3 interpreter found; install python3 or set HOARD_PYTHON").into());
    }
    let a = app.clone();
    tokio::spawn(async move {
        if let Err(e) = a.ytdlp.install(&a).await {
            tracing::warn!("yt-dlp install: {e:#}");
        }
    });
    Ok(Json(json!({ "ok": true })))
}

pub async fn trash(State(app): State<App>) -> ApiResult<Json<Value>> {
    let roots: Vec<(i64, String, String)> = app
        .db
        .run(|c| {
            let mut stmt = c.prepare("SELECT id, name, path FROM roots ORDER BY name")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            Ok(rows)
        })
        .await?;
    let sizes = tokio::task::spawn_blocking(move || {
        roots
            .into_iter()
            .map(|(id, name, path)| {
                let bytes = paths::dir_size(&std::path::Path::new(&path).join(paths::TRASH_DIR));
                json!({ "rootId": id, "name": name, "bytes": bytes })
            })
            .collect::<Vec<_>>()
    })
    .await?;
    Ok(Json(json!({ "trashDays": app.cfg.trash_days, "roots": sizes })))
}

pub async fn empty_trash(State(app): State<App>) -> ApiResult<Json<Value>> {
    let freed = scan::purge_trash(&app, 0).await?;
    Ok(Json(json!({ "freed": freed })))
}
