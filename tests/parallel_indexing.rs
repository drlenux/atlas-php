use code_atlas::{
    config::Config, indexer::IndexCoordinator, repository::MemoryRepository, service::AtlasService,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

fn config(root: &Path, workers: usize) -> Config {
    Config::from_map(&BTreeMap::from([
        ("ATLAS_PROJECT_ROOT".into(), root.display().to_string()),
        ("ATLAS_DATABASE_URL".into(), "dbname=fixture".into()),
        ("ATLAS_AGENT_TOKEN".into(), "agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "review".into()),
        ("ATLAS_INDEX_WORKERS".into(), workers.to_string()),
    ]))
    .unwrap()
}

fn content(service: &mut AtlasService) -> Value {
    let nodes = service.search("", 10000).unwrap();
    json!(nodes
        .iter()
        .map(|node| service.context(&node.id, 3, 10000).unwrap())
        .collect::<Vec<_>>())
}

#[test]
fn parallel_index_has_the_same_php_yii_and_diagnostics_as_one_worker() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("0bad.php"), "<?php function {").unwrap();
    for n in 0..8 {
        let parent = if n == 0 {
            String::new()
        } else {
            format!("extends C{}", n - 1)
        };
        fs::write(
            root.join(format!("C{n}.php")),
            format!(
                "<?php\nnamespace Example;\nclass C{n} {parent} {{\n function value() {{\n return {n};\n }}\n}}\n"
            ),
        )
        .unwrap();
    }
    fs::write(
        root.join("dirty.php"),
        "<?php function dirty() { return 9; }",
    )
    .unwrap();
    let mut one = AtlasService::new(Box::new(MemoryRepository::new()));
    let mut four = AtlasService::new(Box::new(MemoryRepository::new()));
    let first = IndexCoordinator::new(config(root, 1))
        .index(&mut one)
        .unwrap();
    let second = IndexCoordinator::new(config(root, 4))
        .index(&mut four)
        .unwrap();
    assert_eq!(first.diagnostics, 1);
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    assert_eq!(content(&mut one), content(&mut four));
    let method = four.context("method:Example\\C7::value", 0, 10).unwrap();
    assert!(
        method.nodes[0].metadata.get("git").is_none(),
        "git provenance removed from indexed nodes"
    );
    assert_eq!(four.indexing_progress().snapshot().state, "completed");
}

#[test]
fn fatal_parallel_analysis_keeps_the_committed_graph() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..8 {
        fs::write(
            dir.path().join(format!("f{n}.php")),
            format!("<?php function f{n}() {{ return {n}; }}"),
        )
        .unwrap();
    }
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    IndexCoordinator::new(config(dir.path(), 4))
        .index(&mut service)
        .unwrap();
    let before = content(&mut service);
    let mut limited = config(dir.path(), 4);
    limited.max_graph_elements = 1;
    assert!(IndexCoordinator::new(limited).index(&mut service).is_err());
    assert_eq!(content(&mut service), before);
    let status = service.indexing_progress().snapshot();
    assert_eq!(status.state, "failed");
    assert_eq!(status.active_workers, 0);
}
