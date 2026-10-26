//! The library over HTTP: scanning, probing, serving, search, tags, moves,
//! deletes, storyboards and the password.

mod common;

use common::Server;
use reqwest::{Method, StatusCode};
use serde_json::json;

#[test]
fn library_is_scanned_probed_and_served() {
    let s = Server::start();
    let all = s.processed(3);

    let red = s.by_name("red.mp4");
    assert_eq!(red["kind"], "video");
    assert_eq!(red["folder"], "clips");
    assert_eq!(red["title"], "red");
    assert_eq!((red["width"].as_i64(), red["height"].as_i64()), (Some(320), Some(180)));
    assert!((red["duration"].as_f64().unwrap() - 2.0).abs() < 0.2, "{}", red["duration"]);

    let photo = s.by_name("photo.png");
    assert_eq!(photo["kind"], "image");
    assert_eq!(photo["width"], 200);

    for m in &all {
        let thumb = s.get(m["thumbUrl"].as_str().unwrap());
        assert_eq!(thumb.status(), StatusCode::OK);
        assert_eq!(thumb.headers()["content-type"], "image/jpeg");
    }

    // Seeking in the player rests on range requests.
    let id = red["id"].as_i64().unwrap();
    let part = s.request(Method::GET, &format!("/f/{id}")).header("Range", "bytes=0-99").send().unwrap();
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.bytes().unwrap().len(), 100);

    let detail = s.json(&format!("/api/media/{id}"));
    assert_eq!(detail["videoCodec"], "h264");
    assert_eq!(detail["audioCodec"], "aac");
}

#[test]
fn search_and_tags() {
    let s = Server::start();
    s.processed(3);
    let red = s.by_name("red.mp4")["id"].as_i64().unwrap();
    let blue = s.by_name("blue.mp4")["id"].as_i64().unwrap();
    s.patch(&format!("/api/media/{red}"), json!({ "tags": ["Sunset", "beach"] }));
    s.patch(&format!("/api/media/{blue}"), json!({ "tags": ["beach"], "title": "Ocean waves" }));

    let names = |q: &str| -> Vec<String> {
        let mut v: Vec<String> = s
            .media(&format!("q={}", urlencode(q)))
            .iter()
            .map(|m| m["filename"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names("tag:sunset"), ["red.mp4"], "tags are lowercased");
    assert_eq!(names("tag:beach"), ["blue.mp4", "red.mp4"]);
    assert_eq!(names("tag:beach -tag:sunset"), ["blue.mp4"]);
    assert_eq!(names("kind:image"), ["photo.png"]);
    assert_eq!(names("ocean"), ["blue.mp4"], "titles are searched");
    assert_eq!(names("folder:clips"), ["red.mp4"]);
    assert_eq!(names("sunet"), ["red.mp4"], "a misspelt word still finds the tag");

    let suggest = s.json("/api/tags/suggest?q=be");
    assert_eq!(suggest["tags"][0], "beach");
}

#[test]
fn moves_and_deletes_touch_the_disk() {
    let s = Server::start();
    s.processed(3);
    let root = s.root_id();
    let blue = s.by_name("blue.mp4")["id"].as_i64().unwrap();

    s.post("/api/folders", json!({ "rootId": root, "path": "archive/2026" }));
    assert!(s.lib.join("archive/2026").is_dir());

    s.post("/api/media/move", json!({ "select": { "ids": [blue] }, "rootId": root, "folder": "archive/2026" }));
    s.wait("the move", || s.lib.join("archive/2026/blue.mp4").is_file());
    assert!(!s.lib.join("blue.mp4").exists());
    s.wait("the row to follow", || s.json(&format!("/api/media/{blue}"))["folder"] == "archive/2026");

    s.send(Method::DELETE, &format!("/api/media/{blue}"), json!(null));
    assert!(!s.lib.join("archive/2026/blue.mp4").exists());
    let trashed = walk(&s.lib.join(".hoard-trash"));
    assert!(trashed.iter().any(|p| p.ends_with("blue.mp4")), "deleted files go to the trash: {trashed:?}");
    assert_eq!(s.get(&format!("/api/media/{blue}")).status(), StatusCode::NOT_FOUND);
    assert_eq!(s.media("").len(), 2);
}

#[test]
fn files_moved_outside_keep_their_tags() {
    let s = Server::start();
    s.processed(3);
    let red = s.by_name("red.mp4")["id"].as_i64().unwrap();
    s.patch(&format!("/api/media/{red}"), json!({ "tags": ["keep"] }));

    std::fs::create_dir_all(s.lib.join("elsewhere")).unwrap();
    std::fs::rename(s.lib.join("clips/red.mp4"), s.lib.join("elsewhere/renamed.mp4")).unwrap();
    s.post(&format!("/api/roots/{}/scan", s.root_id()), json!(null));

    s.wait("the rescan", || s.media("").iter().any(|m| m["filename"] == "renamed.mp4"));
    let moved = s.by_name("renamed.mp4");
    assert_eq!(moved["id"], red, "recognised as moved, not deleted and re-added");
    assert_eq!(moved["folder"], "elsewhere");
    assert_eq!(moved["tags"], json!(["keep"]));
}

#[test]
fn storyboards_are_built_on_request() {
    let s = Server::start();
    s.processed(3);
    let red = s.by_name("red.mp4")["id"].as_i64().unwrap();
    assert_eq!(s.json(&format!("/api/media/{red}/storyboard"))["state"], "none");

    s.post(&format!("/api/media/{red}/storyboard"), json!(null));
    s.wait("the storyboard", || s.json(&format!("/api/media/{red}/storyboard"))["state"] == "ready");
    let sb = s.json(&format!("/api/media/{red}/storyboard"));
    assert!(sb["count"].as_i64().unwrap() > 1);
    let sheet = s.get(sb["url"].as_str().unwrap());
    assert_eq!(sheet.status(), StatusCode::OK);
    assert_eq!(sheet.headers()["content-type"], "image/jpeg");
}

#[test]
fn a_password_guards_everything() {
    let mut s = Server::start_with(&[("HOARD_PASSWORD", "hunter22")]);
    assert_eq!(s.json("/api/config")["authRequired"], true);
    assert_eq!(s.get("/api/media").status(), StatusCode::UNAUTHORIZED);
    assert_eq!(s.get("/f/1").status(), StatusCode::UNAUTHORIZED);

    assert_eq!(s.login("wrong"), StatusCode::UNAUTHORIZED);
    assert_eq!(s.login("hunter22"), StatusCode::OK);
    assert_eq!(s.get("/api/media").status(), StatusCode::OK);
    s.processed(3);
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
