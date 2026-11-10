//! Stderr lines for indexing diagnostics so `docker logs` shows what the
//! analyzer could not interpret (parse gaps), without dumping every unresolved call.
use crate::domain::Diagnostic;
use std::collections::BTreeMap;

/// Codes that mean the file/body was not fully understood — useful in support logs.
pub const STRUCTURAL_CODES: &[&str] = &[
    "parse_error",
    "unsupported_syntax",
    "analysis_limitation",
    "path_outside_root",
    "read_error",
    "file_limit",
    "file_unavailable",
];

const DEFAULT_MAX_LINES: usize = 200;

/// One summary line plus detail lines for structural diagnostics (capped).
pub fn format_diagnostic_log(diagnostics: &[Diagnostic], max_lines: usize) -> Vec<String> {
    if diagnostics.is_empty() {
        return Vec::new();
    }
    let mut by_code: BTreeMap<&str, usize> = BTreeMap::new();
    for d in diagnostics {
        *by_code.entry(d.code.as_str()).or_default() += 1;
    }
    let summary = by_code
        .iter()
        .map(|(code, count)| format!("{code}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut lines = vec![format!(
        "diagnostics summary ({}): {summary}",
        diagnostics.len()
    )];
    let structural: Vec<_> = diagnostics
        .iter()
        .filter(|d| STRUCTURAL_CODES.contains(&d.code.as_str()))
        .collect();
    let limit = max_lines.max(1);
    for d in structural.iter().take(limit) {
        lines.push(format!(
            "diagnostic: {} {}:{}: {}",
            d.code, d.path, d.line, d.message
        ));
    }
    if structural.len() > limit {
        lines.push(format!(
            "diagnostic: … {} more structural (of {}); full list in atlas diagnostics store",
            structural.len() - limit,
            structural.len()
        ));
    }
    lines
}

/// Write structural/interpretation diagnostics to stderr for operators and support.
pub fn log_diagnostics(diagnostics: &[Diagnostic]) {
    for line in format_diagnostic_log(diagnostics, DEFAULT_MAX_LINES) {
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_summary_and_structural_details_skips_unresolved() {
        let diagnostics = vec![
            Diagnostic {
                path: "a.php".into(),
                line: 3,
                code: "parse_error".into(),
                message: "unexpected token".into(),
            },
            Diagnostic {
                path: "b.php".into(),
                line: 10,
                code: "unresolved_call".into(),
                message: "Cannot resolve call $x->foo()".into(),
            },
            Diagnostic {
                path: "c.php".into(),
                line: 1,
                code: "unsupported_syntax".into(),
                message: "anonymous classes are not supported".into(),
            },
        ];
        let lines = format_diagnostic_log(&diagnostics, 200);
        assert_eq!(
            lines[0],
            "diagnostics summary (3): parse_error=1 unresolved_call=1 unsupported_syntax=1"
        );
        assert!(lines.iter().any(|l| l.contains("parse_error a.php:3:")));
        assert!(lines
            .iter()
            .any(|l| l.contains("unsupported_syntax c.php:1:")));
        assert!(lines.iter().all(|l| !l.contains("unresolved_call b.php")));
    }

    #[test]
    fn caps_structural_detail_lines() {
        let diagnostics: Vec<_> = (0..5)
            .map(|i| Diagnostic {
                path: format!("{i}.php"),
                line: 1,
                code: "parse_error".into(),
                message: "bad".into(),
            })
            .collect();
        let lines = format_diagnostic_log(&diagnostics, 2);
        assert_eq!(lines.len(), 1 + 2 + 1);
        assert!(lines.last().unwrap().contains("3 more structural"));
    }
}
