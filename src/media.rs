//! The media table: how rows read, how they are listed, and tag edits.

use std::path::PathBuf;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::search::{self, Filter};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub filename: String,
    pub root_id: i64,
    pub folder: String,
    pub width: i64,
    pub height: i64,
    pub duration: f64,
    pub size: i64,
    pub thumb_url: Option<String>,
    pub tags: Vec<String>,
    pub created_at: String,
    /// Videos only: whether a storyboard exists, is queued, or neither —
    /// what the grid's hover preview and its "none yet" icon go by.
    pub storyboard: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    #[serde(flatten)]
    pub item: ListItem,
    pub description: String,
    pub source_url: String,
    pub url: String,
    pub mime: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub root_name: String,
    pub file_modified_at: String,
    pub processed: i64,
}

/// What a bulk operation acts on: hand-picked ids, or everything a filter
/// matches — most of a large result set is not in the browser, so "all of
/// these" cannot be sent as a list.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub ids: Option<Vec<i64>>,
    pub query: Option<Filter>,
}

/// Where a media file lives on disk.
#[derive(Debug, Clone)]
pub struct MediaFile {
    pub id: i64,
    pub root_id: i64,
    pub root_path: PathBuf,
    pub folder: String,
    pub filename: String,
}

impl MediaFile {
    pub fn path(&self) -> PathBuf {
        crate::paths::folder_dir(&self.root_path, &self.folder).join(&self.filename)
    }
}

const COLUMNS: &str = "m.id, m.kind, m.title, m.filename, m.root_id, m.folder, m.width, m.height,
    m.duration, m.size, m.thumb_ver, m.added_at,
    (SELECT group_concat(tag, char(31)) FROM media_tags t WHERE t.media_id = m.id),
    m.storyboard, EXISTS (SELECT 1 FROM storyboard_queue q WHERE q.media_id = m.id)";

pub fn iso(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).map(|d| d.to_rfc3339()).unwrap_or_default()
}

pub fn thumb_url(id: i64, ver: i64) -> Option<String> {
    (ver > 0).then(|| format!("/t/{id}?v={ver}"))
}

fn list_item(r: &Row) -> rusqlite::Result<ListItem> {
    let id: i64 = r.get(0)?;
    let kind: String = r.get(1)?;
    let storyboard = (kind == "video").then(|| -> rusqlite::Result<_> {
        let raw: Option<String> = r.get(13)?;
        Ok(crate::storyboards::state_json(id, raw.as_deref(), r.get(14)?))
    });
    let storyboard = storyboard.transpose()?;
    let tags: Option<String> = r.get(12)?;
    let mut tags: Vec<String> = tags.map(|t| t.split('\u{1f}').map(String::from).collect()).unwrap_or_default();
    tags.sort();
    Ok(ListItem {
        id,
        kind,
        title: r.get(2)?,
        filename: r.get(3)?,
        root_id: r.get(4)?,
        folder: r.get(5)?,
        width: r.get(6)?,
        height: r.get(7)?,
        duration: r.get(8)?,
        size: r.get(9)?,
        thumb_url: thumb_url(id, r.get(10)?),
        tags,
        created_at: iso(r.get(11)?),
        storyboard,
    })
}

