//! User and permission management (administrators only).

use crate::auth::{hash_password, password_problem, AdminUser};
use crate::error::{AppError, Result};
use crate::models::{level_allows, User, ROLE_ADMIN, ROLE_USER};
use crate::state::AppState;
use crate::util::{uuid_str, valid_username};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessEntry {
    pub server_id: String,
    pub server_name: String,
    pub level: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserView {
    #[serde(flatten)]
    pub user: User,
    pub access: Vec<AccessEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessRequest {
    pub server_id: String,
    /// null removes access
    #[serde(default)]
    pub level: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Vec<UserView>>> {
    let servers = state.db.list_servers()?;
    let mut views = Vec::new();
    for user in state.db.list_users()? {
        let access = state
            .db
            .list_access(&user.id)?
            .into_iter()
            .map(|entry| AccessEntry {
                server_name: servers
                    .iter()
                    .find(|s| s.id == entry.server_id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "(deleted server)".to_string()),
                server_id: entry.server_id,
                level: entry.level,
            })
            .collect();
        views.push(UserView { user, access });
    }
    Ok(Json(views))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    admin: AdminUser,
    Json(body): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<User>)> {
    let username = body.username.trim().to_string();
    if !valid_username(&username) {
        return Err(AppError::bad_request(
            "Username must be 3-32 characters: letters, digits, dot, dash or underscore",
        ));
    }
    if let Some(problem) = password_problem(&body.password) {
        return Err(AppError::bad_request(problem));
    }
    if state.db.find_credential(&username)?.is_some() {
        return Err(AppError::conflict("That username is already taken"));
    }
    let role = match body.role.as_deref().unwrap_or(ROLE_USER) {
        ROLE_ADMIN => ROLE_ADMIN,
        ROLE_USER => ROLE_USER,
        _ => return Err(AppError::bad_request("Role must be admin or user")),
    };
    let id = uuid_str();
    let hash = hash_password(&body.password)?;
    state.db.create_user(&id, &username, &hash, role)?;
    let user = state
        .db
        .user_by_id(&id)?
        .ok_or_else(|| AppError::internal("User could not be created"))?;
    state.db.log_activity(
        "auth",
        None,
        Some(&admin.0.user.id),
        &format!("Created account {username}"),
        Some(role),
    )?;
    Ok((StatusCode::CREATED, Json(user)))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<User>> {
    let target = state
        .db
        .user_by_id(&id)?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if let Some(role) = &body.role {
        if !matches!(role.as_str(), ROLE_ADMIN | ROLE_USER) {
            return Err(AppError::bad_request("Role must be admin or user"));
        }
        if target.role == ROLE_ADMIN && role == ROLE_USER {
            ensure_not_last_admin(&state, &target.id, "demote")?;
        }
        state.db.update_role(&id, role)?;
    }

    if let Some(disabled) = body.disabled {
        if disabled && admin.0.user.id == target.id {
            return Err(AppError::bad_request("You cannot disable your own account"));
        }
        if disabled {
            ensure_not_last_admin(&state, &target.id, "disable")?;
            state.db.delete_user_sessions(&target.id, None)?;
        }
        state.db.set_disabled(&id, disabled)?;
    }

    if let Some(password) = &body.password {
        if let Some(problem) = password_problem(password) {
            return Err(AppError::bad_request(problem));
        }
        let hash = hash_password(password)?;
        state.db.update_password(&id, &hash)?;
        if admin.0.user.id != target.id {
            state.db.delete_user_sessions(&target.id, None)?;
        }
    }

    state.db.log_activity(
        "auth",
        None,
        Some(&admin.0.user.id),
        &format!("Updated account {}", target.username),
        None,
    )?;
    let updated = state
        .db
        .user_by_id(&id)?
        .ok_or_else(|| AppError::not_found("User not found"))?;
    Ok(Json(updated))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let target = state
        .db
        .user_by_id(&id)?
        .ok_or_else(|| AppError::not_found("User not found"))?;
    if admin.0.user.id == target.id {
        return Err(AppError::bad_request("You cannot delete your own account"));
    }
    ensure_not_last_admin(&state, &target.id, "delete")?;
    state.db.delete_user(&id)?;
    state.db.log_activity(
        "auth",
        None,
        Some(&admin.0.user.id),
        &format!("Deleted account {}", target.username),
        None,
    )?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn set_access(
    State(state): State<Arc<AppState>>,
    admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<AccessRequest>,
) -> Result<Json<serde_json::Value>> {
    let target = state
        .db
        .user_by_id(&id)?
        .ok_or_else(|| AppError::not_found("User not found"))?;
    let server = state
        .db
        .server_by_id(&body.server_id)?
        .ok_or_else(|| AppError::not_found("Server not found"))?;
    match body.level {
        Some(level) => {
            let normalized = level.to_lowercase();
            if !["view", "console", "files", "manage"].contains(&normalized.as_str()) {
                return Err(AppError::bad_request(
                    "Access level must be view, console, files or manage",
                ));
            }
            // Sanity-check the hierarchy helper with the submitted value.
            debug_assert!(level_allows(&normalized, "view"));
            state.db.set_access(&target.id, &server.id, &normalized)?;
            state.db.log_activity(
                "auth",
                Some(&server.id),
                Some(&admin.0.user.id),
                &format!(
                    "Granted {normalized} access to {} for {}",
                    server.name, target.username
                ),
                None,
            )?;
        }
        None => {
            state.db.remove_access(&target.id, &server.id)?;
            state.db.log_activity(
                "auth",
                Some(&server.id),
                Some(&admin.0.user.id),
                &format!("Revoked access to {} for {}", server.name, target.username),
                None,
            )?;
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

fn ensure_not_last_admin(state: &Arc<AppState>, user_id: &str, verb: &str) -> Result<()> {
    let admins: Vec<User> = state
        .db
        .list_users()?
        .into_iter()
        .filter(|user| user.role == ROLE_ADMIN && !user.disabled)
        .collect();
    if admins.len() <= 1 && admins.iter().any(|user| user.id == user_id) {
        return Err(AppError::conflict(format!(
            "Cannot {verb} the last active administrator account"
        )));
    }
    Ok(())
}
