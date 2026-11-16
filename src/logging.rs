//! Project-owned file logging from `.atlas/setup.yaml` `logging`.
//!
//! Progress still goes to stderr (`docker logs`). This file captures selected
//! parse/traffic detail without raising memory limits or dumping secrets headers.
use crate::domain::Diagnostic;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// Codes treated as parse/analysis failures when `parse_errors` is on.
pub const PARSE_ERROR_CODES: &[&str] = &[
    "parse_error",
    "unsupported_syntax",
    "analysis_limitation",
    "path_outside_root",
    "read_error",
    "file_limit",
    "file_unavailable",
    "sql_statement_error",
];

/// Soft skips (non-CREATE SQL, etc.). `declaration_only` is opt-in via `declaration_skips`.
pub const PARSE_SKIP_CODES: &[&str] = &["sql_statement_skip"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoggingConfig {
    /// Absolute path inside the project, set only when at least one topic is on.
    pub file: Option<PathBuf>,
    pub parse_errors: bool,
    pub parse_skips: bool,
    /// Also log `declaration_only` path skips (very noisy on large vendors).
    pub declaration_skips: bool,
    pub requests: bool,
    pub responses: bool,
}

impl LoggingConfig {
    pub fn any_enabled(&self) -> bool {
        self.parse_errors || self.parse_skips || self.requests || self.responses
    }
}

#[derive(Clone, Default)]
pub struct AtlasLog {
    inner: Option<Arc<Inner>>,
}

impl std::fmt::Debug for AtlasLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            None => f.write_str("AtlasLog(off)"),
            Some(inner) => f
                .debug_struct("AtlasLog")
                .field("path", &inner.path)
                .field("parse_errors", &inner.parse_errors)
                .field("parse_skips", &inner.parse_skips)
                .field("declaration_skips", &inner.declaration_skips)
                .field("requests", &inner.requests)
                .field("responses", &inner.responses)
                .finish(),
        }
    }
}

struct Inner {
    path: PathBuf,
    file: Mutex<File>,
    parse_errors: bool,
    parse_skips: bool,
    declaration_skips: bool,
    requests: bool,
    responses: bool,
}

impl AtlasLog {
    /// Opens (or creates) the log file. No-op when every topic is off.
    pub fn open(project_root: &Path, config: &LoggingConfig) -> Result<Self, String> {
        if !config.any_enabled() {
            return Ok(Self::default());
        }
        let Some(path) = config.file.as_ref() else {
            return Err(
                "logging.file in .atlas/setup.yaml is required when a logging topic is enabled"
                    .into(),
            );
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("logging.file parent: {e}"))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("logging.file {}: {e}", path.display()))?;
        let relative = path
            .strip_prefix(project_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        let log = Self {
            inner: Some(Arc::new(Inner {
                path: path.clone(),
                file: Mutex::new(file),
                parse_errors: config.parse_errors,
                parse_skips: config.parse_skips,
                declaration_skips: config.declaration_skips,
                requests: config.requests,
                responses: config.responses,
            })),
        };
        log.line(
            "info",
            &format!(
                "logging started file={relative} parse_errors={} parse_skips={} declaration_skips={} requests={} responses={}",
                config.parse_errors,
                config.parse_skips,
                config.declaration_skips,
                config.requests,
                config.responses
            ),
        );
        Ok(log)
    }

    pub fn parse_errors_enabled(&self) -> bool {
        self.inner.as_ref().is_some_and(|i| i.parse_errors)
    }

    pub fn parse_skips_enabled(&self) -> bool {
        self.inner.as_ref().is_some_and(|i| i.parse_skips)
    }

    pub fn requests_enabled(&self) -> bool {
        self.inner.as_ref().is_some_and(|i| i.requests)
    }

    pub fn responses_enabled(&self) -> bool {
        self.inner.as_ref().is_some_and(|i| i.responses)
    }

    pub fn parse_error(&self, diagnostic: &Diagnostic) {
        if !self.parse_errors_enabled() {
            return;
        }
        if !PARSE_ERROR_CODES.contains(&diagnostic.code.as_str()) {
            return;
        }
        // Kind = diagnostic code (not a blanket "parse_error") so logs stay searchable.
        self.line(
            &diagnostic.code,
            &format!(
                "{}:{}: {}",
                diagnostic.path, diagnostic.line, diagnostic.message
            ),
        );
    }

    pub fn parse_skip(&self, path: &str, line: usize, code: &str, message: &str) {
        let allow = match code {
            "declaration_only" => self.inner.as_ref().is_some_and(|i| i.declaration_skips),
            _ => self.parse_skips_enabled() && PARSE_SKIP_CODES.contains(&code),
        };
        if !allow {
            return;
        }
        self.line(code, &format!("{path}:{line}: {message}"));
    }

    /// Write matching diagnostics (errors and/or skip codes) after a pass.
    pub fn diagnostics(&self, diagnostics: &[Diagnostic]) {
        if self.inner.is_none() {
            return;
        }
        for diagnostic in diagnostics {
            if self.parse_errors_enabled() && PARSE_ERROR_CODES.contains(&diagnostic.code.as_str())
            {
                self.parse_error(diagnostic);
            } else if diagnostic.code == "declaration_only"
                || PARSE_SKIP_CODES.contains(&diagnostic.code.as_str())
            {
                self.parse_skip(
                    &diagnostic.path,
                    diagnostic.line,
                    &diagnostic.code,
                    &diagnostic.message,
                );
            }
        }
    }

