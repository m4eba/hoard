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
    paths, publish, scan,
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
    let place_of = |c: &rusqlite::Connection, key: &str| -> anyhow::Result<Option<Place>> {
        Ok(db::get_setting(c, key)?.and_then(|v| serde_json::from_str::<Place>(&v).ok()))
    };
    let mut v = app
        .db
        .run(move |c| {
            Ok(json!({
                "downloadPlace": place_of(c, "download_place")?,
                "ytdlpArgs": db::get_setting(c, "ytdlp_args")?.unwrap_or_else(|| imports::DEFAULT_ARGS.to_string()),
                "fifflyUrl": publish::fiffly_url(c)?,
                "fifflySite": db::get_setting(c, "fiffly_site")?,
                "fifflyPlace": place_of(c, "fiffly_place")?,
            }))
        })
        .await?;
    v["defaultYtdlpArgs"] = imports::DEFAULT_ARGS.into();
    v["defaultFifflyUrl"] = publish::DEFAULT_FIFFLY_URL.into();
    v["trashDays"] = app.cfg.trash_days.into();
    Ok(Json(v))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSettings {
    download_place: Option<Option<Place>>,
    ytdlp_args: Option<String>,
    fiffly_url: Option<String>,
    /// The address fiffly reaches hoard at; none means wherever the browser
    /// has it open.
    fiffly_site: Option<Option<String>>,
    fiffly_place: Option<Option<Place>>,
}

/// `https://host[:port]` with nothing after it: an origin, which is all
/// fiffly and the OAuth redirect check deal in.
fn origin(raw: &str, what: &str) -> Result<String, crate::error::Reject> {
    let s = raw.trim().trim_end_matches('/');
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"));
    match rest {
        Some(host) if !host.is_empty() && !host.contains(['/', '?', '#', ' ']) => Ok(s.to_lowercase()),
        _ => Err(bad_request(format!("{what} must be an address like https://example.com, without a path"))),
    }
}

pub async fn save_settings(State(app): State<App>, Json(body): Json<SaveSettings>) -> ApiResult<Json<Value>> {
    if let Some(args) = &body.ytdlp_args {
        if shlex::split(args).is_none() {
            return Err(bad_request("the yt-dlp arguments have unbalanced quotes").into());
        }
    }
    for place in [&body.download_place, &body.fiffly_place].into_iter().flatten().flatten() {
        ops::prepare_place(&app, place).await?;
    }
    let fiffly_url = body.fiffly_url.as_deref().map(|u| origin(u, "the fiffly address")).transpose()?;
    let fiffly_site = match &body.fiffly_site {
        Some(Some(s)) if !s.trim().is_empty() => Some(Some(origin(s, "hoard's address")?)),
        Some(_) => Some(None),
        None => None,
    };
    app.db
        .run(move |c| {
            let put = |key: &str, value: Option<Option<String>>| -> anyhow::Result<()> {
                match value {
                    Some(Some(v)) => db::set_setting(c, key, &v)?,
                    Some(None) => {
                        c.execute("DELETE FROM settings WHERE key = ?", [key])?;
                    }
                    None => {}
                }
                Ok(())
            };
            let json = |p: Option<Option<Place>>| p.map(|p| p.map(|p| serde_json::to_string(&p).unwrap_or_default()));
            put("download_place", json(body.download_place))?;
            put("fiffly_place", json(body.fiffly_place))?;
            put("fiffly_site", fiffly_site)?;
            put("fiffly_url", fiffly_url.map(Some))?;
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
