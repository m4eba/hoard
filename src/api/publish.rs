//! The endpoints fiffly calls. They are cross-origin by nature — fiffly runs
//! on its own site, in the user's browser — so all of them answer CORS for
//! any origin; the bearer token or the URL signature is the proof, never the
//! session cookie. fiffly is cross-origin isolated, so everything also says
//! it may be embedded cross-origin.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequest, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{export_folders, media::serve, upload};
use crate::{
    error::{ApiResult, AppError, not_found},
    media, process,
    publish::{self, Signature, oauth, post},
    search::{self, Filter},
    state::App,
};

pub fn router() -> Router<App> {
    Router::new()
        .route("/.well-known/media-publish.json", get(manifest))
        .route("/api/publish/icon.svg", get(icon))
        .route("/api/oauth/token", post(token))
        .route("/api/oauth/revoke-self", post(revoke_self))
        .route("/api/publish/me", get(me))
        .route("/api/publish/search", get(search))
        .route("/api/publish/media/{slug}", get(media_post))
        .route("/api/publish/tags", get(tags))
        .route("/api/publish/folders", get(export_folders::folders))
        .route("/api/publish/upload", post(upload::upload).layer(DefaultBodyLimit::disable()))
        .route("/api/publish/s/{id}/{exp}/{sig}/file/{name}", get(signed_file))
        .route("/api/publish/s/{id}/{exp}/{sig}/thumb", get(signed_thumb))
        .route("/api/publish/s/{id}/{exp}/{sig}/storyboard/{ver}/{n}", get(signed_storyboard))
        .layer(middleware::from_fn(cors))
}

async fn cors(req: Request, next: Next) -> Response {
    let mut res = if req.method() == Method::OPTIONS {
        let mut res = StatusCode::NO_CONTENT.into_response();
        let h = res.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Authorization, Content-Type, Range, X-Anonymous-Key"),
        );
        h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
        // fiffly is a public site and hoard usually sits on this machine or
        // the local network; Chrome asks the local side to opt in.
        h.insert("access-control-allow-private-network", HeaderValue::from_static("true"));
        res
    } else {
        next.run(req).await
    };
    let h = res.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Range, Accept-Ranges, Content-Length"),
    );
    h.insert("cross-origin-resource-policy", HeaderValue::from_static("cross-origin"));
    res
}

/// What fiffly learns about hoard before adding it as a site. Reading the
/// library and saving into it both need a connected account.
async fn manifest() -> Json<Value> {
    Json(json!({
        "version": 1,
        "name": "hoard",
        "icon": "/api/publish/icon.svg",
        "about": {
            "description": "Your own hoard library: open its videos and images in fiffly, and save exports back into it.",
            "visibility": "private",
        },
        "oauth": {
            "authorizationEndpoint": "/oauth/authorize",
            "tokenEndpoint": "/api/oauth/token",
            "revocationEndpoint": "/api/oauth/revoke-self",
            "scopes": publish::SCOPES,
        },
        "endpoints": {
            "me": "/api/publish/me",
            "search": "/api/publish/search",
            "media": "/api/publish/media/{slug}",
            "upload": "/api/publish/upload",
        },
        "capabilities": {
            "searchAuth": "required",
            "anonymousUpload": false,
            "kinds": ["gif", "clip", "image"],
            "formats": upload::FORMATS,
            "maxFiles": upload::MAX_FILES,
            "maxFileBytes": { "authenticated": upload::MAX_FILE_BYTES },
        },
        "uploadForm": [
            { "type": "text", "name": "title", "label": "Title", "maxLength": 200 },
            { "type": "tags", "name": "tags", "label": "Tags", "suggest": "/api/publish/tags", "max": upload::MAX_TAGS },
            { "type": "folder", "name": "folder", "label": "Folder", "list": "/api/publish/folders" },
        ],
        "mediaUrlTemplate": "/m/{slug}",
    }))
}

async fn icon() -> Response {
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#141619"/><path d="M14 24 32 14l18 10v18L32 52 14 42z" fill="none" stroke="#6ea8fe" stroke-width="5" stroke-linejoin="round"/><path d="M14 24l18 10 18-10M32 34v18" fill="none" stroke="#6ea8fe" stroke-width="5" stroke-linejoin="round"/></svg>"##;
    ([(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "public, max-age=86400")], SVG).into_response()
}

/// The token endpoint takes JSON (what fiffly sends) or a form, as OAuth
/// specifies.
async fn token(State(app): State<App>, req: Request) -> Response {
    let form = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    let body = if form {
        axum::Form::<oauth::TokenRequest>::from_request(req, &()).await.map(|f| f.0).ok()
    } else {
        Json::<oauth::TokenRequest>::from_request(req, &()).await.map(|j| j.0).ok()
    };
    let Some(body) = body else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_request" }))).into_response();
    };
    match oauth::exchange(&app, body).await {
        Ok(issued) => Json(json!({
            "access_token": issued.token,
            "token_type": "Bearer",
            "scope": issued.scopes.join(" "),
        }))
        .into_response(),
        Err(code) => (StatusCode::BAD_REQUEST, Json(json!({ "error": code }))).into_response(),
    }
}

