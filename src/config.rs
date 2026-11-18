//! Bootstrap configuration (JSON file next to the database) and user-editable
//! application settings (stored in SQLite so they survive restarts).

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::paths::Layout;
use crate::util::atomic_write;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FileConfig {
    /// Address the management API binds to. Loopback by default.
    pub listen: String,
    /// Must be enabled explicitly before a non-loopback bind is accepted.
    pub allow_remote: bool,
    pub open_browser: bool,
    pub log_level: String,
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            listen: DEFAULT_LISTEN.to_string(),
            allow_remote: false,
            open_browser: true,
            log_level: "info".to_string(),
        }
    }
}

impl FileConfig {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(raw) => match serde_json::from_str::<FileConfig>(&raw) {
                    Ok(cfg) => return Ok(cfg.sanitized()),
                    Err(err) => {
                        // Keep the broken file for diagnostics and continue with
                        // defaults instead of refusing to start.
                        let backup =
                            path.with_extension(format!("corrupt-{}.json", crate::util::now_ms()));
                        let _ = std::fs::rename(path, &backup);
                        tracing::warn!(
                            "config.json was invalid ({err}); moved to {} and defaults restored",
                            backup.display()
                        );
                    }
                },
                Err(err) => {
                    tracing::warn!("could not read config.json ({err}); using defaults");
                }
            }
        }
        let cfg = FileConfig::default();
        cfg.persist(path)?;
        Ok(cfg)
    }

    pub fn persist(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        atomic_write(path, &json)
            .map_err(|e| AppError::internal(format!("Cannot write config.json: {e}")))?;
        Ok(())
    }

    fn sanitized(mut self) -> Self {
        if self.listen.trim().is_empty() {
            self.listen = DEFAULT_LISTEN.to_string();
        }
        if !["trace", "debug", "info", "warn", "error"].contains(&self.log_level.as_str()) {
            self.log_level = "info".to_string();
        }
        self
    }

    /// Resolve the configured listen address. Accepts `host:port` or a bare port.
    pub fn resolve_listen(&self) -> Result<SocketAddr> {
        let raw = self.listen.trim();
        if let Ok(port) = raw.parse::<u16>() {
            return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
        }
        if let Ok(addr) = raw.parse::<SocketAddr>() {
            return Ok(addr);
        }
        if let Some(rest) = raw.strip_prefix("localhost:") {
            if let Ok(port) = rest.parse::<u16>() {
                return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
            }
        }
        Err(AppError::bad_request(format!(
            "Invalid listen address \"{raw}\". Use the form 127.0.0.1:8787"
        )))
    }
}

/// Application preferences, editable from the Settings screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub default_install_dir: String,
    pub default_memory_mb: u64,
    pub default_storage_mb: u64,
    pub default_port: u16,
    pub auto_restart_default: bool,
    pub stop_timeout_ms: u64,
    pub backups_enabled: bool,
    pub backup_interval_hours: u32,
    pub backup_keep: u32,
    pub backup_dir: String,
    pub log_level: String,
    pub log_retention_days: u32,
    pub console_buffer_lines: usize,
    pub metrics_interval_ms: u64,
    pub update_check_enabled: bool,
    pub session_hours: u64,
    pub allow_registration: bool,
    pub max_upload_mb: u64,
    pub disk_alert_percent: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_install_dir: "servers".to_string(),
            default_memory_mb: 2048,
            default_storage_mb: 10240,
            default_port: 25565,
            auto_restart_default: false,
            stop_timeout_ms: 30_000,
            backups_enabled: false,
            backup_interval_hours: 24,
            backup_keep: 5,
            backup_dir: "backups".to_string(),
            log_level: "info".to_string(),
            log_retention_days: 14,
            console_buffer_lines: 2000,
            metrics_interval_ms: 3000,
            update_check_enabled: true,
            session_hours: 24 * 7,
            allow_registration: false,
            max_upload_mb: 2048,
            disk_alert_percent: 90,
        }
    }
}

impl Settings {
    pub fn defaults(layout: &Layout) -> Self {
        Self {
            default_install_dir: layout.servers_dir().to_string_lossy().to_string(),
            default_memory_mb: 2048,
            default_storage_mb: 10240,
            default_port: 25565,
            auto_restart_default: false,
            stop_timeout_ms: 30_000,
            backups_enabled: false,
            backup_interval_hours: 24,
            backup_keep: 5,
            backup_dir: layout.backups_dir().to_string_lossy().to_string(),
            log_level: "info".to_string(),
            log_retention_days: 14,
            console_buffer_lines: 2000,
            metrics_interval_ms: 3000,
            update_check_enabled: true,
            session_hours: 24 * 7,
            allow_registration: false,
            max_upload_mb: 2048,
            disk_alert_percent: 90,
        }
    }

    pub fn load(db: &Db, layout: &Layout) -> Result<Self> {
        let defaults = Settings::defaults(layout);
        match db.get_setting("settings")? {
            Some(raw) => match serde_json::from_str::<Settings>(&raw) {
                Ok(mut settings) => {
                    settings.validate_and_clamp();
                    Ok(settings)
                }
                Err(err) => {
                    tracing::warn!("stored settings were invalid ({err}); defaults restored");
                    Ok(defaults)
                }
            },
            None => Ok(defaults),
        }
    }

    pub fn save(&self, db: &Db) -> Result<()> {
        db.set_setting("settings", &serde_json::to_string(self)?)
    }

    /// Coerce user-supplied values into a safe operating range.
    pub fn validate_and_clamp(&mut self) {
        if self.default_install_dir.trim().is_empty() {
            self.default_install_dir = "servers".to_string();
        }
        if self.backup_dir.trim().is_empty() {
            self.backup_dir = "backups".to_string();
        }
        if !["trace", "debug", "info", "warn", "error"].contains(&self.log_level.as_str()) {
            self.log_level = "info".to_string();
        }
        self.default_memory_mb = self.default_memory_mb.min(1024 * 1024);
        self.default_storage_mb = self.default_storage_mb.min(1024 * 1024);
        self.default_port = self.default_port.clamp(1, 65535);
        self.stop_timeout_ms = self.stop_timeout_ms.clamp(2_000, 300_000);
        self.backup_interval_hours = self.backup_interval_hours.clamp(1, 24 * 30);
        self.backup_keep = self.backup_keep.clamp(1, 100);
        self.log_retention_days = self.log_retention_days.clamp(1, 365);
        self.console_buffer_lines = self.console_buffer_lines.clamp(200, 20_000);
        self.metrics_interval_ms = self.metrics_interval_ms.clamp(1_000, 60_000);
        self.session_hours = self.session_hours.clamp(1, 24 * 365);
        self.max_upload_mb = self.max_upload_mb.clamp(1, 8192);
        self.disk_alert_percent = self.disk_alert_percent.clamp(50, 100);
    }
}
