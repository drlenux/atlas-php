use code_atlas::{
    config::Config,
    domain::{GraphSnapshot, Node, SourceSpan},
    indexer::IndexCoordinator,
    repository::MemoryRepository,
    service::AtlasService,
};
use serde_json::json;
use std::{collections::BTreeMap, fs};

fn setup(root: &std::path::Path) -> Config {
    Config::from_map(&BTreeMap::from([
        ("ATLAS_PROJECT_ROOT".into(), root.display().to_string()),
        ("ATLAS_DATABASE_URL".into(), "dbname=test".into()),
        ("ATLAS_AGENT_TOKEN".into(), "agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "reviewer".into()),
    ]))
    .unwrap()
}
fn seeded() -> AtlasService {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    service
        .index(
            GraphSnapshot {
                nodes: vec![Node {
                    id: "function:f".into(),
                    kind: "function".into(),
                    name: "f".into(),
                    qualified_name: "f".into(),
                    span: SourceSpan {
                        path: "a.php".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    metadata: json!({"type":"int"}),
                    trust: 1,
                    revision: 0,
                }],
                ..Default::default()
            },
            vec!["a.php".into()],
            vec![],
        )
        .unwrap();
    service
}

#[test]
fn syntax_failure_preserves_previous_graph_and_exposes_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.php"), "<?php function {").unwrap();
    let mut service = seeded();
    let revision = service.context("function:f", 0, 1).unwrap().nodes[0].revision;
    let correction = service
        .correct(
            "node",
            "function:f",
            revision,
            json!({"metadata":{"note":"confirmed"}}),
            "verified",
        )
        .unwrap();
    service
        .review(&correction.id, correction.revision, true)
        .unwrap();
    let mut indexer = IndexCoordinator::new(setup(root.path()));
    indexer.index(&mut service).unwrap();
    let context = service.context("function:f", 1, 10).unwrap();
    assert!(!context.nodes.is_empty());
    assert_eq!(context.nodes[0].trust, 3);
    assert!(context.diagnostics.iter().any(|d| d.code == "parse_error"));
    fs::write(root.path().join("a.php"), "<?php\nfunction f() {return 2;}").unwrap();
    indexer.index(&mut service).unwrap();
    let restored = service
        .context("function:f", 0, 10)
        .unwrap()
        .nodes
        .remove(0);
    assert_eq!(restored.trust, 3);
    assert_eq!(restored.metadata["note"], "confirmed");
    assert_eq!(restored.span.start_line, 2);
}

struct BrokenLanguage;
impl code_atlas::analyzer::LanguagePlugin for BrokenLanguage {
    fn id(&self) -> &'static str {
        "php"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["php"]
    }
    fn analyze(
        &self,
        _: &[code_atlas::analyzer::SourceFile],
        _: &code_atlas::analyzer::AnalysisOptions,
    ) -> Result<GraphSnapshot, String> {
        Err("native analyzer failure".into())
    }
}
#[test]
fn failed_native_plugin_does_not_modify_index() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.php"), "<?php function f() {return 2;}").unwrap();
    let mut config = setup(root.path());
    config.framework_plugins.clear();
    let registry =
        code_atlas::analyzer::PluginRegistry::new(vec![Box::new(BrokenLanguage)], vec![]).unwrap();
    let mut indexer = IndexCoordinator::with_plugins(config, registry);
    let mut service = seeded();
    assert!(indexer
        .index(&mut service)
        .unwrap_err()
        .contains("native analyzer failure"));
    assert_eq!(
        service.context("function:f", 1, 10).unwrap().nodes[0].metadata["type"],
        "int"
    );
}
#[test]
fn restart_detects_removed_files_from_stored_source_paths() {
    let root = tempfile::tempdir().unwrap();
    let mut service = seeded();
    let report = IndexCoordinator::new(setup(root.path()))
        .index(&mut service)
        .unwrap();
    assert_eq!(report.deleted_nodes, 1);
    assert!(service.search("f", 10).unwrap().is_empty());
}
#[test]
fn input_and_graph_limits_abort_before_publication() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.php"),
        "<?php function f() { $x=7; return $x; }",
    )
    .unwrap();
    let mut config = setup(root.path());
    config.max_analysis_bytes = 5;
    let mut service = seeded();
    assert!(IndexCoordinator::new(config.clone())
        .index(&mut service)
        .unwrap_err()
        .contains("ATLAS_MAX_ANALYSIS_BYTES"));
    config.max_analysis_bytes = 1024;
    config.max_graph_elements = 1;
    assert!(IndexCoordinator::new(config).index(&mut service).is_err());
    assert_eq!(
        service.context("function:f", 0, 10).unwrap().nodes[0].metadata["type"],
        "int"
    );
}

