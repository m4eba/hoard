//! hoard's own side of the fiffly integration, behind the usual sign-in: the
//! consent screen's checks and answer, the list of connected browsers, and
//! the signed slugs "open in fiffly" links carry.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::{ApiResult, not_found},
    media,
    publish::{self, Signature, oauth::AuthorizeRequest},
    state::App,
};

async fn validate(app: &App, req: &AuthorizeRequest) -> ApiResult<publish::oauth::Valid> {
    let fiffly = app.db.run(|c| publish::fiffly_url(c)).await?;
    Ok(publish::oauth::validate(req, &fiffly)?)
}

/// What the consent screen shows; refuses a request it must not show.
pub async fn check(State(app): State<App>, Query(req): Query<AuthorizeRequest>) -> ApiResult<Json<Value>> {
    let valid = validate(&app, &req).await?;
    Ok(Json(json!({ "client": publish::CLIENT_ID, "scopes": valid.scopes, "redirectHost": valid.redirect_host })))
}

#[derive(Deserialize)]
pub struct Answer {
    #[serde(flatten)]
    req: AuthorizeRequest,
    approve: bool,
}

/// Percent-encodes a query value.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The user's answer on the consent screen, as where to send the browser.
/// JSON only: a cross-site form cannot post it, and no CORS is offered.
pub async fn answer(State(app): State<App>, Json(a): Json<Answer>) -> ApiResult<Json<Value>> {
    let valid = validate(&app, &a.req).await?;
    let state = a.req.state.as_deref().map(|s| format!("&state={}", enc(s))).unwrap_or_default();
    let sep = if a.req.redirect_uri.contains('?') { '&' } else { '?' };
    let to = if a.approve {
        let code = app.publish.codes.issue(&a.req, valid.scopes);
        format!("{}{sep}code={}{state}", a.req.redirect_uri, enc(&code))
    } else {
        format!("{}{sep}error=access_denied{state}", a.req.redirect_uri)
    };
    Ok(Json(json!({ "redirectTo": to })))
}

pub async fn connections(State(app): State<App>) -> ApiResult<Json<Value>> {
    let rows = app
        .db
        .run(|c| {
            let mut stmt =
                c.prepare("SELECT id, client, scopes, created_at, last_used FROM api_tokens ORDER BY id DESC")?;
            let rows = stmt
                .query_map([], |r| {
                    let last: Option<i64> = r.get(4)?;
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?,
                        "client": r.get::<_, String>(1)?,
                        "scopes": r.get::<_, String>(2)?.split(' ').collect::<Vec<_>>(),
                        "createdAt": media::iso(r.get(3)?),
                        "lastUsed": last.map(media::iso),
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(json!({ "connections": rows })))
}

pub async fn revoke(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let n = app.db.run(move |c| Ok(c.execute("DELETE FROM api_tokens WHERE id = ?", [id])?)).await?;
    if n == 0 {
        return Err(not_found("connection").into());
    }
    Ok(Json(json!({ "ok": true })))
}

/// A slug that opens this media in fiffly for a few hours, connected or not.
pub async fn link(State(app): State<App>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let exists = app.db.run(move |c| media::file(c, id)).await?.is_some();
    if !exists {
        return Err(not_found("media").into());
    }
    Ok(Json(json!({ "slug": Signature::new(&app, id).await?.slug() })))
}
