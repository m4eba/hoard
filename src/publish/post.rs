//! Media rows as media-publish posts: the shape fiffly lists, searches and
//! opens. Every file URL is signed for the post's media.

use anyhow::Result;
use rusqlite::{Connection, params_from_iter};
use serde_json::{Value, json};

use super::Signature;
use crate::{media, storyboards::Storyboard};

pub struct PostRow {
    pub id: i64,
    kind: String,
    title: String,
    filename: String,
    width: i64,
    height: i64,
    duration: f64,
    size: i64,
    thumb_ver: i64,
    added_at: i64,
    file_mtime: i64,
    source_url: String,
    storyboard: Option<Storyboard>,
    tags: Vec<String>,
}

/// The rows for `ids`, in that order; ids that no longer exist are skipped.
pub fn rows(conn: &Connection, ids: &[i64]) -> Result<Vec<PostRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let marks = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT m.id, m.kind, m.title, m.filename, m.width, m.height, m.duration, m.size, m.thumb_ver,
            m.added_at, m.file_mtime, m.source_url, m.storyboard,
            (SELECT group_concat(tag, char(31)) FROM media_tags t WHERE t.media_id = m.id)
         FROM media m WHERE m.id IN ({marks})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut found: Vec<PostRow> = stmt
        .query_map(params_from_iter(ids), |r| {
            let storyboard: Option<String> = r.get(12)?;
            let tags: Option<String> = r.get(13)?;
            let mut tags: Vec<String> = tags.map(|t| t.split('\u{1f}').map(String::from).collect()).unwrap_or_default();
            tags.sort();
            Ok(PostRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                filename: r.get(3)?,
                width: r.get(4)?,
                height: r.get(5)?,
                duration: r.get(6)?,
                size: r.get(7)?,
                thumb_ver: r.get(8)?,
                added_at: r.get(9)?,
                file_mtime: r.get(10)?,
                source_url: r.get(11)?,
                storyboard: storyboard.and_then(|s| serde_json::from_str(&s).ok()),
                tags,
            })
        })?
        .collect::<Result<_, _>>()?;
    found.sort_by_key(|row| ids.iter().position(|&id| id == row.id));
    Ok(found)
}

/// The size a thumbnail was written at: the media scaled into the 640 box.
fn thumb_size(w: i64, h: i64) -> (i64, i64) {
    let scale = (640.0 / w.max(h).max(1) as f64).min(1.0);
    ((w as f64 * scale).round() as i64, (h as f64 * scale).round() as i64)
}

pub fn to_json(row: &PostRow, sig: &Signature) -> Value {
    let base = sig.base();
    let ext = std::path::Path::new(&row.filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = mime_guess::from_path(&row.filename).first_or_octet_stream().to_string();
    // The name rides along at the end of the URL, so a saved file is named
    // after what it is rather than "file".
    let name: String = row
        .filename
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
        .collect();
    let mut files = vec![json!({
        "variant": "source",
        "format": ext,
        "mime": mime,
        "url": format!("{base}/file/{name}"),
        "width": row.width,
        "height": row.height,
        "size": row.size,
    })];
    if row.thumb_ver > 0 {
        let (w, h) = thumb_size(row.width, row.height);
        files.push(json!({
            "variant": "thumb",
            "format": "jpg",
            "mime": "image/jpeg",
            "url": format!("{base}/thumb?v={}", row.thumb_ver),
            "width": w,
            "height": h,
        }));
    }
    let kind = match (row.kind.as_str(), ext.as_str()) {
        ("video", _) => "clip",
        (_, "gif") => "gif",
        _ => "image",
    };
    let mut post = json!({
        "slug": row.id.to_string(),
        "title": row.title,
        "kind": kind,
        "duration": row.duration,
        "width": row.width,
        "height": row.height,
        "created": media::iso(row.added_at),
        "modified": media::iso(row.file_mtime),
        "listed": false,
        "user": { "id": "hoard", "name": "hoard" },
        "tags": row.tags,
        "files": files,
        "hasTimeline": false,
    });
    if !row.source_url.is_empty() {
        post["source"] = json!({ "url": row.source_url });
    }
    if let Some(sb) = row.storyboard.as_ref().filter(|_| row.kind == "video") {
        // One sheet holds every tile: sheet 0 is the whole storyboard.
        post["storyboard"] = json!({
            "interval": sb.interval,
            "width": sb.tile_width,
            "height": sb.tile_height,
            "columns": sb.cols,
            "rows": sb.rows,
            "count": sb.count,
            "sheetBase": format!("{base}/storyboard/{}", sb.ver),
            "sheetCount": 1,
        });
    }
    post
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_only_shrink() {
        assert_eq!(thumb_size(1920, 1080), (640, 360));
        assert_eq!(thumb_size(1080, 1920), (360, 640));
        assert_eq!(thumb_size(320, 240), (320, 240));
    }
}
