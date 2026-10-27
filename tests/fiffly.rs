//! fiffly's side of the media-publish protocol, driven the way fiffly drives
//! it: the manifest, the OAuth connect with PKCE, search and media reads with
//! the token, signed file URLs, uploads, and disconnect.

mod common;

use common::Server;
use reqwest::{Method, StatusCode, blocking::multipart};
use serde_json::{Value, json};

// RFC 7636 appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT: &str = "https://fiffly.com/publish/callback/127.0.0.1";

fn authorize(scope: &str, redirect: &str) -> Value {
    json!({
        "response_type": "code",
        "client_id": "fiffly",
        "redirect_uri": redirect,
        "scope": scope,
        "state": "s/1",
        "code_challenge": CHALLENGE,
        "code_challenge_method": "S256",
    })
}

/// The user clicks Allow; returns the code from the redirect.
fn approve(s: &Server, scope: &str) -> String {
    let mut body = authorize(scope, REDIRECT);
    body["approve"] = true.into();
    let to = s.post("/api/oauth/authorize", body)["redirectTo"].as_str().unwrap().to_string();
    assert!(to.starts_with(&format!("{REDIRECT}?code=")), "{to}");
    assert!(to.ends_with("&state=s%2F1"), "{to}");
    to.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string()
}

fn exchange(s: &Server, code: &str) -> reqwest::blocking::Response {
    s.request(Method::POST, "/api/oauth/token")
        .json(&json!({
            "grant_type": "authorization_code",
            "code": code,
            "code_verifier": VERIFIER,
            "client_id": "fiffly",
            "redirect_uri": REDIRECT,
        }))
        .send()
        .unwrap()
}

fn connect(s: &Server, scope: &str) -> String {
    let res = exchange(s, &approve(s, scope));
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = res.json().unwrap();
    assert_eq!(body["token_type"], "Bearer");
    body["access_token"].as_str().unwrap().to_string()
}

fn bearer(s: &Server, token: &str, path: &str) -> reqwest::blocking::Response {
    s.request(Method::GET, path).bearer_auth(token).send().unwrap()
}

#[test]
fn manifest_and_cors() {
    let s = Server::start();
    let res = s.get("/.well-known/media-publish.json");
    assert_eq!(res.headers()["access-control-allow-origin"], "*");
    assert_eq!(res.headers()["cross-origin-resource-policy"], "cross-origin");
    let m: Value = res.json().unwrap();
    assert_eq!(m["name"], "hoard");
    assert_eq!(m["capabilities"]["searchAuth"], "required");
    assert_eq!(m["oauth"]["scopes"], json!(["media:read", "media:write"]));

    let pre = s
        .request(Method::OPTIONS, "/api/publish/search")
        .header("Origin", "https://fiffly.com")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .send()
        .unwrap();
    assert_eq!(pre.status(), StatusCode::NO_CONTENT);
    assert!(pre.headers()["access-control-allow-headers"].to_str().unwrap().contains("Authorization"));
    assert_eq!(pre.headers()["access-control-allow-private-network"], "true");
}

