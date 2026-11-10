//! text-mcp-and-grep-supplement: withGrep returns only lines atlas did not return.
use code_atlas::domain::*;
use code_atlas::project::SourceWalk;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::{AtlasService, SupplementRequest};
use serde_json::json;
use std::path::{Path, PathBuf};

const SOURCE: &str = "<?php
class Bonus {
  public function checkLifetime() {}
  public function run() {
    $this->checkLifetime();
    $this->checkLifetime();
  }
}
// checkLifetime note
$x = new BonusCodLog();
";

fn project() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "atlas-supplement-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.php"), SOURCE).unwrap();
    dir
}

fn node(id: &str, kind: &str, name: &str, line: usize, metadata: serde_json::Value) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        qualified_name: String::new(),
        span: SourceSpan {
            path: "a.php".into(),
            start_line: line,
            end_line: line,
        },
        metadata,
        trust: 1,
        revision: 0,
    }
}

fn service() -> AtlasService {
    let site = |line: usize| {
        node(
            &format!("call_site:run:{line}"),
            "call_site",
            "$this->checkLifetime()",
            line,
            json!({"expression":"$this->checkLifetime()","member_name":"checkLifetime"}),
        )
    };
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes: vec![
                node(
                    "method:Bonus::checkLifetime",
                    "method",
                    "checkLifetime",
                    3,
                    json!({}),
                ),
                site(5),
                site(6),
            ],
            edges: vec![],
            diagnostics: vec![],
        },
        vec!["a.php".into()],
        vec![],
    )
    .unwrap();
    s
}

fn walk(root: &Path) -> SourceWalk<'_> {
    SourceWalk {
        project_root: root,
        watch_dirs: &[],
        excludes: &[],
        include_vendor: false,
        declaration_paths: &[],
        language_plugins: &[],
        framework_plugins: &[],
        max_file_bytes: 2_000_000,
    }
}

fn supplement_lines(
    text: &str,
    mode: SearchMode,
    after: Option<&str>,
    grep_cursor: Option<&str>,
) -> Option<Vec<usize>> {
    let root = project();
    let search = NodeSearch {
        text: text.into(),
        limit: 1,
        mode,
        after: after.map(str::to_owned),
        ..NodeSearch::default()
    };
    service()
        .grep_supplement(
            &root,
            walk(&root),
            &SupplementRequest {
                search: &search,
                grep_cursor,
                include_vendor: false,
                max_file_bytes: 2_000_000,
            },
        )
        .unwrap()
        .map(|page| page.hits.into_iter().map(|h| h.line).collect())
}

#[test]
fn lines_of_any_matching_node_are_hidden_not_only_the_page() {
    // Page limit 1 returns only the method; both call sites are on later pages.
    let lines = supplement_lines("checkLifetime", SearchMode::Text, None, None).unwrap();
    assert_eq!(
        lines,
        [9],
        "declaration and call-site lines must not repeat"
    );
}

#[test]
fn text_mode_grep_matches_whole_identifiers() {
    let lines = supplement_lines("Bonus", SearchMode::Text, None, None).unwrap();
    assert!(lines.contains(&2), "{lines:?}");
    assert!(!lines.contains(&10), "BonusCodLog is not Bonus: {lines:?}");
}

#[test]
fn substring_mode_grep_keeps_partial_matches() {
    let lines = supplement_lines("Bonus", SearchMode::Substring, None, None).unwrap();
    assert!(lines.contains(&10), "{lines:?}");
}

#[test]
fn later_node_pages_omit_the_supplement_unless_grep_cursor_is_given() {
    assert!(supplement_lines("checkLifetime", SearchMode::Text, Some("1|x"), None).is_none());
    assert!(supplement_lines(
        "checkLifetime",
        SearchMode::Text,
        Some("1|x"),
        Some("a.php:0")
    )
    .is_some());
}

#[test]
fn excluded_test_paths_still_cover_their_lines() {
    let root = project();
    let search = NodeSearch {
        text: "checkLifetime".into(),
        limit: 1,
        // Pretend a.php is test code: its atlas nodes are hidden from the page…
        exclude_paths: Some("^a\\.php$".into()),
        ..NodeSearch::default()
    };
    let page = service()
        .grep_supplement(
            &root,
            walk(&root),
            &SupplementRequest {
                search: &search,
                grep_cursor: None,
                include_vendor: false,
                max_file_bytes: 2_000_000,
            },
        )
        .unwrap()
        .unwrap();
    // …but must not come back as grep hits.
    let lines: Vec<usize> = page.hits.iter().map(|h| h.line).collect();
    assert_eq!(lines, [9]);
}