/// One page of a filtered, sorted listing. `next_offset` is zero when there is
/// nothing more.
pub fn list(conn: &Connection, filter: &Filter, sort: Option<&str>, offset: i64, limit: i64) -> Result<(Vec<ListItem>, i64)> {
    let c = search::compile(conn, filter)?;
    let sql = format!(
        "SELECT {COLUMNS} FROM media m {} {} ORDER BY {} LIMIT {} OFFSET {}",
        c.join,
        c.where_sql(),
        search::order_by(sort, c.rank.as_deref()),
        limit + 1,
        offset.max(0)
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut items: Vec<ListItem> = stmt.query_map(params_from_iter(c.params.iter()), list_item)?.collect::<Result<_, _>>()?;
    // One extra row was asked for, which answers "is there a next page"
    // without a second count query.
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    Ok((items, if more { offset + limit } else { 0 }))
}

pub fn count(conn: &Connection, filter: &Filter) -> Result<i64> {
    let c = search::compile(conn, filter)?;
    let sql = format!("SELECT count(*) FROM media m {} {}", c.join, c.where_sql());
    Ok(conn.query_row(&sql, params_from_iter(c.params.iter()), |r| r.get(0))?)
}

/// The ids a selection stands for.
pub fn resolve(conn: &Connection, sel: &Selection) -> Result<Vec<i64>> {
    if let Some(ids) = &sel.ids {
        let mut ids = ids.clone();
        ids.sort_unstable();
        ids.dedup();
        return Ok(ids);
    }
    let Some(filter) = &sel.query else { return Ok(Vec::new()) };
    let c = search::compile(conn, filter)?;
    let sql = format!("SELECT m.id FROM media m {} {}", c.join, c.where_sql());
    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt.query_map(params_from_iter(c.params.iter()), |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Detail>> {
    let sql = format!(
        "SELECT {COLUMNS}, m.description, m.source_url, m.video_codec, m.audio_codec, r.name,
            m.file_mtime, m.processed
         FROM media m JOIN roots r ON r.id = m.root_id WHERE m.id = ?"
    );
    Ok(conn
        .query_row(&sql, [id], |r| {
            let item = list_item(r)?;
            let mime = mime_guess::from_path(&item.filename).first_or_octet_stream().to_string();
            Ok(Detail {
                url: format!("/f/{}", item.id),
                mime,
                description: r.get(15)?,
                source_url: r.get(16)?,
                video_codec: r.get(17)?,
                audio_codec: r.get(18)?,
                root_name: r.get(19)?,
                file_modified_at: iso(r.get(20)?),
                processed: r.get(21)?,
                item,
            })
        })
        .optional()?)
}

pub fn file(conn: &Connection, id: i64) -> Result<Option<MediaFile>> {
    Ok(conn
        .query_row(
            "SELECT m.id, m.root_id, r.path, m.folder, m.filename
             FROM media m JOIN roots r ON r.id = m.root_id WHERE m.id = ?",
            [id],
            |r| {
                Ok(MediaFile {
                    id: r.get(0)?,
                    root_id: r.get(1)?,
                    root_path: PathBuf::from(r.get::<_, String>(2)?),
                    folder: r.get(3)?,
                    filename: r.get(4)?,
                })
            },
        )
        .optional()?)
}

/// Trimmed, lowercased, deduplicated. Tags are matched exactly, so "Beach"
/// and "beach " must not become two different tags.
pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = tags
        .iter()
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase())
        .filter(|t| !t.is_empty() && t.chars().count() <= 100)
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn tags_of(conn: &Connection, id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached("SELECT tag FROM media_tags WHERE media_id = ? ORDER BY tag")?;
    let tags = stmt.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(tags)
}

pub fn set_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<()> {
    let current = tags_of(conn, id)?;
    for tag in current.iter().filter(|t| !tags.contains(t)) {
        conn.execute("DELETE FROM media_tags WHERE media_id = ? AND tag = ?", params![id, tag])?;
    }
    for tag in tags.iter().filter(|t| !current.contains(t)) {
        conn.execute("INSERT OR IGNORE INTO media_tags (media_id, tag) VALUES (?, ?)", params![id, tag])?;
    }
    Ok(())
}

pub fn add_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<()> {
    for tag in tags {
        conn.execute("INSERT OR IGNORE INTO media_tags (media_id, tag) VALUES (?, ?)", params![id, tag])?;
    }
    Ok(())
}

pub fn remove_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<()> {
    for tag in tags {
        conn.execute("DELETE FROM media_tags WHERE media_id = ? AND tag = ?", params![id, tag])?;
    }
    Ok(())
}

/// Ensures a folder and all its ancestors are listed for a root.
pub fn ensure_folder(conn: &Connection, root_id: i64, folder: &str) -> Result<()> {
    let mut path = String::new();
    conn.execute("INSERT OR IGNORE INTO folders (root_id, path) VALUES (?, '')", [root_id])?;
    for part in folder.split('/').filter(|p| !p.is_empty()) {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(part);
        conn.execute("INSERT OR IGNORE INTO folders (root_id, path) VALUES (?, ?)", params![root_id, path])?;
    }
    Ok(())
}

/// A new row for a file that is already on disk. Probing and the thumbnail
/// follow from the processing worker.
pub struct NewMedia<'a> {
    pub root_id: i64,
    pub folder: &'a str,
    pub filename: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub source_url: &'a str,
    pub size: i64,
    pub file_mtime: i64,
    pub added_at: i64,
}

pub fn insert(conn: &Connection, m: &NewMedia) -> Result<i64> {
    conn.execute(
        "INSERT INTO media (root_id, folder, filename, kind, title, description, source_url,
            size, file_mtime, added_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            m.root_id, m.folder, m.filename, m.kind, m.title, m.description, m.source_url,
            m.size, m.file_mtime, m.added_at
        ],
    )?;
    ensure_folder(conn, m.root_id, m.folder)?;
    Ok(conn.last_insert_rowid())
}

/// The title a file gets when nothing better is known: its name without the
/// extension.
pub fn title_from_filename(filename: &str) -> String {
    std::path::Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename)
        .to_string()
}
