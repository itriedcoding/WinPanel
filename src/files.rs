//! Sandboxed file manager operations.
//!
//! Every path is resolved through [`crate::paths::safe_join`], so nothing can
//! escape the server directory. Destructive operations refuse to touch the
//! root, refuse to follow symlinks while recursing, and require an explicit
//! recursive flag for directories.

use crate::error::{AppError, Result};
use crate::paths::safe_join;
use crate::util::normalize_rel_path;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Files larger than this are not offered for in-browser editing.
pub const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024;
/// Hard cap for a single in-browser edit write.
pub const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "log",
    "json",
    "yml",
    "yaml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "config",
    "properties",
    "md",
    "xml",
    "html",
    "htm",
    "css",
    "js",
    "mjs",
    "cjs",
    "ts",
    "jsx",
    "tsx",
    "rs",
    "py",
    "java",
    "kt",
    "sh",
    "bat",
    "cmd",
    "ps1",
    "env",
    "lua",
    "sql",
    "csv",
    "gitignore",
    "editorconfig",
    "mcmeta",
    "mcfunction",
    "snbt",
    "lang",
    "list",
    "prefs",
    "lock",
    "jar.txt",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
    pub readonly: bool,
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<FileEntry>,
    pub total_size: u64,
    pub truncated: bool,
}

pub fn rel_of(path: &Path, root: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

pub fn is_editable_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.ends_with(".lock") && lower != "session.lock" {
        return false;
    }
    match lower.rsplit_once('.') {
        Some((_, ext)) => TEXT_EXTENSIONS.contains(&ext),
        None => !lower.is_empty() && !lower.contains("jar") && lower.len() < 64,
    }
}

