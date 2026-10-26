//! ffprobe and ffmpeg: reading a file's dimensions and duration, its
//! thumbnail, and on request a storyboard sprite sheet for scrubbing.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{
    db::params,
    events::Event,
    media,
    state::App,
};

#[derive(Debug, Default)]
pub struct Probe {
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub video_codec: String,
    pub audio_codec: String,
}

#[derive(Deserialize)]
struct FfprobeOut {
    #[serde(default)]
    streams: Vec<FfStream>,
    format: Option<FfFormat>,
}

#[derive(Deserialize)]
struct FfStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
    #[serde(default)]
    side_data_list: Vec<serde_json::Value>,
    disposition: Option<std::collections::HashMap<String, i64>>,
}

#[derive(Deserialize)]
struct FfFormat {
    duration: Option<String>,
}

pub async fn probe(ffprobe: &str, file: &Path) -> Result<Probe> {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .with_context(|| format!("running {ffprobe}"))?;
    if !out.status.success() {
        bail!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let parsed: FfprobeOut = serde_json::from_slice(&out.stdout).context("parsing ffprobe output")?;
    let mut p = Probe {
        duration: parsed.format.and_then(|f| f.duration).and_then(|d| d.parse().ok()).unwrap_or(0.0),
        ..Default::default()
    };
    // Cover art in an audio-only or music file shows up as a video stream;
    // the real picture is the first one that is not an attachment.
    let video = parsed.streams.iter().find(|s| {
        s.codec_type.as_deref() == Some("video")
            && !s.disposition.as_ref().is_some_and(|d| d.get("attached_pic") == Some(&1))
    });
    if let Some(v) = video {
        p.width = v.width.unwrap_or(0);
        p.height = v.height.unwrap_or(0);
        p.video_codec = v.codec_name.clone().unwrap_or_default();
        // Phone footage is stored sideways with a rotation flag; the size the
        // grid lays out with has to be the size it is displayed at.
        let rotation = v
            .tags
            .get("rotate")
            .and_then(|r| r.parse::<i64>().ok())
            .or_else(|| v.side_data_list.iter().find_map(|d| d.get("rotation")?.as_i64()))
            .unwrap_or(0);
        if rotation.rem_euclid(180) == 90 {
            std::mem::swap(&mut p.width, &mut p.height);
        }
    }
    if let Some(a) = parsed.streams.iter().find(|s| s.codec_type.as_deref() == Some("audio")) {
        p.audio_codec = a.codec_name.clone().unwrap_or_default();
    }
    Ok(p)
}

/// Thumbnails fit inside this box, which is enough for the large grid on a
/// high-density screen.
const THUMB_BOX: u32 = 640;

pub fn thumb_path(data: &crate::config::Config, id: i64) -> PathBuf {
    data.thumbs_dir().join((id / 1000).to_string()).join(format!("{id}.jpg"))
}

pub fn storyboard_path(data: &crate::config::Config, id: i64) -> PathBuf {
    data.storyboards_dir().join((id / 1000).to_string()).join(format!("{id}.jpg"))
}

async fn ffmpeg_frame(ffmpeg: &str, input: &Path, at: f64, filter: &str, quality: u8, out: &Path) -> Result<()> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-v", "error", "-y"]);
    if at > 0.0 {
        // Before -i, so ffmpeg seeks by keyframe instead of decoding up to the
        // timestamp — the difference between milliseconds and minutes on a
        // long film.
        cmd.args(["-ss", &format!("{at:.3}")]);
    }
    let out_status = cmd
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1", "-vf", filter, "-q:v", &quality.to_string()])
        .arg(out)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .with_context(|| format!("running {ffmpeg}"))?;
    if !out_status.status.success() || !out.exists() {
        bail!("ffmpeg: {}", String::from_utf8_lossy(&out_status.stderr).trim());
    }
    Ok(())
}

