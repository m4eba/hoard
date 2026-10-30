//! Media endpoints: listing and search, details, tag edits, bulk operations,
//! uploads, storyboards, and serving the bytes.

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::{
    db::{self, params},
    error::{ApiResult, bad_request, conflict, not_found},
    events::Event,
    imports, media,
    ops::{self, Place},
    paths, process,
    search::{self, Filter},
    state::App,
    storyboards,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListParams {
    q: Option<String>,
    root_id: Option<i64>,
    folder: Option<String>,
    recursive: Option<bool>,
    kind: Option<String>,
    sort: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

pub async fn list(State(app): State<App>, Query(p): Query<ListParams>) -> ApiResult<Json<Value>> {
    let filter = Filter {
        q: p.q.filter(|q| !q.trim().is_empty()),
        root_id: p.root_id,
        folder: p.folder,
        recursive: p.recursive.unwrap_or(false),
        kind: p.kind,
    };
    let (offset, limit) = (p.offset.unwrap_or(0), p.limit.unwrap_or(120).clamp(1, 500));
    let sort = p.sort;
    let (items, next) = app.db.run(move |c| media::list(c, &filter, sort.as_deref(), offset, limit)).await?;
    Ok(Json(json!({ "media": items, "nextOffset": next })))
}

async fn detail(app: &App, id: i64) -> ApiResult<Value> {
    let d = app.db.run(move |c| media::get(c, id)).await?.ok_or_else(|| not_found("media"))?;
    let mut v = serde_json::to_value(d)?;
    v["storyboard"] = storyboard_state(app, id).await?;
    Ok(v)
}

pub async fn get(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    Ok(Json(detail(&app, id).await?))
}

#[derive(Deserialize)]
pub struct Update {
    title: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
}

pub async fn update(State(app): State<App>, Path(id): Path<i64>, Json(body): Json<Update>) -> ApiResult<Json<Value>> {
    app.db
        .run(move |c| {
            let tx = c.transaction()?;
            if tx.query_row("SELECT count(*) FROM media WHERE id = ?", [id], |r| r.get::<_, i64>(0))? == 0 {
                return Err(not_found("media").into());
            }
            if let Some(title) = body.title {
                tx.execute("UPDATE media SET title = ? WHERE id = ?", params![title.trim(), id])?;
            }
            if let Some(description) = body.description {
                tx.execute("UPDATE media SET description = ? WHERE id = ?", params![description, id])?;
            }
            if let Some(tags) = body.tags {
                media::set_tags(&tx, id, &media::normalize_tags(&tags))?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(Json(detail(&app, id).await?))
}

pub async fn delete(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let n = ops::delete_media(&app, vec![id]).await?;
    if n == 0 {
        return Err(not_found("media").into());
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct Select {
    select: media::Selection,
}

pub async fn count(State(app): State<App>, Json(body): Json<Select>) -> ApiResult<Json<Value>> {
    let n = app.db.run(move |c| Ok(media::resolve(c, &body.select)?.len())).await?;
    Ok(Json(json!({ "count": n })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bulk {
    select: Option<media::Selection>,
    add_tags: Option<Vec<String>>,
    remove_tags: Option<Vec<String>>,
    set_tags: Option<Vec<String>>,
    /// Undo: each item's tags exactly as they were.
    restore: Option<Vec<Previous>>,
}

#[derive(Deserialize, serde::Serialize)]
pub struct Previous {
    id: i64,
    tags: Vec<String>,
}

/// Bulk tag edits. Every edit returns what it replaced, so the client can
/// offer undo by putting things back rather than replaying an inverse.
pub async fn bulk_update(State(app): State<App>, Json(body): Json<Bulk>) -> ApiResult<Json<Value>> {
    let result = app
        .db
        .run(move |c| {
            let tx = c.transaction()?;
            if let Some(restore) = body.restore {
                for p in &restore {
                    media::set_tags(&tx, p.id, &media::normalize_tags(&p.tags))?;
                }
                tx.commit()?;
                return Ok(json!({ "updated": restore.len(), "previous": [] }));
            }
            let select = body.select.ok_or_else(|| bad_request("nothing selected"))?;
            let ids = media::resolve(&tx, &select)?;
            let add = media::normalize_tags(&body.add_tags.unwrap_or_default());
            let remove = media::normalize_tags(&body.remove_tags.unwrap_or_default());
            let set = body.set_tags.map(|t| media::normalize_tags(&t));
            let mut previous = Vec::new();
            for &id in &ids {
                let before = media::tags_of(&tx, id)?;
                if let Some(set) = &set {
                    media::set_tags(&tx, id, set)?;
                }
                media::add_tags(&tx, id, &add)?;
                media::remove_tags(&tx, id, &remove)?;
                previous.push(Previous { id, tags: before });
            }
            tx.commit()?;
            Ok(json!({ "updated": ids.len(), "previous": previous }))
        })
        .await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct BulkDelete {
    select: media::Selection,
    confirm: usize,
}

/// The count has to be echoed back and is checked here too, so a stray
/// request cannot empty a library even if the dialog is bypassed.
pub async fn bulk_delete(State(app): State<App>, Json(body): Json<BulkDelete>) -> ApiResult<Json<Value>> {
    let ids = app.db.run(move |c| media::resolve(c, &body.select)).await?;
    if ids.len() != body.confirm {
        return Err(conflict(format!("the selection is now {} files, not {}; check and try again", ids.len(), body.confirm)).into());
    }
    let deleted = ops::delete_media(&app, ids).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkMove {
    select: media::Selection,
    #[serde(flatten)]
    place: Place,
}

pub async fn bulk_move(State(app): State<App>, Json(body): Json<BulkMove>) -> ApiResult<Json<Value>> {
    let ids = app.db.run(move |c| media::resolve(c, &body.select)).await?;
    if ids.is_empty() {
        return Err(bad_request("nothing selected").into());
    }
    let count = ids.len();
    let task = ops::start_move(&app, ids, body.place).await?;
    Ok(Json(json!({ "taskId": task, "count": count })))
}

#[derive(Deserialize)]
pub struct Suggest {
    q: String,
}

pub async fn suggest(State(app): State<App>, Query(p): Query<Suggest>) -> ApiResult<Json<Value>> {
    let tags = app.db.run(move |c| search::suggest(c, &p.q, 10)).await?;
    Ok(Json(json!({ "tags": tags })))
}

pub async fn tags(State(app): State<App>) -> ApiResult<Json<Value>> {
    let mut tags = app.db.run(|c| search::known_tags(c)).await?;
    tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let tags: Vec<Value> = tags.into_iter().map(|(tag, count)| json!({ "tag": tag, "count": count })).collect();
    Ok(Json(json!({ "tags": tags })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadParams {
    name: String,
    title: Option<String>,
    tags: Option<String>,
    root_id: Option<i64>,
    folder: Option<String>,
}

/// Stores one uploaded file in a folder. The body is the raw file, streamed
/// to disk under a hidden name and renamed into place once complete, so a
/// scan running meanwhile never imports half a file.
pub async fn upload(State(app): State<App>, Query(p): Query<UploadParams>, body: Body) -> ApiResult<Json<Value>> {
    let filename = paths::clean_filename(&p.name)?;
    let kind = paths::media_kind(std::path::Path::new(&filename))
        .ok_or_else(|| bad_request(format!("{filename}: not a supported image or video type")))?;
    let place = match p.root_id {
        Some(root_id) => Place { root_id, folder: p.folder.unwrap_or_default() },
        None => app
            .db
            .run(|c| imports::default_place(c))
            .await?
            .ok_or_else(|| bad_request("add a storage folder in Settings before uploading"))?,
    };
    let (root, folder) = ops::prepare_place(&app, &place).await?;
    let dir = paths::folder_dir(&root, &folder);
    let tmp = dir.join(format!(".{filename}.hoard-upload-{}", rand::random::<u32>()));

    let written = async {
        let mut file = paths::UploadFile::create(&tmp).await?;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.try_next().await? {
            file.write(&chunk).await?;
        }
        file.finish().await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e.into());
    }
    let target = paths::unique_path(&dir, &filename);
    tokio::fs::rename(&tmp, &target).await?;

    let final_name = target.file_name().and_then(|n| n.to_str()).unwrap_or(&filename).to_string();
    let size = tokio::fs::metadata(&target).await?.len() as i64;
    let title = p.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| media::title_from_filename(&final_name));
    let tags: Vec<String> = p.tags.unwrap_or_default().split(',').map(String::from).collect();
    let kind = kind.to_string();
    let id = app
        .db
        .run(move |c| {
            let tx = c.transaction()?;
            let now = db::now();
            let id = media::insert(
                &tx,
                &media::NewMedia {
                    root_id: place.root_id,
                    folder: &folder,
                    filename: &final_name,
                    kind: &kind,
                    title: &title,
                    description: "",
                    source_url: "",
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
    app.work.notify_one();
    app.events.send(Event::Library);
    Ok(Json(detail(&app, id).await?))
}

async fn storyboard_state(app: &App, id: i64) -> ApiResult<Value> {
    let (raw, queued): (Option<String>, bool) = app
        .db
        .run(move |c| {
            let raw = c.query_row("SELECT storyboard FROM media WHERE id = ?", [id], |r| r.get(0))?;
            Ok((raw, storyboards::is_queued(c, id)?))
        })
        .await?;
    let building = app.storyboards.lock().unwrap().contains(&id);
    // A sheet whose file has gone missing reads as none, so it can be rebuilt.
    let raw = raw.filter(|_| process::storyboard_path(&app.cfg, id).exists());
    Ok(storyboards::state_json(id, raw.as_deref(), queued || building))
}

pub async fn storyboard(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    Ok(Json(storyboard_state(&app, id).await?))
}

/// Queues one video's storyboard ahead of any bulk work; the viewer hears
/// it is ready over the event stream.
pub async fn make_storyboard(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let kind: Option<String> = app
        .db
        .run(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT kind FROM media WHERE id = ?", [id], |r| r.get(0)).optional()?)
        })
        .await?;
    match kind.as_deref() {
        None => return Err(not_found("media").into()),
        Some("video") => {}
        Some(_) => return Err(bad_request("storyboards are for videos").into()),
    }
    // Asking again for one that exists means "rebuild it".
    app.db.run(move |c| storyboards::enqueue(c, &[id], storyboards::PRIORITY_VIEWER, true)).await?;
    app.storyboard_wake.notify_one();
    Ok(Json(json!({ "state": "pending" })))
}

#[derive(Deserialize)]
pub struct QueueStoryboards {
    select: media::Selection,
    #[serde(default)]
    rebuild: bool,
}

/// Queues storyboards for every video in a selection — hand-picked, or a
/// whole folder through a filter. Images and videos that already have one
/// are skipped unless `rebuild` is set.
pub async fn queue_storyboards(State(app): State<App>, Json(body): Json<QueueStoryboards>) -> ApiResult<Json<Value>> {
    let (queued, selected) = app
        .db
        .run(move |c| {
            let ids = media::resolve(c, &body.select)?;
            let n = storyboards::enqueue(c, &ids, storyboards::PRIORITY_BULK, body.rebuild)?;
            Ok((n, ids.len()))
        })
        .await?;
    app.storyboard_wake.notify_one();
    Ok(Json(json!({ "queued": queued, "skipped": selected - queued })))
}

pub async fn clear_storyboards(State(app): State<App>) -> ApiResult<Json<Value>> {
    let cleared = app.db.run(|c| storyboards::clear(c)).await?;
    Ok(Json(json!({ "cleared": cleared })))
}

pub(super) async fn serve(path: std::path::PathBuf, req: Request, cache: &'static str) -> ApiResult<Response> {
    if !path.is_file() {
        return Err(not_found("file").into());
    }
    let mut res = ServeFile::new(path).oneshot(req).await?.into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    Ok(res)
}

/// The original file. ServeFile answers Range requests, which is what makes
/// seeking in a long video work.
pub async fn serve_file(State(app): State<App>, Path(id): Path<i64>, req: Request) -> ApiResult<Response> {
    let file = app.db.run(move |c| media::file(c, id)).await?.ok_or_else(|| not_found("media"))?;
    serve(file.path(), req, "private, no-cache").await
}

/// Thumbnail and storyboard URLs carry a version, so they never change
/// content and can be cached for good.
pub async fn serve_thumb(State(app): State<App>, Path(id): Path<i64>, req: Request) -> ApiResult<Response> {
    serve(process::thumb_path(&app.cfg, id), req, "private, max-age=31536000, immutable").await
}

pub async fn serve_storyboard(State(app): State<App>, Path(id): Path<i64>, req: Request) -> ApiResult<Response> {
    serve(process::storyboard_path(&app.cfg, id), req, "private, max-age=31536000, immutable").await
}
