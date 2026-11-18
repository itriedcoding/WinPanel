//! Server CRUD, lifecycle actions and console endpoints.

use crate::auth::{load_server, AuthUser};
use crate::error::{AppError, Result};
use crate::install::InstallRequest;
use crate::models::*;
use crate::paths;
use crate::process::console::LogLine;
use crate::state::AppState;
use crate::templates::{self, Template};
use crate::util::{now_ms, uuid_str, valid_display_name, valid_env_key};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateServerRequest {
    #[serde(flatten)]
    pub server: ServerInput,
    #[serde(default)]
    pub install_options: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    pub action: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOptionsRequest {
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    /// Wipe existing files before installing. Defaults to true for reinstalls.
    #[serde(default)]
    pub wipe: Option<bool>,
}

#[derive(Deserialize)]
pub struct ConsoleQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct LimitQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct CommandRequest {
    pub command: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleResponse {
    pub status: String,
    pub pid: Option<u32>,
    pub lines: Vec<LogLine>,
    pub installing: bool,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<Json<Vec<ServerView>>> {
    let visible = state.db.list_servers_visible(&auth.user)?;
    let views = visible
        .into_iter()
        .map(|(server, level)| state.server_view_with_level(server, &level))
        .collect();
    Ok(Json(views))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ServerView>> {
    let (server, level) = load_server(&state, &auth, &id, "view")?;
    Ok(Json(state.server_view_with_level(server, &level)))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Json(request): Json<CreateServerRequest>,
) -> Result<(StatusCode, Json<ServerView>)> {
    auth.require_admin()?;
    let template = {
        let collection = state
            .templates
            .read()
            .map_err(|_| AppError::internal("Template registry is unavailable"))?;
        templates::require_template(&collection, &request.server.template_id)?.clone()
    };
    validate_input(&state, None, &request.server)?;

    let id = uuid_str();
    let install_dir = state.install_dir_for(&request.server.name, &id);
    std::fs::create_dir_all(&install_dir)
        .map_err(|e| AppError::internal(format!("Cannot create the server directory: {e}")))?;

    let ports = if request.server.ports.is_empty() {
        vec![PortMapping {
            label: "Game".to_string(),
            port: template.port.max(1),
            protocol: template.protocol.clone(),
        }]
    } else {
        request.server.ports.clone()
    };
    let first_port = ports.first().map(|p| p.port).unwrap_or(template.port);
    let mut values = templates::runtime_values(request.server.memory_mb, first_port);
    for (key, value) in &request.install_options {
        values.insert(key.clone(), value.clone());
    }

    let startup_exe = if request.server.startup_exe.trim().is_empty() {
        templates::render(&template.startup_exe, &values)
    } else {
        request.server.startup_exe.trim().to_string()
    };
    let startup_args = if request.server.startup_args.trim().is_empty() {
        templates::render(&template.startup_args, &values)
    } else {
        request.server.startup_args.clone()
    };
    let settings = state.settings();
    let env = if request.server.env.is_empty() {
        template.env.clone()
    } else {
        request.server.env.clone()
    };
    let now = now_ms();
    let server = Server {
        id: id.clone(),
        name: request.server.name.trim().to_string(),
        description: request.server.description.clone(),
        template_id: template.id.clone(),
        install_dir: install_dir.to_string_lossy().to_string(),
        startup_exe,
        startup_args,
        env,
        memory_mb: if request.server.memory_mb == 0 {
            template.memory_mb.max(settings.default_memory_mb)
        } else {
            request.server.memory_mb
        },
        cpu_cores: request.server.cpu_cores.clone(),
        cpu_limit_percent: request.server.cpu_limit_percent,
        storage_mb: request.server.storage_mb,
        ports,
        auto_restart: request.server.auto_restart,
        auto_restart_max: request.server.auto_restart_max,
        auto_restart_delay_ms: request.server.auto_restart_delay_ms,
        stop_command: if request.server.stop_command.trim().is_empty() {
            template.stop_command.clone()
        } else {
            request.server.stop_command.clone()
        },
        stop_timeout_ms: request.server.stop_timeout_ms.max(2_000),
        status: STATUS_STOPPED.to_string(),
        installed: false,
        install_error: None,
        created_at: now,
        updated_at: now,
        created_by: Some(auth.user.id.clone()),
        last_exit_code: None,
        backups_enabled: request.server.backups_enabled,
        backup_interval_hours: request.server.backup_interval_hours.max(1),
        backup_keep: request.server.backup_keep.max(1),
        install_options: request.install_options.clone(),
    };

    state.db.insert_server(&server)?;
    state.db.log_activity(
        "create",
        Some(&server.id),
        Some(&auth.user.id),
        &format!("Created server {}", server.name),
        Some(&template.name),
    )?;

    spawn_install(&state, server.clone(), template, request.install_options);
    Ok((StatusCode::CREATED, Json(state.server_view(server))))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<ServerInput>,
) -> Result<Json<ServerView>> {
    let (mut server, _) = load_server(&state, &auth, &id, "manage")?;
    validate_input(&state, Some(&id), &input)?;
    let runtime = state.supervisor.runtime(&id);

    server.name = input.name.trim().to_string();
    server.description = input.description;
    server.startup_exe = input.startup_exe.trim().to_string();
    server.startup_args = input.startup_args;
    server.env = input.env;
    server.memory_mb = input.memory_mb;
    server.cpu_cores = input.cpu_cores;
    server.cpu_limit_percent = input.cpu_limit_percent;
    server.storage_mb = input.storage_mb;
    server.ports = input.ports;
    server.auto_restart = input.auto_restart;
    server.auto_restart_max = input.auto_restart_max;
    server.auto_restart_delay_ms = input.auto_restart_delay_ms.max(500);
    server.stop_command = input.stop_command;
    server.stop_timeout_ms = input.stop_timeout_ms.max(2_000);
    server.backups_enabled = input.backups_enabled;
    server.backup_interval_hours = input.backup_interval_hours.max(1);
    server.backup_keep = input.backup_keep.max(1);
    server.updated_at = now_ms();
    state.db.update_server(&server)?;
    state.db.log_activity(
        "update",
        Some(&server.id),
        Some(&auth.user.id),
        &format!("Updated settings for {}", server.name),
        None,
    )?;
    if runtime.status == STATUS_RUNNING {
        // Limits such as memory or CPU affinity only apply to new processes.
        tracing::info!("settings for {} changed while running", server.id);
    }
    Ok(Json(state.server_view(server)))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "manage")?;
    if state.is_installing(&id) {
        return Err(AppError::conflict(
            "Wait for the installation to finish (or fail) first",
        ));
    }
    let _ = state.supervisor.stop(&server, true).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    state.supervisor.forget(&id);
    state.backups.purge_server(&id);
    let dir = PathBuf::from(&server.install_dir);
    let result = tokio::task::spawn_blocking(move || paths::remove_dir_all(&dir)).await;
    if let Ok(Err(err)) = result {
        return Err(AppError::internal(format!(
            "Server was removed from the panel but its files could not be deleted: {err}"
        )));
    }
    state.db.delete_server(&id)?;
    state.db.log_activity(
        "delete",
        None,
        Some(&auth.user.id),
        &format!("Deleted server {}", server.name),
        None,
    )?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn action(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ActionRequest>,
) -> Result<Json<ServerView>> {
    let action = body.action.trim().to_lowercase();
    let needed = match action.as_str() {
        "start" | "stop" | "restart" | "kill" => "console",
        _ => "manage",
    };
    let (server, _) = load_server(&state, &auth, &id, needed)?;

    match action.as_str() {
        "start" => {
            if !server.installed {
                return Err(AppError::conflict(
                    "Server files are not installed. Run the installer first.",
                ));
            }
            state.supervisor.start(server.clone()).await?;
        }
        "stop" => state.supervisor.stop(&server, false).await?,
        "kill" => state.supervisor.stop(&server, true).await?,
        "restart" => {
            if !server.installed {
                return Err(AppError::conflict("Server files are not installed"));
            }
            state.supervisor.restart(server.clone(), false).await?;
        }
        "rename" => {
            let name = body
                .name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .ok_or_else(|| AppError::bad_request("A new name is required"))?;
            if !valid_display_name(&name) {
                return Err(AppError::bad_request("Server name is not valid"));
            }
            state.db.rename_server(&server.id, &name)?;
            state.db.log_activity(
                "update",
                Some(&server.id),
                Some(&auth.user.id),
                &format!("Renamed server to {name}"),
                None,
            )?;
        }
        "clone" => {
            return clone_server(&state, &auth, &server, body.name).await;
        }
        other => return Err(AppError::bad_request(format!("Unknown action \"{other}\""))),
    }

    let refreshed = state.server_or_404(&id)?;
    Ok(Json(state.server_view(refreshed)))
}

async fn clone_server(
    state: &Arc<AppState>,
    auth: &AuthUser,
    source: &Server,
    name: Option<String>,
) -> Result<Json<ServerView>> {
    let runtime = state.supervisor.runtime(&source.id);
    if runtime.status == STATUS_RUNNING {
        return Err(AppError::conflict(
            "Stop the server before cloning it so files are consistent",
        ));
    }
    let base_name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{} copy", source.name));
    if !valid_display_name(&base_name) {
        return Err(AppError::bad_request("Clone name is not valid"));
    }
    let id = uuid_str();
    let target_dir = state.install_dir_for(&base_name, &id);
    let source_dir = PathBuf::from(&source.install_dir);
    let target_clone = target_dir.clone();
    tokio::task::spawn_blocking(move || paths::copy_dir(&source_dir, &target_clone))
        .await
        .map_err(|e| AppError::internal(format!("Clone task failed: {e}")))?
        .map_err(|e| AppError::internal(format!("Could not copy server files: {e}")))?;

    let now = now_ms();
    let mut clone = source.clone();
    clone.id = id.clone();
    clone.name = base_name;
    clone.install_dir = target_dir.to_string_lossy().to_string();
    clone.status = STATUS_STOPPED.to_string();
    clone.last_exit_code = None;
    clone.created_at = now;
    clone.updated_at = now;
    clone.created_by = Some(auth.user.id.clone());
    state.db.insert_server(&clone)?;
    for (user_id, level) in state.db.list_server_users(&source.id)? {
        state.db.set_access(&user_id, &id, &level)?;
    }
    state.db.log_activity(
        "create",
        Some(&id),
        Some(&auth.user.id),
        &format!("Cloned {} into {}", source.name, clone.name),
        None,
    )?;
    Ok(Json(state.server_view(clone)))
}

pub async fn install(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<InstallOptionsRequest>,
) -> Result<Json<ServerView>> {
    let (mut server, _) = load_server(&state, &auth, &id, "manage")?;
    let runtime = state.supervisor.runtime(&id);
    if matches!(
        runtime.status.as_str(),
        STATUS_RUNNING | STATUS_STARTING | STATUS_STOPPING
    ) {
        return Err(AppError::conflict("Stop the server before reinstalling it"));
    }
    if state.is_installing(&id) {
        return Err(AppError::conflict("An installation is already running"));
    }
    let template = {
        let collection = state
            .templates
            .read()
            .map_err(|_| AppError::internal("Template registry is unavailable"))?;
        templates::require_template(&collection, &server.template_id)?.clone()
    };
    let options = if body.options.is_empty() {
        server.install_options.clone()
    } else {
        body.options.clone()
    };
    if body.wipe.unwrap_or(true) {
        let dir = PathBuf::from(&server.install_dir);
        if dir.exists() {
            tokio::task::spawn_blocking(move || paths::remove_dir_all(&dir))
                .await
                .map_err(|e| AppError::internal(format!("Cleanup failed: {e}")))??;
        }
    }
    std::fs::create_dir_all(&server.install_dir)?;
    server.installed = false;
    server.install_options = options.clone();
    server.updated_at = now_ms();
    state.db.update_server(&server)?;
    state.db.log_activity(
        "install",
        Some(&server.id),
        Some(&auth.user.id),
        &format!("Reinstalling {}", server.name),
        None,
    )?;
    spawn_install(&state, server.clone(), template, options);
    Ok(Json(state.server_view(server)))
}

pub fn spawn_install(
    state: &Arc<AppState>,
    server: Server,
    template: Template,
    options: BTreeMap<String, String>,
) {
    let state = Arc::clone(state);
    tokio::spawn(async move {
        if state.begin_install(&server.id).is_err() {
            tracing::warn!("install already running for {}", server.id);
            return;
        }
        let id = server.id.clone();
        let _ = state
            .db
            .set_install_state(&id, false, None, STATUS_INSTALLING);
        state.supervisor.handle(&id);
        state
            .supervisor
            .set_install(&id, Some(0), Some("Queued".to_string()));
        let result = crate::install::run(InstallRequest {
            server: server.clone(),
            template,
            options,
            db: state.db.clone(),
            supervisor: state.supervisor.clone(),
            events: state.events.clone(),
            settings: state.settings.clone(),
        })
        .await;
        if let Err(err) = result {
            tracing::warn!("install failed for {}: {err}", id);
        }
        state.end_install(&id);
    });
}

pub async fn console(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ConsoleQuery>,
) -> Result<Json<ConsoleResponse>> {
    let (server, _) = load_server(&state, &auth, &id, "console")?;
    let limit = query.limit.unwrap_or(500).clamp(1, 5000);
    let lines = state.supervisor.console_snapshot(&id, limit);
    let runtime = state.supervisor.runtime(&id);
    Ok(Json(ConsoleResponse {
        status: runtime.status,
        pid: runtime.pid,
        lines,
        installing: state.is_installing(&server.id),
    }))
}

pub async fn console_search(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<LogLine>>> {
    load_server(&state, &auth, &id, "console")?;
    if query.q.trim().len() < 2 {
        return Err(AppError::bad_request("Search needs at least 2 characters"));
    }
    let limit = query.limit.unwrap_or(500).clamp(1, 5000);
    Ok(Json(state.supervisor.console_search(
        &id,
        query.q.trim(),
        limit,
    )))
}

pub async fn console_input(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<CommandRequest>,
) -> Result<Json<serde_json::Value>> {
    load_server(&state, &auth, &id, "console")?;
    let command = body.command.trim();
    if command.is_empty() {
        return Err(AppError::bad_request("Command is empty"));
    }
    state.supervisor.write_input(&id, command).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn clear_console(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    load_server(&state, &auth, &id, "console")?;
    state.supervisor.clear_console(&id);
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn server_activity(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<LimitQuery>,
) -> Result<Json<Vec<ActivityEntry>>> {
    load_server(&state, &auth, &id, "view")?;
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(state.db.recent_activity(limit, Some(&id))?))
}

fn validate_input(
    state: &Arc<AppState>,
    server_id: Option<&str>,
    input: &ServerInput,
) -> Result<()> {
    if !valid_display_name(&input.name) {
        return Err(AppError::bad_request(
            "Server name must be 1-64 characters and cannot contain slashes",
        ));
    }
    if input.description.chars().count() > 500 {
        return Err(AppError::bad_request(
            "Description is too long (max 500 characters)",
        ));
    }
    if !input.startup_exe.trim().is_empty() && input.startup_exe.chars().count() > 260 {
        return Err(AppError::bad_request("Startup executable path is too long"));
    }
    if input.startup_args.chars().count() > 2000 {
        return Err(AppError::bad_request("Startup arguments are too long"));
    }
    if input.memory_mb != 0 && !(128..=1_048_576).contains(&input.memory_mb) {
        return Err(AppError::bad_request(
            "Memory limit must be 0 (unlimited) or between 128 and 1048576 MB",
        ));
    }
    if !(0..=100).contains(&input.cpu_limit_percent) {
        return Err(AppError::bad_request(
            "CPU limit must be between 0 and 100%",
        ));
    }
    if input.cpu_limit_percent > 0 && input.cpu_limit_percent < 5 {
        return Err(AppError::bad_request(
            "CPU limit must be at least 5% or 0 for unlimited",
        ));
    }
    if input.storage_mb > 10_485_760 {
        return Err(AppError::bad_request("Storage limit is out of range"));
    }
    let cores = state.metrics.snapshot().cpu_cores.max(1);
    if let Some(core) = input.cpu_cores.iter().find(|core| **core >= cores) {
        return Err(AppError::bad_request(format!(
            "CPU core {core} does not exist on this machine ({cores} cores available)"
        )));
    }
    if input.cpu_cores.len() > cores {
        return Err(AppError::bad_request("Too many CPU cores selected"));
    }
    if input.auto_restart_max > 50 {
        return Err(AppError::bad_request(
            "Automatic restart limit must be 50 or less",
        ));
    }
    if input.auto_restart_delay_ms < 500 || input.auto_restart_delay_ms > 300_000 {
        return Err(AppError::bad_request(
            "Restart delay must be between 0.5 and 300 seconds",
        ));
    }
    if input.stop_timeout_ms < 2_000 || input.stop_timeout_ms > 300_000 {
        return Err(AppError::bad_request(
            "Stop timeout must be between 2 and 300 seconds",
        ));
    }
    if input.backup_keep < 1 || input.backup_keep > 100 {
        return Err(AppError::bad_request(
            "Backup retention must be between 1 and 100",
        ));
    }
    if input.backup_interval_hours < 1 || input.backup_interval_hours > 720 {
        return Err(AppError::bad_request(
            "Backup interval must be between 1 and 720 hours",
        ));
    }

    let mut seen_ports = std::collections::HashSet::new();
    for port in &input.ports {
        if port.port == 0 {
            return Err(AppError::bad_request("Port 0 is not valid"));
        }
        let protocol = port.protocol.to_lowercase();
        if protocol != "tcp" && protocol != "udp" {
            return Err(AppError::bad_request("Port protocol must be tcp or udp"));
        }
        if !seen_ports.insert((port.port, protocol.clone())) {
            return Err(AppError::bad_request(format!(
                "Port {} is listed twice",
                port.port
            )));
        }
        for other in state.db.list_servers()? {
            if Some(other.id.as_str()) == server_id {
                continue;
            }
            if other
                .ports
                .iter()
                .any(|p| p.port == port.port && p.protocol.eq_ignore_ascii_case(&protocol))
            {
                return Err(AppError::conflict(format!(
                    "Port {}/{} is already assigned to \"{}\"",
                    port.port, protocol, other.name
                )));
            }
        }
    }
    if input.ports.len() > 32 {
        return Err(AppError::bad_request("A server can have at most 32 ports"));
    }

    let mut seen_env = std::collections::HashSet::new();
    for var in &input.env {
        if !valid_env_key(&var.key) {
            return Err(AppError::bad_request(format!(
                "Environment variable name \"{}\" is not valid",
                var.key
            )));
        }
        if var.value.chars().count() > 4096 {
            return Err(AppError::bad_request(
                "Environment variable values are too long",
            ));
        }
        if !seen_env.insert(var.key.to_uppercase()) {
            return Err(AppError::bad_request(format!(
                "Environment variable {} is defined twice",
                var.key
            )));
        }
    }
    Ok(())
}