/// fiffly's disconnect: the token it is called with stops working.
async fn revoke_self(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let grant = publish::bearer(&app, &headers, "").await?;
    app.db.run(move |c| Ok(c.execute("DELETE FROM api_tokens WHERE id = ?", [grant.id])?)).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn me(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    publish::bearer(&app, &headers, "").await?;
    Ok(Json(json!({ "id": "hoard", "name": "hoard", "anonymous": false })))
}

async fn posts(app: &App, ids: Vec<i64>) -> ApiResult<Vec<Value>> {
    let rows = app.db.run(move |c| post::rows(c, &ids)).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(post::to_json(row, &Signature::new(app, row.id).await?));
    }
    Ok(out)
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

/// The library newest first, or ranked by the query — hoard's own search
/// language, so `tag:` and `folder:` work from fiffly too.
async fn search(State(app): State<App>, headers: HeaderMap, Query(p): Query<SearchParams>) -> ApiResult<Json<Value>> {
    publish::bearer(&app, &headers, publish::SCOPE_READ).await?;
    let filter = Filter { q: p.q.filter(|q| !q.trim().is_empty()), ..Default::default() };
    let (offset, limit) = (p.offset.unwrap_or(0).max(0), p.limit.unwrap_or(24).clamp(1, 100));
    let (items, next) = app.db.run(move |c| media::list(c, &filter, None, offset, limit)).await?;
    let results = posts(&app, items.iter().map(|i| i.id).collect()).await?;
    let mut body = json!({ "results": results });
    if next > 0 {
        body["nextOffset"] = next.into();
    }
    Ok(Json(body))
}

/// One post. A signed slug — what "open in fiffly" links carry — is enough on
/// its own; a plain id needs the account.
async fn media_post(State(app): State<App>, headers: HeaderMap, Path(slug): Path<String>) -> ApiResult<Json<Value>> {
    let (id, sig) = publish::parse_slug(&slug).ok_or_else(|| not_found("media"))?;
    let signed = match &sig {
        Some(sig) => sig.valid(&app).await,
        None => false,
    };
    if !signed {
        publish::bearer(&app, &headers, publish::SCOPE_READ).await?;
    }
    let mut post = posts(&app, vec![id]).await?.pop().ok_or_else(|| not_found("media"))?;
    // Keep answering to the slug fiffly asked for, so its later reads of
    // the post work the same way.
    post["slug"] = slug.into();
    Ok(Json(post))
}

#[derive(Deserialize)]
struct TagParams {
    q: String,
}

async fn tags(State(app): State<App>, headers: HeaderMap, Query(p): Query<TagParams>) -> ApiResult<Json<Value>> {
    publish::bearer(&app, &headers, "").await?;
    let tags = app.db.run(move |c| search::suggest(c, &p.q, 10)).await?;
    Ok(Json(json!({ "tags": tags })))
}

async fn check(app: &App, id: i64, exp: i64, sig: String) -> Result<(), AppError> {
    if (Signature { id, exp, sig }).valid(app).await {
        Ok(())
    } else {
        Err(AppError { status: StatusCode::FORBIDDEN, message: "link expired".into() })
    }
}

const SIGNED_CACHE: &str = "private, max-age=21600";
const IMMUTABLE: &str = "private, max-age=31536000, immutable";

async fn signed_file(
    State(app): State<App>,
    Path((id, exp, sig, _name)): Path<(i64, i64, String, String)>,
    req: Request,
) -> ApiResult<Response> {
    check(&app, id, exp, sig).await?;
    let file = app.db.run(move |c| media::file(c, id)).await?.ok_or_else(|| not_found("media"))?;
    serve(file.path(), req, SIGNED_CACHE).await
}

async fn signed_thumb(
    State(app): State<App>,
    Path((id, exp, sig)): Path<(i64, i64, String)>,
    req: Request,
) -> ApiResult<Response> {
    check(&app, id, exp, sig).await?;
    serve(process::thumb_path(&app.cfg, id), req, IMMUTABLE).await
}

async fn signed_storyboard(
    State(app): State<App>,
    Path((id, exp, sig, _ver, n)): Path<(i64, i64, String, i64, u32)>,
    req: Request,
) -> ApiResult<Response> {
    check(&app, id, exp, sig).await?;
    if n != 0 {
        return Err(not_found("sheet").into());
    }
    serve(process::storyboard_path(&app.cfg, id), req, IMMUTABLE).await
}
