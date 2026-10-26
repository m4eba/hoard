//! Storage roots and the folders inside them.

use std::{collections::HashMap, path::PathBuf};

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    db::{self, params},
    error::{ApiResult, bad_request, conflict, not_found},
    events::Event,
    media, ops, paths, process, scan,
    state::App,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Root {
    id: i64,
    name: String,
    path: String,
    online: bool,
    scanning: bool,
    count: i64,
    bytes: i64,
    last_scan_at: Option<String>,
}

pub async fn roots(State(app): State<App>) -> ApiResult<Json<Value>> {
    let rows: Vec<(i64, String, String, Option<i64>, i64, i64)> = app
        .db
        .run(|c| {
            let mut stmt = c.prepare(
                "SELECT r.id, r.name, r.path, r.last_scan_at, count(m.id), coalesce(sum(m.size), 0)
                 FROM roots r LEFT JOIN media m ON m.root_id = r.id
                 GROUP BY r.id ORDER BY r.name COLLATE NOCASE",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
                .collect::<Result<_, _>>()?;
            Ok(rows)
        })
        .await?;
    let scanning = app.scanning.lock().unwrap().clone();
    let roots: Vec<Root> = rows
        .into_iter()
        .map(|(id, name, path, last, count, bytes)| Root {
            online: std::path::Path::new(&path).is_dir(),
            scanning: scanning.contains(&id),
            last_scan_at: last.map(media::iso),
            id,
            name,
            path,
            count,
            bytes,
        })
        .collect();
    Ok(Json(json!({ "roots": roots })))
}

fn check_root_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name.chars().count() > 80 {
        return Err(bad_request("a storage folder name must be 1–80 characters without '/'").into());
    }
    Ok(name.to_string())
}

#[derive(Deserialize)]
pub struct AddRoot {
    path: String,
    name: Option<String>,
}

/// Registers a directory as storage and scans it.
///
/// Roots may not overlap one another or the data directory: a file inside
/// two roots would be imported twice, and a root containing the data
/// directory would import every thumbnail as an image.
pub async fn add_root(State(app): State<App>, Json(body): Json<AddRoot>) -> ApiResult<Json<Value>> {
    let id = add_root_path(&app, PathBuf::from(body.path.trim()), body.name).await?;
    Ok(Json(json!({ "id": id })))
}

pub async fn add_root_path(app: &App, path: PathBuf, name: Option<String>) -> ApiResult<i64> {
    if !path.is_absolute() {
        return Err(bad_request("use an absolute path").into());
    }
    let path = path.canonicalize().map_err(|e| bad_request(format!("{}: {e}", path.display())))?;
    if !path.is_dir() {
        return Err(bad_request(format!("{} is not a directory", path.display())).into());
    }
    std::fs::create_dir_all(&app.cfg.data_dir)?;
    let data = app.cfg.data_dir.canonicalize()?;
    if data.starts_with(&path) || path.starts_with(&data) {
        return Err(bad_request("a storage folder cannot contain, or be inside, the data directory").into());
    }
    let name = check_root_name(&name.unwrap_or_else(|| {
        path.file_name().and_then(|n| n.to_str()).unwrap_or("Storage").to_string()
    }))?;
    let path_str = path.to_str().ok_or_else(|| bad_request("path is not valid UTF-8"))?.to_string();

    let id = app
        .db
        .run(move |c| {
            let existing: Vec<(String, String)> = {
                let mut stmt = c.prepare("SELECT name, path FROM roots")?;
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
            };
            for (other_name, other) in &existing {
                let other = PathBuf::from(other);
                if other.starts_with(&path) || path.starts_with(&other) {
                    return Err(conflict(format!("overlaps the storage folder {}", other.display())).into());
                }
                if other_name.eq_ignore_ascii_case(&name) {
                    return Err(conflict(format!("a storage folder is already called {name}")).into());
                }
            }
            c.execute(
                "INSERT INTO roots (name, path, created_at) VALUES (?, ?, ?)",
                params![name, path_str, db::now()],
            )?;
            let id = c.last_insert_rowid();
            media::ensure_folder(c, id, "")?;
            Ok(id)
        })
        .await?;
    app.events.send(Event::Library);
    let a = app.clone();
    tokio::spawn(async move {
        if let Err(e) = scan::scan_root(&a, id).await {
            tracing::warn!("initial scan: {e:#}");
        }
    });
    Ok(id)
}

