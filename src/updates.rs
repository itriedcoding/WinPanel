//! Update checking and application of updates from GitHub Releases.

use crate::error::{AppError, Result};
use crate::net;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const REPO: &str = "itriedcoding/WinPanel";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub available: bool,
    pub published_at: Option<String>,
    pub notes: Option<String>,
    pub release_url: Option<String>,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
    pub asset_size: Option<u64>,
    pub digest: Option<String>,
    pub checked_at: i64,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Release {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

pub struct UpdateManager {
    client: reqwest::Client,
    cache: Mutex<Option<UpdateInfo>>,
    downloaded: Mutex<Option<PathBuf>>,
    tmp_dir: PathBuf,
}

impl UpdateManager {
    pub fn new(tmp_dir: PathBuf) -> Self {
        Self {
            client: net::client(),
            cache: Mutex::new(None),
            downloaded: Mutex::new(None),
            tmp_dir,
        }
    }

    /// Apply the executable that was downloaded through `download`.
    pub fn apply_downloaded(&self) -> Result<()> {
        let path = self
            .downloaded
            .lock()
            .map_err(|_| AppError::internal("Update state is unavailable"))?
            .clone()
            .ok_or_else(|| AppError::conflict("Download an update before installing it"))?;
        self.apply(&path)
    }

    pub fn cached(&self) -> Option<UpdateInfo> {
        self.cache.lock().ok().and_then(|c| c.clone())
    }

    pub async fn check(&self, force: bool) -> UpdateInfo {
        if !force {
            if let Some(cached) = self.cached() {
                if crate::util::now_ms() - cached.checked_at < 15 * 60 * 1000 {
                    return cached;
                }
            }
        }
        let info = match self.fetch_latest().await {
            Ok(info) => info,
            Err(err) => UpdateInfo {
                current_version: CURRENT_VERSION.to_string(),
                latest_version: CURRENT_VERSION.to_string(),
                available: false,
                checked_at: crate::util::now_ms(),
                error: Some(err.message().to_string()),
                ..Default::default()
            },
        };
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some(info.clone());
        }
        info
    }

    async fn fetch_latest(&self) -> Result<UpdateInfo> {
        let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
        let release: Release = net::get_json(&self.client, &url).await?;
        if release.draft || release.prerelease {
            return Err(AppError::internal(
                "The latest GitHub release is a draft or pre-release",
            ));
        }
        let latest = release.tag_name.trim_start_matches('v').to_string();
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name.to_lowercase().ends_with(".exe"))
            .or_else(|| {
                release
                    .assets
                    .iter()
                    .find(|asset| asset.name.contains("winpanel"))
            });
        let available = is_newer(&latest, CURRENT_VERSION);
        Ok(UpdateInfo {
            current_version: CURRENT_VERSION.to_string(),
            latest_version: latest,
            available,
            published_at: release.published_at,
            notes: Some(if release.body.trim().is_empty() {
                release.name.clone()
            } else {
                release.body.clone()
            }),
            release_url: Some(release.html_url),
            asset_name: asset.map(|a| a.name.clone()),
            asset_url: asset.map(|a| a.browser_download_url.clone()),
            asset_size: asset.map(|a| a.size),
            digest: asset.and_then(|a| a.digest.clone()),
            checked_at: crate::util::now_ms(),
            error: None,
        })
    }

    /// Download and verify the release asset. Returns the local path.
    pub async fn download(&self, info: &UpdateInfo) -> Result<PathBuf> {
        let url = info
            .asset_url
            .as_ref()
            .ok_or_else(|| AppError::bad_request("This release has no downloadable executable"))?;
        if !info.available {
            return Err(AppError::conflict(
                "You are already running the latest version",
            ));
        }
        std::fs::create_dir_all(&self.tmp_dir)?;
        let file_name = format!(
            "winpanel-{}.exe",
            crate::util::sanitize_filename(&info.latest_version)
        );
        let dest = self.tmp_dir.join(file_name);
        net::download(&self.client, url, &dest, |_, _| {}).await?;

        let size = std::fs::metadata(&dest)?.len();
        if let Some(expected) = info.asset_size {
            if expected > 0 && size != expected {
                let _ = std::fs::remove_file(&dest);
                return Err(AppError::internal(format!(
                    "Downloaded update has the wrong size ({size} bytes, expected {expected})"
                )));
            }
        }
        let actual = crate::backups::sha256_file(&dest)?;
        if let Some(digest) = &info.digest {
            let expected = digest
                .strip_prefix("sha256:")
                .unwrap_or(digest.as_str())
                .trim();
            if !actual.eq_ignore_ascii_case(expected) {
                let _ = std::fs::remove_file(&dest);
                return Err(AppError::internal(
                    "Update failed checksum verification and was discarded",
                ));
            }
        }
        let checksum_file = dest.with_extension("exe.sha256");
        std::fs::write(&checksum_file, &actual)?;
        if let Ok(mut slot) = self.downloaded.lock() {
            *slot = Some(dest.clone());
        }

        #[cfg(windows)]
        {
            // Refuse to install something that is not a Windows executable.
            use std::io::Read;
            let mut header = [0u8; 2];
            if let Ok(mut file) = std::fs::File::open(&dest) {
                let _ = file.read_exact(&mut header);
            }
            if header != [b'M', b'Z'] {
                let _ = std::fs::remove_file(&dest);
                return Err(AppError::internal(
                    "Downloaded file is not a valid Windows executable",
                ));
            }
        }
        Ok(dest)
    }

    /// Replace the running executable and restart. On Windows the running image
    /// is locked, so a small helper script swaps it after this process exits.
    pub fn apply(&self, downloaded: &Path) -> Result<()> {
        let current = std::env::current_exe().map_err(|e| {
            AppError::internal(format!("Cannot locate the running executable: {e}"))
        })?;
        if !downloaded.is_file() {
            return Err(AppError::not_found("Downloaded update file is missing"));
        }

        #[cfg(windows)]
        {
            let script = self.tmp_dir.join("apply-update.cmd");
            let log = self.tmp_dir.join("apply-update.log");
            let body = format!(
                "@echo off\r\n\
                 setlocal\r\n\
                 :wait\r\n\
                 tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\n\
                 if not errorlevel 1 (\r\n\
                   timeout /t 1 /nobreak >nul\r\n\
                   goto wait\r\n\
                 )\r\n\
                 if exist \"{current}.old\" del /F /Q \"{current}.old\" >nul 2>&1\r\n\
                 move /Y \"{current}\" \"{current}.old\" >>\"{log}\" 2>&1\r\n\
                 move /Y \"{downloaded}\" \"{current}\" >>\"{log}\" 2>&1\r\n\
                 if errorlevel 1 (\r\n\
                   move /Y \"{current}.old\" \"{current}\" >>\"{log}\" 2>&1\r\n\
                   goto end\r\n\
                 )\r\n\
                 start \"\" \"{current}\"\r\n\
                 :end\r\n\
                 del /F /Q \"%~f0\" >nul 2>&1\r\n",
                pid = std::process::id(),
                current = current.display(),
                downloaded = downloaded.display(),
                log = log.display()
            );
            std::fs::write(&script, body)
                .map_err(|e| AppError::internal(format!("Cannot write the update script: {e}")))?;
            std::process::Command::new("cmd")
                .args(["/C", "start", "/min", ""])
                .arg(&script)
                .spawn()
                .map_err(|e| AppError::internal(format!("Cannot launch the update script: {e}")))?;
            tracing::info!("update helper started; exiting for the swap");
            std::process::exit(0);
        }
        #[cfg(unix)]
        {
            let staged = current.with_extension("new");
            std::fs::copy(downloaded, &staged)
                .map_err(|e| AppError::internal(format!("Cannot stage the new executable: {e}")))?;
            let mut perms = std::fs::metadata(&staged)?.permissions();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                perms.set_mode(0o755);
            }
            std::fs::set_permissions(&staged, perms)?;
            std::fs::rename(&staged, &current)
                .map_err(|e| AppError::internal(format!("Cannot replace the executable: {e}")))?;
            std::process::Command::new(&current)
                .spawn()
                .map_err(|e| AppError::internal(format!("Cannot restart WinPanel: {e}")))?;
            tracing::info!("restarting with the updated executable");
            std::process::exit(0);
        }
        #[allow(unreachable_code)]
        Ok(())
    }
}

/// Compare dotted version strings. Returns true when `candidate` > `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |value: &str| -> Vec<u64> {
        value
            .trim_start_matches('v')
            .split(['.', '-', '+'])
            .map(|part| {
                part.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    };
    let a = parse(candidate);
    let b = parse(current);
    for index in 0..a.len().max(b.len()) {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(is_newer("v2.0.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(is_newer("1.0.1", "1.0"));
    }
}
