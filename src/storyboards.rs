//! Storyboards: sprite sheets of frames for scrubbing, built on request.
//!
//! Requests go into a queue in the database, so a thousand-video folder can
//! be queued at once and survives a restart. One video is built at a time —
//! each build already runs several ffmpeg seeks in parallel — and the whole
//! queue shows as a single activity task.

use std::{
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{
    db::{self, params},
    events::{Event, Task},
    media,
    process::{ffmpeg_frame, storyboard_path},
    state::App,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Storyboard {
    pub cols: u32,
    pub rows: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub interval: f64,
    pub count: u32,
    pub ver: i64,
}

const COLS: u32 = 10;
const MAX_FRAMES: u32 = 100;
const TILE_WIDTH: u32 = 160;

/// Viewer requests jump the queue: someone is looking at that video now.
pub const PRIORITY_VIEWER: i64 = 1;
pub const PRIORITY_BULK: i64 = 0;

/// Queues videos for a storyboard. Without `rebuild`, videos that already
/// have one are skipped. Returns how many were queued.
pub fn enqueue(conn: &mut Connection, ids: &[i64], priority: i64, rebuild: bool) -> Result<usize> {
    let tx = conn.transaction()?;
    let now = db::now();
    let mut queued = 0;
    {
        let mut stmt = tx.prepare(&format!(
            "INSERT INTO storyboard_queue (media_id, priority, queued_at)
             SELECT id, ?2, ?3 FROM media
             WHERE id = ?1 AND kind = 'video' {}
             ON CONFLICT (media_id) DO UPDATE SET priority = max(priority, excluded.priority)",
            if rebuild { "" } else { "AND storyboard IS NULL" }
        ))?;
        for &id in ids {
            queued += stmt.execute(params![id, priority, now])?;
        }
    }
    tx.commit()?;
    Ok(queued)
}

pub fn is_queued(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS (SELECT 1 FROM storyboard_queue WHERE media_id = ?)", [id], |r| r.get(0))?)
}

/// Empties the queue. The video being built when this is called finishes.
pub fn clear(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("DELETE FROM storyboard_queue", [])?)
}

/// Works through the queue, then sleeps until something is added.
pub async fn worker(app: App) {
    // The aggregate task lives while the queue has work; `done` and `failed`
    // count since it started, so the bar reads "37 of 120" across additions.
    let mut task: Option<(Task, u64, u64)> = None;
    loop {
        let next: Option<(i64, String, i64)> = app
            .db
            .run(|c| {
                Ok(c.query_row(
                    // A video still waiting to be probed has no duration yet;
                    // it waits, and the probe worker wakes this one after.
                    "SELECT q.media_id, m.title, (SELECT count(*) FROM storyboard_queue)
                     FROM storyboard_queue q JOIN media m ON m.id = q.media_id
                     WHERE m.processed != 0
                     ORDER BY q.priority DESC, q.queued_at, q.media_id LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?)
            })
            .await
            .unwrap_or(None);

        let Some((id, title, remaining)) = next else {
            if let Some((t, done, failed)) = task.take() {
                let detail = match failed {
                    0 => format!("{done} built"),
                    n => format!("{} built, {n} failed", done - n),
                };
                if failed > 0 && failed == done { t.fail(detail) } else { t.finish(detail) }
            }
            app.storyboard_wake.notified().await;
            continue;
        };

        let (t, done, failed) = task.get_or_insert_with(|| (app.tasks.start("storyboards", "Building storyboards", 0), 0, 0));
        t.progress(*done, *done + remaining as u64, title);

        app.storyboards.lock().unwrap().insert(id);
        let result = build(&app, id).await;
        app.storyboards.lock().unwrap().remove(&id);
        let _ = app
            .db
            .run(move |c| Ok(c.execute("DELETE FROM storyboard_queue WHERE media_id = ?", [id])?))
            .await;
        if let Err(e) = &result {
            tracing::warn!("storyboard for {id}: {e:#}");
            *failed += 1;
        }
        *done += 1;
        app.events.send(Event::Storyboard { id, ready: result.is_ok() });
    }
}

/// Builds the sprite sheet: up to a hundred frames spread evenly over the
/// video, each grabbed with a fast seek, then tiled into one image.
///
/// One ffmpeg per frame looks wasteful but is what keeps a two-hour film to
/// seconds: a single pass with an fps filter would decode every frame of it.
async fn build(app: &App, id: i64) -> Result<Storyboard> {
    let file = app.db.run(move |c| media::file(c, id)).await?.context("media not found")?;
    let (duration, width, height, old): (f64, i64, i64, Option<String>) = app
        .db
        .run(move |c| {
            Ok(c.query_row(
                "SELECT duration, width, height, storyboard FROM media WHERE id = ?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await?;
    if duration <= 0.0 {
        bail!("no duration to build a storyboard from (not probed yet, or not a video)");
    }

    let interval = (duration / MAX_FRAMES as f64).max(1.0);
    let count = ((duration / interval).floor() as u32).clamp(1, MAX_FRAMES);
    let tile_height = if width > 0 && height > 0 {
        (((TILE_WIDTH as f64 * height as f64 / width as f64) / 2.0).round() as u32 * 2).max(2)
    } else {
        90
    };
    let rows = count.div_ceil(COLS);

    let tmp = app.cfg.tmp_dir().join(format!("storyboard-{id}"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    let result = frames_and_tile(app, &file.path(), &tmp, duration, interval, count, tile_height, rows, id).await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    result?;

    let ver = old
        .and_then(|s| serde_json::from_str::<Storyboard>(&s).ok())
        .map(|s| s.ver + 1)
        .unwrap_or(1);
    let sb = Storyboard { cols: COLS, rows, tile_width: TILE_WIDTH, tile_height, interval, count, ver };
    let json = serde_json::to_string(&sb)?;
    app.db
        .run(move |c| Ok(c.execute("UPDATE media SET storyboard = ? WHERE id = ?", params![json, id])?))
        .await?;
    Ok(sb)
}

#[allow(clippy::too_many_arguments)]
async fn frames_and_tile(
    app: &App,
    input: &std::path::Path,
    tmp: &std::path::Path,
    duration: f64,
    interval: f64,
    count: u32,
    tile_height: u32,
    rows: u32,
    id: i64,
) -> Result<()> {
    let filter = format!("scale={TILE_WIDTH}:{tile_height}");
    let grabbed = AtomicU64::new(0);
    stream::iter(0..count)
        .for_each_concurrent(4, |i| {
            let (filter, ffmpeg, grabbed) = (&filter, &app.cfg.ffmpeg, &grabbed);
            async move {
                let at = (i as f64 * interval + interval / 2.0).min(duration - 0.1).max(0.0);
                let out = tmp.join(format!("f{i:03}.jpg"));
                match ffmpeg_frame(ffmpeg, input, at, filter, 5, &out).await {
                    Ok(()) => {
                        grabbed.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => tracing::debug!("storyboard frame {i} of {id}: {e:#}"),
                }
            }
        })
        .await;

    let frame = |i: u32| tmp.join(format!("f{i:03}.jpg"));
    let Some(first_good) = (0..count).map(frame).find(|p| p.exists()) else {
        bail!("no frame could be decoded");
    };
    // A frame that could not be decoded — usually right at the end — borrows
    // its neighbour, so the sheet has no hole that would desync every tile
    // after it.
    let mut last: PathBuf = first_good;
    for i in 0..count {
        let f = frame(i);
        if f.exists() {
            last = f;
        } else {
            tokio::fs::copy(&last, &f).await?;
        }
    }

    let out = storyboard_path(&app.cfg, id);
    if let Some(dir) = out.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let status = Command::new(&app.cfg.ffmpeg)
        .args(["-nostdin", "-v", "error", "-y", "-framerate", "1", "-i"])
        .arg(tmp.join("f%03d.jpg"))
        .args(["-vf", &format!("tile={COLS}x{rows}"), "-frames:v", "1", "-q:v", "4"])
        .arg(&out)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    if !status.status.success() {
        bail!("ffmpeg tile: {}", String::from_utf8_lossy(&status.stderr).trim());
    }
    Ok(())
}
