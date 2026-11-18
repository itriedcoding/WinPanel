//! Small shared helpers: identifiers, hashing, safe file writes and argument parsing.

use rand::RngCore;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

/// Current unix time in milliseconds.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A cryptographically random lowercase hex token.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn uuid_str() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Write a file by writing a sibling temporary file and renaming it into place.
/// This avoids truncated configuration or database-adjacent files on crash.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", random_token(4)));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Split a command line into arguments without invoking a shell.
///
/// Supports single/double quotes and backslash escapes but never expands
/// variables or shell operators, which keeps user-supplied startup arguments
/// from becoming a command injection vector.
pub fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut quote: Option<char> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else if ch == '\\' && q == '"' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    has_token = true;
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                        has_token = true;
                    }
                }
                c if c.is_whitespace() => {
                    if has_token {
                        args.push(std::mem::take(&mut current));
                        has_token = false;
                    }
                }
                c => {
                    current.push(c);
                    has_token = true;
                }
            },
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

/// Accept simple, human-readable names: no control characters, no path separators.
pub fn valid_display_name(name: &str) -> bool {
    let len = name.chars().count();
    (1..=64).contains(&len)
        && !name.chars().any(|c| c.is_control())
        && !name.contains(['/', '\\', '\0'])
}

pub fn valid_username(name: &str) -> bool {
    let len = name.chars().count();
    (3..=32).contains(&len)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

pub fn valid_env_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !key.starts_with(|c: char| c.is_ascii_digit())
}

/// Strip characters that are unsafe in Windows filenames or HTTP headers.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.chars().take(180).collect()
    }
}

/// Normalize a client-supplied relative path.
///
/// Rejects absolute paths, drive letters, UNC prefixes, `..` traversal and
/// NUL bytes so that the result can only ever describe a location *inside* a
/// server directory.
pub fn normalize_rel_path(input: &str) -> Result<String, String> {
    let normalized = input.replace('\\', "/");
    if normalized.contains('\0') {
        return Err("Path contains a NUL byte".into());
    }
    let is_absolute = normalized.starts_with('/')
        || normalized.starts_with("//")
        || (normalized.len() >= 2 && normalized.as_bytes()[1] == b':');
    if is_absolute {
        return Err("Absolute paths are not allowed".into());
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err("Path traversal is not allowed".into()),
            p if p.chars().count() > 255 => return Err("Path component is too long".into()),
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if joined.len() > 1000 {
        return Err("Path is too long".into());
    }
    Ok(joined)
}

/// Format a byte count for logs and diagnostics.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_respect_quotes() {
        assert_eq!(
            split_args("-jar server.jar --name \"My Server\" -x"),
            vec!["-jar", "server.jar", "--name", "My Server", "-x"]
        );
        assert_eq!(split_args("   "), Vec::<String>::new());
        assert_eq!(split_args("'a b' c"), vec!["a b", "c"]);
    }

    #[test]
    fn traversal_is_rejected() {
        assert!(normalize_rel_path("../secret").is_err());
        assert!(normalize_rel_path("world/../../secret").is_err());
        assert!(normalize_rel_path("C:/Windows").is_err());
        assert!(normalize_rel_path("/etc/passwd").is_err());
        assert_eq!(
            normalize_rel_path("world/./level.dat").unwrap(),
            "world/level.dat"
        );
        assert_eq!(normalize_rel_path("").unwrap(), "");
    }

    #[test]
    fn filenames_are_sanitized() {
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename("a:b?.txt"), "a_b_.txt");
    }
}
