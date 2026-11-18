//! Runtime discovery: locate Java, Node.js and Python installations and probe
//! their versions so templates can refuse to install onto a broken host.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub kind: String,
    pub label: String,
    pub available: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub hint: String,
}

pub fn executable_candidates(kind: &str) -> Vec<String> {
    match kind {
        "java" => vec!["java".into()],
        "node" => vec!["node".into()],
        "python" => vec!["python".into(), "python3".into(), "py".into()],
        _ => Vec::new(),
    }
}

/// Resolve a startup executable.
///
/// * absolute path            -> used as-is (must exist)
/// * relative path with dirs  -> resolved against the server directory
/// * bare name                -> searched in PATH and known runtime locations
pub fn resolve_executable(exe: &str, install_dir: &Path) -> Option<PathBuf> {
    let trimmed = exe.trim();
    if trimmed.is_empty() {
        return None;
    }
    let looks_like_path =
        trimmed.contains('/') || trimmed.contains('\\') || Path::new(trimmed).is_absolute();
    if looks_like_path {
        let path = Path::new(trimmed);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            install_dir.join(path)
        };
        return if resolved.exists() {
            Some(resolved)
        } else {
            None
        };
    }
    find_in_path(trimmed).or_else(|| find_known_location(trimmed))
}

pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let suffixes: Vec<String> = if cfg!(windows) {
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
        let mut list = vec![String::new()];
        for ext in pathext.split(';') {
            if !ext.trim().is_empty() {
                list.push(ext.trim().to_lowercase());
                list.push(ext.trim().to_uppercase());
            }
        }
        if !name.to_lowercase().ends_with(".exe") {
            list
        } else {
            vec![String::new()]
        }
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path_var) {
        for suffix in &suffixes {
            let candidate = dir.join(format!("{name}{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Check the usual install locations that are not always on PATH, so a fresh
/// JDK or Node install still works without a shell restart.
fn find_known_location(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        roots.push(PathBuf::from(home).join("bin"));
    }
    if let Some(home) = std::env::var_os("NODE_HOME") {
        roots.push(PathBuf::from(home).join("bin"));
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let base = PathBuf::from(base);
                roots.push(base.join("Java"));
                roots.push(base.join("Eclipse Adoptium"));
                roots.push(base.join("Microsoft").join("jdk"));
                roots.push(base.join("nodejs"));
                roots.push(base.join("Programs").join("Python"));
                roots.push(base.join("Programs").join("Eclipse Adoptium"));
            }
        }
        if let Some(base) = std::env::var_os("ProgramData") {
            roots.push(PathBuf::from(base).join("chocolatey").join("bin"));
        }
    } else {
        roots.push(PathBuf::from("/usr/local/bin"));
        roots.push(PathBuf::from("/usr/bin"));
        roots.push(PathBuf::from("/opt/java/openjdk/bin"));
    }

    let file_names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.to_string()]
    };

    for root in roots {
        for file in &file_names {
            let direct = root.join(file);
            if direct.is_file() {
                return Some(direct);
            }
        }
        // One level of versioned subdirectories (java-17-openjdk, node-v20, ...).
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                for candidate in [dir.join("bin"), dir.clone()] {
                    for file in &file_names {
                        let path = candidate.join(file);
                        if path.is_file() {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }
    None
}

fn probe_version(exe: &Path, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())?
        .trim()
        .to_string();
    if first_line.is_empty() {
        None
    } else {
        Some(first_line.chars().take(160).collect())
    }
}

pub fn detect(kind: &str) -> RuntimeStatus {
    let (label, hint) = match kind {
        "java" => (
            "Java runtime",
            "Install Java 21 (Eclipse Temurin) and make sure it is on PATH",
        ),
        "node" => (
            "Node.js",
            "Install Node.js 20 LTS from nodejs.org and restart WinPanel",
        ),
        "python" => (
            "Python",
            "Install Python 3.11+ from python.org and enable \"Add to PATH\"",
        ),
        _ => (
            "No runtime required",
            "This template runs a native executable",
        ),
    };

    if kind == "none" || kind.is_empty() {
        return RuntimeStatus {
            kind: "none".to_string(),
            label: label.to_string(),
            available: true,
            version: None,
            path: None,
            hint: hint.to_string(),
        };
    }

    let mut found: Option<PathBuf> = None;
    for name in executable_candidates(kind) {
        if let Some(path) = find_in_path(&name).or_else(|| find_known_location(&name)) {
            found = Some(path);
            break;
        }
    }

    let (version, available) = match &found {
        Some(path) => {
            let args: Vec<&str> = match kind {
                "java" => vec!["-version"],
                "python" => vec!["--version"],
                _ => vec!["--version"],
            };
            let output = probe_version(path, &args);
            (output, true)
        }
        None => (None, false),
    };

    RuntimeStatus {
        kind: kind.to_string(),
        label: label.to_string(),
        available,
        version,
        path: found.map(|p| p.to_string_lossy().to_string()),
        hint: hint.to_string(),
    }
}

pub fn detect_all() -> Vec<RuntimeStatus> {
    ["java", "node", "python"]
        .iter()
        .map(|kind| detect(kind))
        .collect()
}

/// Compare a detected version string against a minimum major version.
pub fn version_at_least(version: &str, minimum_major: u32) -> bool {
    let digits: String = version
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    match digits.parse::<u32>() {
        Ok(major) => major >= minimum_major,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(version_at_least("openjdk version \"21.0.1\"", 17));
        assert!(version_at_least("17.0.2", 17));
        assert!(!version_at_least("1.8.0_392", 17));
        assert!(!version_at_least("", 17));
    }

    #[test]
    fn paths_resolve() {
        let dir = std::env::temp_dir();
        assert!(resolve_executable("", &dir).is_none());
        assert!(resolve_executable("../not/here.exe", &dir).is_none());
    }
}
