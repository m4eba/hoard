//! The yt-dlp installation: a Python virtual environment in the data
//! directory that the server creates on request and keeps up to date, or a
//! binary the operator points at and manages themselves.

use std::{
    path::PathBuf,
    process::Stdio,
    sync::Mutex,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tokio::process::Command;

use crate::{config::Config, db, state::App};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Downloads can work at all: a binary was given or Python was found.
    pub available: bool,
    /// The server owns the installation and may update it.
    pub managed: bool,
    pub installed: bool,
    pub version: Option<String>,
    pub updating: bool,
    pub last_update: Option<String>,
    pub error: Option<String>,
    pub python: Option<String>,
    pub env_path: String,
}

pub struct Ytdlp {
    status: Mutex<Status>,
    install_lock: tokio::sync::Mutex<()>,
}

/// The extras pull in the impersonation and JavaScript runtime dependencies
/// that several sites, YouTube among them, now require.
const PACKAGE: &str = "yt-dlp[default,deno]";
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);

fn bin_dir(env: &std::path::Path) -> PathBuf {
    if cfg!(windows) { env.join("Scripts") } else { env.join("bin") }
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

impl Ytdlp {
    pub fn new(cfg: &Config) -> Self {
        let status = Status {
            available: cfg.ytdlp_bin.is_some() || cfg.python.is_some(),
            managed: cfg.ytdlp_bin.is_none(),
            python: cfg.python.clone(),
            env_path: cfg.ytdlp_env.display().to_string(),
            ..Default::default()
        };
        Self { status: Mutex::new(status), install_lock: Default::default() }
    }

    pub fn binary(cfg: &Config) -> PathBuf {
        cfg.ytdlp_bin.clone().unwrap_or_else(|| bin_dir(&cfg.ytdlp_env).join(exe("yt-dlp")))
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn set(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock().unwrap());
    }

    /// Reads the installed version, which is also how "installed" is decided.
    pub async fn refresh(&self, app: &App) {
        let bin = Self::binary(&app.cfg);
        let version = match Command::new(&bin).arg("--version").stdin(Stdio::null()).output().await {
            Ok(out) if out.status.success() => Some(String::from_utf8_lossy(&out.stdout).trim().to_string()),
            _ => None,
        };
        let last = app.db.run(|c| db::get_setting(c, "ytdlp_updated_at")).await.ok().flatten();
        self.set(|s| {
            s.installed = version.is_some();
            s.version = version;
            s.last_update = last.and_then(|t| t.parse().ok()).map(crate::media::iso);
        });
    }

    /// Creates the environment if needed and installs or upgrades yt-dlp in it.
    pub async fn install(&self, app: &App) -> Result<()> {
        if app.cfg.ytdlp_bin.is_some() {
            bail!("yt-dlp is configured with HOARD_YTDLP_BIN and managed outside this server");
        }
        let Some(python) = app.cfg.python.clone() else {
            bail!("no Python 3 interpreter found; install python3 or set HOARD_PYTHON");
        };
        let Ok(_guard) = self.install_lock.try_lock() else { return Ok(()) };
        self.set(|s| {
            s.updating = true;
            s.error = None;
        });
        let task = app.tasks.start("ytdlp", "Updating yt-dlp", 0);
        let result = self.install_inner(app, &python).await;
        self.set(|s| s.updating = false);
        match &result {
            Ok(()) => {
                let now = db::now().to_string();
                let _ = app.db.run(move |c| db::set_setting(c, "ytdlp_updated_at", &now)).await;
                self.refresh(app).await;
                task.finish(self.status().version.unwrap_or_default());
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.set(|s| s.error = Some(msg.clone()));
                task.fail(msg);
            }
        }
        result
    }

    async fn install_inner(&self, app: &App, python: &str) -> Result<()> {
        let env = &app.cfg.ytdlp_env;
        let env_python = bin_dir(env).join(exe("python"));
        if !env_python.exists() {
            tracing::info!("creating yt-dlp environment in {}", env.display());
            run(Command::new(python).args(["-m", "venv"]).arg(env)).await.context("creating the Python environment")?;
        }
        run(Command::new(&env_python).args([
            "-m",
            "pip",
            "install",
            "--upgrade",
            "--disable-pip-version-check",
            "--quiet",
            PACKAGE,
        ]))
        .await
        .context("installing yt-dlp")?;
        Ok(())
    }

    /// Keeps a managed installation current. Sites change their pages often
    /// enough that a yt-dlp a few weeks old is frequently a broken one.
    pub async fn updater(app: App) {
        app.ytdlp.refresh(&app).await;
        if !app.cfg.ytdlp_auto_update || app.cfg.ytdlp_bin.is_some() {
            return;
        }
        loop {
            let status = app.ytdlp.status();
            let last: Option<i64> = app
                .db
                .run(|c| db::get_setting(c, "ytdlp_updated_at"))
                .await
                .ok()
                .flatten()
                .and_then(|t| t.parse().ok());
            let due = last.is_none_or(|t| db::now() - t > UPDATE_EVERY.as_secs() as i64);
            // Only an installation that exists is updated automatically:
            // installing is the operator's choice, made in settings.
            if status.installed && due {
                if let Err(e) = app.ytdlp.install(&app).await {
                    tracing::warn!("yt-dlp update: {e:#}");
                }
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
}

async fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.stdin(Stdio::null()).kill_on_drop(true).output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
        bail!("{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}
