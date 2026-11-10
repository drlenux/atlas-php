use code_atlas::{
    config::Config, indexer::IndexCoordinator, repository::MemoryRepository, service::AtlasService,
};
use std::{collections::BTreeMap, fs, process::Command};

#[test]
fn analyzer_and_yii2_work_with_no_external_programs_in_path() {
    if std::env::var_os("ATLAS_NATIVE_TEST_CHILD").is_none() {
        let empty_path = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "analyzer_and_yii2_work_with_no_external_programs_in_path",
                "--nocapture",
            ])
            .env("ATLAS_NATIVE_TEST_CHILD", "1")
            .env("PATH", empty_path.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    assert!(Command::new("php").output().is_err());
    assert!(Command::new("git").output().is_err());
    let root = tempfile::tempdir().unwrap();
    for (path, source) in [
        ("Domain.php", include_str!("fixtures/php/app/Domain.php")),
        ("Model.php", include_str!("fixtures/php/app/YiiModel.php")),
        (
            "Component.php",
            include_str!("fixtures/php/vendor/yii/base/Component.php"),
        ),
        ("config.php", include_str!("fixtures/php/config/main.php")),
    ] {
        fs::write(root.path().join(path), source).unwrap();
    }
    let config = Config::from_map(&BTreeMap::from([
        (
            "ATLAS_PROJECT_ROOT".into(),
            root.path().display().to_string(),
        ),
        (
            "ATLAS_DATABASE_URL".into(),
            "unused-memory-repository".into(),
        ),
        ("ATLAS_AGENT_TOKEN".into(), "test-agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "test-reviewer".into()),
    ]))
    .unwrap();
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let report = IndexCoordinator::new(config.clone())
        .index(&mut service)
        .unwrap();
    assert!(report.nodes > 20 && report.edges > 20);
    let context = service.context("method:App\\Model::read", 1, 100).unwrap();
    // Yii relation/component property reads become a single `calls` edge to the getter
    // (not magic_get+calls) so outline called_by stays unique.
    assert!(
        context.edges.iter().any(|e| e.kind == "calls"
            && e.to.as_deref() == Some("method:yii\\base\\Component::getTitle")),
        "yii2 should resolve $this->title to getTitle via calls"
    );
    let mut php_only = config;
    php_only.framework_plugins.clear();
    let mut plain = AtlasService::new(Box::new(MemoryRepository::new()));
    IndexCoordinator::new(php_only).index(&mut plain).unwrap();
    let plain = plain.context("method:App\\Model::read", 1, 100).unwrap();
    assert!(!plain.edges.iter().any(|e| {
        e.kind == "calls" && e.to.as_deref() == Some("method:yii\\base\\Component::getTitle")
    }));
    assert!(plain.edges.iter().any(|e| e.kind == "property_access"));
}

#[test]
fn runtime_image_has_only_atlas_and_postgres_application_processes() {
    let docker = include_str!("../Dockerfile");
    let runtime = docker.split("FROM pgvector/").nth(1).unwrap();
    for forbidden in [
        "php-cli",
        "php-mbstring",
        "composer",
        " git ",
        "nodejs",
        "python3",
        "rustc",
        "/plugin/vendor",
        "tini",
        "curl",
    ] {
        assert!(
            !runtime.contains(forbidden),
            "unexpected runtime dependency: {forbidden}"
        );
    }
    assert!(!docker.contains("FROM composer:"));
    let compose = include_str!("../docker-compose.yml");
    // Plugins are project settings now (.atlas/setup.yaml), not container environment.
    assert!(
        !compose.contains("ATLAS_LANGUAGE_PLUGINS") && !compose.contains("ATLAS_FRAMEWORK_PLUGINS")
    );
    let setup = include_str!("../examples/setup.yaml");
    assert!(setup.contains("languages: [php]") && setup.contains("frameworks: [yii2]"));
    assert!(!compose.contains("ATLAS_PHP_BINARY") && !compose.contains("ATLAS_PHP_PLUGIN"));
}

#[test]
fn binary_lists_distinct_language_and_framework_plugins_without_database() {
    let output = Command::new(env!("CARGO_BIN_EXE_code-atlas"))
        .arg("plugins")
        .env_clear()
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let catalog: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let language_ids: Vec<&str> = catalog["languages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();
    assert!(language_ids.contains(&"php"));
    assert!(language_ids.contains(&"pgsql"));
    assert_eq!(catalog["frameworks"][0]["id"], "yii2");
    assert_eq!(catalog["frameworks"][0]["requires_language"], "php");
}
