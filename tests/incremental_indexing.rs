//! incremental-indexing: watcher rebuilds only changed and dependent files (INDEX-09).
use code_atlas::{
    config::Config, domain::Node, indexer::IndexCoordinator, repository::MemoryRepository,
    service::AtlasService,
};
use std::{collections::BTreeMap, fs, path::Path};

fn config(root: &Path) -> Config {
    Config::from_map(&BTreeMap::from([
        ("ATLAS_PROJECT_ROOT".into(), root.display().to_string()),
        ("ATLAS_DATABASE_URL".into(), "dbname=test".into()),
        ("ATLAS_AGENT_TOKEN".into(), "agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "reviewer".into()),
        ("ATLAS_FRAMEWORK_PLUGINS".into(), "[]".into()),
    ]))
    .unwrap()
}
fn write(root: &Path, path: &str, source: &str) {
    let full = root.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, source).unwrap();
}
struct Project {
    dir: tempfile::TempDir,
    coordinator: IndexCoordinator,
    service: AtlasService,
}
impl Project {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (path, source) in files {
            write(dir.path(), path, source);
        }
        let mut coordinator = IndexCoordinator::new(config(dir.path()));
        let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
        coordinator.index(&mut service).unwrap();
        Self {
            dir,
            coordinator,
            service,
        }
    }
    fn change(&mut self, path: &str, source: &str) {
        write(self.dir.path(), path, source);
        self.coordinator.index_changes(&mut self.service).unwrap();
    }
    fn remove(&mut self, path: &str) {
        fs::remove_file(self.dir.path().join(path)).unwrap();
        self.coordinator.index_changes(&mut self.service).unwrap();
    }
    fn scope(&self) -> Vec<String> {
        self.coordinator
            .last_scope()
            .expect("incremental publication")
            .iter()
            .cloned()
            .collect()
    }
    fn node(&mut self, id: &str) -> Option<Node> {
        self.service
            .context(id, 0, 10)
            .ok()
            .and_then(|c| c.nodes.into_iter().find(|n| n.id == id))
    }
    fn methods_named(&mut self, name: &str) -> Vec<String> {
        let search = code_atlas::domain::NodeSearch {
            text: name.into(),
            kinds: vec!["method".into()],
            mode: code_atlas::domain::SearchMode::Symbol,
            limit: 10,
            ..Default::default()
        };
        self.service
            .search_page(&search, code_atlas::domain::Detail::Summary)
            .unwrap()
            .nodes
            .into_iter()
            .map(|n| n.qualified_name)
            .collect()
    }
    fn call(&mut self, method: &str, member: &str) -> code_atlas::domain::Edge {
        let context = self.service.context(method, 1, 100).unwrap();
        context
            .edges
            .into_iter()
            .find(|e| e.kind == "calls" && e.metadata["member_name"] == member)
            .unwrap_or_else(|| panic!("no call {member} in {method}"))
    }
}

const A: &str = "<?php\nnamespace app;\nclass A {\n  public function run() { return 1; }\n}\n";
const B: &str =
    "<?php\nnamespace app;\nclass B {\n  public function go(A $a) { $a->run(); $a->fresh(); }\n}\n";
const OTHER: &str =
    "<?php\nnamespace app;\nclass Other {\n  public function idle() { return 0; }\n}\n";

#[test]
fn body_edit_republishes_only_the_edited_file() {
    let mut p = Project::new(&[("app/A.php", A), ("app/B.php", B), ("app/Other.php", OTHER)]);
    let call_before = p.call("method:app\\B::go", "run");
    let other_before = p.node("method:app\\Other::idle").unwrap().revision;
    p.change("app/A.php", &A.replace("return 1;", "return 1 + 1;"));
    assert_eq!(p.scope(), ["app/A.php"]);
    assert_eq!(
        p.call("method:app\\B::go", "run").revision,
        call_before.revision
    );
    assert_eq!(
        p.node("method:app\\Other::idle").unwrap().revision,
        other_before
    );
    let returns = p
        .service
        .context("method:app\\A::run", 1, 50)
        .unwrap()
        .nodes
        .into_iter()
        .filter(|n| n.kind == "return_site")
        .map(|n| n.name)
        .collect::<Vec<_>>();
    assert_eq!(returns, ["1 + 1"]);
}

#[test]
fn new_method_resolves_an_existing_call_in_an_unchanged_file() {
    let mut p = Project::new(&[("app/A.php", A), ("app/B.php", B), ("app/Other.php", OTHER)]);
    assert_eq!(p.call("method:app\\B::go", "fresh").to, None);
    p.change(
        "app/A.php",
        &A.replace(
            "  public function run()",
            "  public function fresh() {}\n  public function run()",
        ),
    );
    assert_eq!(p.scope(), ["app/A.php", "app/B.php"]);
    assert_eq!(
        p.call("method:app\\B::go", "fresh").to.as_deref(),
        Some("method:app\\A::fresh")
    );
}

#[test]
fn rename_and_removal_drop_old_nodes_and_rebuild_callers() {
    let mut p = Project::new(&[("app/A.php", A), ("app/B.php", B), ("app/Other.php", OTHER)]);
    p.change("app/A.php", &A.replace("function run()", "function walk()"));
    assert_eq!(p.scope(), ["app/A.php", "app/B.php"]);
    // Identity survives a rename (trust and corrections stay); names change.
    assert_eq!(p.methods_named("run"), Vec::<String>::new());
    assert_eq!(p.methods_named("walk").len(), 1);
    assert_eq!(p.call("method:app\\B::go", "run").to, None);

    p.remove("app/A.php");
    assert_eq!(p.scope(), ["app/B.php"]);
    assert!(p.node("class:app\\A").is_none());
    assert!(p.node("method:app\\Other::idle").is_some());
}

#[test]
fn inherited_member_reaches_a_descendant_that_does_not_name_it() {
    let child = "<?php\nnamespace app;\nclass Child extends A {}\n";
    let mut p = Project::new(&[
        ("app/A.php", A),
        ("app/Child.php", child),
        ("app/Other.php", OTHER),
    ]);
    p.change(
        "app/A.php",
        &A.replace(
            "  public function run()",
            "  public function fresh() {}\n  public function run()",
        ),
    );
    assert_eq!(p.scope(), ["app/A.php", "app/Child.php"]);
    let available = p
        .service
        .context("class:app\\Child", 1, 50)
        .unwrap()
        .edges
        .into_iter()
        .filter(|e| e.kind == "available_via")
        .filter_map(|e| e.to)
        .collect::<Vec<_>>();
    assert!(
        available.contains(&"method:app\\A::fresh".to_owned()),
        "{available:?}"
    );
}

#[test]
fn manual_reindex_is_a_full_rebuild() {
    let mut p = Project::new(&[("app/A.php", A), ("app/B.php", B)]);
    p.coordinator.index(&mut p.service).unwrap();
    assert!(p.coordinator.last_scope().is_none());
}

#[test]
fn edit_of_a_caller_publishes_edges_into_unchanged_files() {
    let mut p = Project::new(&[("app/A.php", A), ("app/B.php", B), ("app/Other.php", OTHER)]);
    p.change(
        "app/B.php",
        &B.replace("$a->run();", "$a->run(); $a->run();"),
    );
    assert_eq!(p.scope(), ["app/B.php"]);
    let context = p.service.context("method:app\\B::go", 1, 100).unwrap();
    let runs = context
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.to.as_deref() == Some("method:app\\A::run"))
        .count();
    assert_eq!(runs, 2);
}
