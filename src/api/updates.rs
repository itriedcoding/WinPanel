//! Update endpoints backed by GitHub Releases.

use crate::auth::AdminUser;
use crate::error::{AppError, Result};
use crate::state::AppState;
use crate::updates::UpdateInfo;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct CheckQuery {
    #[serde(default)]
    pub force: Option<bool>,
}

pub async fn check(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(query): Query<CheckQuery>,
) -> Result<Json<UpdateInfo>> {
    Ok(Json(
        state.updates.check(query.force.unwrap_or(false)).await,
    ))
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    admin: AdminUser,
) -> Result<Json<serde_json::Value>> {
    let info = state.updates.check(false).await;
    if let Some(error) = &info.error {
        return Err(AppError::internal(error.clone()));
    }
    if !info.available {
        return Err(AppError::conflict("WinPanel is already up to date"));
    }
    let path = state.updates.download(&info).await?;
    state.db.log_activity(
        "update",
        None,
        Some(&admin.0.user.id),
        &format!("Downloaded update {}", info.latest_version),
        None,
    )?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(Json(serde_json::json!({
        "ok": true,
        "version": info.latest_version,
        "size": size,
        "path": path.to_string_lossy(),
    })))
}

pub async fn install(
    State(state): State<Arc<AppState>>,
    admin: AdminUser,
) -> Result<Json<serde_json::Value>> {
    // This call replaces the running executable and restarts the process.
    state.db.log_activity(
        "update",
        None,
        Some(&admin.0.user.id),
        "Installing update",
        None,
    )?;
    state.updates.apply_downloaded()?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Restarting with the new version"
    })))
}