pub async fn thumbnail(ffmpeg: &str, input: &Path, kind: &str, duration: f64, out: &Path) -> Result<()> {
    if let Some(dir) = out.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let filter = format!(
        "scale='min({THUMB_BOX},iw)':'min({THUMB_BOX},ih)':force_original_aspect_ratio=decrease"
    );
    // A tenth of the way in skips black intro frames and title cards, capped
    // so a long film does not need a far seek.
    let at = if kind == "video" && duration > 2.0 { (duration * 0.1).min(60.0) } else { 0.0 };
    match ffmpeg_frame(ffmpeg, input, at, &filter, 4, out).await {
        Err(_) if at > 0.0 => ffmpeg_frame(ffmpeg, input, 0.0, &filter, 4, out).await,
        other => other,
    }
}

/// Probes and thumbnails everything waiting, then sleeps until woken. Work is
/// found in the table rather than passed in, so a restart picks up whatever a
/// previous run did not finish.
pub async fn worker(app: App) {
    loop {
        let pending = app
            .db
            .run(|c| {
                let mut stmt = c.prepare("SELECT id FROM media WHERE processed = 0 ORDER BY id LIMIT 64")?;
                let ids: Vec<i64> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
                Ok(ids)
            })
            .await
            .unwrap_or_default();
        if pending.is_empty() {
            app.work.notified().await;
            continue;
        }
        stream::iter(pending)
            .for_each_concurrent(app.cfg.workers, |id| {
                let app = app.clone();
                async move {
                    if let Err(e) = process_media(&app, id).await {
                        tracing::warn!("processing media {id}: {e:#}");
                        let _ = app
                            .db
                            .run(move |c| Ok(c.execute("UPDATE media SET processed = 2 WHERE id = ?", [id])?))
                            .await;
                    }
                }
            })
            .await;
    }
}

/// Probes a file and writes its thumbnail, then tells open grids about it.
pub async fn process_media(app: &App, id: i64) -> Result<()> {
    let Some(file) = app.db.run(move |c| media::file(c, id)).await? else { return Ok(()) };
    let kind: String = app.db.run(move |c| Ok(c.query_row("SELECT kind FROM media WHERE id = ?", [id], |r| r.get(0))?)).await?;
    let path = file.path();
    let p = probe(&app.cfg.ffprobe, &path).await?;
    let duration = if kind == "video" { p.duration } else { 0.0 };

    let thumb = thumb_path(&app.cfg, id);
    let has_thumb = match thumbnail(&app.cfg.ffmpeg, &path, &kind, duration, &thumb).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("thumbnail for {}: {e:#}", path.display());
            false
        }
    };

    let ver = app
        .db
        .run(move |c| {
            c.execute(
                "UPDATE media SET processed = 1, duration = ?, width = ?, height = ?,
                    video_codec = ?, audio_codec = ?,
                    thumb_ver = CASE WHEN ? THEN thumb_ver + 1 ELSE 0 END
                 WHERE id = ?",
                params![duration, p.width, p.height, p.video_codec, p.audio_codec, has_thumb, id],
            )?;
            Ok(c.query_row("SELECT thumb_ver FROM media WHERE id = ?", [id], |r| r.get::<_, i64>(0))?)
        })
        .await?;
    app.events.send(Event::Media { id, thumb_url: media::thumb_url(id, ver) });
    Ok(())
}

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

const SB_COLS: u32 = 10;
const SB_MAX: u32 = 100;
const SB_TILE_WIDTH: u32 = 160;

