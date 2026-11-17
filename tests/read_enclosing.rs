//! R7: path:line reads report the enclosing declaration (spec line drift).
use code_atlas::{
    domain::GraphSnapshot,
    mcp_text,
    repository::MemoryRepository,
    service::{AtlasService, ReadSourceRequest},
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlas-read-enclosing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // closeBonus moved +74 in the real project; old line 12 still inside the method.
    fs::write(
        dir.join("UserBonus.php"),
        "<?php\nclass UserBonus {\n  public function closeBonus() {\n    $a = 1;\n    $b = 2;\n    $c = 3;\n    $d = 4;\n    $e = 5;\n    return true;\n  }\n  public function other() {\n    return false;\n  }\n}\n",
    )
    .unwrap();
    dir
}

fn service() -> AtlasService {
    let snapshot: GraphSnapshot = serde_json::from_value(json!({
        "nodes": [
            {"id":"class:UserBonus","kind":"class","name":"UserBonus",
             "span":{"path":"UserBonus.php","start_line":2,"end_line":14}},
            {"id":"method:UserBonus::closeBonus","kind":"method","name":"closeBonus",
             "span":{"path":"UserBonus.php","start_line":3,"end_line":10}},
            {"id":"method:UserBonus::other","kind":"method","name":"other",
             "span":{"path":"UserBonus.php","start_line":11,"end_line":13}}
        ],
        "edges": []
    }))
    .unwrap();
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(snapshot, vec!["UserBonus.php".into()], vec![], &std::collections::BTreeMap::new())
        .unwrap();
    s
}

#[test]
fn around_line_inside_method_reports_enclosing_in_header() {
    let root = fixture_root();
    let mut s = service();
    let slice = s
        .read_source(
            &root,
            &ReadSourceRequest {
                path: Some("UserBonus.php"),
                around_line: Some(5),
                context: 2,
                max_lines: 400,
                read_excludes: &[],
                ..Default::default()
            },
        )
        .unwrap();
    let text = mcp_text::read(&s.freshness().unwrap(), &slice);
    assert!(
        text.contains(
            "# in: method closeBonus UserBonus.php:3-10 | id=method:UserBonus::closeBonus"
        ),
        "{text}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn drifted_line_still_inside_method_body_resolves_to_that_method() {
    let root = fixture_root();
    let mut s = service();
    // Spec said L4; method still covers it after growth.
    let slice = s
        .read_source(
            &root,
            &ReadSourceRequest {
                path: Some("UserBonus.php"),
                start_line: Some(4),
                end_line: Some(6),
                max_lines: 400,
                read_excludes: &[],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        slice.enclosing.as_ref().map(|e| e.id.as_str()),
        Some("method:UserBonus::closeBonus")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn entity_id_read_skips_enclosing_header() {
    let root = fixture_root();
    let mut s = service();
    let slice = s
        .read_source(
            &root,
            &ReadSourceRequest {
                entity_id: Some("method:UserBonus::closeBonus"),
                max_lines: 400,
                read_excludes: &[],
                ..Default::default()
            },
        )
        .unwrap();
    assert!(slice.enclosing.is_none());
    let text = mcp_text::read(&s.freshness().unwrap(), &slice);
    assert!(!text.contains("# in:"), "{text}");
    let _ = fs::remove_dir_all(root);
}
