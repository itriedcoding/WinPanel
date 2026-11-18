//! Zip-based backups with restore, retention and scheduling.

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{emit, Event, EventBus};
use crate::models::{Backup, Server, STATUS_RUNNING, STATUS_STARTING};
use crate::paths;
use crate::process::Supervisor;
use crate::util::{now_ms, sanitize_filename, uuid_str};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zip::write::SimpleFileOptions;

pub struct BackupManager {
    db: Arc<Db>,
    supervisor: Arc<Supervisor>,
    events: EventBus,
    backup_root: PathBuf,
    in_progress: Mutex<HashSet<String>>,
}

impl BackupManager {
    pub fn new(
        db: Arc<Db>,
        supervisor: Arc<Supervisor>,
        events: EventBus,
        backup_root: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            supervisor,
            events,
            backup_root,
            in_progress: Mutex::new(HashSet::new()),
        })
    }

    fn archive_path(&self, server_id: &str, backup_id: &str, name: &str) -> PathBuf {
        self.backup_root.join(server_id).join(format!(
            "{}-{}.zip",
            sanitize_filename(name),
            backup_id
        ))
    }

    pub fn create(self: &Arc<Self>, server: &Server, name: Option<String>) -> Result<Backup> {
        {
            let mut set = self.in_progress.lock().unwrap_or_else(|e| e.into_inner());
            if !set.insert(server.id.clone()) {
                return Err(AppError::conflict(
                    "A backup for this server is already running",
                ));
            }
        }
        let label = name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d %H-%M").to_string());
        if label.chars().count() > 80 {
            self.release(&server.id);
            return Err(AppError::bad_request("Backup name is too long"));
        }

        let id = uuid_str();
        let archive = self.archive_path(&server.id, &id, &label);
        let record = Backup {
            id: id.clone(),
            server_id: server.id.clone(),
            name: label.clone(),
            size: 0,
            created_at: now_ms(),
            status: "running".to_string(),
            checksum: None,
            error: None,
        };
        if let Err(err) = self.db.insert_backup(&record, &archive.to_string_lossy()) {
            self.release(&server.id);
            return Err(err);
        }
        emit(
            &self.events,
            Event::new(
                "backup",
                Some(server.id.clone()),
                json!({ "status": "running", "backupId": id, "name": label }),
            ),
        );

        let source = PathBuf::from(&server.install_dir);
        let result = (|| -> Result<(u64, String)> {
            if !source.is_dir() {
                return Err(AppError::bad_request("Server directory does not exist"));
            }
            if let Some(parent) = archive.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let size = write_archive(&source, &archive)?;
            let checksum = sha256_file(&archive)?;
            Ok((size, checksum))
        })();

        self.release(&server.id);
        match result {
            Ok((size, checksum)) => {
                self.db
                    .update_backup(&id, size, "complete", Some(&checksum), None)?;
                self.db.log_activity(
                    "backup",
                    Some(&server.id),
                    None,
                    &format!("Created backup \"{label}\""),
                    Some(&format!("{size} bytes")),
                )?;
                emit(
                    &self.events,
                    Event::new(
                        "backup",
                        Some(server.id.clone()),
                        json!({ "status": "complete", "backupId": id, "name": label, "size": size }),
                    ),
                );
                Ok(Backup {
                    size,
                    status: "complete".to_string(),
                    checksum: Some(checksum),
                    ..record
                })
            }
            Err(err) => {
                let message = err.message().to_string();
                let _ = std::fs::remove_file(&archive);
                self.db
                    .update_backup(&id, 0, "failed", None, Some(&message))?;
                emit(
                    &self.events,
                    Event::new(
                        "backup",
                        Some(server.id.clone()),
                        json!({ "status": "failed", "backupId": id, "error": message }),
                    ),
                );
                Err(err)
            }
        }
    }

    fn release(&self, server_id: &str) {
        self.in_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(server_id);
    }

    /// Restore a backup. The server must be stopped so files cannot be written
    /// underneath a running process.
    pub fn restore(&self, server: &Server, backup_id: &str) -> Result<()> {
        let (backup, archive) = self
            .db
            .backup_by_id(backup_id)?
            .ok_or_else(|| AppError::not_found("Backup not found"))?;
        if backup.server_id != server.id {
            return Err(AppError::forbidden("Backup belongs to another server"));
        }
        let runtime = self.supervisor.runtime(&server.id);
        if matches!(
            runtime.status.as_str(),
            STATUS_RUNNING | STATUS_STARTING | "stopping"
        ) {
            return Err(AppError::conflict(
                "Stop the server before restoring a backup",
            ));
        }
        let archive = PathBuf::from(archive);
        if !archive.is_file() {
            return Err(AppError::not_found(format!(
                "Backup archive is missing: {}",
                archive.display()
            )));
        }
        if let Some(expected) = &backup.checksum {
            let actual = sha256_file(&archive)?;
            if &actual != expected {
                return Err(AppError::internal(
                    "Backup archive is corrupted (checksum mismatch); restore aborted",
                ));
            }
        }

        let install_dir = PathBuf::from(&server.install_dir);
        let parent = install_dir
            .parent()
            .ok_or_else(|| AppError::internal("Invalid server directory"))?
            .to_path_buf();
        std::fs::create_dir_all(&parent)?;
        let staging = parent.join(format!(".restore-{}", uuid_str()));
        std::fs::create_dir_all(&staging)?;

        let extract_result = extract_zip(&archive, &staging);
        if let Err(err) = extract_result {
            let _ = paths::remove_dir_all(&staging);
            return Err(err);
        }

        let previous = parent.join(format!(".previous-{}", uuid_str()));
        if install_dir.exists() {
            std::fs::rename(&install_dir, &previous).map_err(|e| {
                let _ = paths::remove_dir_all(&staging);
                AppError::internal(format!(
                    "Could not move the current server directory aside: {e}"
                ))
            })?;
        }
        if let Err(err) = std::fs::rename(&staging, &install_dir) {
            // Roll back so the server keeps working with its previous files.
            if previous.exists() {
                let _ = std::fs::rename(&previous, &install_dir);
            }
            let _ = paths::remove_dir_all(&staging);
            return Err(AppError::internal(format!("Restore failed: {err}")));
        }
        let _ = paths::remove_dir_all(&previous);

        self.db.log_activity(
            "backup",
            Some(&server.id),
            None,
            &format!("Restored backup \"{}\"", backup.name),
            None,
        )?;
        emit(
            &self.events,
            Event::new(
                "backup",
                Some(server.id.clone()),
                json!({ "status": "restored", "backupId": backup.id }),
            ),
        );
        Ok(())
    }

    pub fn delete(&self, server_id: &str, backup_id: &str) -> Result<()> {
        let (backup, archive) = self
            .db
            .backup_by_id(backup_id)?
            .ok_or_else(|| AppError::not_found("Backup not found"))?;
        if backup.server_id != server_id {
            return Err(AppError::forbidden("Backup belongs to another server"));
        }
        let path = PathBuf::from(archive);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        self.db.delete_backup(backup_id)?;
        Ok(())
    }

    /// Delete all archives that belong to a server (used when deleting it).
    pub fn purge_server(&self, server_id: &str) {
        let dir = self.backup_root.join(server_id);
        let _ = paths::remove_dir_all(&dir);
    }

    /// Keep at most `keep` backups for a server, newest first.
    pub fn prune(&self, server_id: &str, keep: u32) -> Result<()> {
        let backups = self.db.list_backups(server_id)?;
        for backup in backups.into_iter().skip(keep.max(1) as usize) {
            if let Ok((_, archive)) = self
                .db
                .backup_by_id(&backup.id)
                .map(|opt| opt.unwrap_or((backup.clone(), String::new())))
            {
                if !archive.is_empty() {
                    let _ = std::fs::remove_file(archive);
                }
            }
            let _ = self.db.delete_backup(&backup.id);
        }
        Ok(())
    }

    pub async fn scheduler(self: Arc<Self>) {
        // Give the rest of the panel time to finish booting.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            self.run_due_backups().await;
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }

    async fn run_due_backups(self: &Arc<Self>) {
        let servers = match self.db.list_servers() {
            Ok(servers) => servers,
            Err(err) => {
                tracing::error!("backup scheduler cannot list servers: {err}");
                return;
            }
        };
        for server in servers {
            if !server.backups_enabled {
                continue;
            }
            let backups = match self.db.list_backups(&server.id) {
                Ok(backups) => backups,
                Err(err) => {
                    tracing::error!("backup scheduler cannot list backups: {err}");
                    continue;
                }
            };
            let due = match backups.first() {
                Some(latest) => {
                    now_ms() - latest.created_at
                        >= server.backup_interval_hours.max(1) as i64 * 3_600_000
                }
                None => true,
            };
            if !due {
                continue;
            }
            let manager = Arc::clone(self);
            let server_clone = server.clone();
            let keep = server.backup_keep;
            let result = tokio::task::spawn_blocking(move || {
                let backup = manager.create(&server_clone, Some("Scheduled".to_string()))?;
                if let Err(err) = manager.prune(&server_clone.id, keep) {
                    tracing::warn!("could not prune backups: {err}");
                }
                Ok::<_, AppError>(backup)
            })
            .await;
            match result {
                Ok(Ok(backup)) => {
                    tracing::info!("scheduled backup {} created for {}", backup.id, server.id)
                }
                Ok(Err(err)) => tracing::warn!("scheduled backup failed: {err}"),
                Err(err) => tracing::warn!("scheduled backup task failed: {err}"),
            }
        }
    }
}

