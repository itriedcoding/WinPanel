//! Template installers.
//!
//! Every method below performs a real download from the upstream project's
//! official API and verifies it where the API provides a checksum. Nothing is
//! simulated: a failed download produces a failed install with the upstream
//! error message attached to the server record.

use crate::config::Settings;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::events::{emit, Event, EventBus};
use crate::models::{Server, STATUS_STOPPED};
use crate::net;
use crate::process::Supervisor;
use crate::templates::{self, Template};
use crate::util::now_ms;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub struct InstallRequest {
    pub server: Server,
    pub template: Template,
    pub options: BTreeMap<String, String>,
    pub db: Arc<Db>,
    pub supervisor: Arc<Supervisor>,
    pub events: EventBus,
    pub settings: Arc<RwLock<Settings>>,
}

struct Progress {
    supervisor: Arc<Supervisor>,
    events: EventBus,
    server_id: String,
}

impl Progress {
    fn step(&self, percent: u8, label: &str) {
        let value = percent.min(99);
        self.supervisor
            .set_install(&self.server_id, Some(value), Some(label.to_string()));
        emit(
            &self.events,
            Event::new(
                "install",
                Some(self.server_id.clone()),
                json!({ "progress": value, "step": label }),
            ),
        );
    }
}

/// Run a full installation. The caller spawns this in the background; the
/// returned server reflects the final stored state.
pub async fn run(request: InstallRequest) -> Result<Server> {
    let InstallRequest {
        mut server,
        template,
        options,
        db,
        supervisor,
        events,
        settings,
    } = request;
    let http = net::client();
    let progress = Progress {
        supervisor: supervisor.clone(),
        events: events.clone(),
        server_id: server.id.clone(),
    };
    progress.step(2, "Preparing installation");

    let outcome = perform(
        &mut server,
        &template,
        &options,
        &http,
        &db,
        &settings,
        &progress,
    )
    .await;

    match outcome {
        Ok(()) => {
            server.installed = true;
            server.install_error = None;
            server.status = STATUS_STOPPED.to_string();
            server.updated_at = now_ms();
            server.install_dir = PathBuf::from(&server.install_dir)
                .to_string_lossy()
                .to_string();
            db.update_server(&server)?;
            db.set_install_state(&server.id, true, None, STATUS_STOPPED)?;
            supervisor.set_install(&server.id, Some(100), Some("Installation complete".into()));
            emit(
                &events,
                Event::new(
                    "install",
                    Some(server.id.clone()),
                    json!({ "progress": 100, "step": "Installation complete", "status": "complete" }),
                ),
            );
            emit(
                &events,
                Event::new(
                    "status",
                    Some(server.id.clone()),
                    json!({ "status": STATUS_STOPPED }),
                ),
            );
            db.log_activity(
                "install",
                Some(&server.id),
                None,
                &format!("Installed {}", server.name),
                Some(&template.name),
            )?;
            Ok(server)
        }
        Err(err) => {
            let message = err.message().to_string();
            let _ = db.set_install_state(
                &server.id,
                server.installed,
                Some(&message),
                if server.installed {
                    STATUS_STOPPED
                } else {
                    crate::models::STATUS_INSTALL_FAILED
                },
            );
            supervisor.set_install(&server.id, None, Some(message.clone()));
            emit(
                &events,
                Event::new(
                    "install",
                    Some(server.id.clone()),
                    json!({ "status": "failed", "error": message, "progress": null, "step": null }),
                ),
            );
            let _ = db.log_activity(
                "error",
                Some(&server.id),
                None,
                &format!("Installation of {} failed", server.name),
                Some(&message),
            );
            Err(err)
        }
    }
}

