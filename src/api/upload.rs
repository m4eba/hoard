//! fiffly saving an export into the library. One request carries the `meta`
//! JSON part first, an optional timeline (not kept), then one part per file.
//! Each file becomes its own media item in the folder picked in fiffly's save
//! dialog, else the one chosen in Settings. All or nothing: a request cut off
//! or refused leaves no file behind.

use std::path::PathBuf;

use axum::{
    Json,
    extract::{Multipart, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use super::export_folders;
use crate::{
    db,
    error::{ApiResult, Reject, bad_request},
    events::Event,
    media,
    ops,
    paths, publish,
    state::App,
};

pub const FORMATS: [&str; 6] = ["mp4", "webm", "gif", "webp", "avif", "png"];
pub const MAX_FILES: usize = 8;
pub const MAX_FILE_BYTES: u64 = 4 << 30;
pub const MAX_TAGS: usize = 20;

#[derive(Deserialize)]
struct FileMeta {
    field: String,
    format: String,
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    files: Vec<FileMeta>,
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    source: Option<Source>,
    /// A folder id from the folder picker; the upload form's `folder` field.
    folder: Option<String>,
}

#[derive(Deserialize)]
struct Source {
    url: Option<String>,
}

/// A title as a file name: whatever a filesystem would refuse becomes `_`.
fn stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '_' } else { c })
        .take(120)
        .collect();
    let s = s.trim().trim_start_matches('.').trim();
    if s.is_empty() { "fiffly export".into() } else { s.into() }
}

fn too_large() -> Reject {
    Reject { status: axum::http::StatusCode::PAYLOAD_TOO_LARGE, message: "file too large".into() }
}