    pub fn request(&self, transport: &str, body: &str) {
        if !self.requests_enabled() {
            return;
        }
        self.line("request", &format!("{transport} {}", redact_traffic(body)));
    }

    pub fn response(&self, transport: &str, body: &str) {
        if !self.responses_enabled() {
            return;
        }
        self.line("response", &format!("{transport} {}", redact_traffic(body)));
    }

    fn line(&self, kind: &str, message: &str) {
        let Some(inner) = &self.inner else {
            return;
        };
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let text = format!("{ts} {kind} {message}\n");
        if let Ok(mut file) = inner.file.lock() {
            let _ = file.write_all(text.as_bytes());
            let _ = file.flush();
        }
    }

    #[cfg(test)]
    pub fn path(&self) -> Option<PathBuf> {
        self.inner.as_ref().map(|i| i.path.clone())
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    // Byte budget: never split a UTF-8 codepoint (Cyrillic/emoji in MCP bodies).
    let end = text.floor_char_boundary(max);
    format!("{}… truncated {} bytes", &text[..end], text.len() - end)
}

fn redact_traffic(body: &str) -> String {
    let mut text = body.to_owned();
    for key in [
        "Authorization",
        "ATLAS_AGENT_TOKEN",
        "ATLAS_REVIEW_TOKEN",
        "Bearer ",
    ] {
        if let Some(at) = text.find(key) {
            let rest = &text[at + key.len()..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == ',')
                .unwrap_or(rest.len().min(64));
            text.replace_range(at + key.len()..at + key.len() + end, "***");
        }
    }
    truncate(&text, 16_384)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_log(name: &str) -> (PathBuf, LoggingConfig) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "atlas-log-{}-{}-{}",
            name,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("atlas.log");
        (
            root,
            LoggingConfig {
                file: Some(file),
                parse_errors: true,
                parse_skips: true,
                declaration_skips: false,
                requests: true,
                responses: true,
            },
        )
    }

    #[test]
    fn writes_parse_and_traffic_lines() {
        let (root, cfg) = temp_log("all");
        let log = AtlasLog::open(&root, &cfg).unwrap();
        log.parse_error(&Diagnostic {
            path: "a.php".into(),
            line: 3,
            code: "parse_error".into(),
            message: "unexpected".into(),
        });
        log.parse_skip("b.sql", 10, "sql_statement_skip", "non-CREATE");
        log.request("mcp", r#"{"method":"tools/call"}"#);
        log.response("mcp", r#"{"result":"ok"}"#);
        let text = std::fs::read_to_string(log.path().unwrap()).unwrap();
        assert!(text.contains("parse_error a.php:3: unexpected"));
        assert!(text.contains("sql_statement_skip b.sql:10: non-CREATE"));
        assert!(text.contains("request mcp"));
        assert!(text.contains("response mcp"));
    }

    #[test]
    fn truncate_does_not_split_cyrillic_codepoints() {
        // "я" is 2 bytes; cutting at an odd byte must not panic.
        let s = "привет мир и ещё текст";
        assert!(s.is_char_boundary(0));
        assert!(!s.is_char_boundary(1));
        let out = truncate(s, 1);
        assert!(out.contains('…'), "{out}");
        assert!(out.starts_with('п') || out.starts_with('…'), "{out}");
        // Larger cut that lands mid-codepoint.
        let mid = 7; // inside a multi-byte char in this string
        assert!(!s.is_char_boundary(mid), "fixture mid={mid} text={s:?}");
        let out = truncate(s, mid);
        let prefix = out.split('…').next().unwrap();
        assert!(
            prefix.is_char_boundary(prefix.len()),
            "prefix must be valid UTF-8: {prefix:?}"
        );
    }

    #[test]
    fn redacted_traffic_never_splits_a_codepoint_at_the_16k_limit() {
        // Benchmark crash: a >16 KiB Cyrillic response was cut at byte 16384 inside "и".
        // Try every alignment of the multi-byte letters against the limit.
        for lead in 0..4 {
            for letter in ["я", "и", "ї", "字", "😀"] {
                let body = format!("{}{}", "a".repeat(lead), letter.repeat(20_000));
                let out = redact_traffic(&body);
                let kept = out.split('…').next().unwrap();
                assert!(kept.len() <= 16_384, "lead={lead} {letter}: {}", kept.len());
                assert!(out.contains("… truncated "), "lead={lead} {letter}");
            }
        }
    }

    #[test]
    fn long_cyrillic_response_is_logged_without_panicking() {
        let (root, cfg) = temp_log("cyrillic");
        let log = AtlasLog::open(&root, &cfg).unwrap();
        // Ukrainian comments, like the atlas_batch/atlas_read answer that took the service down.
        let body = "// перевіряємо ставку й виграш гравця\n".repeat(1_000);
        assert!(body.len() > 16_384);
        log.response("mcp", &body);
        let text = std::fs::read_to_string(log.path().unwrap()).unwrap();
        assert!(text.contains("response mcp"));
        assert!(text.contains("… truncated "));
    }

    #[test]
    fn disabled_topics_are_silent() {
        let (root, mut cfg) = temp_log("off");
        cfg.parse_errors = false;
        cfg.parse_skips = false;
        cfg.declaration_skips = false;
        cfg.requests = false;
        cfg.responses = false;
        cfg.file = None;
        let log = AtlasLog::open(&root, &cfg).unwrap();
        log.parse_error(&Diagnostic {
            path: "a.php".into(),
            line: 1,
            code: "parse_error".into(),
            message: "x".into(),
        });
        assert!(log.path().is_none());
    }
}
