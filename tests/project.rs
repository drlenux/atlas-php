use code_atlas::{
    config::Config,
    project::{discover, Debouncer},
};
use std::{collections::BTreeMap, fs};

fn config(root: &std::path::Path) -> Config {
    Config::from_map(&BTreeMap::from([
        ("ATLAS_PROJECT_ROOT".into(), root.display().to_string()),
        ("ATLAS_DATABASE_URL".into(), "dbname=test".into()),
        ("ATLAS_AGENT_TOKEN".into(), "agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "reviewer".into()),
        (
            "ATLAS_EXCLUDE".into(),
            "[\"generated/*\",\"*-local.php\"]".into(),
        ),
    ]))
    .unwrap()
}

#[test]
fn discovery_is_code_only_bounded_and_honors_exclusions() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["app", ".git", ".agents", "vendor", "generated"] {
        fs::create_dir(dir.path().join(name)).unwrap();
    }
    for file in [
        "app/a.php",
        ".git/x.php",
        ".agents/x.php",
        "vendor/x.php",
        "generated/x.php",
        "db-local.php",
        "notes.md",
    ] {
        fs::write(dir.path().join(file), "<?php return 1;").unwrap();
    }
    let mut cfg = config(dir.path());
    assert_eq!(
        discover(&cfg)
            .unwrap()
            .files
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["app/a.php"]
    );
    cfg.include_vendor = true;
    assert!(discover(&cfg).unwrap().files.contains_key("vendor/x.php"));
    cfg.max_file_bytes = 2;
    let scan = discover(&cfg).unwrap();
    assert!(scan.files.is_empty());
    assert!(!scan.diagnostics.is_empty());
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_escape_the_project() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("outside.php"), "<?php return 1;").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    assert!(discover(&config(root.path())).unwrap().files.is_empty());
}

#[test]
fn content_change_is_detected_even_at_same_file_length() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("a.php");
    fs::write(&file, "<?php return 1;").unwrap();
    let cfg = config(root.path());
    let before = discover(&cfg).unwrap().files;
    fs::write(&file, "<?php return 2;").unwrap();
    assert_ne!(before, discover(&cfg).unwrap().files);
}

#[test]
fn debounce_uses_last_change_and_emits_initial_snapshot_once() {
    let first = BTreeMap::from([("a.php".to_owned(), 1)]);
    let second = BTreeMap::from([("a.php".to_owned(), 2)]);
    let third = BTreeMap::from([("a.php".to_owned(), 3)]);
    let mut debounce = Debouncer::new(1000);
    assert!(debounce.observe(&first, 0));
    assert!(!debounce.observe(&first, 10));
    assert!(!debounce.observe(&second, 20));
    assert!(!debounce.observe(&third, 900));
    assert!(!debounce.observe(&third, 1899));
    assert!(debounce.observe(&third, 1900));
    assert!(!debounce.observe(&third, 3000));
}

#[test]
fn failed_publication_retries_even_without_another_source_edit() {
    let files = BTreeMap::from([("a.php".to_owned(), 1)]);
    let mut debounce = Debouncer::new(1000);
    assert!(debounce.observe(&files, 0));
    debounce.retry_after(20);
    assert!(!debounce.observe(&files, 1019));
    assert!(debounce.observe(&files, 1020));
}

#[test]
fn declaration_paths_are_discovered_while_the_rest_of_vendor_stays_excluded() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["app", "vendor/yiisoft/yii2/db", "vendor/other"] {
        fs::create_dir_all(dir.path().join(name)).unwrap();
    }
    for file in [
        "app/a.php",
        "vendor/yiisoft/yii2/db/ActiveRecord.php",
        "vendor/other/x.php",
    ] {
        fs::write(dir.path().join(file), "<?php return 1;").unwrap();
    }
    let mut cfg = config(dir.path());
    let files: Vec<_> = discover(&cfg).unwrap().files.keys().cloned().collect();
    assert_eq!(
        files,
        ["app/a.php", "vendor/yiisoft/yii2/db/ActiveRecord.php"]
    );
    cfg.declaration_paths = vec!["vendor/missing".into()];
    let files: Vec<_> = discover(&cfg).unwrap().files.keys().cloned().collect();
    assert_eq!(files, ["app/a.php"], "missing declaration path is skipped");
}

#[test]
fn debounce_reports_pending_changes_until_they_are_released() {
    let mut debounce = Debouncer::new(100);
    let first = BTreeMap::from([("a.php".to_string(), 1_u64)]);
    assert!(debounce.observe(&first, 0));
    assert!(!debounce.pending());
    let changed = BTreeMap::from([("a.php".to_string(), 2_u64)]);
    assert!(!debounce.observe(&changed, 10));
    assert!(debounce.pending());
    assert!(debounce.observe(&changed, 200));
    assert!(!debounce.pending());
}

#[test]
fn pending_paths_list_changed_new_and_removed_files_against_the_publication() {
    use code_atlas::project::pending_paths;
    let published = BTreeMap::from([
        ("a.php".to_string(), 1_u64),
        ("b.php".to_string(), 2),
        ("gone.php".to_string(), 3),
    ]);
    let current = BTreeMap::from([
        ("a.php".to_string(), 1_u64),
        ("b.php".to_string(), 20),
        ("new.php".to_string(), 4),
    ]);
    assert_eq!(
        pending_paths(&published, &current),
        ["b.php", "gone.php", "new.php"]
    );
    assert!(pending_paths(&current, &current).is_empty());
}

#[test]
fn env_files_are_never_walked() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("config")).unwrap();
    for file in [".env", ".env.local", "config/.env.prod", "config/app.php"] {
        fs::write(dir.path().join(file), "SECRET=1").unwrap();
    }
    let paths = code_atlas::project::list_source_paths(&config(dir.path())).unwrap();
    assert_eq!(paths, ["config/app.php"]);
}