pub async fn upload(State(app): State<App>, headers: HeaderMap, mut form: Multipart) -> ApiResult<Json<Value>> {
    publish::bearer(&app, &headers, publish::SCOPE_WRITE).await?;
    let bad = |e: axum::extract::multipart::MultipartError| bad_request(format!("malformed upload: {e}"));

    let first = form.next_field().await.map_err(bad)?.ok_or_else(|| bad_request("empty upload"))?;
    if first.name() != Some("meta") {
        return Err(bad_request("the meta part must come first").into());
    }
    let meta: Meta = serde_json::from_slice(&first.bytes().await.map_err(bad)?)
        .map_err(|e| bad_request(format!("meta: {e}")))?;
    if meta.files.is_empty() {
        return Err(bad_request("meta names no files").into());
    }
    if meta.files.len() > MAX_FILES {
        return Err(Reject { status: axum::http::StatusCode::PAYLOAD_TOO_LARGE, message: format!("at most {MAX_FILES} files") }.into());
    }
    for (i, f) in meta.files.iter().enumerate() {
        if !FORMATS.contains(&f.format.as_str()) {
            return Err(Reject {
                status: axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                message: format!("{}: unsupported format", f.format),
            }
            .into());
        }
        if meta.files[..i].iter().any(|g| g.field == f.field) {
            return Err(bad_request(format!("{}: named twice", f.field)).into());
        }
    }

    let place = export_folders::target(&app, meta.folder.as_deref()).await?;
    let _claim = export_folders::Claim::new(&app, place.root_id)?;
    let (root, folder) = ops::prepare_place(&app, &place).await?;
    let dir = paths::folder_dir(&root, &folder);

    // Every part lands under a hidden name first; only once all of them are
    // in do they become files of the library.
    let mut parts: Vec<Option<(PathBuf, i64)>> = meta.files.iter().map(|_| None).collect();
    let received = async {
        while let Some(mut field) = form.next_field().await.map_err(bad)? {
            let name = field.name().unwrap_or_default().to_string();
            if name == "timeline" {
                while field.chunk().await.map_err(bad)?.is_some() {}
                continue;
            }
            let i = meta.files.iter().position(|f| f.field == name).ok_or_else(|| bad_request(format!("{name}: not in meta")))?;
            if parts[i].is_some() {
                return Err(bad_request(format!("{name}: sent twice")).into());
            }
            let tmp = dir.join(format!(".fiffly-{}.hoard-upload", publish::random_hex(8)));
            parts[i] = Some((tmp.clone(), 0));
            let mut file = tokio::fs::File::create(&tmp).await?;
            let mut size = 0u64;
            while let Some(chunk) = field.chunk().await.map_err(bad)? {
                size += chunk.len() as u64;
                if size > MAX_FILE_BYTES {
                    return Err(too_large().into());
                }
                file.write_all(&chunk).await?;
            }
            file.sync_all().await?;
            parts[i] = Some((tmp, size as i64));
        }
        if let Some(f) = meta.files.iter().zip(&parts).find(|(_, p)| p.is_none()) {
            return Err(bad_request(format!("{}: never arrived", f.0.field)).into());
        }
        anyhow::Ok(())
    }
    .await;
    let cleanup = |parts: &[Option<(PathBuf, i64)>]| {
        for (path, _) in parts.iter().flatten() {
            let _ = std::fs::remove_file(path);
        }
    };
    if let Err(e) = received {
        cleanup(&parts);
        return Err(e.into());
    }

    let title = meta.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("fiffly export").to_string();
    let several = meta.files.len() > 1;
    let mut placed: Vec<(PathBuf, String, String, &'static str, i64)> = Vec::new();
    for (f, part) in meta.files.iter().zip(&parts) {
        let (tmp, size) = part.as_ref().expect("checked above");
        let target = paths::unique_path(&dir, &format!("{}.{}", stem(&title), f.format));
        if let Err(e) = std::fs::rename(tmp, &target) {
            cleanup(&parts);
            for (path, ..) in &placed {
                let _ = std::fs::remove_file(path);
            }
            return Err(anyhow::Error::from(e).into());
        }
        let name = target.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        let title = if several { format!("{title} ({})", f.format) } else { title.clone() };
        let kind = paths::media_kind(&target).unwrap_or("video");
        placed.push((target, name, title, kind, *size));
    }

    let mut tags = media::normalize_tags(&meta.tags);
    tags.truncate(MAX_TAGS);
    let source = meta.source.and_then(|s| s.url).unwrap_or_default();
    let rows: Vec<(String, String, &'static str, i64)> =
        placed.iter().map(|(_, name, title, kind, size)| (name.clone(), title.clone(), *kind, *size)).collect();
    let root_id = place.root_id;
    let inserted = app
        .db
        .run(move |c| {
            let tx = c.transaction()?;
            // deleted in the library while this upload was on its way: the
            // files are in it on disk, so it is back
            media::ensure_folder(&tx, root_id, &folder)?;
            let now = db::now();
            let mut ids = Vec::new();
            for (name, title, kind, size) in &rows {
                let id = media::insert(
                    &tx,
                    &media::NewMedia {
                        root_id,
                        folder: &folder,
                        filename: name,
                        kind,
                        title,
                        description: "",
                        source_url: &source,
                        size: *size,
                        file_mtime: now,
                        added_at: now,
                    },
                )?;
                media::set_tags(&tx, id, &tags)?;
                ids.push(id);
            }
            tx.commit()?;
            Ok(ids)
        })
        .await;
    let ids = match inserted {
        Ok(ids) => ids,
        Err(e) => {
            for (path, ..) in &placed {
                let _ = std::fs::remove_file(path);
            }
            return Err(e.into());
        }
    };
    app.work.notify_one();
    app.events.send(Event::Library);
    let id = ids[0];
    Ok(Json(json!({ "slug": id.to_string(), "url": format!("/m/{id}"), "grouped": false })))
}

#[cfg(test)]
mod tests {
    use super::stem;

    #[test]
    fn titles_become_safe_names() {
        assert_eq!(stem("a/b: c?"), "a_b_ c_");
        assert_eq!(stem("  ..hidden "), "hidden");
        assert_eq!(stem(" "), "fiffly export");
    }
}
