//! fiffly's folder picker: where in the library an export is saved. The
//! save dialog asks for one folder at a time — the default first, then
//! whichever the user moves to — or searches by path, so a large library is
//! never sent whole. A folder's id is `root_id:path`.

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    db,
    error::{ApiResult, bad_request, conflict, not_found},
    ops::Place,
    publish,
    state::App,
};

/// Search results per request.
const MAX_RESULTS: usize = 20;

fn folder_id(root_id: i64, folder: &str) -> String {
    format!("{root_id}:{folder}")
}

fn parse_folder_id(id: &str) -> Option<Place> {
    let (root, folder) = id.split_once(':')?;
    Some(Place { root_id: root.parse().ok()?, folder: folder.to_string() })
}

/// A folder's path as the user reads it: the storage folder's name, then
/// each folder down to it.
fn segments(root: &str, folder: &str) -> Vec<String> {
    std::iter::once(root).chain(folder.split('/').filter(|p| !p.is_empty())).map(String::from).collect()
}

/// Where exports land unless one is picked: the folder chosen in Settings,
/// else where downloads go.
fn destination(c: &Connection) -> anyhow::Result<Option<Place>> {
    let picked = db::get_setting(c, "fiffly_place")?.and_then(|v| serde_json::from_str::<Place>(&v).ok());
    match picked {
        Some(p) if c.query_row("SELECT EXISTS (SELECT 1 FROM roots WHERE id = ?)", [p.root_id], |r| r.get(0))? => {
            Ok(Some(p))
        }
        _ => crate::imports::default_place(c),
    }
}

fn no_storage() -> crate::error::Reject {
    bad_request("hoard has no storage folder yet; add one in its Settings")
}

/// One folder with its parent and the folders directly inside it.
fn view(c: &Connection, place: &Place) -> anyhow::Result<Option<Value>> {
    let root: Option<String> = c
        .query_row(
            "SELECT r.name FROM folders f JOIN roots r ON r.id = f.root_id WHERE f.root_id = ? AND f.path = ?",
            params![place.root_id, place.folder],
            |r| r.get(0),
        )
        .optional()?;
    let Some(root) = root else { return Ok(None) };
    let prefix = if place.folder.is_empty() { String::new() } else { format!("{}/", place.folder) };
    let mut stmt = c.prepare(
        // `path/` < anything below it < `path0` ('0' follows '/'): a range on
        // the primary key, not a scan per child
        "SELECT f.path, EXISTS (SELECT 1 FROM folders c WHERE c.root_id = f.root_id
                                AND c.path > f.path || '/' AND c.path < f.path || '0')
         FROM folders f
         WHERE f.root_id = ?1 AND f.path <> '' AND substr(f.path, 1, length(?2)) = ?2
           AND instr(substr(f.path, length(?2) + 1), '/') = 0
         ORDER BY f.path COLLATE NOCASE",
    )?;
    let children: Vec<Value> = stmt
        .query_map(params![place.root_id, prefix], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?
        .map(|row| {
            let (path, has_children) = row?;
            let name = path.rsplit('/').next().unwrap_or_default().to_string();
            Ok(json!({ "id": folder_id(place.root_id, &path), "name": name, "hasChildren": has_children }))
        })
        .collect::<rusqlite::Result<_>>()?;
    let parent = (!place.folder.is_empty())
        .then(|| folder_id(place.root_id, place.folder.rsplit_once('/').map(|(p, _)| p).unwrap_or("")));
    Ok(Some(json!({
        "id": folder_id(place.root_id, &place.folder),
        "path": segments(&root, &place.folder),
        "parent": parent,
        "children": children,
    })))
}

