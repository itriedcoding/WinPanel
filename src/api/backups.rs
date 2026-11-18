//! Backup endpoints.

use crate::auth::{load_server, AuthUser};
use crate::error::{AppError, Result};
use crate::models::Backup;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupRequest {
    #[serde(default)]
    pub name: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<Backup>>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    Ok(Json(state.db.list_backups(&server.id)?))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<CreateBackupRequest>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let manager = state.backups.clone();
    let name = body.name;
    let user_id = auth.user.id.clone();
    let db = state.db.clone();
    let keep = server.backup_keep;
    let server_clone = server.clone();
    tokio::task::spawn_blocking(move || match manager.create(&server_clone, name) {
        Ok(backup) => {
            let _ = db.log_activity(
                "backup",
                Some(&server_clone.id),
                Some(&user_id),
                &format!("Created backup \"{}\"", backup.name),
                None,
            );
            if let Err(err) = manager.prune(&server_clone.id, keep) {
                tracing::warn!("backup pruning failed: {err}");
            }
        }
        Err(err) => {
            tracing::warn!("backup failed for {}: {err}", server_clone.id);
        }
    });
    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "Backup started"
    })))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path((id, backup_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    state.backups.delete(&server.id, &backup_id)?;
    state.db.log_activity(
        "backup",
        Some(&server.id),
        Some(&auth.user.id),
        "Deleted a backup",
        None,
    )?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn restore(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path((id, backup_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "manage")?;
    if state.supervisor.runtime(&server.id).status == crate::models::STATUS_RUNNING {
        return Err(AppError::conflict(
            "Stop the server before restoring a backup",
        ));
    }
    let manager = state.backups.clone();
    let server_clone = server.clone();
    let backup_for_log = backup_id.clone();
    tokio::task::spawn_blocking(move || manager.restore(&server_clone, &backup_for_log))
        .await
        .map_err(|e| AppError::internal(format!("Restore task failed: {e}")))??;
    state.db.log_activity(
        "backup",
        Some(&server.id),
        Some(&auth.user.id),
        "Restored a backup",
        Some(&backup_id),
    )?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