#[derive(Deserialize)]
pub struct RenameRoot {
    name: String,
}

pub async fn rename_root(State(app): State<App>, Path(id): Path<i64>, Json(body): Json<RenameRoot>) -> ApiResult<Json<Value>> {
    let name = check_root_name(&body.name)?;
    app.db
        .run(move |c| {
            let taken: bool = c.query_row(
                "SELECT EXISTS (SELECT 1 FROM roots WHERE name = ? COLLATE NOCASE AND id != ?)",
                params![name, id],
                |r| r.get(0),
            )?;
            if taken {
                return Err(conflict(format!("a storage folder is already called {name}")).into());
            }
            if c.execute("UPDATE roots SET name = ? WHERE id = ?", params![name, id])? == 0 {
                return Err(not_found("storage folder").into());
            }
            Ok(())
        })
        .await?;
    app.events.send(Event::Library);
    Ok(Json(json!({ "ok": true })))
}

/// Forgets a root. Its files are not touched; only the library's records of
/// them, and their thumbnails, go.
pub async fn remove_root(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let ids: Vec<i64> = app
        .db
        .run(move |c| {
            let mut stmt = c.prepare("SELECT id FROM media WHERE root_id = ?")?;
            let ids = stmt.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
            Ok(ids)
        })
        .await?;
    let removed = app.db.run(move |c| Ok(c.execute("DELETE FROM roots WHERE id = ?", [id])?)).await?;
    if removed == 0 {
        return Err(not_found("storage folder").into());
    }
    for media_id in ids {
        process::remove_artifacts(&app.cfg, media_id);
    }
    app.events.send(Event::Library);
    Ok(Json(json!({ "ok": true })))
}

