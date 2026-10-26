//! Operations that change files on disk as well as rows: moving media and
//! folders between folders and storage roots, deleting to the trash, creating
//! and removing folders.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    db::params,
    error::{bad_request, conflict, not_found},
    events::Event,
    media, paths, process,
    state::App,
};

/// Where something goes: a folder within a storage root.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub root_id: i64,
    #[serde(default)]
    pub folder: String,
}

/// Checks the place exists as a root and returns its directory, creating the
/// folder on disk if needed.
pub async fn prepare_place(app: &App, place: &Place) -> Result<(PathBuf, String)> {
    let folder = paths::clean_folder(&place.folder)?;
    let root_id = place.root_id;
    let root: String = app
        .db
        .run(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT path FROM roots WHERE id = ?", [root_id], |r| r.get(0)).optional()?)
        })
        .await?
        .ok_or_else(|| not_found("storage folder"))?;
    let root = PathBuf::from(root);
    if !root.is_dir() {
        return Err(bad_request(format!("{} is not reachable", root.display())).into());
    }
    let dir = paths::folder_dir(&root, &folder);
    tokio::fs::create_dir_all(&dir).await.with_context(|| format!("creating {}", dir.display()))?;
    let f = folder.clone();
    app.db.run(move |c| media::ensure_folder(c, root_id, &f)).await?;
    Ok((root, folder))
}

/// Moves media into a folder, on disk as well as in the library, as a
/// background task: between disks this is a copy and can take a while.
pub async fn start_move(app: &App, ids: Vec<i64>, place: Place) -> Result<u64> {
    let (root_path, folder) = prepare_place(app, &place).await?;
    let root_name: String = {
        let id = place.root_id;
        app.db.run(move |c| Ok(c.query_row("SELECT name FROM roots WHERE id = ?", [id], |r| r.get(0))?)).await?
    };
    let label = if folder.is_empty() { root_name } else { format!("{root_name}/{folder}") };
    let task = app.tasks.start("move", format!("Moving {} to {label}", ids.len()), 0);
    let task_id = task.id();
    let app = app.clone();

    tokio::spawn(async move {
        let files: Vec<media::MediaFile> = {
            let ids = ids.clone();
            app.db
                .run(move |c| Ok(ids.iter().filter_map(|&id| media::file(c, id).ok().flatten()).collect()))
                .await
                .unwrap_or_default()
        };
        let files: Vec<_> = files
            .into_iter()
            .filter(|f| !(f.root_id == place.root_id && f.folder == folder))
            .filter(|f| app.moving.lock().unwrap().insert(f.id))
            .collect();
        let total: u64 = files.iter().filter_map(|f| fs::metadata(f.path()).ok()).map(|m| m.len()).sum();
        let (mut moved, mut failed, mut done_bytes) = (0usize, Vec::new(), 0u64);

        for f in &files {
            let src = f.path();
            let size = fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
            let result = {
                let (src, dir, task_ref) = (src.clone(), paths::folder_dir(&root_path, &folder), &task);
                let base = done_bytes;
                // Blocking file I/O on this task's own thread is acceptable: it
                // is a dedicated background task, not a request handler.
                tokio::task::block_in_place(|| move_one(&src, &dir, |n| task_ref.progress(base + n, total, "")))
            };
            done_bytes += size;
            match result {
                Ok(filename) => {
                    let (id, root_id, folder) = (f.id, place.root_id, folder.clone());
                    let update = app
                        .db
                        .run(move |c| {
                            c.execute(
                                "UPDATE media SET root_id = ?, folder = ?, filename = ? WHERE id = ?",
                                params![root_id, folder, filename, id],
                            )?;
                            media::ensure_folder(c, root_id, &folder)
                        })
                        .await;
                    match update {
                        Ok(()) => moved += 1,
                        Err(e) => failed.push(format!("{}: {e:#}", f.filename)),
                    }
                }
                Err(e) => failed.push(format!("{}: {e:#}", f.filename)),
            }
            app.moving.lock().unwrap().remove(&f.id);
        }

        app.events.send(Event::Library);
        if failed.is_empty() {
            task.finish(format!("{moved} moved"));
        } else {
            tracing::warn!("move failures: {failed:?}");
            task.fail(format!("{moved} moved, {} failed: {}", failed.len(), failed[0]));
        }
    });
    Ok(task_id)
}

