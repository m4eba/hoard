//! Configuration. This is the only module that reads the environment; every
//! other part of the program takes what it needs from `Config`.

use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub enum Password {
    Plain(String),
    /// An argon2 PHC string, as printed by `hoard hash-password`.
    Hash(String),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub password: Option<Password>,
    pub ffmpeg: String,
    pub ffprobe: String,
    /// The interpreter used to create the yt-dlp environment.
    pub python: Option<String>,
    /// A yt-dlp binary to use as-is. When set, no environment is managed.
    pub ytdlp_bin: Option<PathBuf>,
    /// The virtual environment yt-dlp is installed into and kept updated in.
    pub ytdlp_env: PathBuf,
    pub ytdlp_auto_update: bool,
    /// Thumbnail and probe workers running at once.
    pub workers: usize,
    /// URL downloads running at once.
    pub downloads: usize,
    pub scan_interval: Option<Duration>,
    /// How long deleted files stay in a folder's trash. Zero deletes at once.
    pub trash_days: u64,
    /// Storage folders registered at startup if missing, for container setups.
    pub initial_roots: Vec<PathBuf>,
    pub secure_cookies: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let data_dir = PathBuf::from(var("HOARD_DATA_DIR").unwrap_or_else(|| "./data".into()));
        let listen = var("HOARD_LISTEN")
            .unwrap_or_else(|| "127.0.0.1:8150".into())
            .parse()
            .context("HOARD_LISTEN must be host:port")?;

        let password = match (var("HOARD_PASSWORD_HASH"), var("HOARD_PASSWORD")) {
            (Some(hash), _) => Some(Password::Hash(hash)),
            (None, Some(plain)) => Some(Password::Plain(plain)),
            (None, None) => None,
        };

        let ytdlp_env = var("HOARD_YTDLP_ENV")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("ytdlp"));

        let scan_minutes: u64 = parse_or("HOARD_SCAN_INTERVAL_MINUTES", 60)?;

        Ok(Self {
            listen,
            password,
            ffmpeg: var("HOARD_FFMPEG").unwrap_or_else(|| "ffmpeg".into()),
            ffprobe: var("HOARD_FFPROBE").unwrap_or_else(|| "ffprobe".into()),
            python: var("HOARD_PYTHON").or_else(find_python),
            ytdlp_bin: var("HOARD_YTDLP_BIN").map(PathBuf::from),
            ytdlp_env,
            ytdlp_auto_update: parse_or("HOARD_YTDLP_AUTO_UPDATE", true)?,
            workers: parse_or("HOARD_WORKERS", 2)?.max(1),
            downloads: parse_or("HOARD_DOWNLOADS", 2)?.max(1),
            scan_interval: (scan_minutes > 0).then(|| Duration::from_secs(scan_minutes * 60)),
            trash_days: parse_or("HOARD_TRASH_DAYS", 30)?,
            initial_roots: var("HOARD_ROOTS")
                .map(|v| env::split_paths(&v).filter(|p| !p.as_os_str().is_empty()).collect())
                .unwrap_or_default(),
            secure_cookies: parse_or("HOARD_SECURE_COOKIES", false)?,
            data_dir,
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("hoard.db")
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        self.data_dir.join("thumbs")
    }
    pub fn storyboards_dir(&self) -> PathBuf {
        self.data_dir.join("storyboards")
    }
    pub fn meta_dir(&self) -> PathBuf {
        self.data_dir.join("meta")
    }
    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }
}

fn var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn parse_or<T: std::str::FromStr>(key: &str, default: T) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    match var(key) {
        None => Ok(default),
        Some(v) => v
            .trim()
            .parse()
            .map_err(|e| anyhow::anyhow!("{key}={v:?} is invalid: {e}")),
    }
}

/// The first Python 3 on PATH. URL downloads are simply unavailable without one.
fn find_python() -> Option<String> {
    ["python3", "python"].into_iter().find_map(|name| {
        let out = std::process::Command::new(name).arg("--version").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        (out.status.success() && text.contains("Python 3")).then(|| name.to_string())
    })
}
