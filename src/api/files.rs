//! File manager endpoints. All operations are sandboxed to the server root.

use crate::auth::{load_server, AuthUser};
use crate::error::{AppError, Result};
use crate::files::{self, DirListing, FileEntry};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub hidden: Option<bool>,
}

#[derive(Deserialize)]
pub struct PathQuery {
    pub path: String,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct PathBody {
    pub path: String,
}

#[derive(Deserialize)]
pub struct ContentBody {
    pub path: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct RenameBody {
    pub from: String,
    pub to: String,
}

#[derive(Deserialize)]
pub struct CopyBody {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Deserialize)]
pub struct DeleteBody {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Deserialize)]
pub struct ChmodBody {
    pub path: String,
    pub readonly: bool,
}

#[derive(Deserialize)]
pub struct UploadQuery {
    pub name: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResult {
    pub path: String,
    pub size: u64,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<DirListing>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    let rel = query.path.unwrap_or_default();
    let hidden = query.hidden.unwrap_or(false);
    let listing = tokio::task::spawn_blocking(move || files::list_dir(&root, &rel, hidden))
        .await
        .map_err(|e| AppError::internal(format!("List task failed: {e}")))??;
    Ok(Json(listing))
}

pub async fn read_file(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<PathQuery>,
) -> Result<Json<FileContent>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    let rel = query.path;
    let rel_for_read = rel.clone();
    let content = tokio::task::spawn_blocking(move || files::read_text(&root, &rel_for_read))
        .await
        .map_err(|e| AppError::internal(format!("Read task failed: {e}")))??;
    Ok(Json(FileContent {
        path: rel,
        size: content.len() as u64,
        content,
    }))
}

