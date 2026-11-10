//! incremental-indexing: declaration surface and analysis scope (pure functions).
use code_atlas::{
    analyzer::{
        incremental::{declaration_surface, incremental_scope, ScopeInput},
        SourceFile,
    },
    domain::{Node, SourceSpan},
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn decl(id: &str, kind: &str, qualified: &str, path: &str, metadata: Value) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: qualified.rsplit("::").next().unwrap().into(),
        qualified_name: qualified.into(),
        span: SourceSpan {
            path: path.into(),
            start_line: 1,
            end_line: 2,
        },
        metadata,
        trust: 1,
        revision: 0,
    }
}
fn file(path: &str, source: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        source: source.into(),
    }
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}
fn base_nodes() -> Vec<Node> {
    vec![
        decl(
            "class:app\\Base",
            "class",
            "app\\Base",
            "base.php",
            json!({}),
        ),
        decl(
            "method:app\\Base::run",
            "method",
            "app\\Base::run",
            "base.php",
            json!({"declared_return_type":"int","parameters":[]}),
        ),
        decl(
            "class:app\\Child",
            "class",
            "app\\Child",
            "child.php",
            json!({"parent":"app\\Base"}),
        ),
        decl(
            "class:app\\Gone",
            "class",
            "app\\Gone",
            "gone.php",
            json!({}),
        ),
        decl(
            "method:app\\Gone::vanish",
            "method",
            "app\\Gone::vanish",
            "gone.php",
            json!({}),
        ),
    ]
}
fn files() -> Vec<SourceFile> {
    vec![
        file(
            "base.php",
            "<?php class Base { function run(): int { return 1; } }",
        ),
        file("child.php", "<?php class Child extends Base {}"),
        file(
            "user.php",
            "<?php $b = new Base(); $b->freshMethod(); Gone::make();",
        ),
        file("near.php", "<?php $x->freshMethodology(); $y->run2();"),
    ]
}

#[test]
fn surface_ignores_positions_fingerprints_and_closures_but_not_types() {
    let mut moved = base_nodes();
    moved[1].span.start_line = 40;
    moved[1].metadata["fingerprint"] = json!("other-body");
    moved.push(decl(
        "function:app\\Base::run:fp:1",
        "function",
        "{closure}",
        "base.php",
        json!({"closure":true}),
    ));
    assert_eq!(
        declaration_surface(&base_nodes()),
        declaration_surface(&moved)
    );
    let mut retyped = base_nodes();
    retyped[1].metadata["declared_return_type"] = json!("string");
    assert_ne!(
        declaration_surface(&base_nodes()),
        declaration_surface(&retyped)
    );
    assert!(declaration_surface(&base_nodes())["gone.php"].len() == 2);
}

fn scope(
    previous: &[Node],
    current: &[Node],
    changed: &[&str],
    removed: &[&str],
) -> Option<BTreeSet<String>> {
    let files = files();
    incremental_scope(&ScopeInput {
        changed: &set(changed),
        removed: &set(removed),
        previous: &declaration_surface(previous),
        current: &declaration_surface(current),
        files: &files,
        nodes: current,
    })
}

#[test]
fn body_only_change_scopes_to_the_changed_file() {
    assert_eq!(
        scope(&base_nodes(), &base_nodes(), &["base.php"], &[]),
        Some(set(&["base.php"]))
    );
}

#[test]
fn new_member_adds_files_naming_it_and_descendants_of_its_class() {
    let mut current = base_nodes();
    current.push(decl(
        "method:app\\Base::freshMethod",
        "method",
        "app\\Base::freshMethod",
        "base.php",
        json!({}),
    ));
    // user.php calls freshMethod; child.php inherits it without naming it;
    // near.php only has a longer identifier that contains the name.
    assert_eq!(
        scope(&base_nodes(), &current, &["base.php"], &[]),
        Some(set(&["base.php", "child.php", "user.php"]))
    );
}

#[test]
fn removed_file_adds_files_naming_its_classes_and_members() {
    let current: Vec<Node> = base_nodes()
        .into_iter()
        .filter(|n| n.span.path != "gone.php")
        .collect();
    assert_eq!(
        scope(&base_nodes(), &current, &[], &["gone.php"]),
        Some(set(&["user.php"]))
    );
}

#[test]
fn too_wide_a_scope_falls_back_to_a_full_rebuild() {
    let mut current = base_nodes();
    current.push(decl(
        "method:app\\Base::freshMethod",
        "method",
        "app\\Base::freshMethod",
        "base.php",
        json!({}),
    ));
    let many: Vec<SourceFile> = (0..80)
        .map(|i| file(&format!("f{i}.php"), "<?php $a->freshMethod();"))
        .collect();
    let result = incremental_scope(&ScopeInput {
        changed: &set(&["base.php"]),
        removed: &BTreeSet::new(),
        previous: &declaration_surface(&base_nodes()),
        current: &declaration_surface(&current),
        files: &many,
        nodes: &current,
    });
    assert_eq!(result, None);
}
