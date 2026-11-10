//! agent-efficiency: callers tree counts call levels, merges sites, drops tests.
use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::json;

fn node(id: &str, kind: &str, path: &str, line: usize, metadata: serde_json::Value) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: id.rsplit("::").next().unwrap().into(),
        qualified_name: String::new(),
        span: SourceSpan {
            path: path.into(),
            start_line: line,
            end_line: line,
        },
        metadata,
        trust: 1,
        revision: 0,
    }
}

fn edge(kind: &str, from: &str, to: &str) -> Edge {
    Edge {
        id: format!("{kind}:{from}->{to}"),
        kind: kind.into(),
        from: from.into(),
        to: Some(to.into()),
        span: SourceSpan::default(),
        metadata: json!({"resolution": "resolved"}),
        trust: 1,
        revision: 0,
    }
}

/// `owner` calls `target` from a call site at `path:line`.
fn call(
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    owner: &str,
    target: &str,
    path: &str,
    line: usize,
) {
    let site = format!("call_site:{owner}:{line}");
    let expression = format!("$x->{}()", target.rsplit("::").next().unwrap());
    nodes.push(node(
        &site,
        "call_site",
        path,
        line,
        json!({"expression": expression, "conditions": ["$ok"]}),
    ));
    edges.push(edge("contains", owner, &site));
    edges.push(edge("calls", &site, target));
}

/// A::a ← B::b (line 20) ← C::c (lines 10, 12); B::b ← T::t in a test; R::r ← R::r.
/// Class K: K::m ← X::x (external) and K::other (internal); K::idle unused.
fn service() -> AtlasService {
    let mut nodes = vec![
        node("method:A::a", "method", "src/A.php", 3, json!({})),
        node("method:B::b", "method", "src/B.php", 18, json!({})),
        node("method:C::c", "method", "src/C.php", 8, json!({})),
        node("method:T::t", "method", "tests/T.php", 5, json!({})),
        node("method:R::r", "method", "src/R.php", 2, json!({})),
        node("class:K", "class", "src/K.php", 1, json!({})),
        node("method:K::m", "method", "src/K.php", 3, json!({})),
        node("method:K::other", "method", "src/K.php", 9, json!({})),
        node("method:K::idle", "method", "src/K.php", 15, json!({})),
        node("method:X::x", "method", "src/X.php", 4, json!({})),
    ];
    let mut edges = vec![
        edge("contains", "class:K", "method:K::m"),
        edge("contains", "class:K", "method:K::other"),
        edge("contains", "class:K", "method:K::idle"),
    ];
    call(
        &mut nodes,
        &mut edges,
        "method:B::b",
        "method:A::a",
        "src/B.php",
        20,
    );
    call(
        &mut nodes,
        &mut edges,
        "method:C::c",
        "method:B::b",
        "src/C.php",
        10,
    );
    call(
        &mut nodes,
        &mut edges,
        "method:C::c",
        "method:B::b",
        "src/C.php",
        12,
    );
    call(
        &mut nodes,
        &mut edges,
        "method:T::t",
        "method:B::b",
        "tests/T.php",
        7,
    );
    call(
        &mut nodes,
        &mut edges,
        "method:R::r",
        "method:R::r",
        "src/R.php",
        4,
    );
    call(
        &mut nodes,
        &mut edges,
        "method:X::x",
        "method:K::m",
        "src/X.php",
        6,
    );
    call(
        &mut nodes,
        &mut edges,
        "method:K::other",
        "method:K::m",
        "src/K.php",
        11,
    );
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    let paths = [
        "src/A.php",
        "src/B.php",
        "src/C.php",
        "tests/T.php",
        "src/R.php",
        "src/K.php",
        "src/X.php",
    ];
    s.index(
        GraphSnapshot {
            nodes,
            edges,
            diagnostics: vec![],
        },
        paths.iter().map(|p| p.to_string()).collect(),
        vec![],
    )
    .unwrap();
    s
}

fn tree(root: &str, levels: usize, max_nodes: usize) -> CallerTree {
    service()
        .callers_tree(&CallersRequest {
            root_id: root.into(),
            levels,
            max_nodes,
            exclude_paths: code_atlas::glob::any_regex(&["tests/*".into()]),
            include_paths: None,
        })
        .unwrap()
}

#[test]
fn each_level_is_one_call_level_and_sites_of_one_caller_merge() {
    let tree = tree("method:A::a", 2, 50);
    assert_eq!(tree.root_label, "A::a");
    assert_eq!(tree.root_path, "src/A.php");
    assert_eq!(tree.children.len(), 1);
    let b = &tree.children[0];
    assert_eq!(b.label, "B::b");
    assert_eq!(b.id.as_deref(), Some("method:B::b"));
    assert_eq!(b.lines, [20]);
    assert_eq!(b.resolutions, ["resolved"]);
    assert_eq!(b.conditions, ["$ok"]);
    assert_eq!(b.expression, "$x->a()");
    assert_eq!(b.children.len(), 1);
    let c = &b.children[0];
    assert_eq!(c.label, "C::c");
    assert_eq!(c.lines, [10, 12]);
    assert!(c.children.is_empty(), "levels=2 stops here");
}

#[test]
fn test_callers_are_counted_not_listed() {
    let tree = tree("method:B::b", 1, 50);
    let labels: Vec<_> = tree.children.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["C::c"]);
    assert_eq!(tree.excluded_tests, 1);
}

#[test]
fn recursion_is_cut_with_a_note() {
    let tree = tree("method:R::r", 5, 50);
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].note.as_deref(), Some("recursion"));
    assert!(tree.children[0].children.is_empty());
}

#[test]
fn class_root_lists_members_used_from_outside() {
    let tree = tree("class:K", 2, 50);
    assert_eq!(tree.children.len(), 1, "{tree:#?}");
    let m = &tree.children[0];
    assert_eq!(m.label, "K::m");
    let callers: Vec<_> = m.children.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(callers, ["X::x"], "internal K::other is not a usage");
    assert_eq!(tree.members_without_usages, 2);
}

#[test]
fn node_budget_truncates() {
    let tree = tree("method:A::a", 3, 1);
    assert!(tree.truncated);
    assert_eq!(tree.children.len(), 1);
    assert!(tree.children[0].children.is_empty());
}

#[test]
fn callers_carry_the_area_of_their_call_site() {
    let mut s = service();
    let mut site = node(
        "call_site:Z::z:3",
        "call_site",
        "src/Z.php",
        3,
        json!({"expression": "$x->a()", "area": "partner"}),
    );
    site.name = "$x->a()".into();
    s.index(
        GraphSnapshot {
            nodes: vec![
                node("method:Z::z", "method", "src/Z.php", 1, json!({})),
                site,
            ],
            edges: vec![
                edge("contains", "method:Z::z", "call_site:Z::z:3"),
                edge("calls", "call_site:Z::z:3", "method:A::a"),
            ],
            diagnostics: vec![],
        },
        vec!["src/Z.php".into()],
        vec![],
    )
    .unwrap();
    let tree = s
        .callers_tree(&CallersRequest {
            root_id: "method:A::a".into(),
            levels: 1,
            max_nodes: 20,
            exclude_paths: None,
            include_paths: None,
        })
        .unwrap();
    let z = tree.children.iter().find(|c| c.label == "Z::z").unwrap();
    assert_eq!(z.area.as_deref(), Some("partner"));
}