async fn perform(
    server: &mut Server,
    template: &Template,
    options: &BTreeMap<String, String>,
    http: &reqwest::Client,
    db: &Db,
    settings: &RwLock<Settings>,
    progress: &Progress,
) -> Result<()> {
    let install_dir = PathBuf::from(&server.install_dir);
    tokio::fs::create_dir_all(&install_dir).await?;

    progress.step(5, "Checking required runtimes");
    for requirement in &template.runtime {
        let status = crate::runtime::detect(&requirement.kind);
        if !status.available {
            if requirement.required {
                return Err(AppError::bad_request(format!(
                    "{} is required but was not found. {}.",
                    status.label, status.hint
                )));
            }
            tracing::warn!(
                "optional runtime {} missing for {}",
                requirement.kind,
                template.id
            );
            continue;
        }
        if requirement.minimum_major > 0 {
            if let Some(version) = &status.version {
                if !crate::runtime::version_at_least(version, requirement.minimum_major) {
                    return Err(AppError::bad_request(format!(
                        "{} {} is installed but version {} or newer is required (detected: {})",
                        status.label, requirement.minimum_major, requirement.minimum_major, version
                    )));
                }
            }
        }
    }

    let mut values: HashMap<String, String> = options
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let port = server
        .ports
        .first()
        .map(|p| p.port)
        .unwrap_or(template.port);
    values.extend(templates::runtime_values(server.memory_mb, port));
    values.insert(
        "serverName".to_string(),
        values
            .get("serverName")
            .cloned()
            .unwrap_or_else(|| server.name.clone()),
    );
    values.insert(
        "memoryMb".to_string(),
        server.memory_mb.max(256).to_string(),
    );

    let method = template.install.method.clone();
    let method = method.as_str();
    match method {
        "none" => {
            progress.step(30, "No download required");
        }
        "download" => {
            progress.step(10, "Resolving download");
            let url = templates::render(&template.install.url, &values);
            let target = if template.install.target.trim().is_empty() {
                url.rsplit('/')
                    .next()
                    .unwrap_or("download.bin")
                    .split('?')
                    .next()
                    .unwrap_or("download.bin")
                    .to_string()
            } else {
                templates::render(&template.install.target, &values)
            };
            let target = crate::util::sanitize_filename(&target);
            let dest = install_dir.join(&target);
            download(http, &url, &dest, progress, "Downloading").await?;
            if template.install.archive == "zip" {
                progress.step(85, "Extracting archive");
                let archive = dest.clone();
                let dir = install_dir.clone();
                tokio::task::spawn_blocking(move || crate::backups::extract_zip(&archive, &dir))
                    .await
                    .map_err(|e| AppError::internal(format!("Extraction task failed: {e}")))??;
                let _ = tokio::fs::remove_file(&dest).await;
            }
        }
        "minecraft_vanilla" => {
            install_vanilla(server, http, &values, &install_dir, progress).await?;
        }
        "paper" => {
            install_paper(http, &values, &install_dir, progress).await?;
        }
        "bedrock" => {
            install_bedrock(http, &install_dir, progress).await?;
        }
        "fivem" => {
            install_fivem(http, &install_dir, progress).await?;
        }
        other => {
            return Err(AppError::bad_request(format!(
                "Template uses unsupported install method \"{other}\""
            )));
        }
    }

    progress.step(92, "Writing configuration files");
    for file in &template.files {
        let relative = crate::util::normalize_rel_path(&file.path)
            .map_err(|e| AppError::bad_request(format!("Invalid template file path: {e}")))?;
        if relative.is_empty() {
            continue;
        }
        let target = crate::paths::safe_join(&install_dir, &relative)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let content = templates::render(&file.content, &values);
        tokio::fs::write(&target, content.as_bytes())
            .await
            .map_err(|e| AppError::internal(format!("Cannot write {}: {e}", target.display())))?;
    }

    // Apply the template's startup command, resolving placeholders with the
    // options that were just used.
    if !template.startup_exe.trim().is_empty() {
        let rendered = templates::render(&template.startup_exe, &values);
        server.startup_exe = rendered;
    } else if let Some(exe) = values
        .get("exeName")
        .filter(|value| !value.trim().is_empty())
    {
        server.startup_exe = exe.clone();
    }
    if !template.startup_args.trim().is_empty() {
        server.startup_args = templates::render(&template.startup_args, &values);
    }
    if !template.stop_command.trim().is_empty() {
        server.stop_command = template.stop_command.clone();
    }
    server.install_options = options.clone();
    server.updated_at = now_ms();

    // Bedrock ships a platform-specific binary name.
    if template.id == "minecraft-bedrock" {
        let (win, unix) = ("bedrock_server.exe", "bedrock_server");
        if install_dir.join(win).exists() {
            server.startup_exe = win.to_string();
        } else if install_dir.join(unix).exists() {
            server.startup_exe = unix.to_string();
        }
    }

    // Storage limit is advisory metadata; warn early when the install already
    // exceeded it instead of failing silently later.
    if server.storage_mb > 0 {
        let size = crate::paths::dir_size(&install_dir);
        if size > server.storage_mb * 1024 * 1024 {
            tracing::warn!(
                "server {} exceeds its storage limit after install",
                server.id
            );
        }
    }
    if !server.installed {
        let _ = db.set_server_status(&server.id, STATUS_STOPPED, None);
    }
    let _ = settings;
    progress.step(98, "Finalizing");
    Ok(())
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: &Progress,
    label: &str,
) -> Result<u64> {
    let supervisor = progress.supervisor.clone();
    let server_id = progress.server_id.clone();
    let step_label = label.to_string();
    let size = net::download(client, url, dest, move |written, total_bytes| {
        let percent = match total_bytes {
            Some(size) if size > 0 => (20u64 + written.saturating_mul(70) / size).min(90) as u8,
            _ => 30,
        };
        let detail = match total_bytes {
            Some(size) => format!(
                "{step_label} - {} / {}",
                crate::util::format_bytes(written),
                crate::util::format_bytes(size)
            ),
            None => format!("{step_label} - {}", crate::util::format_bytes(written)),
        };
        supervisor.set_install(&server_id, Some(percent), Some(detail));
    })
    .await?;
    Ok(size)
}

