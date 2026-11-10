//! Project source text search for `atlas_grep` and `atlas_search` withGrep.
use crate::domain::Node;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const MAX_SNIPPET: usize = 200;
const MAX_QUERY_CHARS: usize = 500;
const DEFAULT_MAX_FILE_BYTES: u64 = 2_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepHit {
    pub path: String,
    pub line: usize,
    /// 1-based column of the match start in the original line (spaces preserved in column).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    pub text: String,
    /// Line looks like a comment / docblock, not live code.
    #[serde(default)]
    pub commented: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepSkip {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepError {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepScope {
    pub file_count: usize,
    pub include_vendor: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_globs: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepPage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<GrepHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, usize>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub scope: GrepScope,
    /// Files skipped only because of a scan incompleteness reason (e.g. size limit).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<GrepSkip>,
    /// Files left out by scope rules, counted per reason (e.g. `vendor_excluded`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<GrepError>,
    /// True when skips/errors prevented a full scan of the intended file set.
    #[serde(default)]
    pub incomplete: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrepOutput {
    #[default]
    Content,
    FilesWithMatches,
    Count,
}

#[derive(Clone, Debug)]
pub struct GrepRequest<'a> {
    pub query: &'a str,
    pub limit: usize,
    pub after: Option<&'a str>,
    pub kinds: &'a [String],
    pub symbol_mode: bool,
    pub regex: bool,
    pub case_sensitive: bool,
    pub word: bool,
    pub path_prefix: Option<&'a str>,
    /// Globs (see `crate::glob`); entries starting with `!` exclude.
    pub path_globs: &'a [String],
    /// Anchored path regex of test code to leave out (counted as `excluded.tests`).
    pub exclude_paths: Option<&'a str>,
    /// Anchored path regex of files to keep (`area` filter); others are not scanned.
    pub include_paths: Option<&'a str>,
    pub include_vendor: bool,
    pub output: GrepOutput,
    pub context: usize,
    pub max_file_bytes: u64,
}

impl Default for GrepRequest<'_> {
    fn default() -> Self {
        Self {
            query: "",
            limit: 50,
            after: None,
            kinds: &[],
            symbol_mode: false,
            regex: false,
            case_sensitive: true,
            word: false,
            path_prefix: None,
            path_globs: &[],
            exclude_paths: None,
            include_paths: None,
            include_vendor: false,
            output: GrepOutput::Content,
            context: 0,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }
}

/// Scan `relative_paths` under `root` for literal `query`; drop hits already covered by `nodes`.
#[cfg(test)]
pub fn grep_missing(
    root: &Path,
    relative_paths: &[String],
    query: &str,
    nodes: &[Node],
    limit: usize,
) -> Vec<GrepHit> {
    grep_page(
        root,
        relative_paths,
        &GrepRequest {
            query,
            limit,
            ..GrepRequest::default()
        },
        nodes,
    )
    .hits
}

pub fn grep_page(
    root: &Path,
    relative_paths: &[String],
    request: &GrepRequest<'_>,
    nodes: &[Node],
) -> GrepPage {
    let mut page = GrepPage {
        scope: GrepScope {
            file_count: 0,
            include_vendor: request.include_vendor,
            path_prefix: request.path_prefix.map(str::to_owned),
            path_globs: request.path_globs.to_vec(),
        },
        ..GrepPage::default()
    };
    if request.limit == 0 || request.query.is_empty() {
        return page;
    }
    if request.query.chars().count() > MAX_QUERY_CHARS {
        page.errors.push(GrepError {
            path: String::new(),
            message: format!("query longer than {MAX_QUERY_CHARS} characters"),
        });
        page.incomplete = true;
        return page;
    }
    let matcher = match compile_matcher(request) {
        Ok(m) => m,
        Err(message) => {
            page.errors.push(GrepError {
                path: String::new(),
                message,
            });
            page.incomplete = true;
            return page;
        }
    };
    let globs = match crate::glob::PathFilter::new(request.path_globs) {
        Ok(filter) => filter,
        Err(message) => {
            page.errors.push(GrepError {
                path: String::new(),
                message,
            });
            page.incomplete = true;
            return page;
        }
    };
    let tests = match request.exclude_paths.map(Regex::new).transpose() {
        Ok(tests) => tests,
        Err(error) => {
            page.errors.push(GrepError {
                path: String::new(),
                message: format!("invalid test path exclusion: {error}"),
            });
            page.incomplete = true;
            return page;
        }
    };
    let area = match request.include_paths.map(Regex::new).transpose() {
        Ok(area) => area,
        Err(error) => {
            page.errors.push(GrepError {
                path: String::new(),
                message: format!("invalid area filter: {error}"),
            });
            page.incomplete = true;
            return page;
        }
    };
    let covered = covered_lines(nodes);
    let after = parse_cursor(request.after);
    let write_only = kinds_write_only(request.kinds);
    let context = request.context.min(5);
    let mut ranked: Vec<(u8, GrepHit)> = Vec::new();
    let mut file_counts: BTreeMap<String, usize> = BTreeMap::new();

    for relative in relative_paths {
        if !path_allowed(relative, request)
            || !globs.allows(relative)
            || area.as_ref().is_some_and(|re| !re.is_match(relative))
        {
            continue;
        }
        if tests.as_ref().is_some_and(|re| re.is_match(relative)) {
            *page.excluded.entry("tests".into()).or_default() += 1;
            continue;
        }
        if !request.include_vendor && is_vendor_path(relative) {
            *page.excluded.entry("vendor_excluded".into()).or_default() += 1;
            continue;
        }
        let full = join_root(root, relative);
        let meta = match full.metadata() {
            Ok(m) => m,
            Err(error) => {
                page.errors.push(GrepError {
                    path: relative.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if !meta.is_file() {
            continue;
        }
        page.scope.file_count += 1;
        // Binary content is checked first: a large binary is not an incomplete scan.
        match starts_binary(&full) {
            Ok(true) => {
                *page.excluded.entry("binary".into()).or_default() += 1;
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                page.errors.push(GrepError {
                    path: relative.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        }
        if meta.len() > request.max_file_bytes {
            page.skipped.push(GrepSkip {
                path: relative.clone(),
                reason: format!("file_too_large:{}>{}", meta.len(), request.max_file_bytes),
            });
            continue;
        }
        let bytes = match std::fs::read(&full) {
            Ok(bytes) => bytes,
            Err(error) => {
                page.errors.push(GrepError {
                    path: relative.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let lines = text_lines(&bytes);
        for (idx, line) in lines.iter().enumerate() {
            let line_no = idx + 1;
            if covered.contains(&(relative.clone(), line_no)) {
                continue;
            }
            let Some((score, column)) = matcher.score(line) else {
                continue;
            };
            if write_only && !looks_like_write(line, request.query) {
                continue;
            }
            *file_counts.entry(relative.clone()).or_default() += 1;
            if request.output != GrepOutput::Content {
                continue;
            }
            let commented = line_is_commented(line);
            let match_len = match_length(request, column, line);
            let (before, after_ctx) = context_lines(&lines, line_no, context);
            ranked.push((
                score,
                GrepHit {
                    path: relative.clone(),
                    line: line_no,
                    column: Some(column),
                    text: snippet_around(
                        line,
                        char_to_byte(line, column.saturating_sub(1)),
                        match_len,
                    ),
                    commented,
                    before,
                    after: after_ctx,
                },
            ));
        }
    }

    page.incomplete = !page.errors.is_empty()
        || page
            .skipped
            .iter()
            .any(|s| s.reason.starts_with("file_too_large"));

    match request.output {
        GrepOutput::Content => {
            ranked.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.path.cmp(&b.1.path).then(a.1.line.cmp(&b.1.line)))
            });
            let start = after
                .as_ref()
                .and_then(|(ap, al)| {
                    ranked
                        .iter()
                        .position(|(_, h)| h.path == *ap && h.line == *al)
                        .map(|i| i + 1)
                })
                .unwrap_or(0);
            let rest = ranked.len().saturating_sub(start);
            page.truncated = rest > request.limit;
            page.hits = ranked
                .into_iter()
                .skip(start)
                .take(request.limit)
                .map(|(_, h)| h)
                .collect();
            page.next_cursor = if page.truncated {
                page.hits.last().map(hit_cursor)
            } else {
                None
            };
        }
        GrepOutput::FilesWithMatches => {
            let mut files: Vec<_> = file_counts.into_keys().collect();
            files.sort();
            let start = after
                .as_ref()
                .and_then(|(ap, _)| files.iter().position(|p| p == ap).map(|i| i + 1))
                .unwrap_or(0);
            page.truncated = files.len().saturating_sub(start) > request.limit;
            page.files = files.into_iter().skip(start).take(request.limit).collect();
            page.next_cursor = if page.truncated {
                page.files.last().map(|p| format!("{p}:0"))
            } else {
                None
            };
        }
        GrepOutput::Count => {
            let mut rows: Vec<_> = file_counts.into_iter().collect();
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            let start = after
                .as_ref()
                .and_then(|(ap, _)| rows.iter().position(|(p, _)| p == ap).map(|i| i + 1))
                .unwrap_or(0);
            page.truncated = rows.len().saturating_sub(start) > request.limit;
            page.counts = rows.into_iter().skip(start).take(request.limit).collect();
            page.next_cursor = if page.truncated {
                page.counts.keys().next_back().map(|p| format!("{p}:0"))
            } else {
                None
            };
        }
    }
    page
}

/// `Owner::member` / FQCN → short member / class name for literal file scan.
pub fn grep_query_for_mode(query: &str, symbol_mode: bool) -> String {
    if !symbol_mode {
        return query.to_owned();
    }
    let q = query.trim().trim_start_matches('\\');
    if let Some((_, member)) = q.rsplit_once("::") {
        let member = member.trim().trim_start_matches('$');
        if !member.is_empty() {
            return member.to_owned();
        }
    }
    if let Some(short) = q.rsplit('\\').next() {
        if !short.is_empty() && short != q {
            return short.to_owned();
        }
    }
    q.to_owned()
}

enum Matcher {
    Literal {
        needle: String,
        case_sensitive: bool,
        word: bool,
        symbol_mode: bool,
    },
    Regex(Regex),
}

impl Matcher {
    /// Score (lower better) and 1-based column of first match.
    fn score(&self, line: &str) -> Option<(u8, usize)> {
        match self {
            Self::Literal {
                needle,
                case_sensitive,
                word,
                symbol_mode,
            } => literal_score(line, needle, *case_sensitive, *word, *symbol_mode),
            Self::Regex(re) => re.find(line).map(|m| {
                let col = line[..m.start()].chars().count() + 1;
                (50u8, col)
            }),
        }
    }
}

fn compile_matcher(request: &GrepRequest<'_>) -> Result<Matcher, String> {
    if request.regex {
        let mut pattern = request.query.to_owned();
        if request.word {
            pattern = format!("(?:\\b(?:{pattern})\\b)");
        }
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!request.case_sensitive)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 20)
            .build()
            .map_err(|e| format!("invalid regex: {e}"))?;
        return Ok(Matcher::Regex(re));
    }
    Ok(Matcher::Literal {
        needle: if request.case_sensitive {
            request.query.to_owned()
        } else {
            request.query.to_ascii_lowercase()
        },
        case_sensitive: request.case_sensitive,
        word: request.word || request.symbol_mode,
        symbol_mode: request.symbol_mode,
    })
}

fn literal_score(
    line: &str,
    needle: &str,
    case_sensitive: bool,
    word: bool,
    symbol_mode: bool,
) -> Option<(u8, usize)> {
    if needle.is_empty() {
        return None;
    }
    let hay = if case_sensitive {
        line.to_owned()
    } else {
        line.to_ascii_lowercase()
    };
    let bytes = hay.as_bytes();
    let q = needle.as_bytes();
    let mut i = 0;
    let mut best: Option<(u8, usize)> = None;
    while i + q.len() <= bytes.len() {
        if bytes[i..].starts_with(q) {
            // Boundaries only where the needle itself starts/ends with an identifier
            // character (same rule as atlas `text` search), so `->pay(` still matches.
            let ok = !word
                || ((!is_ident_byte(q[0]) || is_boundary_before(bytes, i))
                    && (!is_ident_byte(q[q.len() - 1]) || is_boundary_after(bytes, i + q.len())));
            if ok {
                let col = line[..i].chars().count() + 1;
                let score = if symbol_mode {
                    symbol_form_score(line, i, needle)
                } else {
                    50
                };
                best = Some(match best {
                    Some((s, c)) if s <= score => (s, c),
                    _ => (score, col),
                });
                i += q.len();
                continue;
            }
        }
        i += 1;
    }
    best
}

fn is_boundary_before(bytes: &[u8], i: usize) -> bool {
    i == 0 || !is_ident_byte(bytes[i - 1])
}

fn is_boundary_after(bytes: &[u8], end: usize) -> bool {
    end >= bytes.len() || !is_ident_byte(bytes[end])
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn symbol_form_score(line: &str, at: usize, query: &str) -> u8 {
    let before = &line[..at];
    let after = &line[at + query.len()..];
    let after_trim = after.trim_start();
    if before.ends_with("->") && after_trim.starts_with('(') {
        return 0;
    }
    if before.ends_with("::") && after_trim.starts_with('(') {
        return 1;
    }
    if before.ends_with("->") {
        return 2;
    }
    if before.ends_with("::") {
        return 3;
    }
    if before.ends_with('\'') && after.starts_with('\'')
        || before.ends_with('"') && after.starts_with('"')
    {
        return 4;
    }
    10
}

fn match_length(request: &GrepRequest<'_>, column: usize, line: &str) -> usize {
    if request.regex {
        // Approximate: from column to end of first word-ish run for snippet window.
        let start = column.saturating_sub(1);
        let rest: String = line.chars().skip(start).collect();
        return rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '\\')
            .count()
            .max(1);
    }
    request.query.chars().count().max(1)
}

fn char_to_byte(line: &str, char_offset: usize) -> usize {
    line.chars().take(char_offset).map(|c| c.len_utf8()).sum()
}

fn hit_cursor(hit: &GrepHit) -> String {
    format!("{}:{}", hit.path, hit.line)
}

fn parse_cursor(raw: Option<&str>) -> Option<(String, usize)> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let (path, line) = raw.rsplit_once(':')?;
    let line: usize = line.parse().ok()?;
    if path.is_empty() {
        return None;
    }
    Some((path.to_owned(), line))
}

/// NUL in the first 8 KiB, the usual grep/ripgrep binary heuristic.
fn starts_binary(path: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut head = Vec::with_capacity(8192);
    File::open(path)?.take(8192).read_to_end(&mut head)?;
    Ok(head.contains(&0))
}

/// Lines of a file decoded lossily (legacy encodings still match ASCII queries).
fn text_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    // Same split as `BufRead::lines`: a final newline does not start another line.
    let text = text.strip_suffix('\n').unwrap_or(&text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect()
}

fn is_vendor_path(path: &str) -> bool {
    path == "vendor"
        || path.starts_with("vendor/")
        || path.starts_with("vendor\\")
        || path.contains("/vendor/")
        || path.contains("\\vendor\\")
}

fn path_allowed(path: &str, request: &GrepRequest<'_>) -> bool {
    if let Some(prefix) = request.path_prefix {
        let prefix = prefix.trim_end_matches('/');
        if path != prefix && !path.starts_with(&format!("{prefix}/")) {
            return false;
        }
    }
    true
}

fn line_is_commented(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//")
        || t.starts_with('#')
        || t.starts_with('*')
        || t.starts_with("/*")
        || t.starts_with("<!--")
}

fn kinds_write_only(kinds: &[String]) -> bool {
    !kinds.is_empty()
        && kinds.iter().all(|k| {
            matches!(
                k.as_str(),
                "write_site" | "writes" | "magic_set" | "assignment"
            )
        })
}

fn looks_like_write(line: &str, query: &str) -> bool {
    let t = line.trim_start();
    if line_is_commented(t) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if lower.contains("updateattributes")
        || lower.contains("setattributes")
        || lower.contains("setattribute(")
        || lower.contains("->load(")
        || lower.contains("updateall(")
    {
        return true;
    }
    if let Some(idx) = t.find(query) {
        let before = t[..idx].trim_end();
        let after = t[idx + query.len()..].trim_start();
        let property_write = (before.ends_with("->") || before.ends_with("?->"))
            && after.starts_with('=')
            && !after.starts_with("=>");
        if property_write {
            return true;
        }
    }
    false
}

fn join_root(root: &Path, relative: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in relative.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            continue;
        }
        out.push(part);
    }
    out
}

/// Lines atlas already returned: occurrence sites cover their span, declarations cover
/// only their start (declaration) line, so text inside a class or method stays visible.
fn covered_lines(nodes: &[Node]) -> BTreeSet<(String, usize)> {
    nodes
        .iter()
        .flat_map(|n| {
            let path = n.span.path.clone();
            let start = n.span.start_line.max(1);
            let end = if is_occurrence_kind(&n.kind) {
                n.span.end_line.max(start)
            } else {
                start
            };
            (start..=end).map(move |line| (path.clone(), line))
        })
        .collect()
}

fn is_occurrence_kind(kind: &str) -> bool {
    matches!(
        kind,
        "call_site" | "write_site" | "read_site" | "return_site" | "url_site" | "branch"
    )
}

fn context_lines(lines: &[String], line_no: usize, context: usize) -> (Vec<String>, Vec<String>) {
    if context == 0 || line_no == 0 {
        return (Vec::new(), Vec::new());
    }
    let idx = line_no - 1;
    let before = lines[idx.saturating_sub(context)..idx]
        .iter()
        .map(|l| trim_plain(l))
        .collect();
    let after_end = (idx + 1 + context).min(lines.len());
    let after = lines[idx + 1..after_end]
        .iter()
        .map(|l| trim_plain(l))
        .collect();
    (before, after)
}

fn trim_plain(line: &str) -> String {
    let t = line.trim_end();
    if t.chars().count() <= MAX_SNIPPET {
        t.to_owned()
    } else {
        t.chars().take(MAX_SNIPPET).collect::<String>() + "…"
    }
}

/// Window centered on the match so the hit itself is never cut off the left edge.
fn snippet_around(line: &str, match_byte: usize, match_len: usize) -> String {
    let t = line.trim_end();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= MAX_SNIPPET {
        return t.trim_start().to_owned();
    }
    let match_start_char = t[..match_byte.min(t.len())].chars().count();
    let match_end_char = (match_start_char + match_len.max(1)).min(chars.len());
    let half = MAX_SNIPPET / 2;
    let mut start = match_start_char.saturating_sub(half);
    let mut end = (start + MAX_SNIPPET).min(chars.len());
    if end - start < MAX_SNIPPET {
        start = end.saturating_sub(MAX_SNIPPET);
    }
    if match_end_char > end {
        end = match_end_char.min(chars.len());
        start = end.saturating_sub(MAX_SNIPPET);
    }
    let mut out: String = chars[start..end].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// Live file slice for `atlas_read`.
pub fn read_lines(
    root: &Path,
    relative: &str,
    start_line: usize,
    end_line: usize,
    max_lines: usize,
) -> Result<(Vec<String>, bool), String> {
    let start = start_line.max(1);
    let end = end_line.max(start);
    let full = join_root(root, relative);
    let file = File::open(&full).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    let mut truncated = false;
    for (idx, line) in reader.lines().enumerate() {
        let line_no = idx + 1;
        if line_no < start {
            continue;
        }
        if line_no > end {
            break;
        }
        if out.len() >= max_lines {
            truncated = true;
            break;
        }
        out.push(line.map_err(|e| e.to_string())?);
    }
    Ok((out, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Node, SourceSpan};
    use serde_json::json;
    use std::fs;

    #[test]
    fn class_span_does_not_suppress_inner_text_hits() {
        let dir = tempfile_dir();
        fs::write(
            dir.join("a.php"),
            "<?php\nclass Big {\n  function pay() {}\n  // UNIQUE_TOKEN inside class\n}\n",
        )
        .unwrap();
        let nodes = vec![Node {
            id: "class:Big".into(),
            kind: "class".into(),
            name: "Big".into(),
            qualified_name: "Big".into(),
            span: SourceSpan {
                path: "a.php".into(),
                start_line: 2,
                end_line: 5,
            },
            metadata: json!({}),
            trust: 1,
            revision: 0,
        }];
        let hits = grep_missing(&dir, &["a.php".into()], "UNIQUE_TOKEN", &nodes, 10);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].line, 4);
    }

    #[test]
    fn literal_word_checks_boundaries_only_at_identifier_edges() {
        let dir = tempfile_dir();
        fs::write(dir.join("a.php"), "<?php\n$w->pay();\n$w->payAll();\n").unwrap();
        let page = grep_page(
            &dir,
            &["a.php".into()],
            &GrepRequest {
                query: "->pay(",
                word: true,
                limit: 10,
                ..GrepRequest::default()
            },
            &[],
        );
        assert_eq!(page.hits.iter().map(|h| h.line).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn occurrence_site_line_still_deduped() {
        let dir = tempfile_dir();
        fs::write(dir.join("a.php"), "<?php\n$w->pay();\n// pay note\n").unwrap();
        let nodes = vec![Node {
            id: "call_site:1".into(),
            kind: "call_site".into(),
            name: "$w->pay()".into(),
            qualified_name: String::new(),
            span: SourceSpan {
                path: "a.php".into(),
                start_line: 2,
                end_line: 2,
            },
            metadata: json!({"expression":"$w->pay()"}),
            trust: 1,
            revision: 0,
        }];
        let hits = grep_missing(&dir, &["a.php".into()], "pay", &nodes, 10);
        assert!(hits.iter().all(|h| h.line != 2), "{hits:?}");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 3);
    }

    #[test]
    fn one_char_query_is_allowed() {
        let dir = tempfile_dir();
        fs::write(dir.join("a.php"), "<?php\n$x = 1;\n").unwrap();
        let page = grep_page(
            &dir,
            &["a.php".into()],
            &GrepRequest {
                query: "x",
                limit: 10,
                ..GrepRequest::default()
            },
            &[],
        );
        assert!(!page.hits.is_empty(), "{page:?}");
    }

    #[test]
    fn snippet_keeps_match_when_line_is_long() {
        let prefix = "a".repeat(180);
        let suffix = "b".repeat(180);
        let line = format!("{prefix}MATCH_HERE{suffix}");
        let col = prefix.len();
        let snip = snippet_around(&line, col, "MATCH_HERE".len());
        assert!(snip.contains("MATCH_HERE"), "{snip}");
    }

    #[test]
    fn vendor_exclusion_is_counted_not_listed() {
        let dir = tempfile_dir();
        fs::create_dir_all(dir.join("vendor/pkg")).unwrap();
        let vendor: Vec<String> = (0..3).map(|i| format!("vendor/pkg/x{i}.php")).collect();
        for path in &vendor {
            fs::write(dir.join(path), "<?php TOKEN_X\n").unwrap();
        }
        fs::write(dir.join("app.php"), "<?php TOKEN_X\n").unwrap();
        let mut paths = vendor.clone();
        paths.push("app.php".into());
        let page = grep_page(
            &dir,
            &paths,
            &GrepRequest {
                query: "TOKEN_X",
                limit: 10,
                include_vendor: false,
                ..GrepRequest::default()
            },
            &[],
        );
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.excluded.get("vendor_excluded"), Some(&3));
        assert!(
            page.skipped.iter().all(|s| s.reason != "vendor_excluded"),
            "exclusions must not be listed per file: {:?}",
            page.skipped
        );
        assert!(!page.incomplete);
        let with_vendor = grep_page(
            &dir,
            &paths,
            &GrepRequest {
                query: "TOKEN_X",
                limit: 10,
                include_vendor: true,
                ..GrepRequest::default()
            },
            &[],
        );
        assert_eq!(with_vendor.hits.len(), 4);
        assert!(with_vendor.excluded.is_empty());
    }

    #[test]
    fn oversized_binary_is_counted_as_binary_not_incomplete() {
        let dir = tempfile_dir();
        let mut blob = vec![0u8; 64];
        blob.extend_from_slice(b"TOKEN");
        fs::write(dir.join("socket"), &blob).unwrap();
        let page = grep_page(
            &dir,
            &["socket".into()],
            &GrepRequest {
                query: "TOKEN",
                limit: 10,
                max_file_bytes: 8,
                ..GrepRequest::default()
            },
            &[],
        );
        assert_eq!(page.excluded.get("binary"), Some(&1));
        assert!(page.skipped.is_empty(), "{:?}", page.skipped);
        assert!(!page.incomplete);
    }

    #[test]
    fn oversized_file_is_skipped_with_incomplete() {
        let dir = tempfile_dir();
        fs::write(dir.join("big.php"), "TOKEN\n").unwrap();
        let page = grep_page(
            &dir,
            &["big.php".into()],
            &GrepRequest {
                query: "TOKEN",
                limit: 10,
                max_file_bytes: 1,
                ..GrepRequest::default()
            },
            &[],
        );
        assert!(page.hits.is_empty());
        assert!(page.incomplete);
        assert!(page
            .skipped
            .iter()
            .any(|s| s.reason.starts_with("file_too_large")));
    }

    #[test]
    fn regex_and_count_modes_work() {
        let dir = tempfile_dir();
        fs::write(dir.join("a.php"), "<?php\nfoo1\nfoo2\nbar\n").unwrap();
        let page = grep_page(
            &dir,
            &["a.php".into()],
            &GrepRequest {
                query: "foo[0-9]",
                limit: 10,
                regex: true,
                output: GrepOutput::Count,
                ..GrepRequest::default()
            },
            &[],
        );
        assert_eq!(page.counts.get("a.php"), Some(&2));
    }

    #[test]
    fn symbol_mode_prefers_arrow_call_and_word_boundary() {
        let dir = tempfile_dir();
        fs::write(
            dir.join("a.php"),
            "<?php\n$affiliate->deposit_sum;\n$w->deposit();\n$x->deposit_sum = 1;\n",
        )
        .unwrap();
        let page = grep_page(
            &dir,
            &["a.php".into()],
            &GrepRequest {
                query: "deposit",
                limit: 10,
                symbol_mode: true,
                ..GrepRequest::default()
            },
            &[],
        );
        assert_eq!(page.hits.len(), 1, "{:?}", page.hits);
        assert!(page.hits[0].text.contains("->deposit()"));
    }

    #[test]
    fn write_site_kinds_skip_query_arrays() {
        let dir = tempfile_dir();
        fs::write(
            dir.join("a.php"),
            "<?php\n$q->andWhere(['is_take_bonus' => false]);\n$row->is_take_bonus = true;\n",
        )
        .unwrap();
        let kinds = vec!["write_site".into()];
        let page = grep_page(
            &dir,
            &["a.php".into()],
            &GrepRequest {
                query: "is_take_bonus",
                limit: 10,
                kinds: &kinds,
                ..GrepRequest::default()
            },
            &[],
        );
        assert_eq!(page.hits.len(), 1, "{:?}", page.hits);
        assert!(page.hits[0].text.contains("= true"));
    }

    #[test]
    fn symbol_query_uses_short_name() {
        assert_eq!(
            grep_query_for_mode("common\\models\\Wallet::deposit", true),
            "deposit"
        );
        assert_eq!(grep_query_for_mode("deposit", false), "deposit");
    }

    #[test]
    fn read_lines_returns_slice() {
        let dir = tempfile_dir();
        fs::write(dir.join("a.php"), "one\ntwo\nthree\nfour\n").unwrap();
        let (lines, truncated) = read_lines(&dir, "a.php", 2, 3, 40).unwrap();
        assert_eq!(lines, ["two", "three"]);
        assert!(!truncated);
    }

    /// Unique per test even when parallel tests read the same clock value.
    fn tempfile_dir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "atlas-grep-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
