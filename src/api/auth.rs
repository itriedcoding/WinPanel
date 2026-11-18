//! Sign-in, first-run setup, session and password endpoints.

use crate::auth::{
    clear_cookie, hash_password, new_csrf_token, new_session_token, password_problem,
    session_cookie, session_id_from_headers, verify_password, AuthUser,
};
use crate::error::{AppError, Result};
use crate::models::{User, ROLE_ADMIN};
use crate::state::AppState;
use crate::util::{now_ms, sha256_hex, uuid_str, valid_username};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub user: User,
    pub csrf: String,
    pub expires_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub setup_required: bool,
    pub authenticated: bool,
    pub user: Option<User>,
    pub csrf: Option<String>,
    pub version: String,
    pub platform: String,
    pub remote_access: bool,
    pub allow_registration: bool,
    pub uptime_secs: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePassword {
    pub current_password: String,
    pub new_password: String,
}

pub async fn status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<StatusResponse>> {
    let setup_required = state.db.count_users()? == 0;
    let mut user = None;
    let mut csrf = None;
    if let Some(session_id) = session_id_from_headers(&headers) {
        if let Some(session) = state.db.session(&session_id)? {
            if session.expires_at > now_ms() {
                if let Some(found) = state.db.user_by_id(&session.user_id)? {
                    if !found.disabled {
                        csrf = Some(session.csrf.clone());
                        user = Some(found);
                    }
                }
            }
        }
    }
    Ok(Json(StatusResponse {
        setup_required,
        authenticated: user.is_some(),
        user,
        csrf,
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        remote_access: !state.listen.ip().is_loopback(),
        allow_registration: state.settings().allow_registration,
        uptime_secs: state.uptime_secs(),
    }))
}

/// Create the first administrator account. Only available while no users exist.
pub async fn setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Credentials>,
) -> Result<Response> {
    if state.db.count_users()? > 0 {
        return Err(AppError::conflict("WinPanel has already been set up"));
    }
    if !valid_username(&body.username) {
        return Err(AppError::bad_request(
            "Username must be 3-32 characters: letters, digits, dot, dash or underscore",
        ));
    }
    if let Some(problem) = password_problem(&body.password) {
        return Err(AppError::bad_request(problem));
    }
    let hash = hash_password(&body.password)?;
    let id = uuid_str();
    state
        .db
        .create_user(&id, &body.username, &hash, ROLE_ADMIN)?;
    let user = state
        .db
        .user_by_id(&id)?
        .ok_or_else(|| AppError::internal("Account could not be created"))?;
    state.db.log_activity(
        "auth",
        None,
        Some(&user.id),
        "Administrator account created",
        None,
    )?;
    let (cookie, response) = start_session(&state, &user)?;
    Ok(respond(StatusCode::CREATED, response, Some(cookie)))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Credentials>,
) -> Result<Response> {
    let key = body.username.trim().to_lowercase();
    let guard = state.login_limiter.guard(&key);
    guard.check()?;

    let credential = state.db.find_credential(body.username.trim())?;
    let Some(credential) = credential else {
        guard.failure();
        return Err(AppError::unauthorized("Invalid username or password"));
    };
    if !verify_password(&credential.password_hash, &body.password) {
        guard.failure();
        let _ = state.db.log_activity(
            "auth",
            None,
            Some(&credential.user.id),
            "Failed sign-in attempt",
            None,
        );
        return Err(AppError::unauthorized("Invalid username or password"));
    }
    if credential.user.disabled {
        return Err(AppError::forbidden("This account has been disabled"));
    }
    guard.success();
    state.db.touch_login(&credential.user.id)?;
    state.db.log_activity(
        "auth",
        None,
        Some(&credential.user.id),
        &format!("{} signed in", credential.user.username),
        None,
    )?;
    let user = state
        .db
        .user_by_id(&credential.user.id)?
        .unwrap_or(credential.user);
    let (cookie, response) = start_session(&state, &user)?;
    Ok(respond(StatusCode::OK, response, Some(cookie)))
}

pub async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(session_id) = session_id_from_headers(&headers) {
        let _ = state.db.delete_session(&session_id);
    }
    respond(
        StatusCode::OK,
        serde_json::json!({ "ok": true }),
        Some(clear_cookie()),
    )
}

pub async fn session_info(
    State(_state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<Json<AuthResponse>> {
    Ok(Json(AuthResponse {
        user: auth.user,
        csrf: auth.session.csrf,
        expires_at: auth.session.expires_at,
    }))
}

pub async fn change_password(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<ChangePassword>,
) -> Result<Json<serde_json::Value>> {
    let credential = state
        .db
        .find_credential(&auth.user.username)?
        .ok_or_else(|| AppError::unauthorized("Account no longer exists"))?;
    if !verify_password(&credential.password_hash, &body.current_password) {
        return Err(AppError::bad_request("Current password is incorrect"));
    }
    if let Some(problem) = password_problem(&body.new_password) {
        return Err(AppError::bad_request(problem));
    }
    let hash = hash_password(&body.new_password)?;
    state.db.update_password(&auth.user.id, &hash)?;
    let keep = session_id_from_headers(&headers);
    state
        .db
        .delete_user_sessions(&auth.user.id, keep.as_deref())?;
    state
        .db
        .log_activity("auth", None, Some(&auth.user.id), "Password changed", None)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

fn start_session(state: &Arc<AppState>, user: &User) -> Result<(String, AuthResponse)> {
    let token = new_session_token();
    let session_id = sha256_hex(&token);
    let csrf = new_csrf_token();
    let hours = state.settings().session_hours.max(1) as i64;
    let expires_at = now_ms() + hours * 3_600_000;
    state
        .db
        .create_session(&session_id, &user.id, &csrf, expires_at, None, None)?;
    state.db.purge_expired_sessions().ok();
    let cookie = session_cookie(&token, hours * 3600, false);
    Ok((
        cookie,
        AuthResponse {
            user: user.clone(),
            csrf,
            expires_at,
        },
    ))
}

fn respond<T: Serialize>(status: StatusCode, body: T, cookie: Option<String>) -> Response {
    let mut response = (status, Json(body)).into_response();
    if let Some(cookie) = cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    response
}
