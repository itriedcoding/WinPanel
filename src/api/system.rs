//! Dashboard aggregates, system monitoring, settings and templates.

use crate::auth::AuthUser;
use crate::config::Settings;
use crate::error::{AppError, Result};
use crate::metrics::{HistoryPoint, SystemSnapshot};
use crate::models::{
    ActivityEntry, ServerView, STATUS_CRASHED, STATUS_INSTALL_FAILED, STATUS_RUNNING,
};
use crate::state::AppState;
use crate::templates::TemplateCollection;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct LimitQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub servers: usize,
    pub running: usize,
    pub offline: usize,
    pub errors: usize,
    pub memory_bytes: u64,
    pub storage_bytes: u64,
    pub server_cpu_percent: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub system: SystemSnapshot,
    pub history: Vec<HistoryPoint>,
    pub totals: Totals,
    pub recent_activity: Vec<ActivityEntry>,
    pub crashes: Vec<ActivityEntry>,
    pub recent_servers: Vec<ServerView>,
    pub backup_count: usize,
    pub backup_bytes: u64,
    pub uptime_secs: u64,
}

pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<Json<Dashboard>> {
    let visible = state.db.list_servers_visible(&auth.user)?;
    let mut views = Vec::with_capacity(visible.len());
    let mut totals = Totals {
        servers: visible.len(),
        running: 0,
        offline: 0,
        errors: 0,
        memory_bytes: 0,
        storage_bytes: 0,
        server_cpu_percent: 0.0,
    };
    for (server, _) in visible {
        let view = state.server_view(server);
        match view.runtime.status.as_str() {
            STATUS_RUNNING => {
                totals.running += 1;
                totals.memory_bytes = totals
                    .memory_bytes
                    .saturating_add(view.runtime.memory_bytes);
                totals.server_cpu_percent += view.runtime.cpu_percent;
                totals.storage_bytes = totals.storage_bytes.saturating_add(view.runtime.disk_bytes);
            }
            STATUS_CRASHED | STATUS_INSTALL_FAILED => {
                totals.errors += 1;
                totals.storage_bytes = totals.storage_bytes.saturating_add(view.runtime.disk_bytes);
            }
            _ => {
                totals.offline += 1;
                totals.storage_bytes = totals.storage_bytes.saturating_add(view.runtime.disk_bytes);
            }
        }
        views.push(view);
    }
    views.sort_by_key(|view| view.server.name.to_lowercase());

    let recent_servers: Vec<ServerView> = {
        let mut sorted = views.clone();
        sorted.sort_by_key(|view| std::cmp::Reverse(view.server.updated_at));
        sorted.into_iter().take(6).collect()
    };

    let activity = state.db.recent_activity(25, None)?;
    let crashes: Vec<ActivityEntry> = activity
        .iter()
        .filter(|entry| entry.kind == "crash" || entry.kind == "error")
        .take(8)
        .cloned()
        .collect();
    let backups = state.db.all_backups().unwrap_or_default();
    let backup_count = backups.len();
    let backup_bytes = backups.iter().map(|b| b.size).sum();

    Ok(Json(Dashboard {
        system: state.metrics.snapshot(),
        history: state.metrics.history(),
        totals,
        recent_activity: activity,
        crashes,
        recent_servers,
        backup_count,
        backup_bytes,
        uptime_secs: state.uptime_secs(),
    }))
}

pub async fn system_info(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
) -> Result<Json<SystemSnapshot>> {
    Ok(Json(state.metrics.snapshot()))
}

