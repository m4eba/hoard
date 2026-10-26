//! Running one download: yt-dlp into a scratch directory, then the result
//! moved into its folder and registered as media.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_util::sync::CancellationToken;

use super::{DEFAULT_ARGS, default_place, emit, get_job};
use crate::{
    db::{self, params},
    events::Event,
    media,
    ops::{self, Place},
    paths, process,
    state::App,
    ytdlp::Ytdlp,
};

fn user_args(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let raw = db::get_setting(conn, "ytdlp_args")?.unwrap_or_else(|| DEFAULT_ARGS.to_string());
    shlex::split(&raw).context("the yt-dlp arguments setting has unbalanced quotes")
}

struct Progress {
    title: Option<String>,
    thumb: Option<String>,
    done: i64,
    total: i64,
    files: Vec<PathBuf>,
    stderr: Vec<String>,
}

/// Downloads one job. Ok(false) means it was cancelled.
pub(super) async fn run_job(app: &App, id: i64, token: CancellationToken) -> Result<bool> {
    let job = app.db.run(move |c| get_job(c, id)).await?.context("job vanished")?;
    let bin = Ytdlp::binary(&app.cfg);
    if !bin.exists() && app.cfg.ytdlp_bin.is_none() {
        bail!("yt-dlp is not installed — install it in Settings");
    }
    let extra = app.db.run(|c| user_args(c)).await?;
    let tmp = app.cfg.tmp_dir().join(format!("job-{id}"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;

    let mut cmd = Command::new(&bin);
    cmd.args(["--quiet", "--progress", "--newline", "--no-simulate", "--no-playlist", "--no-mtime"])
        .args(["--write-info-json", "--color", "never"])
        .args([
            "--progress-template",
            "download:HOARDPROG %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s",
        ])
        .args(["--print", "before_dl:HOARDTITLE %(title)s"])
        .args(["--print", "before_dl:HOARDTHUMB %(thumbnail)s"])
        .args(["--print", "after_move:HOARDFILE %(filepath)s"])
        .arg("-P")
        .arg(&tmp)
        .args(["-o", "%(title).150B [%(id)s].%(ext)s"]);
    if app.cfg.ffmpeg.contains(std::path::MAIN_SEPARATOR) {
        cmd.arg("--ffmpeg-location").arg(&app.cfg.ffmpeg);
    }
    cmd.args(&extra).arg("--").arg(&job.url);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

    let mut child = cmd.spawn().with_context(|| format!("starting {}", bin.display()))?;
    let (stdout, stderr) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let state = Arc::new(Mutex::new(Progress { title: None, thumb: None, done: 0, total: 0, files: vec![], stderr: vec![] }));

    // With --quiet, yt-dlp writes progress to stderr and --print to stdout;
    // both streams are read for markers so neither placement matters.
    let read = |stream: Box<dyn tokio::io::AsyncRead + Unpin + Send>, is_err: bool| {
        let state = state.clone();
        async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut s = state.lock().unwrap();
                if let Some(rest) = line.strip_prefix("HOARDPROG ") {
                    let n: Vec<i64> = rest.split(' ').map(|v| v.parse::<f64>().map(|f| f as i64).unwrap_or(0)).collect();
                    s.done = n.first().copied().unwrap_or(0);
                    s.total = n.get(1).copied().filter(|&t| t > 0).or(n.get(2).copied()).unwrap_or(0);
                } else if let Some(t) = line.strip_prefix("HOARDTITLE ") {
                    s.title = Some(t.to_string());
                } else if let Some(t) = line.strip_prefix("HOARDTHUMB ") {
                    s.thumb = (t.starts_with("http")).then(|| t.to_string());
                } else if let Some(f) = line.strip_prefix("HOARDFILE ") {
                    s.files.push(PathBuf::from(f));
                } else if is_err && !line.trim().is_empty() {
                    s.stderr.push(line);
                    if s.stderr.len() > 30 {
                        s.stderr.remove(0);
                    }
                }
            }
        }
    };
    let readers = tokio::spawn(futures_util::future::join(read(Box::new(stdout), false), read(Box::new(stderr), true)));

    // Progress is written to the table at most a few times a second; every
    // line yt-dlp prints would be a write and a frame to every open tab.
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let mut last = (String::new(), 0i64);
    let status = loop {
        tokio::select! {
            status = child.wait() => break status?,
            _ = token.cancelled() => {
                let _ = child.kill().await;
                readers.abort();
                let _ = tokio::fs::remove_dir_all(&tmp).await;
                return Ok(false);
            }
            _ = tick.tick() => {
                let (title, thumb, done, total) = {
                    let s = state.lock().unwrap();
                    (s.title.clone(), s.thumb.clone(), s.done, s.total)
                };
                let key = (title.clone().unwrap_or_default(), done);
                if key == last { continue; }
                last = key;
                let phase = if title.is_some() { "downloading" } else { "extracting" };
                app.db.run(move |c| Ok(c.execute(
                    "UPDATE import_jobs SET state = ?, title = coalesce(?, title),
                        thumb_url = coalesce(?, thumb_url), bytes_done = ?, bytes_total = ?
                     WHERE id = ? AND state NOT IN ('done', 'failed', 'cancelled')",
                    params![phase, title, thumb, done, total, id],
                )?)).await?;
                emit(app, id).await;
            }
        }
    };
    let _ = readers.await;
    let (files, stderr) = {
        let s = state.lock().unwrap();
        (s.files.clone(), s.stderr.clone())
    };
    if !status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let msg = stderr
            .iter()
            .rev()
            .find(|l| l.starts_with("ERROR"))
            .or(stderr.last())
            .cloned()
            .unwrap_or_else(|| format!("yt-dlp exited with {status}"));
        bail!("{}", msg.trim_start_matches("ERROR: "));
    }
    if files.is_empty() {
        bail!("yt-dlp finished without producing a file");
    }

    app.db
        .run(move |c| Ok(c.execute("UPDATE import_jobs SET state = 'processing' WHERE id = ?", [id])?))
        .await?;
    emit(app, id).await;

    let mut first = None;
    for file in files {
        let media_id = land(app, id, &file).await?;
        first.get_or_insert(media_id);
    }
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    if let Some(media_id) = first {
        app.db
            .run(move |c| {
                let ver: i64 = c.query_row("SELECT thumb_ver FROM media WHERE id = ?", [media_id], |r| r.get(0))?;
                let thumb = media::thumb_url(media_id, ver).unwrap_or_default();
                c.execute(
                    "UPDATE import_jobs SET media_id = ?,
                        thumb_url = CASE WHEN ? = '' THEN thumb_url ELSE ? END,
                        bytes_done = bytes_total
                     WHERE id = ?",
                    params![media_id, thumb, thumb, id],
                )?;
                Ok(())
            })
            .await?;
    }
    app.events.send(Event::Library);
    Ok(true)
}

