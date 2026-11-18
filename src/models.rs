//! Data structures shared between the API, the database and the UI.

use serde::{Deserialize, Serialize};

pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_USER: &str = "user";

pub const STATUS_STOPPED: &str = "stopped";
pub const STATUS_STARTING: &str = "starting";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_STOPPING: &str = "stopping";
pub const STATUS_CRASHED: &str = "crashed";
pub const STATUS_INSTALLING: &str = "installing";
pub const STATUS_INSTALL_FAILED: &str = "install_failed";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub username: String,
    pub role: String,
    pub created_at: i64,
    pub last_login: Option<i64>,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct UserCredential {
    pub user: User,
    pub password_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub user_id: String,
    pub csrf: String,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    #[serde(default)]
    pub label: String,
    pub port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    "tcp".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub description: String,
    pub template_id: String,
    pub install_dir: String,
    pub startup_exe: String,
    pub startup_args: String,
    pub env: Vec<EnvVar>,
    /// Memory limit in MB, 0 = unlimited.
    pub memory_mb: u64,
    /// CPU cores (0-based) the server may run on. Empty = all cores.
    pub cpu_cores: Vec<usize>,
    /// Hard CPU rate cap as a percentage of all cores, 0 = unlimited.
    pub cpu_limit_percent: u64,
    /// Storage limit in MB, 0 = unlimited.
    pub storage_mb: u64,
    pub ports: Vec<PortMapping>,
    pub auto_restart: bool,
    pub auto_restart_max: u32,
    pub auto_restart_delay_ms: u64,
    pub stop_command: String,
    pub stop_timeout_ms: u64,
    pub status: String,
    pub installed: bool,
    pub install_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub created_by: Option<String>,
    pub last_exit_code: Option<i32>,
    pub backups_enabled: bool,
    pub backup_interval_hours: u32,
    pub backup_keep: u32,
    /// Options chosen in the template installer (kept for reinstalls).
    pub install_options: std::collections::BTreeMap<String, String>,
}

/// Create/update payload from the UI.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerInput {
    pub name: String,
    pub description: String,
    pub template_id: String,
    pub startup_exe: String,
    pub startup_args: String,
    pub env: Vec<EnvVar>,
    pub memory_mb: u64,
    pub cpu_cores: Vec<usize>,
    pub cpu_limit_percent: u64,
    pub storage_mb: u64,
    pub ports: Vec<PortMapping>,
    pub auto_restart: bool,
    pub auto_restart_max: u32,
    pub auto_restart_delay_ms: u64,
    pub stop_command: String,
    pub stop_timeout_ms: u64,
    pub backups_enabled: bool,
    pub backup_interval_hours: u32,
    pub backup_keep: u32,
}

impl Default for ServerInput {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            template_id: String::new(),
            startup_exe: String::new(),
            startup_args: String::new(),
            env: Vec::new(),
            memory_mb: 2048,
            cpu_cores: Vec::new(),
            cpu_limit_percent: 0,
            storage_mb: 10240,
            ports: Vec::new(),
            auto_restart: false,
            auto_restart_max: 3,
            auto_restart_delay_ms: 5000,
            stop_command: String::new(),
            stop_timeout_ms: 30_000,
            backups_enabled: false,
            backup_interval_hours: 24,
            backup_keep: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRuntime {
    pub status: String,
    pub pid: Option<u32>,
    pub uptime_ms: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub restarts: u32,
    pub started_at: Option<i64>,
    pub last_exit_code: Option<i32>,
    pub disk_bytes: u64,
    pub console_lines: usize,
    pub install_progress: Option<u8>,
    pub install_step: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    #[serde(flatten)]
    pub server: Server,
    pub runtime: ServerRuntime,
    /// Effective access level for the requesting account.
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub size: u64,
    pub created_at: i64,
    pub status: String,
    pub checksum: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerAccess {
    pub server_id: String,
    pub level: String,
}

/// Hierarchical access levels for non-admin users.
pub fn level_allows(level: &str, needed: &str) -> bool {
    let rank = |value: &str| match value {
        "view" => 1,
        "console" => 2,
        "files" => 3,
        "manage" => 4,
        _ => 0,
    };
    rank(level) >= rank(needed) && rank(needed) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_levels_are_hierarchical() {
        assert!(level_allows("manage", "files"));
        assert!(level_allows("files", "console"));
        assert!(!level_allows("view", "console"));
        assert!(!level_allows("", "view"));
    }
}
