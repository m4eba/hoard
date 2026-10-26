//! The HTTP surface. Everything except signing in and the frontend itself sits
//! behind the password check when one is configured.

mod imports;
pub mod library;
mod media;
mod system;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, patch, post},
};
use tower_http::trace::TraceLayer;

use crate::{auth, state::App, web};

pub fn router(app: App) -> Router {
    let protected = Router::new()
        .route("/api/media", get(media::list).patch(media::bulk_update))
        .route("/api/media/count", post(media::count))
        .route("/api/media/delete", post(media::bulk_delete))
        .route("/api/media/move", post(media::bulk_move))
        .route("/api/media/{id}", get(media::get).patch(media::update).delete(media::delete))
        .route("/api/media/{id}/storyboard", get(media::storyboard).post(media::make_storyboard))
        .route("/api/tags", get(media::tags))
        .route("/api/tags/suggest", get(media::suggest))
        .route("/api/upload", post(media::upload).layer(DefaultBodyLimit::disable()))
        .route("/api/roots", get(library::roots).post(library::add_root))
        .route("/api/roots/{id}", patch(library::rename_root).delete(library::remove_root))
        .route("/api/roots/{id}/scan", post(library::rescan))
        .route(
            "/api/folders",
            get(library::folders).post(library::create_folder).delete(library::delete_folder),
        )
        .route("/api/fs", get(library::browse))
        .route("/api/stats", get(library::stats))
        .route("/api/import", get(imports::recent).post(imports::start))
        .route("/api/import/{batch}", get(imports::batch).patch(imports::patch_batch))
        .route("/api/import/{batch}/events", get(imports::batch_events))
        .route("/api/import/{batch}/job/{job}", patch(imports::patch_job).delete(imports::cancel_job))
        .route("/api/import/{batch}/job/{job}/retry", post(imports::retry_job))
        .route("/api/events", get(system::events))
        .route("/api/tasks", get(system::tasks))
        .route("/api/settings", get(system::settings).put(system::save_settings))
        .route("/api/ytdlp", get(system::ytdlp_status))
        .route("/api/ytdlp/install", post(system::ytdlp_install))
        .route("/api/trash", get(system::trash).delete(system::empty_trash))
        .route("/f/{id}", get(media::serve_file))
        .route("/t/{id}", get(media::serve_thumb))
        .route("/s/{id}", get(media::serve_storyboard))
        .route_layer(middleware::from_fn_with_state(app.clone(), auth::require));

    Router::new()
        .route("/api/config", get(auth::config))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .merge(protected)
        .fallback(web::static_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(app)
}