pub fn list_dir(root: &Path, rel: &str, show_hidden: bool) -> Result<DirListing> {
    let dir = safe_join(root, rel)?;
    if !dir.is_dir() {
        return Err(AppError::not_found("Directory not found"));
    }
    let mut entries = Vec::new();
    let mut total_size = 0u64;
    let mut truncated = false;

    let mut read = std::fs::read_dir(&dir)
        .map_err(|e| AppError::bad_request(format!("Cannot read directory: {e}")))?;
    for entry in &mut read {
        if entries.len() >= 2000 {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let is_dir = meta.is_dir();
        let size = if is_dir { 0 } else { meta.len() };
        total_size += size;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        entries.push(FileEntry {
            path: if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            },
            editable: !is_dir && size <= MAX_EDIT_BYTES && is_editable_name(&name),
            name,
            is_dir,
            size,
            modified,
            readonly: meta.permissions().readonly(),
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let parent = if rel.is_empty() {
        None
    } else {
        let parent = Path::new(rel)
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        Some(parent)
    };

    Ok(DirListing {
        path: rel.to_string(),
        parent,
        entries,
        total_size,
        truncated,
    })
}

pub fn read_text(root: &Path, rel: &str) -> Result<String> {
    let path = safe_join(root, rel)?;
    let meta = std::fs::metadata(&path).map_err(|_| AppError::not_found("File not found"))?;
    if meta.is_dir() {
        return Err(AppError::bad_request("Path is a directory"));
    }
    if meta.len() > MAX_EDIT_BYTES {
        return Err(AppError::bad_request(format!(
            "File is too large to edit in the browser ({}). Download it instead.",
            crate::util::format_bytes(meta.len())
        )));
    }
    let bytes = std::fs::read(&path)?;
    match String::from_utf8(bytes.clone()) {
        Ok(text) => Ok(text),
        Err(_) => {
            // Try a lossy decode only when the file is clearly text-like.
            let text = String::from_utf8_lossy(&bytes).to_string();
            if text.contains('\u{0}') {
                Err(AppError::bad_request(
                    "This file appears to be binary and cannot be edited as text",
                ))
            } else {
                Ok(text)
            }
        }
    }
}

pub fn write_text(root: &Path, rel: &str, content: &str) -> Result<u64> {
    if content.len() > MAX_WRITE_BYTES {
        return Err(AppError::bad_request("Content is too large"));
    }
    let path = safe_join(root, rel)?;
    if path.is_dir() {
        return Err(AppError::bad_request("Path is a directory"));
    }
    crate::util::atomic_write(&path, content.as_bytes())
        .map_err(|e| AppError::internal(format!("Could not save file: {e}")))?;
    Ok(content.len() as u64)
}

pub fn create_dir(root: &Path, rel: &str) -> Result<()> {
    let path = safe_join(root, rel)?;
    if path.exists() {
        return Err(AppError::conflict(
            "A file or folder with that name already exists",
        ));
    }
    std::fs::create_dir_all(&path)?;
    Ok(())
}

pub fn create_file(root: &Path, rel: &str) -> Result<()> {
    let path = safe_join(root, rel)?;
    if path.exists() {
        return Err(AppError::conflict(
            "A file or folder with that name already exists",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"")?;
    Ok(())
}

pub fn rename(root: &Path, from: &str, to: &str) -> Result<()> {
    let from_path = safe_join(root, from)?;
    let to_path = safe_join(root, to)?;
    if !from_path.exists() {
        return Err(AppError::not_found("Source path not found"));
    }
    if to_path.exists() {
        return Err(AppError::conflict("Destination already exists"));
    }
    if to_path.starts_with(&from_path) {
        return Err(AppError::bad_request("Cannot move a folder into itself"));
    }
    std::fs::rename(&from_path, &to_path)
        .map_err(|e| AppError::internal(format!("Could not move: {e}")))?;
    Ok(())
}

pub fn copy(root: &Path, from: &str, to: &str, overwrite: bool) -> Result<()> {
    let from_path = safe_join(root, from)?;
    let to_path = safe_join(root, to)?;
    if !from_path.exists() {
        return Err(AppError::not_found("Source path not found"));
    }
    if to_path.starts_with(&from_path) {
        return Err(AppError::bad_request("Cannot copy a folder into itself"));
    }
    if to_path.exists() && !overwrite {
        return Err(AppError::conflict("Destination already exists"));
    }
    copy_path(&from_path, &to_path, 0)?;
    Ok(())
}

fn copy_path(from: &Path, to: &Path, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(AppError::bad_request("Directory tree is too deep"));
    }
    let meta = std::fs::symlink_metadata(from)?;
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let child_meta = entry.metadata()?;
            if child_meta.is_symlink() {
                continue;
            }
            copy_path(&entry.path(), &to.join(entry.file_name()), depth + 1)?;
        }
    } else if meta.is_file() {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

pub fn delete(root: &Path, rel: &str, recursive: bool) -> Result<()> {
    let path = safe_join(root, rel)?;
    let root_canonical = root
        .canonicalize()
        .map_err(|_| AppError::internal("Server directory missing"))?;
    if path == root_canonical {
        return Err(AppError::forbidden(
            "The server root directory cannot be deleted",
        ));
    }
    let meta =
        std::fs::symlink_metadata(&path).map_err(|_| AppError::not_found("Path not found"))?;
    if meta.is_dir() {
        let empty = std::fs::read_dir(&path)?.next().is_none();
        if !empty && !recursive {
            return Err(AppError::bad_request(
                "Folder is not empty. Enable recursive delete to remove it.",
            ));
        }
        crate::paths::remove_dir_all(&path)?;
    } else {
        if meta.permissions().readonly() {
            let _ = crate::paths::set_readonly(&path, false);
        }
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn set_readonly(root: &Path, rel: &str, readonly: bool) -> Result<()> {
    let path = safe_join(root, rel)?;
    if !path.exists() {
        return Err(AppError::not_found("Path not found"));
    }
    crate::paths::set_readonly(&path, readonly)?;
    Ok(())
}

pub fn search(root: &Path, query: &str, limit: usize) -> Result<Vec<FileEntry>> {
    let query = query.trim().to_lowercase();
    if query.len() < 2 {
        return Err(AppError::bad_request("Search needs at least 2 characters"));
    }
    let root = root
        .canonicalize()
        .map_err(|_| AppError::not_found("Server directory not found"))?;
    let mut results = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    let mut scanned = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        if depth > 16 || scanned > 25_000 || results.len() >= limit {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            scanned += 1;
            if scanned > 25_000 {
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_symlink() {
                continue;
            }
            if name.to_lowercase().contains(&query) {
                results.push(FileEntry {
                    editable: !meta.is_dir()
                        && meta.len() <= MAX_EDIT_BYTES
                        && is_editable_name(&name),
                    name: name.clone(),
                    path: rel_of(&entry.path(), &root),
                    is_dir: meta.is_dir(),
                    size: if meta.is_dir() { 0 } else { meta.len() },
                    modified: meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64),
                    readonly: meta.permissions().readonly(),
                });
            }
            if meta.is_dir() && depth < 16 {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    results.sort_by_key(|entry| entry.path.to_lowercase());
    Ok(results)
}

/// Reject writes that would exceed the server's storage allowance.
pub fn check_quota(root: &Path, limit_mb: u64, additional_bytes: u64) -> Result<()> {
    if limit_mb == 0 {
        return Ok(());
    }
    let limit = limit_mb.saturating_mul(1024 * 1024);
    let used = crate::paths::dir_size(root);
    if used.saturating_add(additional_bytes) > limit {
        return Err(AppError::bad_request(format!(
            "Storage limit reached: {} of {} used",
            crate::util::format_bytes(used),
            crate::util::format_bytes(limit)
        )));
    }
    Ok(())
}

/// Resolve an upload destination, creating parent directories as needed.
pub fn upload_target(root: &Path, rel_dir: &str, filename: &str) -> Result<PathBuf> {
    let safe_name = crate::util::sanitize_filename(filename);
    let dir = normalize_rel_path(rel_dir).map_err(AppError::bad_request)?;
    let combined = if dir.is_empty() {
        safe_name
    } else {
        format!("{dir}/{safe_name}")
    };
    let path = safe_join(root, &combined)?;
    if path.is_dir() {
        return Err(AppError::conflict("A folder with that name already exists"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("winpanel-files-{}", crate::util::random_token(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn listing_and_writing_stays_inside_root() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("world")).unwrap();
        std::fs::write(root.join("server.properties"), "motd=hi\n").unwrap();

        let listing = list_dir(&root, "", true).unwrap();
        assert!(listing
            .entries
            .iter()
            .any(|e| e.name == "world" && e.is_dir));
        assert!(listing
            .entries
            .iter()
            .any(|e| e.name == "server.properties" && e.editable));

        write_text(&root, "world/level.dat", "hello").unwrap();
        assert_eq!(read_text(&root, "world/level.dat").unwrap(), "hello");

        assert!(read_text(&root, "../../etc/passwd").is_err());
        assert!(write_text(&root, "../escape.txt", "nope").is_err());
        assert!(delete(&root, "", true).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn copy_and_rename_work() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a/file.txt"), "content").unwrap();
        copy(&root, "a", "b", false).unwrap();
        assert!(root.join("b/file.txt").exists());
        rename(&root, "b/file.txt", "b/renamed.txt").unwrap();
        assert!(root.join("b/renamed.txt").exists());
        assert!(copy(&root, "a", "a/inner", false).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
