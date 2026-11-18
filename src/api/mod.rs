//! HTTP routing: JSON API, WebSocket console and the embedded web UI.

pub mod auth;
pub mod backups;
pub mod files;
pub mod servers;
pub mod system;
pub mod updates;
pub mod users;
pub mod ws;

use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        // ------------------------------------------------------------------ auth
        .route("/api/auth/status", get(auth::status))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/session", get(auth::session_info))
        .route("/api/auth/password", post(auth::change_password))
        // ------------------------------------------------------------- dashboard
        .route("/api/dashboard", get(system::dashboard))
        .route("/api/system", get(system::system_info))
        .route("/api/system/history", get(system::history))
        .route("/api/system/runtimes", get(system::runtimes))
        .route("/api/system/info", get(system::about))
        .route("/api/activity", get(system::activity))
        // --------------------------------------------------------------- servers
        .route("/api/servers", get(servers::list).post(servers::create))
        .route(
            "/api/servers/:id",
            get(servers::get)
                .patch(servers::update)
                .delete(servers::delete),
        )
        .route("/api/servers/:id/action", post(servers::action))
        .route("/api/servers/:id/install", post(servers::install))
        .route("/api/servers/:id/console", get(servers::console))
        .route(
            "/api/servers/:id/console/input",
            post(servers::console_input),
        )
        .route(
            "/api/servers/:id/console/search",
            get(servers::console_search),
        )
        .route(
            "/api/servers/:id/console/clear",
            post(servers::clear_console),
        )
        .route("/api/servers/:id/activity", get(servers::server_activity))
        .route("/api/servers/:id/ws", get(ws::console_socket))
        .route("/api/events", get(ws::events_socket))
        // ----------------------------------------------------------------- files
        .route("/api/servers/:id/files", get(files::list))
        .route(
            "/api/servers/:id/files/content",
            get(files::read_file).put(files::write_file),
        )
        .route("/api/servers/:id/files/mkdir", post(files::mkdir))
        .route("/api/servers/:id/files/touch", post(files::touch))
        .route("/api/servers/:id/files/rename", post(files::rename))
        .route("/api/servers/:id/files/copy", post(files::copy))
        .route("/api/servers/:id/files/delete", post(files::remove))
        .route("/api/servers/:id/files/chmod", post(files::chmod))
        .route("/api/servers/:id/files/search", get(files::search))
        .route("/api/servers/:id/files/upload", put(files::upload))
        .route("/api/servers/:id/files/download", get(files::download))
        // --------------------------------------------------------------- backups
        .route(
            "/api/servers/:id/backups",
            get(backups::list).post(backups::create),
        )
        .route("/api/servers/:id/backups/:backup", delete(backups::delete))
        .route(
            "/api/servers/:id/backups/:backup/restore",
            post(backups::restore),
        )
        // --------------------------------------------------------------- users
        .route("/api/users", get(users::list).post(users::create))
        .route("/api/users/:id", patch(users::update).delete(users::delete))
        .route("/api/users/:id/access", post(users::set_access))
        // ------------------------------------------------------------ templates
        .route("/api/templates", get(system::templates))
        .route("/api/templates/reload", post(system::reload_templates))
        // ------------------------------------------------------------- settings
        .route(
            "/api/settings",
            get(system::get_settings).put(system::save_settings),
        )
        // -------------------------------------------------------------- updates
        .route("/api/updates/check", get(updates::check))
        .route("/api/updates/download", post(updates::download))
        .route("/api/updates/install", post(updates::install))
        .fallback(api_not_found)
        .layer(middleware::from_fn_with_state(state.clone(), csrf_guard))
        .with_state(state);

    Router::new()
        .merge(api)
        .fallback(static_assets)
        .layer(middleware::map_response(security_headers))
}

async fn api_not_found() -> AppError {
    AppError::not_found("Unknown API endpoint")
}

/// Reject cross-site mutations: the request must carry the per-session CSRF
/// token and, when a browser sends an Origin header, it must match the host.
async fn csrf_guard(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    if matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(request).await;
    }
    let path = request.uri().path().to_string();
    if path == "/api/auth/login" || path == "/api/auth/setup" {
        return next.run(request).await;
    }

    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let origin = origin.to_str().unwrap_or_default();
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let origin_host = origin
            .split("://")
            .nth(1)
            .unwrap_or(origin)
            .trim_end_matches('/');
        if !host.is_empty() && !origin_host.eq_ignore_ascii_case(host) {
            return AppError::forbidden("Cross-origin requests are not allowed").into_response();
        }
    }

    let Some(session_id) = crate::auth::session_id_from_headers(request.headers()) else {
        return AppError::unauthorized("Sign in required").into_response();
    };
    let session = match state.db.session(&session_id) {
        Ok(Some(session)) => session,
        Ok(None) => return AppError::unauthorized("Session expired").into_response(),
        Err(err) => return err.into_response(),
    };
    if session.expires_at <= crate::util::now_ms() {
        return AppError::unauthorized("Session expired").into_response();
    }
    let token = request
        .headers()
        .get(crate::auth::CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if token.is_empty() || token != session.csrf {
        return AppError::forbidden("Invalid or missing CSRF token").into_response();
    }
    next.run(request).await
}

async fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self' ws: wss:; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(rust_embed::RustEmbed)]
#[folder = "web/"]
struct WebAssets;

async fn static_assets(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let candidate = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = WebAssets::get(candidate) {
        return asset_response(candidate, file.data.into_owned());
    }
    // Single-page app fallback for client-side routes.
    match WebAssets::get("index.html") {
        Some(file) => asset_response("index.html", file.data.into_owned()),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "The embedded web interface is missing from this build.",
        )
            .into_response(),
    }
}

fn asset_response(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_for(path);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        data,
    )
        .into_response()
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
