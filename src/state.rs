//! Shared application state.

use crate::auth::LoginLimiter;
use crate::backups::BackupManager;
use crate::config::Settings;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{event_bus, EventBus};
use crate::metrics::Metrics;
use crate::models::{Server, ServerView};
use crate::paths::Layout;
use crate::process::Supervisor;
use crate::templates::TemplateCollection;
use crate::updates::UpdateManager;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

pub struct AppState {
    pub db: Arc<Db>,
    pub layout: Layout,
    pub settings: Arc<RwLock<Settings>>,
    pub supervisor: Arc<Supervisor>,
    pub metrics: Arc<Metrics>,
    pub templates: Arc<RwLock<TemplateCollection>>,
    pub backups: Arc<BackupManager>,
    pub updates: Arc<UpdateManager>,
    pub events: EventBus,
    pub started_at: Instant,
    pub login_limiter: LoginLimiter,
    pub installs: Mutex<HashSet<String>>,
    pub listen: SocketAddr,
}

impl AppState {
    pub fn build(db: Arc<Db>, layout: Layout, listen: SocketAddr) -> Result<Arc<Self>> {
        let settings = Settings::load(&db, &layout)?;
        settings.save(&db)?;
        let settings = Arc::new(RwLock::new(settings));
        let events = event_bus();
        let supervisor = Supervisor::new(db.clone(), settings.clone(), events.clone());
        let backup_root = resolve_path(&layout, &settings_snapshot(&settings).backup_dir);
        let backups =
            BackupManager::new(db.clone(), supervisor.clone(), events.clone(), backup_root);
        let templates = Arc::new(RwLock::new(TemplateCollection::load(
            &layout.templates_dir(),
        )));

        Ok(Arc::new(Self {
            db,
            layout: layout.clone(),
            settings,
            supervisor,
            metrics: Arc::new(Metrics::new()),
            templates,
            backups,
            updates: Arc::new(UpdateManager::new(layout.tmp_dir())),
            events,
            started_at: Instant::now(),
            login_limiter: LoginLimiter::default(),
            installs: Mutex::new(HashSet::new()),
            listen,
        }))
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .map(|s| s.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    pub fn save_settings(&self, mut settings: Settings) -> Result<()> {
        settings.validate_and_clamp();
        settings.save(&self.db)?;
        let mut guard = self.settings.write().unwrap_or_else(|e| e.into_inner());
        *guard = settings;
        Ok(())
    }

    pub fn server_view(&self, server: Server) -> ServerView {
        let runtime = self.supervisor.runtime(&server.id);
        ServerView {
            server,
            runtime,
            level: "manage".to_string(),
        }
    }

    pub fn server_view_with_level(&self, server: Server, level: &str) -> ServerView {
        let runtime = self.supervisor.runtime(&server.id);
        ServerView {
            server,
            runtime,
            level: level.to_string(),
        }
    }

    pub fn default_install_dir(&self) -> PathBuf {
        let configured = self.settings().default_install_dir;
        resolve_path(&self.layout, &configured)
    }

    /// A deterministic, collision-free directory for a new server.
    pub fn install_dir_for(&self, name: &str, id: &str) -> PathBuf {
        let base = self.default_install_dir();
        let slug = slugify(name);
        let short = id.chars().take(8).collect::<String>();
        let candidate = base.join(format!("{slug}-{short}"));
        if candidate.exists() {
            base.join(format!(
                "{slug}-{}",
                id.chars().take(13).collect::<String>()
            ))
        } else {
            candidate
        }
    }

    pub fn begin_install(&self, server_id: &str) -> Result<()> {
        let mut set = self.installs.lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(server_id.to_string()) {
            return Err(AppError::conflict(
                "An installation is already running for this server",
            ));
        }
        Ok(())
    }

    pub fn end_install(&self, server_id: &str) {
        self.installs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(server_id);
    }

    pub fn is_installing(&self, server_id: &str) -> bool {
        self.installs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(server_id)
    }

    pub fn server_or_404(&self, id: &str) -> Result<Server> {
        self.db
            .server_by_id(id)?
            .ok_or_else(|| AppError::not_found("Server not found"))
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

pub fn settings_snapshot(settings: &Arc<RwLock<Settings>>) -> Settings {
    settings
        .read()
        .map(|s| s.clone())
        .unwrap_or_else(|e| e.into_inner().clone())
}

/// Resolve a possibly-relative configured path against the data directory.
pub fn resolve_path(layout: &Layout, value: &str) -> PathBuf {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        layout.data_dir.join(candidate)
    }
}

pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "server".to_string()
    } else {
        trimmed.chars().take(40).collect()
    }
}
