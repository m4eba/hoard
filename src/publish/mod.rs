//! fiffly, a browser video editor, reaches the library over its media-publish
//! protocol: it lists and searches media, opens them in its editor, and saves
//! exports back into a folder. Access is granted per browser through OAuth
//! (authorization code + PKCE); media bytes go out on signed URLs, because a
//! video element cannot send a bearer header.
//!
//! This module holds what the handlers share: the signing key, access tokens
//! and authorization codes. `post` maps media rows onto the protocol's posts.

pub mod oauth;
pub mod post;

use std::sync::OnceLock;

use anyhow::Result;
use axum::http::{HeaderMap, StatusCode, header};
use hmac::{Hmac, KeyInit, Mac};
use rand::RngCore;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

use crate::{
    db::{self, params},
    error::AppError,
    state::App,
};

/// The OAuth client id fiffly uses with every site.
pub const CLIENT_ID: &str = "fiffly";
pub const SCOPE_READ: &str = "media:read";
pub const SCOPE_WRITE: &str = "media:write";
pub const SCOPES: [&str; 2] = [SCOPE_READ, SCOPE_WRITE];
pub const DEFAULT_FIFFLY_URL: &str = "https://fiffly.com";

/// Signed URLs last six hours, with the expiry rounded up to the quarter
/// hour: every URL handed out within one window is the same URL, so fiffly's
/// cache of thumbnails and sheets keeps hitting.
const SIGNED_SECS: i64 = 6 * 3600;
const SIGNED_ROUND: i64 = 15 * 60;

#[derive(Default)]
pub struct Publish {
    key: OnceLock<Vec<u8>>,
    pub codes: oauth::Codes,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

pub fn sha256_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// The signing key, made on first use and kept in the settings table so
/// URLs survive a restart.
async fn key(app: &App) -> Result<&[u8]> {
    if let Some(k) = app.publish.key.get() {
        return Ok(k);
    }
    let stored = app
        .db
        .run(|c| {
            c.execute("INSERT OR IGNORE INTO settings (key, value) VALUES ('publish_key', ?)", [random_hex(32)])?;
            Ok(db::get_setting(c, "publish_key")?.unwrap_or_default())
        })
        .await?;
    let _ = app.publish.key.set(stored.into_bytes());
    Ok(app.publish.key.get().expect("just set"))
}

fn mac(key: &[u8], id: i64, exp: i64) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    m.update(format!("{id}.{exp}").as_bytes());
    hex(&m.finalize().into_bytes()[..16])
}

/// A grant to read one media item until `exp`.
#[derive(Debug, Clone)]
pub struct Signature {
    pub id: i64,
    pub exp: i64,
    pub sig: String,
}

impl Signature {
    pub async fn new(app: &App, id: i64) -> Result<Self> {
        let exp = (db::now() + SIGNED_SECS + SIGNED_ROUND - 1) / SIGNED_ROUND * SIGNED_ROUND;
        Ok(Self { id, exp, sig: mac(key(app).await?, id, exp) })
    }

    /// Whether the signature is genuine and still current.
    pub async fn valid(&self, app: &App) -> bool {
        if self.exp < db::now() {
            return false;
        }
        let Ok(key) = key(app).await else { return false };
        use subtle::ConstantTimeEq;
        mac(key, self.id, self.exp).as_bytes().ct_eq(self.sig.as_bytes()).into()
    }

    /// The prefix of every signed file URL of this media.
    pub fn base(&self) -> String {
        format!("/api/publish/s/{}/{}/{}", self.id, self.exp, self.sig)
    }

    /// A post slug that opens the media without a connected account: the
    /// "open in fiffly" links carry one.
    pub fn slug(&self) -> String {
        format!("{}-{}-{}", self.id, self.exp, self.sig)
    }
}

/// A slug is a media id, or a signed one (`id-exp-sig`).
pub fn parse_slug(slug: &str) -> Option<(i64, Option<Signature>)> {
    let mut parts = slug.splitn(3, '-');
    let id = parts.next()?.parse().ok()?;
    match (parts.next(), parts.next()) {
        (None, _) => Some((id, None)),
        (Some(exp), Some(sig)) => Some((id, Some(Signature { id, exp: exp.parse().ok()?, sig: sig.to_string() }))),
        _ => None,
    }
}

/// The access token a request carries, if it is one we issued.
pub struct Grant {
    pub id: i64,
}

fn reject(status: StatusCode, error: &str) -> AppError {
    AppError { status, message: error.into() }
}

/// Checks the bearer token and that it was granted `scope` (any, if empty);
/// the errors are OAuth's codes, which fiffly reads.
pub async fn bearer(app: &App, headers: &HeaderMap, scope: &str) -> Result<Grant, AppError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| reject(StatusCode::UNAUTHORIZED, "invalid_token"))?;
    let hash = sha256_hex(token);
    let found: Option<(i64, String)> = app
        .db
        .run(move |c| {
            let row = c
                .query_row("SELECT id, scopes FROM api_tokens WHERE token_hash = ?", [&hash], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            if let Some((id, _)) = &row {
                c.execute("UPDATE api_tokens SET last_used = ? WHERE id = ?", params![db::now(), id])?;
            }
            Ok(row)
        })
        .await?;
    let (id, scopes) = found.ok_or_else(|| reject(StatusCode::UNAUTHORIZED, "invalid_token"))?;
    if !scope.is_empty() && !scopes.split(' ').any(|s| s == scope) {
        return Err(reject(StatusCode::FORBIDDEN, "insufficient_scope"));
    }
    Ok(Grant { id })
}

/// The fiffly instance allowed to ask for access, from the settings.
pub fn fiffly_url(c: &rusqlite::Connection) -> Result<String> {
    Ok(db::get_setting(c, "fiffly_url")?.unwrap_or_else(|| DEFAULT_FIFFLY_URL.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert!(matches!(parse_slug("42"), Some((42, None))));
        let (id, sig) = parse_slug("42-1700000000-abcd").unwrap();
        assert_eq!(id, 42);
        let sig = sig.unwrap();
        assert_eq!((sig.exp, sig.sig.as_str()), (1_700_000_000, "abcd"));
        assert!(parse_slug("42-17").is_none());
        assert!(parse_slug("x").is_none());
    }

    #[test]
    fn signatures_bind_id_and_expiry() {
        let key = b"k";
        assert_eq!(mac(key, 1, 10), mac(key, 1, 10));
        assert_ne!(mac(key, 1, 10), mac(key, 2, 10));
        assert_ne!(mac(key, 1, 10), mac(key, 1, 11));
    }
}