// ------------------------------------------------------------------ vanilla

#[derive(Deserialize)]
struct VersionManifest {
    latest: LatestVersions,
    versions: Vec<VersionEntry>,
}

#[derive(Deserialize)]
struct LatestVersions {
    release: String,
}

#[derive(Deserialize)]
struct VersionEntry {
    id: String,
    url: String,
}

#[derive(Deserialize)]
struct VersionDetails {
    downloads: VersionDownloads,
}

#[derive(Deserialize)]
struct VersionDownloads {
    server: Option<ServerDownload>,
}

#[derive(Deserialize)]
struct ServerDownload {
    url: String,
    #[serde(default)]
    size: u64,
}

const VANILLA_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

async fn install_vanilla(
    server: &Server,
    http: &reqwest::Client,
    values: &HashMap<String, String>,
    install_dir: &Path,
    progress: &Progress,
) -> Result<()> {
    progress.step(8, "Fetching the Minecraft version manifest");
    let manifest: VersionManifest = net::get_json(http, VANILLA_MANIFEST).await?;
    let requested = values
        .get("mcVersion")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v.to_lowercase() != "latest")
        .unwrap_or_else(|| manifest.latest.release.clone());

    let entry = manifest
        .versions
        .iter()
        .find(|entry| entry.id == requested)
        .ok_or_else(|| {
            AppError::bad_request(format!(
                "Minecraft version \"{requested}\" does not exist. Use a release such as {} or \"latest\".",
                manifest.latest.release
            ))
        })?;

    progress.step(20, &format!("Resolving server jar for {requested}"));
    let details: VersionDetails = net::get_json(http, &entry.url).await?;
    let server_download = details.downloads.server.ok_or_else(|| {
        AppError::bad_request(format!(
            "Minecraft {requested} does not provide a server jar"
        ))
    })?;

    let target = install_dir.join("server.jar");
    let size = download(
        http,
        &server_download.url,
        &target,
        progress,
        "Downloading server.jar",
    )
    .await?;
    if server_download.size > 0 && size != server_download.size {
        return Err(AppError::internal(format!(
            "server.jar has an unexpected size ({size} bytes, expected {})",
            server_download.size
        )));
    }
    if server.memory_mb > 0 && server.memory_mb < 1024 {
        tracing::info!("Minecraft servers usually need at least 1024 MB of RAM");
    }
    Ok(())
}

// --------------------------------------------------------------------- paper

#[derive(Deserialize)]
struct PaperBuild {
    downloads: HashMap<String, PaperDownload>,
}

#[derive(Deserialize)]
struct PaperDownload {
    url: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    checksums: HashMap<String, String>,
}

async fn install_paper(
    http: &reqwest::Client,
    values: &HashMap<String, String>,
    install_dir: &Path,
    progress: &Progress,
) -> Result<()> {
    let requested = values
        .get("mcVersion")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v.to_lowercase() != "latest");

    let candidates: Vec<String> = match requested {
        Some(version) => vec![version],
        None => {
            progress.step(8, "Fetching the Minecraft version manifest");
            let manifest: VersionManifest = net::get_json(http, VANILLA_MANIFEST).await?;
            let mut versions: Vec<String> = manifest
                .versions
                .iter()
                .map(|entry| entry.id.clone())
                .collect();
            versions.truncate(25);
            versions
        }
    };

    let mut last_error: Option<String> = None;
    for version in &candidates {
        progress.step(15, &format!("Looking up Paper builds for {version}"));
        let url =
            format!("https://fill.papermc.io/v3/projects/paper/versions/{version}/builds/latest");
        let build: PaperBuild = match net::get_json(http, &url).await {
            Ok(build) => build,
            Err(err) => {
                last_error = Some(err.message().to_string());
                continue;
            }
        };
        let Some(artifact) = build.downloads.get("server:default") else {
            last_error = Some(format!("Paper {version} has no server download"));
            continue;
        };
        let target = install_dir.join("server.jar");
        progress.step(30, &format!("Downloading Paper {version}"));
        let size = download(
            http,
            &artifact.url,
            &target,
            progress,
            "Downloading server.jar",
        )
        .await?;
        if let Some(expected) = artifact.checksums.get("sha256") {
            let actual = crate::backups::sha256_file(&target)?;
            if !actual.eq_ignore_ascii_case(expected) {
                let _ = std::fs::remove_file(&target);
                return Err(AppError::internal(
                    "Paper download failed checksum verification and was discarded",
                ));
            }
        }
        tracing::info!(
            "installed Paper {} ({}, {} bytes)",
            version,
            artifact.name,
            size
        );
        return Ok(());
    }

    Err(AppError::internal(format!(
        "Could not find a Paper build to download. {}",
        last_error.unwrap_or_else(|| "No builds were available for the requested versions".into())
    )))
}

