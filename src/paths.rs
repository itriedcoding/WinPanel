//! Filesystem layout and sandboxed path resolution.

use crate::error::{AppError, Result};
use crate::util::normalize_rel_path;
use std::path::{Path, PathBuf};

/// WinPanel data directory.
///
/// Windows: `%APPDATA%\WinPanel`
/// Linux/macOS: `$XDG_DATA_HOME/winpanel` or `~/.local/share/winpanel`
pub fn default_data_dir() -> PathBuf {
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("WinPanel");
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return PathBuf::from(profile)
                .join("AppData")
                .join("Roaming")
                .join("WinPanel");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("winpanel");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("winpanel");
    }
    PathBuf::from("winpanel-data")
}

#[derive(Clone)]
pub struct Layout {
    pub data_dir: PathBuf,
}

impl Layout {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join("winpanel.db")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn servers_dir(&self) -> PathBuf {
        self.data_dir.join("servers")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.data_dir.join("templates")
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [
            self.data_dir.clone(),
            self.log_dir(),
            self.servers_dir(),
            self.backups_dir(),
            self.templates_dir(),
            self.tmp_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// Resolve `rel` inside `root`, guaranteeing the result cannot escape it.
///
/// For existing targets the canonicalized path must stay under the canonical
/// root, which also defeats symlink/junction escapes. For new targets the
/// nearest existing ancestor is verified instead.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let normalized = normalize_rel_path(rel).map_err(AppError::bad_request)?;
    let root = root.canonicalize().map_err(|_| {
        AppError::internal(format!(
            "Server directory is unavailable: {}",
            root.display()
        ))
    })?;

    let mut target = root.clone();
    if !normalized.is_empty() {
        for part in normalized.split('/') {
            target.push(part);
        }
    }

    if target.exists() {
        let canonical = target
            .canonicalize()
            .map_err(|e| AppError::bad_request(format!("Cannot resolve path: {e}")))?;
        if !canonical.starts_with(&root) {
            return Err(AppError::forbidden("Path escapes the server directory"));
        }
        return Ok(canonical);
    }

    // Verify the deepest existing ancestor stays inside the root.
    let mut ancestor = target.parent().map(Path::to_path_buf);
    while let Some(candidate) = ancestor {
        if candidate.exists() {
            let canonical = candidate
                .canonicalize()
                .map_err(|e| AppError::bad_request(format!("Cannot resolve path: {e}")))?;
            if !canonical.starts_with(&root) {
                return Err(AppError::forbidden("Path escapes the server directory"));
            }
            break;
        }
        ancestor = candidate.parent().map(Path::to_path_buf);
    }

    Ok(target)
}

/// Approximate the on-disk size of a directory tree, following no symlinks.
pub fn dir_size(path: &Path) -> u64 {
    fn walk(dir: &Path, total: &mut u64, depth: usize) {
        if depth > 64 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                walk(&entry.path(), total, depth + 1);
            } else {
                *total = total.saturating_add(meta.len());
            }
        }
    }
    let mut total = 0;
    if path.is_dir() {
        walk(path, &mut total, 0);
    }
    total
}

/// Recursively copy `from` into `to` (creating `to`).
pub fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let meta = entry.metadata()?;
        if meta.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if meta.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Recursively delete a directory tree without following symlinks.
pub fn remove_dir_all(path: &Path) -> std::io::Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if meta.is_dir() {
        // Make read-only files deletable (common on Windows server files).
        for entry in std::fs::read_dir(path)?.flatten() {
            let entry_path = entry.path();
            if let Ok(m) = std::fs::symlink_metadata(&entry_path) {
                if m.is_dir() {
                    remove_dir_all(&entry_path)?;
                } else {
                    if m.permissions().readonly() {
                        let _ = set_readonly(&entry_path, false);
                    }
                    std::fs::remove_file(&entry_path)?;
                }
            }
        }
        std::fs::remove_dir(path)
    } else {
        if meta.permissions().readonly() {
            let _ = set_readonly(path, false);
        }
        std::fs::remove_file(path)
    }
}

/// Set or clear the read-only attribute in a cross-platform way.
///
/// On Unix this toggles the write bits instead of clearing every permission,
/// which is what `Permissions::set_readonly(false)` would otherwise do.
pub fn set_readonly(path: &Path, readonly: bool) -> std::io::Result<()> {
    let meta = std::fs::metadata(path)?;
    let mut perms = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        let updated = if readonly {
            mode & !0o222
        } else {
            mode | 0o200
        };
        perms.set_mode(updated);
    }
    #[cfg(not(unix))]
    {
        perms.set_readonly(readonly);
    }
    std::fs::set_permissions(path, perms)
}