/// Builds the sprite sheet: up to a hundred frames spread evenly over the
/// video, each grabbed with a fast seek, then tiled into one image.
///
/// One ffmpeg per frame looks wasteful but is what keeps a two-hour film to
/// seconds: a single pass with an fps filter would decode every frame of it.
pub async fn build_storyboard(app: &App, id: i64) -> Result<Storyboard> {
    let file = app.db.run(move |c| media::file(c, id)).await?.context("media not found")?;
    let (duration, width, height, title, old): (f64, i64, i64, String, Option<String>) = app
        .db
        .run(move |c| {
            Ok(c.query_row(
                "SELECT duration, width, height, title, storyboard FROM media WHERE id = ?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?)
        })
        .await?;
    if duration <= 0.0 {
        bail!("this file has no duration to build a storyboard from");
    }

    let interval = (duration / SB_MAX as f64).max(1.0);
    let count = ((duration / interval).floor() as u32).clamp(1, SB_MAX);
    let tile_height = if width > 0 && height > 0 {
        (((SB_TILE_WIDTH as f64 * height as f64 / width as f64) / 2.0).round() as u32 * 2).max(2)
    } else {
        90
    };
    let rows = count.div_ceil(SB_COLS);
    let task = app.tasks.start("storyboard", format!("Storyboard: {title}"), count as u64);

    let tmp = app.cfg.tmp_dir().join(format!("storyboard-{id}"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    let input = file.path();
    let filter = format!("scale={SB_TILE_WIDTH}:{tile_height}");
    let done = std::sync::atomic::AtomicU64::new(0);

    stream::iter(0..count)
        .for_each_concurrent(4, |i| {
            let (input, tmp, filter, ffmpeg) = (&input, &tmp, &filter, &app.cfg.ffmpeg);
            let (done, task) = (&done, &task);
            async move {
                let at = i as f64 * interval + interval / 2.0;
                let out = tmp.join(format!("f{i:03}.jpg"));
                if let Err(e) = ffmpeg_frame(ffmpeg, input, at.min(duration - 0.1).max(0.0), filter, 5, &out).await {
                    tracing::debug!("storyboard frame {i} of {id}: {e:#}");
                }
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                task.progress(n, count as u64, "");
            }
        })
        .await;

    // A frame that could not be decoded — usually right at the end — borrows
    // its neighbour, so the sheet has no hole that would desync every tile
    // after it.
    let mut last: Option<PathBuf> = None;
    for i in 0..count {
        let frame = tmp.join(format!("f{i:03}.jpg"));
        if frame.exists() {
            last = Some(frame);
        } else if let Some(prev) = &last {
            tokio::fs::copy(prev, &frame).await?;
        }
    }
    if last.is_none() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        task.fail("no frame could be decoded");
        bail!("no frame could be decoded");
    }
    // Fill any leading gap from the first good frame.
    let first_good = (0..count).map(|i| tmp.join(format!("f{i:03}.jpg"))).find(|p| p.exists()).unwrap();
    for i in 0..count {
        let frame = tmp.join(format!("f{i:03}.jpg"));
        if !frame.exists() {
            tokio::fs::copy(&first_good, &frame).await?;
        }
    }

    let out = storyboard_path(&app.cfg, id);
    if let Some(dir) = out.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let status = Command::new(&app.cfg.ffmpeg)
        .args(["-nostdin", "-v", "error", "-y", "-framerate", "1", "-i"])
        .arg(tmp.join("f%03d.jpg"))
        .args(["-vf", &format!("tile={SB_COLS}x{rows}"), "-frames:v", "1", "-q:v", "4"])
        .arg(&out)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    if !status.status.success() {
        let msg = String::from_utf8_lossy(&status.stderr).trim().to_string();
        task.fail(msg.clone());
        bail!("ffmpeg tile: {msg}");
    }

    let ver = old
        .and_then(|s| serde_json::from_str::<Storyboard>(&s).ok())
        .map(|s| s.ver + 1)
        .unwrap_or(1);
    let sb = Storyboard { cols: SB_COLS, rows, tile_width: SB_TILE_WIDTH, tile_height, interval, count, ver };
    let json = serde_json::to_string(&sb)?;
    app.db
        .run(move |c| Ok(c.execute("UPDATE media SET storyboard = ? WHERE id = ?", params![json, id])?))
        .await?;
    task.finish("");
    Ok(sb)
}

/// Removes everything the data directory holds for a media item.
pub fn remove_artifacts(cfg: &crate::config::Config, id: i64) {
    let _ = std::fs::remove_file(thumb_path(cfg, id));
    let _ = std::fs::remove_file(storyboard_path(cfg, id));
    let _ = std::fs::remove_file(cfg.meta_dir().join(format!("{id}.info.json")));
}