/// Folders whose path contains `q`, any case: paths starting with it first
/// (so `library/Trips/` lists what is inside Trips), then those whose own
/// name does, each group shallowest first.
fn search(c: &Connection, q: &str) -> anyhow::Result<Vec<Value>> {
    let q = q.to_lowercase();
    let mut stmt = c.prepare("SELECT f.root_id, r.name, f.path FROM folders f JOIN roots r ON r.id = f.root_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut hits = Vec::new();
    for row in rows {
        let (root_id, root, path) = row?;
        let segs = segments(&root, &path);
        let full = segs.join("/").to_lowercase();
        if !full.contains(&q) {
            continue;
        }
        let name = segs.last().map(|s| s.to_lowercase()).unwrap_or_default();
        let rank = if full.starts_with(&q) { 0 } else if name.contains(&q) { 1 } else { 2 };
        hits.push(((rank, segs.len(), full), json!({ "id": folder_id(root_id, &path), "path": segs })));
    }
    hits.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(hits.into_iter().take(MAX_RESULTS).map(|(_, v)| v).collect())
}

#[derive(Deserialize)]
pub struct Params {
    id: Option<String>,
    q: Option<String>,
}

/// `?q=` searches; otherwise the folder `?id=`, or the default one.
pub async fn folders(State(app): State<App>, headers: HeaderMap, Query(p): Query<Params>) -> ApiResult<Json<Value>> {
    publish::bearer(&app, &headers, publish::SCOPE_WRITE).await?;
    if let Some(q) = p.q.map(|q| q.trim().to_string()).filter(|q| !q.is_empty()) {
        let results = app.db.run(move |c| search(c, &q)).await?;
        return Ok(Json(json!({ "results": results })));
    }
    if let Some(id) = p.id.filter(|id| !id.is_empty()) {
        let place = parse_folder_id(&id).ok_or_else(|| not_found("folder"))?;
        return Ok(Json(app.db.run(move |c| view(c, &place)).await?.ok_or_else(|| not_found("folder"))?));
    }
    // The default may have been moved or deleted since it was set; then the
    // top of its storage folder, so the picker always has somewhere to start.
    let found = app
        .db
        .run(|c| {
            let Some(place) = destination(c)? else { return Ok(None) };
            let top = Place { root_id: place.root_id, folder: String::new() };
            Ok(match view(c, &place)? {
                Some(v) => Some(v),
                None => view(c, &top)?,
            })
        })
        .await?;
    Ok(Json(found.ok_or_else(no_storage)?))
}

/// Holds a storage folder for an upload while it writes there, so no folder
/// move starts underneath it; let go when dropped.
pub struct Claim {
    app: App,
    root_id: i64,
}

impl Claim {
    pub fn new(app: &App, root_id: i64) -> Result<Self, crate::error::Reject> {
        let busy = app.folder_moves.lock().unwrap();
        if busy.contains(&root_id) {
            return Err(conflict("that storage folder is being reorganised; try again in a moment"));
        }
        *app.uploads.lock().unwrap().entry(root_id).or_default() += 1;
        Ok(Self { app: app.clone(), root_id })
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut uploads = self.app.uploads.lock().unwrap();
        if let Some(n) = uploads.get_mut(&self.root_id) {
            *n -= 1;
            if *n == 0 {
                uploads.remove(&self.root_id);
            }
        }
    }
}

/// Where an upload goes: the folder picked in fiffly, which must still be in
/// the library, else the default.
pub async fn target(app: &App, picked: Option<&str>) -> ApiResult<Place> {
    let Some(id) = picked.filter(|id| !id.is_empty()) else {
        return Ok(app.db.run(|c| destination(c)).await?.ok_or_else(no_storage)?);
    };
    let place = parse_folder_id(id).ok_or_else(|| bad_request(format!("{id}: not a folder")))?;
    let p = place.clone();
    let known: bool = app
        .db
        .run(move |c| {
            Ok(c.query_row(
                "SELECT EXISTS (SELECT 1 FROM folders WHERE root_id = ? AND path = ?)",
                params![p.root_id, p.folder],
                |r| r.get(0),
            )?)
        })
        .await?;
    if !known {
        return Err(bad_request("that folder is no longer in the library; pick another").into());
    }
    Ok(place)
}

#[cfg(test)]
mod tests {
    use super::{folder_id, parse_folder_id, segments};

    #[test]
    fn folder_ids_round_trip() {
        let p = parse_folder_id(&folder_id(3, "Trips/2024: summer")).unwrap();
        assert_eq!((p.root_id, p.folder.as_str()), (3, "Trips/2024: summer"));
        assert_eq!(parse_folder_id("3:").unwrap().folder, "");
        assert!(parse_folder_id("Trips").is_none());
        assert!(parse_folder_id("x:Trips").is_none());
    }

    #[test]
    fn paths_start_at_the_storage_folder() {
        assert_eq!(segments("library", ""), ["library"]);
        assert_eq!(segments("library", "Trips/2024"), ["library", "Trips", "2024"]);
    }
}