#[test]
fn connect_search_open_upload_and_disconnect() {
    let s = Server::start();
    s.processed(3);
    let token = connect(&s, "media:read media:write");

    assert_eq!(bearer(&s, &token, "/api/publish/me").json::<Value>().unwrap()["anonymous"], false);

    let page: Value = bearer(&s, &token, "/api/publish/search?limit=2").json().unwrap();
    assert_eq!(page["results"].as_array().unwrap().len(), 2);
    assert_eq!(page["nextOffset"], 2);
    let found: Value = bearer(&s, &token, "/api/publish/search?q=kind:video").json().unwrap();
    assert_eq!(found["results"].as_array().unwrap().len(), 2);

    let red = s.by_name("red.mp4")["id"].as_i64().unwrap();
    let post: Value = bearer(&s, &token, &format!("/api/publish/media/{red}")).json().unwrap();
    assert_eq!(post["kind"], "clip");
    let file = &post["files"][0];
    assert_eq!((file["variant"].as_str(), file["format"].as_str()), (Some("source"), Some("mp4")));
    // A player cannot send the token: the file URL is signed instead.
    let part = s.request(Method::GET, file["url"].as_str().unwrap()).header("Range", "bytes=0-9").send().unwrap();
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.headers()["access-control-allow-origin"], "*");
    assert_eq!(s.get(post["files"][1]["url"].as_str().unwrap()).status(), StatusCode::OK, "thumbnail");

    // Saving an export back.
    let clip = std::fs::read(s.lib.join("blue.mp4")).unwrap();
    let meta = json!({
        "kind": "clip",
        "title": "my edit: final",
        "tags": ["Fiffly", "edit"],
        "source": { "url": "https://example.com/original" },
        "files": [{ "field": "f0", "variant": "", "format": "mp4", "width": 160, "height": 90 }],
    });
    let form = multipart::Form::new()
        .part("meta", multipart::Part::text(meta.to_string()).mime_str("application/json").unwrap())
        .part("timeline", multipart::Part::text("{}"))
        .part("f0", multipart::Part::bytes(clip).file_name("f0.mp4"));
    let res = s.request(Method::POST, "/api/publish/upload").bearer_auth(&token).multipart(form).send().unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{}", res.text().unwrap_or_default());
    let saved: Value = res.json().unwrap();
    let id = saved["slug"].as_str().unwrap();
    let item = s.json(&format!("/api/media/{id}"));
    assert_eq!(item["title"], "my edit: final");
    assert_eq!(item["filename"], "my edit_ final.mp4");
    assert_eq!(item["tags"], json!(["edit", "fiffly"]));
    assert_eq!(item["sourceUrl"], "https://example.com/original");
    assert!(s.lib.join("my edit_ final.mp4").is_file());
    s.processed(4);

    let revoke = s.request(Method::POST, "/api/oauth/revoke-self").bearer_auth(&token).send().unwrap();
    assert_eq!(revoke.status(), StatusCode::OK);
    assert_eq!(bearer(&s, &token, "/api/publish/search").status(), StatusCode::UNAUTHORIZED);
    assert_eq!(s.json("/api/fiffly/connections")["connections"], json!([]));
}

fn save(s: &Server, token: &str, title: &str, folder: Option<&str>) -> reqwest::blocking::Response {
    let mut meta = json!({
        "kind": "clip",
        "title": title,
        "files": [{ "field": "f0", "variant": "", "format": "mp4", "width": 160, "height": 90 }],
    });
    if let Some(f) = folder {
        meta["folder"] = f.into();
    }
    let form = multipart::Form::new()
        .part("meta", multipart::Part::text(meta.to_string()).mime_str("application/json").unwrap())
        .part("f0", multipart::Part::bytes(std::fs::read(s.lib.join("blue.mp4")).unwrap()).file_name("f0.mp4"));
    s.request(Method::POST, "/api/publish/upload").bearer_auth(token).multipart(form).send().unwrap()
}

#[test]
fn exports_save_into_a_picked_folder() {
    let s = Server::start();
    s.processed(3);
    let m: Value = s.get("/.well-known/media-publish.json").json().unwrap();
    let field = m["uploadForm"].as_array().unwrap().iter().find(|f| f["type"] == "folder").unwrap().clone();
    assert_eq!(field["name"], "folder");
    let list = field["list"].as_str().unwrap();

    assert_eq!(s.get(list).status(), StatusCode::UNAUTHORIZED);
    let token = connect(&s, "media:read media:write");
    let folder = |query: &str| bearer(&s, &token, &format!("{list}{query}"));

    // The default — the download folder here — with what is directly inside.
    let root: Value = folder("").json().unwrap();
    assert_eq!(root["path"].as_array().unwrap().len(), 1, "paths start at the storage folder");
    assert_eq!(root["parent"], Value::Null);
    assert_eq!(root["children"], json!([{ "id": format!("{}:clips", s.root_id()), "name": "clips", "hasChildren": false }]));
    let clips: Value = folder(&format!("?id={}:clips", s.root_id())).json().unwrap();
    assert_eq!(clips["parent"], root["id"]);
    assert_eq!(clips["path"][1], "clips");
    assert_eq!(folder(&format!("?id={}:gone", s.root_id())).status(), StatusCode::NOT_FOUND);

    // Search: by path from the top, or by name anywhere.
    let top = format!("{}/", root["path"][0].as_str().unwrap());
    let found: Value = folder(&format!("?q={}", top.to_uppercase())).json().unwrap();
    assert_eq!(found["results"], json!([{ "id": clips["id"], "path": clips["path"] }]));
    assert_eq!(folder("?q=lip").json::<Value>().unwrap()["results"][0]["id"], clips["id"]);
    assert_eq!(folder("?q=nothing").json::<Value>().unwrap()["results"], json!([]));

    let res = save(&s, &token, "into clips", clips["id"].as_str());
    assert_eq!(res.status(), StatusCode::OK, "{}", res.text().unwrap_or_default());
    let id = res.json::<Value>().unwrap()["slug"].as_str().unwrap().to_string();
    assert_eq!(s.json(&format!("/api/media/{id}"))["folder"], "clips");
    assert!(s.lib.join("clips/into clips.mp4").is_file());

    let res = save(&s, &token, "nowhere", Some(&format!("{}:gone", s.root_id())));
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(!s.lib.join("gone").exists());
    assert_eq!(save(&s, &token, "nowhere", Some("../etc")).status(), StatusCode::BAD_REQUEST);
}

