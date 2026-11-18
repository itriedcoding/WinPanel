//! Server console buffers: bounded, timestamped, stream-classified log lines.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub id: u64,
    pub ts: i64,
    /// "stdout" | "stderr" | "system"
    pub stream: String,
    /// "info" | "warn" | "error"
    pub level: String,
    pub text: String,
}

#[derive(Debug)]
pub struct LogBuffer {
    lines: Mutex<VecDeque<LogLine>>,
    capacity: usize,
    next_id: AtomicU64,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Mutex::new(VecDeque::with_capacity(capacity.min(4096))),
            capacity: capacity.max(200),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn push(&self, stream: &str, text: String) -> LogLine {
        let line = LogLine {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            ts: crate::util::now_ms(),
            stream: stream.to_string(),
            level: classify(stream, &text).to_string(),
            text: strip_ansi(&text),
        };
        {
            let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
            lines.push_back(line.clone());
            while lines.len() > self.capacity {
                lines.pop_front();
            }
        }
        line
    }

    pub fn snapshot(&self, limit: usize) -> Vec<LogLine> {
        let lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        let skip = lines.len().saturating_sub(limit);
        lines.iter().skip(skip).cloned().collect()
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<LogLine> {
        let needle = query.to_lowercase();
        let lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        let mut found: Vec<LogLine> = lines
            .iter()
            .filter(|line| line.text.to_lowercase().contains(&needle))
            .rev()
            .take(limit)
            .cloned()
            .collect();
        found.reverse();
        found
    }

    pub fn clear(&self) {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn len(&self) -> usize {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

fn classify(stream: &str, text: &str) -> &'static str {
    if stream == "stderr" {
        return "error";
    }
    let upper = text.to_uppercase();
    if upper.contains("ERROR")
        || upper.contains("FATAL")
        || upper.contains("EXCEPTION")
        || upper.contains("SEVERE")
        || upper.contains("FAILED")
        || upper.contains("CRASH")
    {
        "error"
    } else if upper.contains("WARN") {
        "warn"
    } else {
        "info"
    }
}

/// Remove ANSI escape sequences and Minecraft section-sign colour codes so the
/// web console renders clean, readable text.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if ch == '\u{a7}' {
            // Section sign + one formatting character.
            chars.next();
            continue;
        }
        if ch == '\u{0}' {
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_is_bounded() {
        let buffer = LogBuffer::new(200);
        for i in 0..500 {
            buffer.push("stdout", format!("line {i}"));
        }
        assert_eq!(buffer.len(), 200);
        let snapshot = buffer.snapshot(10);
        assert_eq!(snapshot.len(), 10);
        assert_eq!(snapshot.last().unwrap().text, "line 499");
    }

    #[test]
    fn errors_are_classified() {
        let buffer = LogBuffer::new(200);
        assert_eq!(buffer.push("stdout", "Starting".into()).level, "info");
        assert_eq!(buffer.push("stderr", "boom".into()).level, "error");
        assert_eq!(
            buffer.push("stdout", "[WARN] low memory".into()).level,
            "warn"
        );
        assert_eq!(
            buffer.push("stdout", "java.lang.Exception".into()).level,
            "error"
        );
    }

    #[test]
    fn ansi_codes_are_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{a7}ctext"), "text");
    }
}
