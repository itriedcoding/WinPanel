//! Password hashing, sessions, cookies, CSRF and permission checks.

use crate::error::{AppError, Result};
use crate::models::{level_allows, Server, User, ROLE_ADMIN};
use crate::state::AppState;
use crate::util::{now_ms, random_token, sha256_hex};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use std::sync::Arc;

pub const SESSION_COOKIE: &str = "winpanel_session";
pub const CSRF_HEADER: &str = "x-csrf-token";

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| AppError::internal(format!("Could not hash password: {err}")))
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(err) => {
            tracing::error!("stored password hash is unreadable: {err}");
            false
        }
    }
}

/// Minimum password policy for new accounts and password changes.
pub fn password_problem(password: &str) -> Option<&'static str> {
    if password.chars().count() < 8 {
        return Some("Password must be at least 8 characters long");
    }
    if password.chars().count() > 200 {
        return Some("Password is too long (max 200 characters)");
    }
    None
}

pub fn header_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            if key.trim() == name {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

pub fn session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    header_cookie(headers, SESSION_COOKIE).map(|token| sha256_hex(&token))
}

/// Build the Set-Cookie value for a session.
pub fn session_cookie(token: &str, max_age_secs: i64, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_secs}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

pub fn new_session_token() -> String {
    random_token(32)
}

pub fn new_csrf_token() -> String {
    random_token(24)
}

pub struct AuthUser {
    pub user: User,
    pub session: crate::models::Session,
}

#[axum::async_trait]
impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> std::result::Result<Self, Self::Rejection> {
        let session_id = session_id_from_headers(&parts.headers)
            .ok_or_else(|| AppError::unauthorized("Sign in required"))?;
        let session = state
            .db
            .session(&session_id)?
            .ok_or_else(|| AppError::unauthorized("Session expired, please sign in again"))?;
        if session.expires_at <= now_ms() {
            state.db.delete_session(&session_id)?;
            return Err(AppError::unauthorized(
                "Session expired, please sign in again",
            ));
        }
        let user = state
            .db
            .user_by_id(&session.user_id)?
            .ok_or_else(|| AppError::unauthorized("Account no longer exists"))?;
        if user.disabled {
            return Err(AppError::forbidden("This account has been disabled"));
        }
        Ok(AuthUser { user, session })
    }
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.user.role == ROLE_ADMIN
    }

    pub fn require_admin(&self) -> Result<()> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(AppError::forbidden("Administrator access is required"))
        }
    }
}

pub struct AdminUser(pub AuthUser);

#[axum::async_trait]
impl FromRequestParts<Arc<AppState>> for AdminUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> std::result::Result<Self, Self::Rejection> {
        let auth = AuthUser::from_request_parts(parts, state).await?;
        auth.require_admin()?;
        Ok(AdminUser(auth))
    }
}

/// Load a server the caller is allowed to act on, with their access level.
pub fn load_server(
    state: &Arc<AppState>,
    auth: &AuthUser,
    server_id: &str,
    needed: &str,
) -> Result<(Server, String)> {
    let server = state
        .db
        .server_by_id(server_id)?
        .ok_or_else(|| AppError::not_found("Server not found"))?;
    if auth.is_admin() {
        return Ok((server, "manage".to_string()));
    }
    let level = state
        .db
        .access_level(&auth.user.id, &server.id)?
        .ok_or_else(|| AppError::forbidden("You do not have access to this server"))?;
    if !level_allows(&level, needed) {
        return Err(AppError::forbidden(format!(
            "This action requires \"{needed}\" access to the server"
        )));
    }
    Ok((server, level))
}

/// Login throttling: five failures per identifier starts a lockout window.
#[derive(Default)]
pub struct LoginLimiter {
    attempts: std::sync::Mutex<std::collections::HashMap<String, Attempt>>,
}

#[derive(Clone, Copy)]
struct Attempt {
    failures: u32,
    first_failure_ms: i64,
    locked_until_ms: i64,
}

pub struct LoginGuard<'a> {
    limiter: &'a LoginLimiter,
    key: String,
}

impl LoginLimiter {
    pub fn guard<'a>(&'a self, key: &str) -> LoginGuard<'a> {
        LoginGuard {
            limiter: self,
            key: key.to_string(),
        }
    }

    fn state(&self, key: &str) -> Option<Attempt> {
        self.attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .copied()
    }

    fn record_failure(&self, key: &str) {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(key.to_string()).or_insert(Attempt {
            failures: 0,
            first_failure_ms: now_ms(),
            locked_until_ms: 0,
        });
        entry.failures += 1;
        if entry.failures >= 5 {
            entry.locked_until_ms = now_ms() + 5 * 60 * 1000;
            entry.failures = 0;
        }
        // Bound memory: drop stale entries when the map grows.
        if map.len() > 512 {
            let cutoff = now_ms() - 30 * 60 * 1000;
            map.retain(|_, a| a.locked_until_ms > now_ms() || a.first_failure_ms > cutoff);
        }
    }

    fn record_success(&self, key: &str) {
        self.attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }
}

impl LoginGuard<'_> {
    pub fn check(&self) -> Result<()> {
        if let Some(state) = self.limiter.state(&self.key) {
            if state.locked_until_ms > now_ms() {
                let secs = (state.locked_until_ms - now_ms()) / 1000 + 1;
                return Err(AppError::too_many(format!(
                    "Too many failed sign-in attempts. Try again in {secs}s"
                )));
            }
        }
        Ok(())
    }

    pub fn failure(&self) {
        self.limiter.record_failure(&self.key);
    }

    pub fn success(&self) {
        self.limiter.record_success(&self.key);
    }
}
