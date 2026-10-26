//! OAuth 2.1 authorization code with mandatory PKCE (S256), for one public
//! client: fiffly. Codes live a minute and only in memory — a restart in
//! between costs a click on "connect" again. Tokens do not expire; they are
//! revoked from Settings or by fiffly's disconnect.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::Result;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{CLIENT_ID, SCOPES, random_hex, sha256_hex};
use crate::{
    db::{self, params},
    error::{Reject, bad_request},
    state::App,
};

const CODE_TTL: Duration = Duration::from_secs(60);
/// A second exchange of a code this soon, with the right verifier, is the
/// same client retrying a request whose answer it lost — not a replay.
const RETRY_WINDOW: Duration = Duration::from_secs(10);
const CALLBACK: &str = "/publish/callback/";

#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    pub response_type: Option<String>,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
}

/// An authorization request that may be shown to the user.
pub struct Valid {
    pub scopes: Vec<String>,
    /// The host the grant goes back to, shown on the consent page.
    pub redirect_host: String,
}

/// Checks a request before anything is shown or granted. The redirect must
/// be the configured fiffly's callback, exactly: anything else would hand a
/// code to whoever wrote the link.
pub fn validate(req: &AuthorizeRequest, fiffly_url: &str) -> Result<Valid, Reject> {
    if req.response_type.as_deref().is_some_and(|t| t != "code") {
        return Err(bad_request("unsupported response_type"));
    }
    if req.client_id != CLIENT_ID {
        return Err(bad_request(format!("unknown client {:?}", req.client_id)));
    }
    let allowed = format!("{}{CALLBACK}", fiffly_url.trim_end_matches('/'));
    let site = req.redirect_uri.strip_prefix(&allowed).unwrap_or_default();
    if site.is_empty() || site.contains(['/', '?', '#']) {
        return Err(bad_request(format!(
            "redirect_uri must be {allowed}<host>; set the fiffly address in Settings if fiffly runs elsewhere"
        )));
    }
    match (req.code_challenge.as_deref(), req.code_challenge_method.as_deref()) {
        (Some(c), Some("S256")) if (43..=128).contains(&c.len()) => {}
        _ => return Err(bad_request("a PKCE code_challenge with method S256 is required")),
    }
    let requested: Vec<String> = req.scope.as_deref().unwrap_or("").split_whitespace().map(String::from).collect();
    if let Some(bad) = requested.iter().find(|s| !SCOPES.contains(&s.as_str())) {
        return Err(bad_request(format!("unknown scope {bad:?}")));
    }
    let scopes = if requested.is_empty() { SCOPES.iter().map(|s| s.to_string()).collect() } else { requested };
    let host = fiffly_url.split("://").nth(1).unwrap_or(fiffly_url).trim_end_matches('/').to_string();
    Ok(Valid { scopes, redirect_host: host })
}

struct Code {
    redirect_uri: String,
    challenge: String,
    scopes: Vec<String>,
    created: Instant,
    /// When it was exchanged, and the token that minted.
    used: Option<(Instant, i64)>,
}

#[derive(Default)]
pub struct Codes(Mutex<HashMap<String, Code>>);

impl Codes {
    /// Mints a code for an approved request.
    pub fn issue(&self, req: &AuthorizeRequest, scopes: Vec<String>) -> String {
        let code = random_hex(24);
        let mut map = self.0.lock().unwrap();
        map.retain(|_, c| c.created.elapsed() < Duration::from_secs(86_400));
        map.insert(
            sha256_hex(&code),
            Code {
                redirect_uri: req.redirect_uri.clone(),
                challenge: req.code_challenge.clone().unwrap_or_default(),
                scopes,
                created: Instant::now(),
                used: None,
            },
        );
        code
    }
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub code_verifier: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
}

pub struct Issued {
    pub token: String,
    pub scopes: Vec<String>,
}

fn b64url(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// Swaps a code for an access token. Errors are OAuth's error codes.
pub async fn exchange(app: &App, req: TokenRequest) -> Result<Issued, &'static str> {
    if req.grant_type.as_deref() != Some("authorization_code") {
        return Err("unsupported_grant_type");
    }
    let (Some(code), Some(verifier)) = (req.code, req.code_verifier) else { return Err("invalid_request") };
    let hash = sha256_hex(&code);
    let proof = b64url(&Sha256::digest(verifier.as_bytes()));

    // Err carries the token to revoke, if the attempt was a replay.
    let checked: Result<Vec<String>, Option<i64>> = {
        let mut map = app.publish.codes.0.lock().unwrap();
        match map.get_mut(&hash) {
            None => Err(None),
            Some(c) => {
                let genuine = req.client_id.as_deref() == Some(CLIENT_ID)
                    && req.redirect_uri.as_deref() == Some(c.redirect_uri.as_str())
                    && proof == c.challenge;
                match c.used {
                    // A replayed code means it leaked: the token it minted goes too.
                    Some((at, token)) => Err((!(genuine && at.elapsed() < RETRY_WINDOW)).then_some(token)),
                    None if c.created.elapsed() > CODE_TTL || !genuine => Err(None),
                    None => {
                        c.used = Some((Instant::now(), 0));
                        Ok(c.scopes.clone())
                    }
                }
            }
        }
    };
    let scopes = match checked {
        Ok(scopes) => scopes,
        Err(revoke) => {
            if let Some(token) = revoke {
                let _ = app.db.run(move |c| Ok(c.execute("DELETE FROM api_tokens WHERE id = ?", [token])?)).await;
            }
            return Err("invalid_grant");
        }
    };

    let token = random_hex(32);
    let (t, s) = (sha256_hex(&token), scopes.join(" "));
    let id = app
        .db
        .run(move |c| {
            c.execute(
                "INSERT INTO api_tokens (token_hash, client, scopes, created_at) VALUES (?, ?, ?, ?)",
                params![t, CLIENT_ID, s, db::now()],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
        .map_err(|_| "server_error")?;
    if let Some(c) = app.publish.codes.0.lock().unwrap().get_mut(&hash) {
        c.used = Some((Instant::now(), id));
    }
    Ok(Issued { token, scopes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(redirect: &str) -> AuthorizeRequest {
        AuthorizeRequest {
            response_type: Some("code".into()),
            client_id: "fiffly".into(),
            redirect_uri: redirect.into(),
            scope: Some("media:read".into()),
            state: None,
            code_challenge: Some("a".repeat(43)),
            code_challenge_method: Some("S256".into()),
        }
    }

    #[test]
    fn redirect_must_be_the_configured_callback() {
        let ok = validate(&req("https://fiffly.com/publish/callback/127.0.0.1:8150"), "https://fiffly.com/");
        assert_eq!(ok.unwrap().scopes, vec!["media:read"]);
        for bad in [
            "https://evil.example/publish/callback/x",
            "https://fiffly.com.evil.example/publish/callback/x",
            "https://fiffly.com/publish/callback/",
            "https://fiffly.com/publish/callback/x/../../steal",
            "https://fiffly.com/other",
        ] {
            assert!(validate(&req(bad), "https://fiffly.com").is_err(), "{bad}");
        }
    }

    #[test]
    fn pkce_challenge_encoding() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64url(&Sha256::digest(verifier)), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
