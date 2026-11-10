use code_atlas::config::Config;
use std::collections::BTreeMap;

fn settings() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "ATLAS_PROJECT_ROOT".into(),
            std::env::temp_dir().display().to_string(),
        ),
        ("ATLAS_DATABASE_URL".into(), "host=/tmp dbname=atlas".into()),
        ("ATLAS_AGENT_TOKEN".into(), "agent-test-token".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "review-test-token".into()),
    ])
}

#[test]
fn environment_values_override_defaults_without_rebuilding() {
    let mut values = settings();
    values.insert("ATLAS_DEBOUNCE_MS".into(), "1200".into());
    values.insert("ATLAS_WATCH_DIRS".into(), "[\"app\",\"common\"]".into());
    values.insert("ATLAS_FRAMEWORK_PLUGINS".into(), "[]".into());
    values.insert("ATLAS_INCLUDE_VENDOR".into(), "true".into());
    let config = Config::from_map(&values).unwrap();
    assert_eq!(config.debounce_ms, 1200);
    assert_eq!(config.watch_dirs, ["app", "common"]);
    assert!(config.framework_plugins.is_empty() && config.include_vendor);
    values.insert("ATLAS_DEBOUNCE_MS".into(), "300".into());
    assert_eq!(Config::from_map(&values).unwrap().debounce_ms, 300);
}

#[test]
fn rejects_invalid_types_paths_and_limits_with_parameter_name() {
    for (key, value) in [
        ("ATLAS_INCLUDE_VENDOR", "yes"),
        ("ATLAS_PHP_VERSION", "invalid"),
        ("ATLAS_PHP_VERSION", "9.0"),
        ("ATLAS_LANGUAGE_PLUGINS", "php"),
        ("ATLAS_LANGUAGE_PLUGINS", "[]"),
        ("ATLAS_LANGUAGE_PLUGINS", "[\"python\"]"),
        ("ATLAS_FRAMEWORK_PLUGINS", "[\"unknown\"]"),
        ("ATLAS_MAX_ANALYSIS_BYTES", "0"),
        ("ATLAS_MAX_GRAPH_ELEMENTS", "0"),
        ("ATLAS_WATCH_DIRS", "app,common"),
        ("ATLAS_WATCH_DIRS", "[\"../other\"]"),
        ("ATLAS_WATCH_DIRS", "[]"),
        ("ATLAS_POLL_MS", "0"),
        ("ATLAS_MAX_CONTEXT_NODES", "0"),
        ("ATLAS_CPU_LIMIT", "NaN"),
        ("ATLAS_MEMORY_LIMIT_MB", "0"),
        ("ATLAS_BIND", "invalid"),
    ] {
        let mut values = settings();
        values.insert(key.into(), value.into());
        let error = Config::from_map(&values).unwrap_err().to_string();
        assert!(error.contains(key), "{key}: {error}");
    }
}

#[test]
fn distinct_roles_need_distinct_nonempty_tokens_without_secret_echo() {
    let mut values = settings();
    values.insert("ATLAS_REVIEW_TOKEN".into(), "agent-test-token".into());
    let error = Config::from_map(&values).unwrap_err().to_string();
    assert!(error.contains("ATLAS_REVIEW_TOKEN"));
    assert!(!error.contains("agent-test-token"));
    values.remove("ATLAS_AGENT_TOKEN");
    assert!(Config::from_map(&values)
        .unwrap_err()
        .to_string()
        .contains("ATLAS_AGENT_TOKEN"));
}

#[test]
fn defaults_cover_bounded_context_and_read_only_project() {
    let config = Config::from_map(&settings()).unwrap();
    assert_eq!(config.debounce_ms, 5000);
    assert_eq!(config.max_graph_elements, 2_000_000);
    assert_eq!(config.max_context_nodes, 100);
    assert_eq!(config.max_context_depth, 3);
    assert_eq!(config.language_plugins, ["php"]);
    assert_eq!(config.framework_plugins, ["yii2"]);
    assert!(!config.include_vendor);
    assert!(config.project_root.is_absolute());
}

