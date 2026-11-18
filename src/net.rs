//! HTTP helpers for installers and the updater.

use crate::error::{AppError, Result};
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use std::path::Path;
use tokio::io::AsyncWriteExt;

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!(
            "WinPanel/{} (+https://github.com/itriedcoding/WinPanel)",
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub async fn get_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| AppError::internal(format!("Request to {url} failed: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::internal(format!(
            "{} returned HTTP {status}",
            shorten(url)
        )));
    }
    let text = response
        .text()
        .await
        .map_err(|e| AppError::internal(format!("Could not read response from {url}: {e}")))?;
    serde_json::from_str(&text)
        .map_err(|e| AppError::internal(format!("Unexpected response from {}: {e}", shorten(url))))
}

/// Stream a URL into `dest`, reporting progress. Returns the byte count.
pub async fn download<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    mut on_progress: F,
) -> Result<u64>
where
    F: FnMut(u64, Option<u64>),
{
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::internal(format!("Download failed: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::internal(format!(
            "Download failed: {} returned HTTP {status}",
            shorten(url)
        )));
    }
    let total = response.content_length();
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| AppError::internal(format!("Cannot write {}: {e}", dest.display())))?;
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::internal(format!("Download interrupted: {e}")))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::internal(format!("Cannot write download: {e}")))?;
        written += chunk.len() as u64;
        on_progress(written, total);
    }
    file.flush().await?;
    file.sync_all().await.ok();
    if written == 0 {
        return Err(AppError::internal("Downloaded file is empty"));
    }
    if let Some(expected) = total {
        if written != expected {
            return Err(AppError::internal(format!(
                "Download incomplete: got {written} of {expected} bytes"
            )));
        }
    }
    Ok(written)
}

fn shorten(url: &str) -> String {
    if url.len() > 120 {
        format!("{}…", &url[..120])
    } else {
        url.to_string()
    }
}
