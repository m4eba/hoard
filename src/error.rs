//! The one error type handlers return. Every failure answers `{ "error": … }`
//! so the UI can show the server's message instead of a generic apology.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// What a handler returns on failure. Anything convertible into
/// `anyhow::Error` turns into one, as a 500 unless it carries a `Reject`.
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
}

pub type ApiResult<T> = Result<T, AppError>;

/// A failure with a deliberate status. It is a real error type so it can
/// travel through `anyhow` out of blocking database closures and still come
/// out the other side as a 400 or 404 rather than a 500.
#[derive(Debug)]
pub struct Reject {
    pub status: StatusCode,
    pub message: String,
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Reject {}

pub fn bad_request(message: impl Into<String>) -> Reject {
    Reject { status: StatusCode::BAD_REQUEST, message: message.into() }
}

pub fn not_found(what: &str) -> Reject {
    Reject { status: StatusCode::NOT_FOUND, message: format!("{what} not found") }
}

pub fn conflict(message: impl Into<String>) -> Reject {
    Reject { status: StatusCode::CONFLICT, message: message.into() }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        let err = err.into();
        match err.downcast::<Reject>() {
            Ok(r) => Self { status: r.status, message: r.message },
            Err(err) => {
                tracing::error!("{err:#}");
                Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("{err:#}") }
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