#[test]
fn indexing_workers_follow_cpu_or_an_explicit_bounded_override() {
    let mut values = settings();
    assert_eq!(Config::from_map(&values).unwrap().index_workers, 1);
    values.insert("ATLAS_CPU_LIMIT".into(), "3.5".into());
    assert_eq!(Config::from_map(&values).unwrap().index_workers, 4);
    values.insert("ATLAS_INDEX_WORKERS".into(), "1".into());
    assert_eq!(Config::from_map(&values).unwrap().index_workers, 1);
    values.insert("ATLAS_INDEX_WORKERS".into(), "4".into());
    assert_eq!(Config::from_map(&values).unwrap().index_workers, 4);
    values.insert("ATLAS_INDEX_WORKERS".into(), "0".into());
    values.insert("ATLAS_CPU_LIMIT".into(), "256".into());
    assert_eq!(Config::from_map(&values).unwrap().index_workers, 32);
    for invalid in ["-1", "33", "1.5", "auto"] {
        values.insert("ATLAS_INDEX_WORKERS".into(), invalid.into());
        assert!(Config::from_map(&values)
            .unwrap_err()
            .contains("ATLAS_INDEX_WORKERS"));
    }
}

#[test]
fn obsolete_runtime_settings_fail_with_migration_message() {
    for key in [
        "ATLAS_PHP_BINARY",
        "ATLAS_PHP_PLUGIN",
        "ATLAS_PHP_LINT",
        "ATLAS_YII",
        "ATLAS_MAX_PLUGIN_OUTPUT_BYTES",
        "ATLAS_PLUGIN_TIMEOUT_SECS",
    ] {
        let mut values = settings();
        values.insert(key.into(), "old-value".into());
        let error = Config::from_map(&values).unwrap_err();
        assert!(error.contains(key) && error.contains("removed"), "{error}");
    }
}

#[test]
fn declaration_paths_default_to_yii_vendor_and_reject_escaping_paths() {
    let config = Config::from_map(&settings()).unwrap();
    assert_eq!(config.declaration_paths, ["vendor/yiisoft"]);
    let mut values = settings();
    values.insert(
        "ATLAS_DECLARATION_PATHS".into(),
        "[\"vendor/yiisoft\",\"vendor/symfony/console\"]".into(),
    );
    assert_eq!(
        Config::from_map(&values).unwrap().declaration_paths,
        ["vendor/yiisoft", "vendor/symfony/console"]
    );
    values.insert("ATLAS_DECLARATION_PATHS".into(), "[]".into());
    assert!(Config::from_map(&values)
        .unwrap()
        .declaration_paths
        .is_empty());
    for bad in ["[\"../other\"]", "[\"/abs\"]", "[\"\"]", "\"vendor\""] {
        values.insert("ATLAS_DECLARATION_PATHS".into(), bad.into());
        let error = Config::from_map(&values).unwrap_err();
        assert!(error.contains("ATLAS_DECLARATION_PATHS"), "{bad}: {error}");
    }
}

#[test]
fn grep_size_limit_and_test_paths_have_defaults_and_overrides() {
    let config = Config::from_map(&settings()).unwrap();
    assert_eq!(config.grep_max_file_bytes, 16 * 1024 * 1024);
    assert_eq!(
        config.test_paths,
        ["tests/*", "*/tests/*", "*Test.php", "*Cest.php"]
    );
    let mut values = settings();
    values.insert("ATLAS_GREP_MAX_FILE_BYTES".into(), "4096".into());
    values.insert("ATLAS_TEST_PATHS".into(), "[\"spec/*\"]".into());
    let config = Config::from_map(&values).unwrap();
    assert_eq!(config.grep_max_file_bytes, 4096);
    assert_eq!(config.test_paths, ["spec/*"]);
    for (key, value) in [
        ("ATLAS_GREP_MAX_FILE_BYTES", "0"),
        ("ATLAS_TEST_PATHS", "tests/*"),
    ] {
        let mut values = settings();
        values.insert(key.into(), value.into());
        let error = Config::from_map(&values).unwrap_err().to_string();
        assert!(error.contains(key), "{key}: {error}");
    }
}

/// A fresh project root containing `.atlas/setup.yaml` with `yaml`.
fn project_with_setup(yaml: &str) -> BTreeMap<String, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "atlas-setup-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join(".atlas")).unwrap();
    std::fs::write(root.join(".atlas/setup.yaml"), yaml).unwrap();
    let mut values = settings();
    values.insert("ATLAS_PROJECT_ROOT".into(), root.display().to_string());
    values
}