#[test]
fn indexing_progress_tracks_failure_retry_and_commit() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.php"),
        "<?php function f() { return 2; }",
    )
    .unwrap();
    let mut config = setup(root.path());
    config.poll_ms = 1700;
    config.max_analysis_bytes = 5;
    let mut service = seeded();
    let progress = service.indexing_progress();
    assert_eq!(progress.snapshot().state, "waiting");
    let error = IndexCoordinator::new(config.clone())
        .index(&mut service)
        .unwrap_err();
    let failed = progress.snapshot();
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.stage, "reading");
    assert_eq!(failed.error.as_deref(), Some(error.as_str()));
    assert_eq!(failed.eta_seconds, None);
    assert_eq!(failed.elapsed_ms, progress.snapshot().elapsed_ms);
    config.max_analysis_bytes = 1024;
    let mut coordinator = IndexCoordinator::new(config).with_progress(progress.clone());
    let prepared = coordinator.prepare().unwrap();
    let ready = progress.snapshot();
    assert_eq!(
        ready.state, "running",
        "preparation is not a committed index"
    );
    assert_eq!(ready.run_id, failed.run_id + 1);
    assert_eq!(ready.error, None);
    assert_eq!(ready.poll_interval_ms, 1700);
    coordinator.publish(&mut service, prepared).unwrap();
    assert_eq!(progress.snapshot().state, "completed");
    assert_eq!(service.search("f", 10).unwrap().len(), 1);

    let prepared = coordinator.prepare().unwrap();
    fs::write(root.path().join("a.php"), "<?php function g() {}").unwrap();
    assert!(coordinator.publish(&mut service, prepared).is_err());
    assert_eq!(progress.snapshot().state, "failed");
    assert_eq!(progress.snapshot().stage, "validation");
    assert_eq!(service.search("f", 10).unwrap().len(), 1);
}

struct ObservedLanguage;
impl code_atlas::analyzer::LanguagePlugin for ObservedLanguage {
    fn id(&self) -> &'static str {
        "php"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["php"]
    }
    fn analyze(
        &self,
        files: &[code_atlas::analyzer::SourceFile],
        options: &code_atlas::analyzer::AnalysisOptions,
    ) -> Result<GraphSnapshot, String> {
        assert_eq!(options.progress.snapshot().state, "running");
        assert_eq!(options.progress.snapshot().stage, "analysis");
        options
            .progress
            .phase("analysis", "test", Some(files.len()), "files");
        options.progress.advance(0, Some(&files[0].path));
        assert_eq!(
            options.progress.snapshot().current_path.as_deref(),
            Some("a.php")
        );
        options.progress.advance(1, None);
        assert_eq!(options.progress.snapshot().completed, 1);
        Err("observed plugin failure".into())
    }
}

#[test]
fn plugins_share_live_progress_and_errors_reach_the_service_status() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.php"), "<?php function f() {}").unwrap();
    let mut config = setup(root.path());
    config.framework_plugins.clear();
    let registry =
        code_atlas::analyzer::PluginRegistry::new(vec![Box::new(ObservedLanguage)], vec![])
            .unwrap();
    let mut service = seeded();
    assert!(IndexCoordinator::with_plugins(config, registry)
        .index(&mut service)
        .is_err());
    let status = service.indexing_progress().snapshot();
    assert_eq!(status.state, "failed");
    assert_eq!(status.stage, "analysis");
    assert_eq!(status.completed, 1);
    assert_eq!(status.error.as_deref(), Some("observed plugin failure"));
}