/// Where a job's file goes: the card's own choice, read now rather than when
/// it was queued, since the card can be refiled while it downloads.
async fn destination(app: &App, id: i64) -> Result<Place> {
    app.db
        .run(move |c| {
            let (root, folder): (Option<i64>, String) =
                c.query_row("SELECT root_id, folder FROM import_jobs WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if let Some(root_id) = root {
                return Ok(Place { root_id, folder });
            }
            default_place(c)?.context("add a storage folder in Settings before downloading")
        })
        .await
}

/// Moves a finished download into the library and registers it.
async fn land(app: &App, job_id: i64, file: &Path) -> Result<i64> {
    let place = destination(app, job_id).await?;
    let (root, folder) = ops::prepare_place(app, &place).await?;
    let name = file.file_name().and_then(|n| n.to_str()).context("download has no file name")?.to_string();
    let target = paths::unique_path(&paths::folder_dir(&root, &folder), &name);
    {
        let (from, to) = (file.to_path_buf(), target.clone());
        tokio::task::spawn_blocking(move || paths::move_file(&from, &to, |_| {})).await??;
    }

    // yt-dlp writes the metadata next to the file as <stem>.info.json.
    let info_path = file.with_extension("info.json");
    let info: serde_json::Value = tokio::fs::read(&info_path)
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let str_field = |k: &str| info.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let filename = target.file_name().and_then(|n| n.to_str()).unwrap_or(&name).to_string();
    let title = Some(str_field("title")).filter(|t| !t.is_empty()).unwrap_or_else(|| media::title_from_filename(&filename));
    let description = str_field("description");
    let job_url: String = app.db.run(move |c| Ok(c.query_row("SELECT url FROM import_jobs WHERE id = ?", [job_id], |r| r.get(0))?)).await?;
    let source = Some(str_field("webpage_url")).filter(|u| !u.is_empty()).unwrap_or(job_url);
    let size = tokio::fs::metadata(&target).await?.len() as i64;
    let kind = paths::media_kind(&target).unwrap_or("video").to_string();

    let media_id = app
        .db
        .run(move |c| {
            let tx = c.transaction()?;
            let tags: String = tx.query_row("SELECT tags FROM import_jobs WHERE id = ?", [job_id], |r| r.get(0))?;
            let tags: Vec<String> = serde_json::from_str(&tags).unwrap_or_default();
            let now = db::now();
            let id = media::insert(
                &tx,
                &media::NewMedia {
                    root_id: place.root_id,
                    folder: &folder,
                    filename: &filename,
                    kind: &kind,
                    title: &title,
                    description: &description,
                    source_url: &source,
                    size,
                    file_mtime: now,
                    added_at: now,
                },
            )?;
            media::set_tags(&tx, id, &media::normalize_tags(&tags))?;
            tx.commit()?;
            Ok(id)
        })
        .await?;

    if info_path.exists() {
        let meta = app.cfg.meta_dir();
        tokio::fs::create_dir_all(&meta).await?;
        let _ = tokio::fs::rename(&info_path, meta.join(format!("{media_id}.info.json"))).await;
    }
    // Processed here rather than left to the worker, so the card can show the
    // real thumbnail the moment it reports done.
    if let Err(e) = process::process_media(app, media_id).await {
        tracing::warn!("processing download {media_id}: {e:#}");
    }
    Ok(media_id)
}