fn write_archive(source: &Path, dest: &Path) -> Result<u64> {
    let file = std::fs::File::create(dest)
        .map_err(|e| AppError::internal(format!("Cannot create backup archive: {e}")))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let dir_options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);

    let mut stack = vec![(source.to_path_buf(), 0usize)];
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;

    while let Some((dir, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!("backup cannot read {}: {err}", dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_symlink() {
                continue;
            }
            let Ok(relative) = path.strip_prefix(source) else {
                continue;
            };
            let name = relative.to_string_lossy().replace('\\', "/");
            if name.starts_with(".restore-") || name.starts_with(".previous-") {
                continue;
            }
            if meta.is_dir() {
                zip.add_directory(format!("{name}/"), dir_options)?;
                stack.push((path, depth + 1));
            } else if meta.is_file() {
                zip.start_file(&name, options)?;
                let mut input = match std::fs::File::open(&path) {
                    Ok(input) => input,
                    Err(err) => {
                        tracing::warn!("backup cannot open {}: {err}", path.display());
                        continue;
                    }
                };
                loop {
                    let read = input.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    zip.write_all(&buffer[..read])?;
                    total += read as u64;
                }
            }
        }
    }
    zip.finish()?;
    Ok(std::fs::metadata(dest).map(|m| m.len()).unwrap_or(total))
}

pub fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let out_path = dest.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
        written += entry.size();
        if written > 64 * 1024 * 1024 * 1024 {
            return Err(AppError::bad_request("Backup archive is too large"));
        }
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
