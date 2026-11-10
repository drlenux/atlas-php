use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::*,
    plugins::php::PhpLanguagePlugin,
    repository::MemoryRepository,
    service::AtlasService,
};
use serde_json::json;
use std::path::{Path, PathBuf};

fn analyze(root: &Path, files: &[&str]) -> GraphSnapshot {
    let sources = files
        .iter()
        .map(|path| SourceFile {
            path: (*path).into(),
            source: std::fs::read_to_string(root.join(path)).unwrap(),
        })
        .collect::<Vec<_>>();
    PhpLanguagePlugin
        .analyze(&sources, &AnalysisOptions::default())
        .unwrap()
}

#[test]
fn actual_php_fixture_snapshot_is_accepted_and_context_preserves_candidates() {
    let root = std::env::var_os("ATLAS_TEST_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/php"));
    let files = [
        "app/Domain.php",
        "vendor/yii/base/Component.php",
        "app/YiiModel.php",
        "config/main.php",
    ];
    let snapshot = analyze(&root, &files);
    assert!(!snapshot.nodes.is_empty());
    assert!(!snapshot.edges.is_empty());
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let report = service
        .index(
            snapshot,
            files.iter().map(|s| s.to_string()).collect(),
            vec![],
        )
        .unwrap();
    assert!(report.nodes > 20);
    let context = service.context("method:App\\Shop::choose", 2, 300).unwrap();
    assert!(context.edges.iter().any(|e| e.kind == "calls"
        && e.to.as_deref() == Some("method:App\\Payment::pay")
        && e.metadata["candidates"]
            .as_array()
            .is_some_and(|c| c.len() == 2)));
    assert!(context
        .nodes
        .iter()
        .any(|n| n.id == "method:App\\Shop::choose"
            && n.metadata["returns"]
                .as_array()
                .is_some_and(|r| r.len() == 3)));
    // Yii-specific getter resolution is tested by the independent Yii2 plugin suite.
    // The PHP-only core contract exposes a structured unresolved property access.
    let yii = service.context("method:App\\Model::read", 1, 100).unwrap();
    assert!(yii
        .edges
        .iter()
        .any(|e| e.kind == "property_access" && e.metadata["member_name"] == "title"));
}

#[test]
fn actual_class_rename_preserves_method_trust_and_metadata_references() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("Source.php");
    std::fs::write(
        &path,
        "<?php namespace Identity; class Before { function value() { $x = 7; return $x; } }",
    )
    .unwrap();
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    service
        .index(
            analyze(root.path(), &["Source.php"]),
            vec!["Source.php".into()],
            vec![],
        )
        .unwrap();
    let id = "method:Identity\\Before::value";
    let revision = service.context(id, 0, 10).unwrap().nodes[0].revision;
    let correction = service
        .correct(
            "node",
            id,
            revision,
            json!({"metadata":{"note":"reviewed"}}),
            "verified",
        )
        .unwrap();
    service
        .review(&correction.id, correction.revision, true)
        .unwrap();
    std::fs::write(
        &path,
        "<?php\n\nnamespace Identity; class After { function value() { $x = 7; return $x; } }",
    )
    .unwrap();
    service
        .index(
            analyze(root.path(), &["Source.php"]),
            vec!["Source.php".into()],
            vec![],
        )
        .unwrap();
    let method = service.context(id, 0, 10).unwrap().nodes.remove(0);
    assert_eq!(method.qualified_name, "Identity\\After::value");
    assert_eq!(method.trust, 3);
    assert_eq!(method.span.start_line, 3);
    let variable = service
        .search("$x", 20)
        .unwrap()
        .into_iter()
        .find(|n| n.kind == "variable")
        .unwrap();
    assert_eq!(variable.metadata["scope"], id);
    assert_eq!(service.source_paths().unwrap(), vec!["Source.php"]);
}