#[test]
fn setup_file_supplies_settings_and_keeps_defaults() {
    let values = project_with_setup(
        r#"
version: 1
exclude: [docker/data_ignore]
index:
  watch: [app, common]
  exclude: ["*-local.php"]
  debounce_ms: 1200
  php_version: 8.1
  frameworks: []
grep:
  exclude: ["*/tests/_output/*"]
  max_file_bytes: 4096
tests:
  paths: ["spec/*"]
mcp:
  max_context_depth: 5
"#,
    );
    let config = Config::from_map(&values).unwrap();
    assert_eq!(config.watch_dirs, ["app", "common"]);
    assert_eq!(config.debounce_ms, 1200);
    assert_eq!(config.language_versions["php"], "8.1");
    assert!(config.framework_plugins.is_empty());
    assert_eq!(config.grep_max_file_bytes, 4096);
    assert_eq!(config.test_paths, ["spec/*"]);
    assert_eq!(config.max_context_depth, 5);
    assert_eq!(config.poll_ms, 1000, "omitted keys keep defaults");
    assert_eq!(config.excludes, ["docker/data_ignore", "*-local.php"]);
    assert_eq!(
        config.grep_excludes,
        ["docker/data_ignore", "*/tests/_output/*"]
    );
    assert_eq!(config.read_excludes, ["docker/data_ignore"]);
}

#[test]
fn moved_environment_keys_conflict_with_the_setup_file() {
    let mut values = project_with_setup("version: 1\n");
    values.insert("ATLAS_WATCH_DIRS".into(), "[\"app\"]".into());
    let error = Config::from_map(&values).unwrap_err();
    assert!(
        error.contains("ATLAS_WATCH_DIRS") && error.contains("index.watch"),
        "{error}"
    );
}

#[test]
fn setup_file_errors_name_the_yaml_path() {
    let typo =
        Config::from_map(&project_with_setup("version: 1\nindex:\n  wach: [app]\n")).unwrap_err();
    assert!(typo.contains("wach"), "{typo}");
    let range =
        Config::from_map(&project_with_setup("version: 1\nindex:\n  poll_ms: 0\n")).unwrap_err();
    assert!(
        range.contains("index.poll_ms") && !range.contains("ATLAS_POLL_MS"),
        "{range}"
    );
    let version = Config::from_map(&project_with_setup("version: 2\n")).unwrap_err();
    assert!(version.contains("version"), "{version}");
}

#[test]
fn without_setup_file_environment_keys_still_work() {
    let mut values = settings();
    values.insert("ATLAS_EXCLUDE".into(), "[\"generated/*\"]".into());
    let config = Config::from_map(&values).unwrap();
    assert_eq!(config.excludes, ["generated/*"]);
    assert_eq!(
        config.grep_excludes,
        ["generated/*"],
        "legacy: one exclude list for both"
    );
    assert!(config.read_excludes.is_empty());
}

#[test]
fn areas_map_prefixes_to_folders_with_longest_prefix_lookup() {
    let values = project_with_setup(
        r#"
version: 1
areas:
  partner: { paths: [partner] }
  api: { paths: [api, api/modules] }
  common: { paths: [common], shared: true }
"#,
    );
    let config = Config::from_map(&values).unwrap();
    let names: Vec<&str> = config.areas.iter().map(|a| a.prefix.as_str()).collect();
    assert_eq!(names, ["api", "common", "partner"]);
    assert!(config
        .areas
        .iter()
        .any(|a| a.prefix == "common" && a.shared));
    let area =
        |path: &str| code_atlas::domain::area_of(&config.areas, path).map(|a| a.prefix.clone());
    assert_eq!(
        area("partner/controllers/A.php").as_deref(),
        Some("partner")
    );
    assert_eq!(area("api/modules/v1/B.php").as_deref(), Some("api"));
    assert_eq!(
        area("partners/x.php"),
        None,
        "folder prefix, not string prefix"
    );
    assert_eq!(area("console/C.php"), None);
    for bad in [
        "version: 1\nareas:\n  Partner: { paths: [partner] }\n",
        "version: 1\nareas:\n  partner: { paths: [../x] }\n",
        "version: 1\nareas:\n  partner: { paths: [] }\n",
    ] {
        let error = Config::from_map(&project_with_setup(bad)).unwrap_err();
        assert!(error.contains("areas"), "{bad}: {error}");
    }
}
