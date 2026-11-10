//! agent-efficiency: grep scans every text file, skips binaries, decodes lossily, globs.
use code_atlas::grep_search::GrepRequest;
use code_atlas::project::SourceWalk;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use std::path::{Path, PathBuf};

fn project() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "atlas-grep-scope-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(dir.join("deploy")).unwrap();
    std::fs::create_dir_all(dir.join("common/tests")).unwrap();
    std::fs::write(
        dir.join("deploy/run.sh"),
        "#!/bin/sh\nphp yii aggregatorua/sync\n",
    )
    .unwrap();
    std::fs::write(dir.join("crontab.yml"), "- aggregatorua/sync\n").unwrap();
    std::fs::write(dir.join("blob.dat"), b"aggregatorua\0\x01\x02").unwrap();
    std::fs::write(dir.join("legacy.php"), b"<?php // caf\xe9 aggregatorua\n").unwrap();
    std::fs::write(dir.join("app.php"), "<?php aggregatorua();\n").unwrap();
    std::fs::write(
        dir.join("common/tests/AppTest.php"),
        "<?php aggregatorua();\n",
    )
    .unwrap();
    dir
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

fn grep(root: &Path, globs: &[String]) -> code_atlas::grep_search::GrepPage {
    let watch = [".".to_owned()];
    let plugins = ["php".to_owned()];
    let walk = SourceWalk {
        watch_dirs: &watch,
        language_plugins: &plugins,
        ..walk(root)
    };
    AtlasService::new(Box::new(MemoryRepository::new()))
        .grep_source(
            root,
            walk,
            GrepRequest {
                query: "aggregatorua",
                limit: 50,
                path_globs: globs,
                ..GrepRequest::default()
            },
        )
        .unwrap()
}

fn hit_paths(page: &code_atlas::grep_search::GrepPage) -> Vec<String> {
    let mut paths: Vec<String> = page.hits.iter().map(|h| h.path.clone()).collect();
    paths.sort();
    paths
}

#[test]
fn every_text_file_is_scanned_not_only_plugin_extensions() {
    let page = grep(&project(), &[]);
    let paths = hit_paths(&page);
    for expected in ["deploy/run.sh", "crontab.yml", "app.php"] {
        assert!(
            paths.contains(&expected.to_owned()),
            "missing {expected}: {paths:?}"
        );
    }
}

#[test]
fn binary_files_are_counted_not_matched() {
    let page = grep(&project(), &[]);
    assert!(!hit_paths(&page).contains(&"blob.dat".to_owned()));
    assert_eq!(page.excluded.get("binary"), Some(&1));
    assert!(!page.incomplete);
}

#[test]
fn invalid_utf8_is_decoded_lossily() {
    let page = grep(&project(), &[]);
    assert!(hit_paths(&page).contains(&"legacy.php".to_owned()));
    assert!(page.errors.is_empty(), "{:?}", page.errors);
}

#[test]
fn path_globs_include_and_exclude() {
    let root = project();
    assert_eq!(hit_paths(&grep(&root, &["*.sh".into()])), ["deploy/run.sh"]);
    let php = hit_paths(&grep(&root, &["*.php".into(), "!*/tests/*".into()]));
    assert_eq!(php, ["app.php", "legacy.php"]);
}

#[test]
fn ide_directories_are_never_scanned() {
    let root = project();
    std::fs::create_dir_all(root.join(".idea/shelf")).unwrap();
    std::fs::write(root.join(".idea/shelf/patch.php"), "aggregatorua\n").unwrap();
    std::fs::create_dir_all(root.join(".vscode")).unwrap();
    std::fs::write(root.join(".vscode/settings.json"), "\"aggregatorua\"\n").unwrap();
    let paths = hit_paths(&grep(&root, &[]));
    assert!(
        paths
            .iter()
            .all(|p| !p.starts_with(".idea") && !p.starts_with(".vscode")),
        "{paths:?}"
    );
}

#[test]
fn test_paths_are_counted_not_matched_when_excluded() {
    let root = project();
    let watch = [".".to_owned()];
    let walk = SourceWalk {
        watch_dirs: &watch,
        ..walk(&root)
    };
    let exclude = code_atlas::glob::any_regex(&["*/tests/*".into()]);
    let page = AtlasService::new(Box::new(MemoryRepository::new()))
        .grep_source(
            &root,
            walk,
            GrepRequest {
                query: "aggregatorua",
                limit: 50,
                exclude_paths: exclude.as_deref(),
                ..GrepRequest::default()
            },
        )
        .unwrap();
    assert!(!hit_paths(&page).contains(&"common/tests/AppTest.php".to_owned()));
    assert_eq!(page.excluded.get("tests"), Some(&1));
}