pub async fn write_file(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ContentBody>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    let limit = server.storage_mb;
    let rel = body.path.clone();
    let content = body.content;
    let additional = content.len() as u64;
    tokio::task::spawn_blocking(move || {
        files::check_quota(&root, limit, additional)?;
        files::write_text(&root, &rel, &content)
    })
    .await
    .map_err(|e| AppError::internal(format!("Write task failed: {e}")))??;
    state.db.log_activity(
        "files",
        Some(&server.id),
        Some(&auth.user.id),
        "Edited a file",
        Some(&body.path),
    )?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn mkdir(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<PathBody>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    tokio::task::spawn_blocking(move || files::create_dir(&root, &body.path))
        .await
        .map_err(|e| AppError::internal(format!("Create task failed: {e}")))??;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn touch(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<PathBody>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    tokio::task::spawn_blocking(move || files::create_file(&root, &body.path))
        .await
        .map_err(|e| AppError::internal(format!("Create task failed: {e}")))??;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn rename(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<RenameBody>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    tokio::task::spawn_blocking(move || files::rename(&root, &body.from, &body.to))
        .await
        .map_err(|e| AppError::internal(format!("Move task failed: {e}")))??;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn copy(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<CopyBody>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    tokio::task::spawn_blocking(move || files::copy(&root, &body.from, &body.to, body.overwrite))
        .await
        .map_err(|e| AppError::internal(format!("Copy task failed: {e}")))??;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<DeleteBody>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    let rel = body.path.clone();
    tokio::task::spawn_blocking(move || files::delete(&root, &rel, body.recursive))
        .await
        .map_err(|e| AppError::internal(format!("Delete task failed: {e}")))??;
    state.db.log_activity(
        "files",
        Some(&server.id),
        Some(&auth.user.id),
        "Deleted a file",
        Some(&body.path),
    )?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn chmod(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ChmodBody>,
) -> Result<Json<serde_json::Value>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    tokio::task::spawn_blocking(move || files::set_readonly(&root, &body.path, body.readonly))
        .await
        .map_err(|e| AppError::internal(format!("Permission task failed: {e}")))??;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<FileEntry>>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let results = tokio::task::spawn_blocking(move || files::search(&root, &query.q, limit))
        .await
        .map_err(|e| AppError::internal(format!("Search task failed: {e}")))??;
    Ok(Json(results))
}

pub async fn upload(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<UploadQuery>,
    body: Body,
) -> Result<Json<UploadResult>> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    let max_bytes = state.settings().max_upload_mb.saturating_mul(1024 * 1024);
    let target = {
        let root = root.clone();
        let dir = query.dir.clone();
        let name = query.name.clone();
        tokio::task::spawn_blocking(move || files::upload_target(&root, &dir, &name))
            .await
            .map_err(|e| AppError::internal(format!("Upload task failed: {e}")))??
    };
    if target.exists() && !query.overwrite {
        return Err(AppError::conflict(
            "A file with that name already exists. Rename or delete it first.",
        ));
    }
    let temp = target.with_extension(format!("{}.upload", crate::util::random_token(4)));
    let mut file = tokio::fs::File::create(&temp)
        .await
        .map_err(|e| AppError::internal(format!("Cannot start the upload: {e}")))?;

    let mut written: u64 = 0;
    let mut stream = body.into_data_stream();
    let failure: Option<AppError> = loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                written += chunk.len() as u64;
                if written > max_bytes {
                    break Some(AppError::bad_request(format!(
                        "Upload exceeds the {} MB limit",
                        state.settings().max_upload_mb
                    )));
                }
                if let Err(err) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
                    break Some(AppError::internal(format!("Upload failed: {err}")));
                }
            }
            Some(Err(err)) => {
                break Some(AppError::bad_request(format!(
                    "Upload was interrupted: {err}"
                )))
            }
            None => break None,
        }
    };
    if let Some(err) = failure {
        drop(file);
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(err);
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| AppError::internal(format!("Upload failed: {e}")))?;
    file.sync_all().await.ok();
    drop(file);

    let storage_limit = server.storage_mb;
    let temp_clone = temp.clone();
    let target_clone = target.clone();
    let root_clone = root.clone();
    let quota_result = tokio::task::spawn_blocking(move || -> Result<()> {
        files::check_quota(&root_clone, storage_limit, 0)?;
        std::fs::rename(&temp_clone, &target_clone)
            .map_err(|e| AppError::internal(format!("Could not finalize the upload: {e}")))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(format!("Upload task failed: {e}")))?;
    if let Err(err) = quota_result {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(err);
    }

    let relative = files::rel_of(&target, &root);
    state.db.log_activity(
        "files",
        Some(&server.id),
        Some(&auth.user.id),
        "Uploaded a file",
        Some(&relative),
    )?;
    Ok(Json(UploadResult {
        path: relative,
        size: written,
    }))
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<PathQuery>,
) -> Result<Response> {
    let (server, _) = load_server(&state, &auth, &id, "files")?;
    let root = PathBuf::from(&server.install_dir);
    let path = crate::paths::safe_join(&root, &query.path)?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|_| AppError::not_found("File not found"))?;
    if metadata.is_dir() {
        return Err(AppError::bad_request("Cannot download a directory"));
    }
    if metadata.len() > 8 * 1024 * 1024 * 1024 {
        return Err(AppError::bad_request(
            "File is too large to download through the panel",
        ));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| AppError::internal(format!("Cannot open file: {e}")))?;

    let stream = futures_util::stream::unfold(file, |mut file| async move {
        use tokio::io::AsyncReadExt;
        let mut buffer = vec![0u8; 64 * 1024];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(read) => {
                buffer.truncate(read);
                Some((Ok::<Vec<u8>, std::io::Error>(buffer), file))
            }
            Err(err) => Some((Err(err), file)),
        }
    });

    let disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        crate::util::sanitize_filename(&name),
        percent_encode(&name)
    );
    let mut response = Response::new(Body::from_stream(stream));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&metadata.len().to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    *response.status_mut() = StatusCode::OK;
    Ok(response.into_response())
}

fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
