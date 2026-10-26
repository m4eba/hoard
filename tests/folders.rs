//! Moving folders: within a storage folder, across storage folders on another
//! filesystem, and what is refused.

mod common;

use common::Server;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

fn move_folder(s: &Server, root: i64, path: &str, to_root: i64, to: &str) -> reqwest::blocking::Response {
    s.request(Method::POST, "/api/folders/move")
        .json(&json!({ "rootId": root, "path": path, "to": { "rootId": to_root, "folder": to } }))
        .send()
        .unwrap()
}

/// Starts a move and waits for it, expecting success.
fn moved(s: &Server, root: i64, path: &str, to_root: i64, to: &str) -> Value {
    let res = move_folder(s, root, path, to_root, to);
    assert_eq!(res.status(), StatusCode::OK, "{}", res.text().unwrap_or_default());
    let body: Value = res.json().unwrap();
    let task = s.task(body["taskId"].as_u64().unwrap());
    assert_eq!(task["state"], "done", "{task}");
    body["place"].clone()
}

fn folders(s: &Server, root: i64) -> Vec<String> {
    let all = s.json("/api/folders");
    let list = all.as_array().or_else(|| all["folders"].as_array()).unwrap().clone();
    let mut paths: Vec<String> = list
        .iter()
        .filter(|f| f["rootId"] == root)
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    paths.sort();
    paths
}

#[test]
fn a_folder_moves_with_its_subfolders_tags_and_ids() {
    let s = Server::start();
    s.processed(3);
    let root = s.root_id();
    let red = s.by_name("red.mp4")["id"].as_i64().unwrap();
    let blue = s.by_name("blue.mp4")["id"].as_i64().unwrap();
    s.patch(&format!("/api/media/{red}"), json!({ "tags": ["keep"] }));
    // A subfolder with media of its own.
    s.post("/api/media/move", json!({ "select": { "ids": [blue] }, "rootId": root, "folder": "clips/deeper" }));
    s.wait("the file move", || s.json(&format!("/api/media/{blue}"))["folder"] == "clips/deeper");
    s.post("/api/folders", json!({ "rootId": root, "path": "archive" }));

    let place = moved(&s, root, "clips", root, "archive");
    assert_eq!(place, json!({ "rootId": root, "folder": "archive/clips" }));

    assert!(s.lib.join("archive/clips/red.mp4").is_file());
    assert!(s.lib.join("archive/clips/deeper/blue.mp4").is_file());
    assert!(!s.lib.join("clips").exists());
    let red_now = s.json(&format!("/api/media/{red}"));
    assert_eq!(red_now["folder"], "archive/clips");
    assert_eq!(red_now["tags"], json!(["keep"]));
    assert_eq!(s.json(&format!("/api/media/{blue}"))["folder"], "archive/clips/deeper");
    assert_eq!(folders(&s, root), ["", "archive", "archive/clips", "archive/clips/deeper"]);
    assert_eq!(s.get(red_now["thumbUrl"].as_str().unwrap()).status(), StatusCode::OK);

    // The folder view and search follow it.
    let listed = s.media(&format!("rootId={root}&folder=archive&recursive=true"));
    assert_eq!(listed.len(), 2);
    assert_eq!(s.media("q=folder:archive/clips").len(), 2);
}

#[test]
fn a_folder_with_a_different_case_is_left_alone() {
    let s = Server::start();
    s.processed(3);
    let root = s.root_id();
    let blue = s.by_name("blue.mp4")["id"].as_i64().unwrap();
    s.post("/api/media/move", json!({ "select": { "ids": [blue] }, "rootId": root, "folder": "Clips/other" }));
    s.wait("the file move", || s.json(&format!("/api/media/{blue}"))["folder"] == "Clips/other");
    s.post("/api/folders", json!({ "rootId": root, "path": "archive" }));

    moved(&s, root, "clips", root, "archive");
    assert_eq!(s.json(&format!("/api/media/{blue}"))["folder"], "Clips/other");
    assert!(s.lib.join("Clips/other/blue.mp4").is_file());
    assert_eq!(s.media(&format!("rootId={root}&folder=clips&recursive=true")).len(), 0);
}

#[test]
fn a_folder_moves_to_another_storage_folder_on_another_filesystem() {
    // /dev/shm is a tmpfs, so this takes the copy path rather than a rename
    // where it exists; elsewhere it is still a move between storage folders.
    let base = if std::path::Path::new("/dev/shm").is_dir() { "/dev/shm" } else { "/tmp" };
    let other = std::path::PathBuf::from(format!("{base}/hoard-e2e-other-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&other);
    std::fs::create_dir_all(&other).unwrap();
    let cleanup = Cleanup(other.clone());

    let s = Server::start_with_roots(&[&other]);
    s.processed(3);
    let (root, second) = (s.root_id(), s.root_named(other.file_name().unwrap().to_str().unwrap()));
    let red = s.by_name("red.mp4");
    let mtime = std::fs::metadata(s.lib.join("clips/red.mp4")).unwrap().modified().unwrap();
    s.patch(&format!("/api/media/{}", red["id"]), json!({ "tags": ["travels"] }));

    moved(&s, root, "clips", second, "");
    assert!(other.join("clips/red.mp4").is_file());
    assert!(!s.lib.join("clips").exists());
    assert_eq!(std::fs::metadata(other.join("clips/red.mp4")).unwrap().modified().unwrap(), mtime);
    let now = s.json(&format!("/api/media/{}", red["id"]));
    assert_eq!((now["rootId"].as_i64(), now["folder"].as_str()), (Some(second), Some("clips")));
    assert_eq!(now["tags"], json!(["travels"]));
    assert!(!walk_names(&other).iter().any(|n| n.contains("hoard-part")), "no temporary copy is left");

    // A rescan agrees with what the move recorded.
    s.post(&format!("/api/roots/{second}/scan"), json!(null));
    s.post(&format!("/api/roots/{root}/scan"), json!(null));
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert_eq!(s.json(&format!("/api/media/{}", red["id"]))["folder"], "clips");
    assert_eq!(s.media("").len(), 3);
    drop(cleanup);
}

#[test]
fn impossible_moves_are_refused() {
    let s = Server::start();
    s.processed(3);
    let root = s.root_id();
    s.post("/api/folders", json!({ "rootId": root, "path": "clips/inner" }));
    s.post("/api/folders", json!({ "rootId": root, "path": "other/clips" }));

    assert_eq!(move_folder(&s, root, "clips", root, "clips/inner").status(), StatusCode::BAD_REQUEST, "into itself");
    assert_eq!(move_folder(&s, root, "clips", root, "").status(), StatusCode::BAD_REQUEST, "already there");
    assert_eq!(move_folder(&s, root, "", root, "other").status(), StatusCode::BAD_REQUEST, "a storage folder's top");
    assert_eq!(move_folder(&s, root, "clips", root, "other").status(), StatusCode::CONFLICT, "name taken");
    assert_eq!(move_folder(&s, root, "nope", root, "other").status(), StatusCode::NOT_FOUND);
    assert!(s.lib.join("clips/red.mp4").is_file(), "nothing moved");
}

struct Cleanup(std::path::PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn walk_names(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        out.push(e.file_name().to_string_lossy().into_owned());
        if e.path().is_dir() {
            out.extend(walk_names(&e.path()));
        }
    }
    out
}
