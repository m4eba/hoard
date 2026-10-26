//! Folder scanning: bringing the database in line with what is on disk.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use rusqlite::OptionalExtension;

use crate::{
    db::{self, params},
    events::{Event, Task},
    media, paths, process,
    state::App,
};

struct Found {
    folder: String,
    filename: String,
    kind: &'static str,
    size: i64,
    mtime: i64,
}

#[derive(Default)]
struct Walk {
    files: Vec<Found>,
    dirs: Vec<String>,
}

fn walk(root: &Path, task: &Task) -> Result<Walk> {
    let mut out = Walk::default();
    let entries = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        e.depth() == 0 || !e.file_name().to_str().is_none_or(paths::is_hidden)
    });
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // One unreadable directory should not abort the whole scan.
                tracing::warn!("scan: {e}");
                continue;
            }
        };
        if entry.file_type().is_dir() {
            if let Some((parent, name)) = paths::split_relative(root, entry.path()) {
                out.dirs.push(if parent.is_empty() { name } else { format!("{parent}/{name}") });
            }
            continue;
        }
        let Some(kind) = paths::media_kind(entry.path()) else { continue };
        let Some((folder, filename)) = paths::split_relative(root, entry.path()) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.files.push(Found { folder, filename, kind, size: meta.len() as i64, mtime });
        if out.files.len() % 500 == 0 {
            task.progress(0, 0, format!("{} files found", out.files.len()));
        }
    }
    Ok(out)
}

#[derive(Default)]
struct Summary {
    added: usize,
    moved: usize,
    changed: usize,
    removed: Vec<i64>,
}

/// Applies a walk to the database in one transaction.
///
/// A file that vanished from one path and appeared at another with the same
/// size and modification time is treated as moved, not as one deletion and
/// one import — reorganising folders outside the app must not lose its tags.
fn apply(conn: &mut rusqlite::Connection, root_id: i64, first_scan: bool, walk: &Walk) -> Result<Summary> {
    let tx = conn.transaction()?;
    let mut existing: HashMap<(String, String), (i64, i64, i64)> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT id, folder, filename, size, file_mtime FROM media WHERE root_id = ?")?;
        let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        for row in rows {
            let (id, folder, filename, size, mtime): (i64, String, String, i64, i64) = row?;
            existing.insert((folder, filename), (id, size, mtime));
        }
    }

    let mut summary = Summary::default();
    let mut fresh = Vec::new();
    for f in &walk.files {
        match existing.remove(&(f.folder.clone(), f.filename.clone())) {
            Some((_, size, mtime)) if size == f.size && mtime == f.mtime => {}
            Some((id, _, _)) => {
                tx.execute(
                    "UPDATE media SET size = ?, file_mtime = ?, processed = 0 WHERE id = ?",
                    params![f.size, f.mtime, id],
                )?;
                summary.changed += 1;
            }
            None => fresh.push(f),
        }
    }

    // What is left in `existing` is gone from where it was.
    let mut missing: HashMap<(i64, i64), Vec<i64>> = HashMap::new();
    for (id, size, mtime) in existing.into_values() {
        missing.entry((size, mtime)).or_default().push(id);
    }

    let now = db::now();
    for f in fresh {
        if let Some(id) = missing.get_mut(&(f.size, f.mtime)).and_then(|ids| ids.pop()) {
            tx.execute(
                "UPDATE media SET folder = ?, filename = ? WHERE id = ?",
                params![f.folder, f.filename, id],
            )?;
            summary.moved += 1;
            continue;
        }
        // On a folder's first scan the file's own date is the best guess at
        // when it was added; otherwise "newest" would put the whole existing
        // collection at the same instant.
        let added_at = if first_scan && f.mtime > 0 { f.mtime.min(now) } else { now };
        media::insert(
            &tx,
            &media::NewMedia {
                root_id,
                folder: &f.folder,
                filename: &f.filename,
                kind: f.kind,
                title: &media::title_from_filename(&f.filename),
                description: "",
                source_url: "",
                size: f.size,
                file_mtime: f.mtime,
                added_at,
            },
        )?;
        summary.added += 1;
    }

    for id in missing.into_values().flatten() {
        tx.execute("DELETE FROM media WHERE id = ?", [id])?;
        summary.removed.push(id);
    }

    tx.execute("DELETE FROM folders WHERE root_id = ?", [root_id])?;
    tx.execute("INSERT INTO folders (root_id, path) VALUES (?, '')", [root_id])?;
    for dir in &walk.dirs {
        tx.execute("INSERT OR IGNORE INTO folders (root_id, path) VALUES (?, ?)", params![root_id, dir])?;
    }
    tx.execute("UPDATE roots SET last_scan_at = ? WHERE id = ?", params![now, root_id])?;
    tx.commit()?;
    Ok(summary)
}

