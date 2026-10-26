//! End-to-end harness: every test starts the real `hoard` binary on a free
//! port, with its own data directory and a storage folder of media generated
//! with ffmpeg, and talks to it over HTTP.

#![allow(dead_code)]

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    thread::sleep,
    time::{Duration, Instant},
};

use reqwest::blocking::{Client, RequestBuilder, Response};
use serde_json::Value;

pub struct Server {
    child: Child,
    pub base: String,
    pub dir: PathBuf,
    /// The storage folder the server was started with.
    pub lib: PathBuf,
    client: Client,
    cookie: Option<String>,
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn ffmpeg(args: &[&str]) {
    let ok = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg is needed for the e2e tests")
        .success();
    assert!(ok, "ffmpeg {args:?}");
}

/// Two videos and an image: `clips/red.mp4` (2 s, with sound),
/// `blue.mp4` (1 s) and `photo.png`.
fn fixtures(lib: &Path) {
    fs::create_dir_all(lib.join("clips")).unwrap();
    let out = |p: &str| lib.join(p).to_str().unwrap().to_string();
    ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc=size=320x180:rate=10:duration=2",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", &out("clips/red.mp4"),
    ]);
    ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=blue:size=160x90:rate=10:duration=1",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", &out("blue.mp4"),
    ]);
    ffmpeg(&["-f", "lavfi", "-i", "testsrc=size=200x100", "-frames:v", "1", &out("photo.png")]);
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

impl Server {
    pub fn start() -> Self {
        Self::start_with(&[])
    }

    pub fn start_with(env: &[(&str, &str)]) -> Self {
        Self::launch(env, &[])
    }

    /// With more storage folders after the generated one; they must exist.
    pub fn start_with_roots(extra: &[&Path]) -> Self {
        Self::launch(&[], extra)
    }

    fn launch(env: &[(&str, &str)], extra_roots: &[&Path]) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "hoard-e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        let lib = dir.join("library");
        fixtures(&lib);
        let port = free_port();
        let log = fs::File::create(dir.join("server.log")).unwrap();

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hoard"));
        // Nothing from the developer's own configuration leaks in.
        for (key, _) in std::env::vars().filter(|(k, _)| k.starts_with("HOARD_")) {
            cmd.env_remove(key);
        }
        cmd.env("HOARD_DATA_DIR", dir.join("data"))
            .env("HOARD_LISTEN", format!("127.0.0.1:{port}"))
            .env("HOARD_ROOTS", std::env::join_paths(std::iter::once(lib.as_path()).chain(extra_roots.iter().copied())).unwrap())
            .env("HOARD_YTDLP_AUTO_UPDATE", "false")
            .env("HOARD_LOG", "hoard=debug")
            .envs(env.iter().copied())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .stdin(Stdio::null());
        let child = cmd.spawn().expect("starting hoard");

        let server = Self {
            child,
            base: format!("http://127.0.0.1:{port}"),
            dir,
            lib,
            client: Client::builder().timeout(Duration::from_secs(30)).build().unwrap(),
            cookie: None,
        };
        server.wait("the server to listen", || {
            server.client.get(server.url("/api/config")).send().is_ok_and(|r| r.status().is_success())
        });
        server
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn with_cookie(&self, req: RequestBuilder) -> RequestBuilder {
        match &self.cookie {
            Some(c) => req.header("Cookie", c),
            None => req,
        }
    }

    pub fn get(&self, path: &str) -> Response {
        self.with_cookie(self.client.get(self.url(path))).send().unwrap()
    }

    pub fn request(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        self.with_cookie(self.client.request(method, self.url(path)))
    }

    /// GET, expecting success, as JSON.
    pub fn json(&self, path: &str) -> Value {
        let res = self.get(path);
        assert!(res.status().is_success(), "GET {path}: {}", res.status());
        res.json().unwrap()
    }

    pub fn send(&self, method: reqwest::Method, path: &str, body: Value) -> Value {
        let res = self.request(method.clone(), path).json(&body).send().unwrap();
        let status = res.status();
        let text = res.text().unwrap();
        assert!(status.is_success(), "{method} {path}: {status} {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    pub fn post(&self, path: &str, body: Value) -> Value {
        self.send(reqwest::Method::POST, path, body)
    }

    pub fn patch(&self, path: &str, body: Value) -> Value {
        self.send(reqwest::Method::PATCH, path, body)
    }

    pub fn login(&mut self, password: &str) -> reqwest::StatusCode {
        let res = self.client.post(self.url("/api/auth/login")).json(&serde_json::json!({ "password": password })).send().unwrap();
        if let Some(c) = res.headers().get("set-cookie") {
            self.cookie = Some(c.to_str().unwrap().split(';').next().unwrap().to_string());
        }
        res.status()
    }

    /// Polls until `ready` holds, failing the test after 60 s.
    pub fn wait(&self, what: &str, mut ready: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ready() {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for {what}; see {}", self.dir.display());
            sleep(Duration::from_millis(100));
        }
    }

    pub fn media(&self, query: &str) -> Vec<Value> {
        let v = self.json(&format!("/api/media?limit=500&{query}"));
        v["media"].as_array().unwrap().clone()
    }

    /// Waits for the first scan and for every file to be probed and
    /// thumbnailed, and returns the library.
    pub fn processed(&self, count: usize) -> Vec<Value> {
        self.wait("media to be processed", || {
            let all = self.media("");
            all.len() == count && all.iter().all(|m| m["thumbUrl"].is_string())
        });
        self.media("")
    }

    pub fn by_name(&self, filename: &str) -> Value {
        self.media("")
            .into_iter()
            .find(|m| m["filename"] == filename)
            .unwrap_or_else(|| panic!("{filename} not in the library"))
    }

    pub fn root_id(&self) -> i64 {
        self.root_named("library")
    }

    pub fn root_named(&self, name: &str) -> i64 {
        let roots = self.json("/api/roots");
        roots["roots"].as_array().unwrap().iter().find(|r| r["name"] == name).expect("root")["id"].as_i64().unwrap()
    }

    /// Waits for a background task to end, returning its final state.
    pub fn task(&self, id: u64) -> Value {
        let mut last = Value::Null;
        self.wait("the task", || {
            let tasks = self.json("/api/tasks");
            last = tasks["tasks"].as_array().unwrap().iter().find(|t| t["id"] == id).cloned().unwrap_or(Value::Null);
            last["state"] == "done" || last["state"] == "failed"
        });
        last
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            eprintln!("kept {} for inspection", self.dir.display());
        } else {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}