/// Moves a folder, with everything in it, into another folder — of the same
/// storage folder or another one — as a background task. Returns the task
/// and where the folder ends up. Media keep their ids, so tags, thumbnails
/// and storyboards come along; scans of both storage folders wait.
pub async fn start_folder_move(app: &App, root_id: i64, folder: String, to: Place) -> Result<(u64, Place)> {
    let folder = paths::clean_folder(&folder)?;
    if folder.is_empty() {
        return Err(bad_request("a storage folder's top level cannot be moved").into());
    }
    let name = folder.rsplit('/').next().unwrap_or(&folder).to_string();
    let target_parent = paths::clean_folder(&to.folder)?;
    let dest = if target_parent.is_empty() { name.clone() } else { format!("{target_parent}/{name}") };
    if to.root_id == root_id {
        if dest == folder {
            return Err(bad_request("the folder is already there").into());
        }
        if target_parent == folder || target_parent.starts_with(&format!("{folder}/")) {
            return Err(bad_request("a folder cannot move into itself").into());
        }
    }
    let src_root: String = app
        .db
        .run(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT path FROM roots WHERE id = ?", [root_id], |r| r.get(0)).optional()?)
        })
        .await?
        .ok_or_else(|| not_found("storage folder"))?;
    let src = paths::folder_dir(Path::new(&src_root), &folder);
    if !src.is_dir() {
        return Err(not_found("folder").into());
    }
    let (dst_root, _) = prepare_place(app, &Place { root_id: to.root_id, folder: target_parent.clone() }).await?;
    let dst = paths::folder_dir(&dst_root, &dest);
    if dst.exists() {
        return Err(conflict(format!("there is already a folder named {name:?} there")).into());
    }

    let ids: Vec<i64> = {
        let (f, below) = (folder.clone(), paths::below_glob(&folder));
        app.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT id FROM media WHERE root_id = ? AND (folder = ? OR folder GLOB ?)",
                )?;
                let ids = stmt.query_map(params![root_id, f, below], |r| r.get(0))?.collect::<Result<_, _>>()?;
                Ok(ids)
            })
            .await?
    };
    // Claim both storage folders and every file in the folder, or nothing.
    {
        let scanning = app.scanning.lock().unwrap();
        let mut busy = app.folder_moves.lock().unwrap();
        let mut moving = app.moving.lock().unwrap();
        if [root_id, to.root_id].iter().any(|r| scanning.contains(r) || busy.contains(r)) {
            return Err(conflict("that storage folder is being scanned or reorganised; try again in a moment").into());
        }
        if ids.iter().any(|id| moving.contains(id)) {
            return Err(conflict("files in this folder are being moved right now").into());
        }
        busy.extend([root_id, to.root_id]);
        moving.extend(ids.iter().copied());
    }

    let task = app.tasks.start("move", format!("Moving folder {name}"), 0);
    let task_id = task.id();
    let app = app.clone();
    let place = Place { root_id: to.root_id, folder: dest.clone() };
    tokio::spawn(async move {
        let moved = tokio::task::block_in_place(|| paths::move_dir(&src, &dst, |done, total| task.progress(done, total, "")));
        let result = match moved {
            Err(e) => Err(e),
            Ok(()) => {
                let (old, new, to_root) = (folder.clone(), dest.clone(), to.root_id);
                app.db
                    .run(move |c| {
                        let tx = c.transaction()?;
                        let below = paths::below_glob(&old);
                        // Characters, as SQLite's substr counts them.
                        let rest = old.chars().count() as i64 + 1;
                        tx.execute(
                            "UPDATE media SET root_id = ?1, folder = ?2 || substr(folder, ?3)
                             WHERE root_id = ?4 AND (folder = ?5 OR folder GLOB ?6)",
                            params![to_root, new, rest, root_id, old, below],
                        )?;
                        let subfolders: Vec<String> = {
                            let mut stmt = tx.prepare(
                                "SELECT path FROM folders WHERE root_id = ? AND (path = ? OR path GLOB ?)",
                            )?;
                            stmt.query_map(params![root_id, old, below], |r| r.get(0))?.collect::<Result<_, _>>()?
                        };
                        tx.execute(
                            "DELETE FROM folders WHERE root_id = ? AND (path = ? OR path GLOB ?)",
                            params![root_id, old, below],
                        )?;
                        for path in subfolders {
                            media::ensure_folder(&tx, to_root, &format!("{new}{}", &path[old.len()..]))?;
                        }
                        tx.commit()?;
                        Ok(())
                    })
                    .await
                    // The files have moved; a rescan of both storage folders
                    // matches them up again by size and modification time.
                    .context("the files moved, but the library could not be updated; rescan both storage folders")
            }
        };
        {
            let mut moving = app.moving.lock().unwrap();
            for id in &ids {
                moving.remove(id);
            }
            let mut busy = app.folder_moves.lock().unwrap();
            busy.remove(&root_id);
            busy.remove(&to.root_id);
        }
        app.events.send(Event::Library);
        match result {
            Ok(()) => task.finish(format!("{} files moved", ids.len())),
            Err(e) => {
                tracing::warn!("moving folder {folder}: {e:#}");
                task.fail(format!("{e:#}"));
            }
        }
    });
    Ok((task_id, place))
}