pub async fn history(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
) -> Result<Json<Vec<HistoryPoint>>> {
    Ok(Json(state.metrics.history()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReport {
    pub runtimes: Vec<crate::runtime::RuntimeStatus>,
    pub templates: Vec<crate::templates::TemplateInfo>,
    pub warnings: Vec<String>,
}

pub async fn runtimes(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
) -> Result<Json<RuntimeReport>> {
    let (templates, warnings) = match state.templates.read() {
        Ok(collection) => (collection.describe(), collection.warnings().to_vec()),
        Err(_) => (
            Vec::new(),
            vec!["Template registry is unavailable".to_string()],
        ),
    };
    Ok(Json(RuntimeReport {
        runtimes: crate::runtime::detect_all(),
        templates,
        warnings,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplatesResponse {
    pub templates: Vec<crate::templates::TemplateInfo>,
    pub warnings: Vec<String>,
}

pub async fn templates(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
) -> Result<Json<TemplatesResponse>> {
    let (templates, warnings) = match state.templates.read() {
        Ok(collection) => (collection.describe(), collection.warnings().to_vec()),
        Err(_) => (
            Vec::new(),
            vec!["Template registry is unavailable".to_string()],
        ),
    };
    Ok(Json(TemplatesResponse {
        templates,
        warnings,
    }))
}

pub async fn reload_templates(
    State(state): State<Arc<AppState>>,
    admin: crate::auth::AdminUser,
) -> Result<Json<TemplatesResponse>> {
    let collection = TemplateCollection::load(&state.layout.templates_dir());
    let warnings = collection.warnings().to_vec();
    let templates = collection.describe();
    let count = templates.len();
    {
        let mut guard = state
            .templates
            .write()
            .map_err(|_| AppError::internal("Template registry is unavailable"))?;
        *guard = collection;
    }
    state.db.log_activity(
        "update",
        None,
        Some(&admin.0.user.id),
        "Reloaded server templates",
        Some(&format!("{count} templates")),
    )?;
    Ok(Json(TemplatesResponse {
        templates,
        warnings,
    }))
}

pub async fn activity(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Query(query): Query<LimitQuery>,
) -> Result<Json<Vec<ActivityEntry>>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(state.db.recent_activity(limit, None)?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    pub version: String,
    pub repository: String,
    pub build_profile: String,
    pub platform: String,
    pub arch: String,
    pub data_dir: String,
    pub config_file: String,
    pub database_file: String,
    pub listen: String,
    pub remote_access: bool,
    pub uptime_secs: u64,
    pub server_count: usize,
    pub running_count: usize,
    pub template_warnings: Vec<String>,
}

pub async fn about(State(state): State<Arc<AppState>>, _auth: AuthUser) -> Result<Json<AboutInfo>> {
    let servers = state.db.list_servers()?;
    let running = state.supervisor.running().len();
    Ok(Json(AboutInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        repository: crate::updates::REPO.to_string(),
        build_profile: if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        },
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        data_dir: state.layout.data_dir.to_string_lossy().to_string(),
        config_file: state.layout.config_file().to_string_lossy().to_string(),
        database_file: state.layout.db_file().to_string_lossy().to_string(),
        listen: state.listen.to_string(),
        remote_access: !state.listen.ip().is_loopback(),
        uptime_secs: state.uptime_secs(),
        server_count: servers.len(),
        running_count: running,
        template_warnings: state
            .templates
            .read()
            .map(|t| t.warnings().to_vec())
            .unwrap_or_default(),
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    pub settings: Settings,
    pub listen: String,
    pub remote_access: bool,
    pub uptime_secs: u64,
    pub restart_required: bool,
}

pub async fn get_settings(
    State(state): State<Arc<AppState>>,
    _admin: crate::auth::AdminUser,
) -> Result<Json<SettingsResponse>> {
    Ok(Json(SettingsResponse {
        settings: state.settings(),
        listen: state.listen.to_string(),
        remote_access: !state.listen.ip().is_loopback(),
        uptime_secs: state.uptime_secs(),
        restart_required: false,
    }))
}

pub async fn save_settings(
    State(state): State<Arc<AppState>>,
    admin: crate::auth::AdminUser,
    Json(mut settings): Json<Settings>,
) -> Result<Json<SettingsResponse>> {
    let previous = state.settings();
    settings.validate_and_clamp();
    let restart_required = previous.log_level != settings.log_level
        || previous.backup_dir != settings.backup_dir
        || previous.metrics_interval_ms != settings.metrics_interval_ms;
    state.save_settings(settings.clone())?;
    state.db.log_activity(
        "update",
        None,
        Some(&admin.0.user.id),
        "Updated application settings",
        None,
    )?;
    Ok(Json(SettingsResponse {
        settings,
        listen: state.listen.to_string(),
        remote_access: !state.listen.ip().is_loopback(),
        uptime_secs: state.uptime_secs(),
        restart_required,
    }))
}