pub async fn rescan(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let a = app.clone();
    tokio::spawn(async move {
        if let Err(e) = scan::scan_root(&a, id).await {
            tracing::warn!("scan: {e:#}");
        }
    });
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Folder {
    root_id: i64,
    path: String,
    /// Media directly in this folder.
    count: i64,
    /// Media in this folder and everything below it.
    total: i64,
    /// A recent thumbnail from inside, for the folder's tile.
    thumb_url: Option<String>,
}

pub async fn folders(State(app): State<App>) -> ApiResult<Json<Value>> {
    let (dirs, counts, previews) = app
        .db
        .run(|c| {
            let mut stmt = c.prepare("SELECT root_id, path FROM folders ORDER BY root_id, path")?;
            let dirs: Vec<(i64, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            let mut stmt = c.prepare("SELECT root_id, folder, count(*) FROM media GROUP BY root_id, folder")?;
            let counts: HashMap<(i64, String), i64> = stmt
                .query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            // SQLite returns the other columns from the row holding max(id).
            let mut stmt = c.prepare(
                "SELECT root_id, folder, max(id), thumb_ver FROM media WHERE thumb_ver > 0 GROUP BY root_id, folder",
            )?;
            let previews: HashMap<(i64, String), String> = stmt
                .query_map([], |r| {
                    let (id, ver): (i64, i64) = (r.get(2)?, r.get(3)?);
                    Ok(((r.get(0)?, r.get(1)?), media::thumb_url(id, ver).unwrap_or_default()))
                })?
                .collect::<Result<_, _>>()?;
            Ok((dirs, counts, previews))
        })
        .await?;

    let mut folders: Vec<Folder> = dirs
        .into_iter()
        .map(|(root_id, path)| Folder {
            count: counts.get(&(root_id, path.clone())).copied().unwrap_or(0),
            total: 0,
            thumb_url: previews.get(&(root_id, path.clone())).cloned(),
            root_id,
            path,
        })
        .collect();
    // Every folder's count is added to itself and each of its ancestors.
    let index: HashMap<(i64, String), usize> =
        folders.iter().enumerate().map(|(i, f)| ((f.root_id, f.path.clone()), i)).collect();
    for i in 0..folders.len() {
        let (root, mut path, count, thumb) =
            (folders[i].root_id, folders[i].path.clone(), folders[i].count, folders[i].thumb_url.clone());
        loop {
            if let Some(&j) = index.get(&(root, path.clone())) {
                folders[j].total += count;
                if folders[j].thumb_url.is_none() {
                    folders[j].thumb_url = thumb.clone();
                }
            }
            if path.is_empty() {
                break;
            }
            path = path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        }
    }
    Ok(Json(json!({ "folders": folders })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolder {
    root_id: i64,
    path: String,
}

pub async fn create_folder(State(app): State<App>, Json(body): Json<CreateFolder>) -> ApiResult<Json<Value>> {
    let folder = paths::clean_folder(&body.path)?;
    if folder.is_empty() {
        return Err(bad_request("enter a folder name").into());
    }
    let (_, folder) = ops::prepare_place(&app, &ops::Place { root_id: body.root_id, folder }).await?;
    app.events.send(Event::Library);
    Ok(Json(json!({ "rootId": body.root_id, "path": folder })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFolder {
    root_id: i64,
    path: String,
    /// The folder it goes into.
    to: ops::Place,
}

/// Moves a folder into another one, in the background; answers with the
/// task and the folder's new place.
pub async fn move_folder(State(app): State<App>, Json(body): Json<MoveFolder>) -> ApiResult<Json<Value>> {
    let (task, place) = ops::start_folder_move(&app, body.root_id, body.path, body.to).await?;
    Ok(Json(json!({ "taskId": task, "place": place })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFolder {
    root_id: i64,
    path: String,
    #[serde(default)]
    confirm: String,
}

/// Deletes a folder and the media in it. A non-empty folder needs its name
/// echoed back — checked here as well as in the dialog.
pub async fn delete_folder(State(app): State<App>, Query(p): Query<DeleteFolder>) -> ApiResult<Json<Value>> {
    let folder = paths::clean_folder(&p.path)?;
    let filter = crate::search::Filter {
        root_id: Some(p.root_id),
        folder: Some(folder.clone()),
        recursive: true,
        ..Default::default()
    };
    let count = app.db.run(move |c| media::count(c, &filter)).await?;
    let name = folder.rsplit('/').next().unwrap_or_default().to_string();
    if count > 0 && p.confirm != name {
        return Err(bad_request(format!("type the folder name ({name}) to confirm")).into());
    }
    let deleted = ops::delete_folder(&app, p.root_id, folder).await?;
    Ok(Json(json!({ "ok": true, "deletedMedia": deleted })))
}

#[derive(Deserialize)]
pub struct Browse {
    path: Option<String>,
}

/// Lists directories on the server, for picking a storage folder without
/// typing an absolute path from memory.
pub async fn browse(Query(p): Query<Browse>) -> ApiResult<Json<Value>> {
    let start = p
        .path
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let dir = start.canonicalize().map_err(|e| bad_request(format!("{}: {e}", start.display())))?;
    let mut dirs: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| bad_request(format!("{}: {e}", dir.display())))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| !paths::is_hidden(n))
        .collect();
    dirs.sort_by_key(|d| d.to_lowercase());
    Ok(Json(json!({
        "path": dir.display().to_string(),
        "parent": dir.parent().map(|p| p.display().to_string()),
        "dirs": dirs,
    })))
}

pub async fn stats(State(app): State<App>) -> ApiResult<Json<Value>> {
    let (count, bytes, videos, images): (i64, i64, i64, i64) = app
        .db
        .run(|c| {
            Ok(c.query_row(
                "SELECT count(*), coalesce(sum(size), 0), coalesce(sum(kind = 'video'), 0), coalesce(sum(kind = 'image'), 0) FROM media",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .await?;
    Ok(Json(json!({ "count": count, "bytes": bytes, "videos": videos, "images": images })))
}