// ------------------------------------------------------------------- bedrock

#[derive(Deserialize)]
struct BedrockLinks {
    result: BedrockResult,
}

#[derive(Deserialize)]
struct BedrockResult {
    links: Vec<BedrockLink>,
}

#[derive(Deserialize)]
struct BedrockLink {
    #[serde(default, rename = "downloadType")]
    download_type: String,
    #[serde(default, rename = "downloadUrl")]
    download_url: String,
}

async fn install_bedrock(
    http: &reqwest::Client,
    install_dir: &Path,
    progress: &Progress,
) -> Result<()> {
    progress.step(10, "Resolving the Bedrock Dedicated Server download");
    let api = "https://net-secondary.web.minecraft-services.net/api/v1.0/download/links";
    let wanted = if cfg!(windows) {
        "serverBedrockWindows"
    } else {
        "serverBedrockLinux"
    };

    let url = match net::get_json::<BedrockLinks>(http, api).await {
        Ok(links) => links
            .result
            .links
            .into_iter()
            .find(|link| link.download_type == wanted)
            .map(|link| link.download_url)
            .filter(|url| !url.is_empty()),
        Err(err) => {
            tracing::warn!("bedrock download service unavailable: {err}");
            None
        }
    }
    .or_else(|| {
        // Documented direct download fallback.
        if cfg!(windows) {
            Some("https://www.minecraft.net/bedrockdedicatedserver/bin-win".to_string())
        } else {
            Some("https://www.minecraft.net/bedrockdedicatedserver/bin-linux".to_string())
        }
    })
    .ok_or_else(|| AppError::internal("No Bedrock download is available for this platform"))?;

    let archive = install_dir.join("bedrock-server.zip");
    download(http, &url, &archive, progress, "Downloading Bedrock").await?;
    progress.step(80, "Extracting Bedrock server");
    let dir = install_dir.to_path_buf();
    let archive_clone = archive.clone();
    tokio::task::spawn_blocking(move || crate::backups::extract_zip(&archive_clone, &dir))
        .await
        .map_err(|e| AppError::internal(format!("Extraction task failed: {e}")))??;
    let _ = tokio::fs::remove_file(&archive).await;
    Ok(())
}

// --------------------------------------------------------------------- fivem

#[derive(Deserialize)]
struct FiveMRelease {
    #[serde(default)]
    recommended_download: Option<String>,
    #[serde(default)]
    optional_download: Option<String>,
}

async fn install_fivem(
    http: &reqwest::Client,
    install_dir: &Path,
    progress: &Progress,
) -> Result<()> {
    progress.step(10, "Resolving the latest FXServer artifact");
    let api = "https://changelogs-live.fivem.net/api/changelog/versions/win32/server";
    let release: FiveMRelease = net::get_json(http, api).await?;
    let url = release
        .recommended_download
        .or(release.optional_download)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| AppError::internal("FiveM did not return a downloadable artifact"))?;

    let archive = install_dir.join("fxserver.zip");
    download(http, &url, &archive, progress, "Downloading FXServer").await?;
    progress.step(80, "Extracting FXServer");
    let dir = install_dir.to_path_buf();
    let archive_clone = archive.clone();
    tokio::task::spawn_blocking(move || crate::backups::extract_zip(&archive_clone, &dir))
        .await
        .map_err(|e| AppError::internal(format!("Extraction task failed: {e}")))??;
    let _ = tokio::fs::remove_file(&archive).await;

    // FXServer archives are flat; make sure the run script exists.
    let dir = install_dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let run_cmd = dir.join("run.cmd");
        if !run_cmd.exists() {
            let script = "@echo off\r\nFXServer.exe +exec server.cfg +set onesync on\r\n";
            std::fs::write(&run_cmd, script)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(format!("Post-install step failed: {e}")))??;
    Ok(())
}
