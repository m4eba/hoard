mod api;
mod auth;
mod config;
mod db;
mod error;
mod events;
mod imports;
mod media;
mod ops;
mod paths;
mod process;
mod scan;
mod search;
mod state;
mod web;
mod ytdlp;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "hoard — a self-hosted media library

USAGE:
    hoard [serve]          run the server (default)
    hoard hash-password    print an argon2 hash for HOARD_PASSWORD_HASH

Configuration is read from the environment; see README.md.";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_env("HOARD_LOG").unwrap_or_else(|_| EnvFilter::new("hoard=info,tower_http=warn")))
        .init();

    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("hash-password") => auth::hash_password_cli(),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("hoard {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => anyhow::bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

async fn serve() -> Result<()> {
    let cfg = config::Config::from_env()?;
    for dir in [cfg.data_dir.clone(), cfg.thumbs_dir(), cfg.storyboards_dir(), cfg.meta_dir(), cfg.tmp_dir()] {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // Leftovers from downloads and storyboards interrupted by a restart.
    if let Ok(entries) = std::fs::read_dir(cfg.tmp_dir()) {
        for e in entries.flatten() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    check_tool(&cfg.ffmpeg, "-version").await;
    check_tool(&cfg.ffprobe, "-version").await;

    let db = db::Db::open(&cfg.db_path())?;
    let listen = cfg.listen;
    let initial_roots = cfg.initial_roots.clone();
    if cfg.password.is_none() && !listen.ip().is_loopback() {
        tracing::warn!("listening on {listen} without a password; set HOARD_PASSWORD to require one");
    }
    let app = state::AppState::new(cfg, db);

    for path in initial_roots {
        let known: bool = {
            let p = path.canonicalize().unwrap_or(path.clone()).display().to_string();
            app.db.run(move |c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM roots WHERE path = ?)", [p], |r| r.get(0))?)).await?
        };
        if !known {
            if let Err(e) = api::library::add_root_path(&app, path.clone(), None).await {
                tracing::warn!("HOARD_ROOTS {}: {}", path.display(), e.message);
            }
        }
    }

    tokio::spawn(process::worker(app.clone()));
    tokio::spawn(imports::dispatcher(app.clone()));
    tokio::spawn(scan::scheduler(app.clone()));
    tokio::spawn(ytdlp::Ytdlp::updater(app.clone()));

    let listener = tokio::net::TcpListener::bind(listen).await.with_context(|| format!("binding {listen}"))?;
    tracing::info!("listening on http://{listen}");
    axum::serve(listener, api::router(app).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Missing ffmpeg is not fatal — the library still lists and plays files — but
/// it is the first thing to check when thumbnails never appear.
async fn check_tool(bin: &str, arg: &str) {
    let ok = tokio::process::Command::new(bin).arg(arg).output().await.is_ok_and(|o| o.status.success());
    if !ok {
        tracing::warn!("{bin} not found; thumbnails, probing and storyboards will fail");
    }
}