/// Scans one storage folder. A second request for a root already being
/// scanned is a no-op rather than a second walk racing the first.
pub async fn scan_root(app: &App, root_id: i64) -> Result<()> {
    if !app.scanning.lock().unwrap().insert(root_id) {
        return Ok(());
    }
    let result = scan_inner(app, root_id).await;
    app.scanning.lock().unwrap().remove(&root_id);
    app.events.send(Event::Library);
    result
}

async fn scan_inner(app: &App, root_id: i64) -> Result<()> {
    let root: Option<(String, String, Option<i64>, i64)> = app
        .db
        .run(move |c| {
            Ok(c.query_row(
                "SELECT name, path, last_scan_at, (SELECT count(*) FROM media WHERE root_id = roots.id)
                 FROM roots WHERE id = ?",
                [root_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?)
        })
        .await?;
    let Some((name, path, last_scan, known)) = root else { bail!("storage folder {root_id} not found") };
    let task = Arc::new(app.tasks.start("scan", format!("Scanning {name}"), 0));
    let root_path = PathBuf::from(&path);

    // An unmounted disk looks exactly like a folder whose files were all
    // deleted. Refusing to scan it is what keeps a missing drive from wiping
    // every tag in the library.
    if !root_path.is_dir() {
        fail(task, "folder is not reachable");
        bail!("{path} is not reachable");
    }

    let walked = {
        let (root_path, task) = (root_path.clone(), task.clone());
        tokio::task::spawn_blocking(move || walk(&root_path, &task)).await??
    };
    if walked.files.is_empty() && known > 0 {
        fail(task, "folder is empty — not removing anything");
        bail!("{path} looks empty or unmounted; nothing was removed");
    }

    let found = walked.files.len();
    let summary = app.db.run(move |c| apply(c, root_id, last_scan.is_none(), &walked)).await?;
    for id in &summary.removed {
        process::remove_artifacts(&app.cfg, *id);
    }
    app.work.notify_one();

    let detail = format!(
        "{found} files: {} new, {} moved, {} changed, {} gone",
        summary.added,
        summary.moved,
        summary.changed,
        summary.removed.len()
    );
    tracing::info!("scanned {name}: {detail}");
    if let Ok(task) = Arc::try_unwrap(task) {
        task.finish(detail);
    }
    Ok(())
}

fn fail(task: Arc<Task>, msg: &str) {
    if let Ok(task) = Arc::try_unwrap(task) {
        task.fail(msg);
    }
}

pub async fn scan_all(app: &App) {
    let ids: Vec<i64> = app
        .db
        .run(|c| {
            let mut stmt = c.prepare("SELECT id FROM roots ORDER BY id")?;
            let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            Ok(ids)
        })
        .await
        .unwrap_or_default();
    for id in ids {
        if let Err(e) = scan_root(app, id).await {
            tracing::warn!("scan of root {id}: {e:#}");
        }
    }
}

/// Rescans every root on an interval and expires old trash once a day.
pub async fn scheduler(app: App) {
    let mut last_purge = None::<std::time::Instant>;
    loop {
        if last_purge.is_none_or(|t| t.elapsed() > Duration::from_secs(86_400)) {
            last_purge = Some(std::time::Instant::now());
            if let Err(e) = purge_trash(&app, app.cfg.trash_days).await {
                tracing::warn!("trash purge: {e:#}");
            }
        }
        scan_all(&app).await;
        match app.cfg.scan_interval {
            Some(every) => tokio::time::sleep(every).await,
            None => tokio::time::sleep(Duration::from_secs(86_400)).await,
        }
    }
}

pub async fn purge_trash(app: &App, days: u64) -> Result<u64> {
    let roots: Vec<String> = app
        .db
        .run(|c| {
            let mut stmt = c.prepare("SELECT path FROM roots")?;
            let paths = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            Ok(paths)
        })
        .await?;
    tokio::task::spawn_blocking(move || {
        let mut freed = 0;
        for root in roots {
            freed += paths::purge_trash(Path::new(&root), days).with_context(|| format!("purging trash in {root}"))?;
        }
        Ok(freed)
    })
    .await?
}
