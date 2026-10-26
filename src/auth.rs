//! Optional password protection. There are no accounts: with a password
//! configured, every API and media request needs a session cookie obtained by
//! entering it; without one, the server is open.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use subtle::ConstantTimeEq;

use crate::{
    config::Password,
    db::{self, params},
    error::{ApiResult, AppError},
    state::App,
};

pub const COOKIE: &str = "hoard_session";
const SESSION_DAYS: i64 = 30;
const MAX_FAILURES: u32 = 8;
const LOCKOUT: Duration = Duration::from_secs(15 * 60);

pub struct Auth {
    password: Option<Password>,
    /// Sessions known to be valid, so serving a page of thumbnails is not a
    /// database read per image. The table stays the source of truth.
    cache: Mutex<HashMap<String, i64>>,
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl Auth {
    pub fn new(password: Option<Password>) -> Self {
        Self { password, cache: Default::default(), failures: Default::default() }
    }

    pub fn required(&self) -> bool {
        self.password.is_some()
    }

    fn verify(&self, given: &str) -> bool {
        match &self.password {
            None => true,
            Some(Password::Plain(p)) => p.as_bytes().ct_eq(given.as_bytes()).into(),
            Some(Password::Hash(h)) => PasswordHash::new(h)
                .map(|parsed| Argon2::default().verify_password(given.as_bytes(), &parsed).is_ok())
                .unwrap_or(false),
        }
    }

    fn locked(&self, ip: IpAddr) -> bool {
        let mut map = self.failures.lock().unwrap();
        match map.get(&ip) {
            Some((n, since)) if since.elapsed() > LOCKOUT => {
                let _ = n;
                map.remove(&ip);
                false
            }
            Some((n, _)) => *n >= MAX_FAILURES,
            None => false,
        }
    }

    fn record_failure(&self, ip: IpAddr) {
        let mut map = self.failures.lock().unwrap();
        let entry = map.entry(ip).or_insert((0, Instant::now()));
        entry.0 += 1;
        entry.1 = Instant::now();
    }
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(&format!("{COOKIE}=")).map(str::to_string))
}

async fn session_valid(app: &App, token: &str) -> bool {
    let now = db::now();
    if app.auth.cache.lock().unwrap().get(token).is_some_and(|&exp| exp > now) {
        return true;
    }
    let t = token.to_string();
    let expires: Option<i64> = app
        .db
        .run(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT expires_at FROM sessions WHERE token = ?", [t], |r| r.get(0)).optional()?)
        })
        .await
        .ok()
        .flatten();
    match expires {
        Some(exp) if exp > now => {
            app.auth.cache.lock().unwrap().insert(token.to_string(), exp);
            true
        }
        _ => false,
    }
}

pub async fn is_authenticated(app: &App, headers: &HeaderMap) -> bool {
    if !app.auth.required() {
        return true;
    }
    match session_token(headers) {
        Some(token) => session_valid(app, &token).await,
        None => false,
    }
}

/// Middleware for everything that is not public.
pub async fn require(State(app): State<App>, req: Request, next: Next) -> Response {
    if is_authenticated(&app, req.headers()).await {
        return next.run(req).await;
    }
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": "sign in required" }))).into_response()
}

pub async fn config(State(app): State<App>, headers: HeaderMap) -> Json<serde_json::Value> {
    Json(json!({
        "authRequired": app.auth.required(),
        "authenticated": is_authenticated(&app, &headers).await,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Deserialize)]
pub struct Login {
    password: String,
}

pub async fn login(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<Login>,
) -> ApiResult<Response> {
    let ip = addr.ip();
    if app.auth.locked(ip) {
        return Err(AppError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: "too many failed attempts; try again later".into(),
        });
    }
    if !app.auth.verify(&body.password) {
        app.auth.record_failure(ip);
        // A fixed delay makes guessing slow without needing to be clever.
        tokio::time::sleep(Duration::from_millis(600)).await;
        return Err(AppError { status: StatusCode::UNAUTHORIZED, message: "wrong password".into() });
    }
    app.auth.failures.lock().unwrap().remove(&ip);

    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let expires = db::now() + SESSION_DAYS * 86_400;
    let t = token.clone();
    app.db
        .run(move |c| {
            c.execute("DELETE FROM sessions WHERE expires_at < ?", [db::now()])?;
            c.execute("INSERT INTO sessions (token, expires_at) VALUES (?, ?)", params![t, expires])?;
            Ok(())
        })
        .await?;
    let cookie = format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        SESSION_DAYS * 86_400,
        if app.cfg.secure_cookies { "; Secure" } else { "" }
    );
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    Ok(res)
}

pub async fn logout(State(app): State<App>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = session_token(&headers) {
        app.auth.cache.lock().unwrap().remove(&token);
        app.db.run(move |c| Ok(c.execute("DELETE FROM sessions WHERE token = ?", [token])?)).await?;
    }
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("hoard_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    Ok(res)
}

/// `hoard hash-password`: reads a password and prints its argon2 hash for
/// HOARD_PASSWORD_HASH, so the plain password need not sit in the environment.
pub fn hash_password_cli() -> anyhow::Result<()> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    eprint!("Password: ");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let password = line.trim_end_matches(['\n', '\r']);
    if password.is_empty() {
        anyhow::bail!("empty password");
    }
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let salt = SaltString::encode_b64(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{hash}");
    Ok(())
}