#[test]
fn signed_links_open_media_without_connecting() {
    let s = Server::start();
    s.processed(3);
    let red = s.by_name("red.mp4")["id"].as_i64().unwrap();
    let slug = s.json(&format!("/api/media/{red}/fiffly"))["slug"].as_str().unwrap().to_string();

    let post: Value = s.get(&format!("/api/publish/media/{slug}")).json().unwrap();
    assert_eq!(post["slug"], slug.as_str());
    assert_eq!(s.get(post["files"][0]["url"].as_str().unwrap()).status(), StatusCode::OK);

    assert_eq!(s.get(&format!("/api/publish/media/{red}")).status(), StatusCode::UNAUTHORIZED, "a plain id needs the token");
    let forged = slug.replace(&format!("{red}-"), &format!("{}-", red + 1));
    assert_eq!(s.get(&format!("/api/publish/media/{forged}")).status(), StatusCode::UNAUTHORIZED, "a signature is per media");
    let url = post["files"][0]["url"].as_str().unwrap();
    let tampered = url.replacen(&format!("/s/{red}/"), &format!("/s/{}/", red + 1), 1);
    assert_eq!(s.get(&tampered).status(), StatusCode::FORBIDDEN);
}

#[test]
fn bad_requests_are_refused() {
    let s = Server::start();
    for (redirect, why) in [
        ("https://evil.example/publish/callback/x", "another site"),
        ("https://fiffly.com.evil.example/publish/callback/x", "a lookalike host"),
        ("https://fiffly.com/elsewhere", "another path"),
    ] {
        let res = s.request(Method::POST, "/api/oauth/authorize").json(&{
            let mut b = authorize("media:read", redirect);
            b["approve"] = true.into();
            b
        });
        assert_eq!(res.send().unwrap().status(), StatusCode::BAD_REQUEST, "{why}");
    }

    // A code works once; a replay also revokes what it minted.
    let code = approve(&s, "media:read");
    let token = exchange(&s, &code).json::<Value>().unwrap()["access_token"].as_str().unwrap().to_string();
    std::thread::sleep(std::time::Duration::from_secs(11));
    assert_eq!(exchange(&s, &code).status(), StatusCode::BAD_REQUEST);
    assert_eq!(bearer(&s, &token, "/api/publish/search").status(), StatusCode::UNAUTHORIZED);

    // A wrong verifier is no proof.
    let code = approve(&s, "media:read");
    let res = s
        .request(Method::POST, "/api/oauth/token")
        .json(&json!({ "grant_type": "authorization_code", "code": code, "code_verifier": "x".repeat(43),
                       "client_id": "fiffly", "redirect_uri": REDIRECT }))
        .send()
        .unwrap();
    assert_eq!(res.json::<Value>().unwrap()["error"], "invalid_grant");

    // Read-only access cannot upload.
    let token = connect(&s, "media:read");
    let form = multipart::Form::new().part("meta", multipart::Part::text("{}"));
    let res = s.request(Method::POST, "/api/publish/upload").bearer_auth(&token).multipart(form).send().unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