/// Moves one file and its companions into `dir`, returning the name it ended
/// up with — a clash with an existing file gets a numbered name instead of
/// overwriting it.
fn move_one(src: &Path, dir: &Path, progress: impl FnMut(u64)) -> Result<String> {
    if !src.is_file() {
        anyhow::bail!("file is missing on disk");
    }
    let name = src.file_name().and_then(|n| n.to_str()).context("file name is not valid UTF-8")?;
    fs::create_dir_all(dir)?;
    let target = paths::unique_path(dir, name);
    let companions = paths::companions(src);
    paths::move_file(src, &target, progress)?;

    let old_stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    let new_stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    for c in companions {
        let cname = c.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        // Subtitles follow a renamed video under its new name, or players
        // stop finding them.
        let renamed = format!("{new_stem}{}", &cname[old_stem.len()..]);
        if let Err(e) = paths::move_file(&c, &paths::unique_path(dir, &renamed), |_| {}) {
            tracing::warn!("moving companion {}: {e:#}", c.display());
        }
    }
    Ok(target.file_name().and_then(|n| n.to_str()).unwrap_or(name).to_string())
}

/// Deletes media: files go to their root's trash (or are removed outright when
/// trash is disabled), rows and generated files are dropped.
pub async fn delete_media(app: &App, ids: Vec<i64>) -> Result<usize> {
    let files: Vec<media::MediaFile> = app
        .db
        .run(move |c| Ok(ids.iter().filter_map(|&id| media::file(c, id).ok().flatten()).collect()))
        .await?;
    let keep_trash = app.cfg.trash_days > 0;
    let removed = {
        let files = files.clone();
        tokio::task::spawn_blocking(move || {
            let mut ok = Vec::new();
            for f in files {
                let path = f.path();
                let result = if !path.exists() {
                    Ok(())
                } else if keep_trash {
                    let mut r = paths::trash_file(&f.root_path, &path);
                    for c in paths::companions(&path) {
                        r = r.and(paths::trash_file(&f.root_path, &c));
                    }
                    r
                } else {
                    fs::remove_file(&path).map_err(Into::into)
                };
                match result {
                    Ok(()) => ok.push(f.id),
                    Err(e) => tracing::warn!("deleting {}: {e:#}", path.display()),
                }
            }
            ok
        })
        .await?
    };
    let count = removed.len();
    for id in &removed {
        process::remove_artifacts(&app.cfg, *id);
    }
    app.db
        .run(move |c| {
            let tx = c.transaction()?;
            for id in removed {
                tx.execute("DELETE FROM media WHERE id = ?", [id])?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.events.send(Event::Library);
    Ok(count)
}

/// Deletes a folder: its media go to the trash, then whatever directories are
/// left empty are removed. Files the library does not manage are left alone,
/// and so is any directory still holding them.
pub async fn delete_folder(app: &App, root_id: i64, folder: String) -> Result<usize> {
    let folder = paths::clean_folder(&folder)?;
    if folder.is_empty() {
        return Err(bad_request("a storage folder's top level cannot be deleted; remove the storage folder instead").into());
    }
    let ids = {
        let filter = crate::search::Filter {
            root_id: Some(root_id),
            folder: Some(folder.clone()),
            recursive: true,
            ..Default::default()
        };
        app.db.run(move |c| media::resolve(c, &media::Selection { ids: None, query: Some(filter) })).await?
    };
    let deleted = delete_media(app, ids).await?;
    let root: String = app.db.run(move |c| Ok(c.query_row("SELECT path FROM roots WHERE id = ?", [root_id], |r| r.get(0))?)).await?;
    let dir = paths::folder_dir(Path::new(&root), &folder);
    tokio::task::spawn_blocking(move || remove_empty_dirs(&dir)).await?;

    let below = paths::below_glob(&folder);
    app.db
        .run(move |c| {
            c.execute(
                "DELETE FROM folders WHERE root_id = ? AND (path = ? OR path GLOB ?)",
                params![root_id, folder, below],
            )?;
            Ok(())
        })
        .await?;
    app.events.send(Event::Library);
    Ok(deleted)
}

fn remove_empty_dirs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.filter_map(|e| e.ok()) {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_dirs(&e.path());
        }
    }
    // Fails, harmlessly, when something is still inside.
    let _ = fs::remove_dir(dir);
}
