//! undeclared-members: unresolved call sites with an unknown receiver are offered as
//! possible callers when few methods share the name.
use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::json;

fn node(
    id: &str,
    kind: &str,
    name: &str,
    path: &str,
    line: usize,
    metadata: serde_json::Value,
) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
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
fn contains(from: &str, to: &str) -> Edge {
    Edge {
        id: format!("contains:{from}->{to}"),
        kind: "contains".into(),
        from: from.into(),
        to: Some(to.into()),
        span: SourceSpan::default(),
        metadata: json!({}),
        trust: 1,
        revision: 0,
    }
}
fn untyped_call(id: &str, member: &str, line: usize) -> Node {
    node(
        id,
        "call_site",
        &format!("$x->{member}()"),
        "src/Caller.php",
        line,
        json!({"member_name": member, "resolution": "unresolved", "receiver_class": "unknown",
               "expression": format!("$x->{member}()")}),
    )
}

fn service() -> AtlasService {
    let mut nodes = vec![
        node(
            "method:A::rareMethod",
            "method",
            "rareMethod",
            "src/A.php",
            3,
            json!({}),
        ),
        node(
            "method:Caller::run",
            "method",
            "run",
            "src/Caller.php",
            1,
            json!({}),
        ),
        untyped_call("call_site:rare", "rareMethod", 5),
        untyped_call("call_site:save", "save", 6),
        // A typed but unresolved site is not a guess for A::rareMethod.
        node(
            "call_site:typed",
            "call_site",
            "$w->rareMethod()",
            "src/Caller.php",
            7,
            json!({"member_name": "rareMethod", "resolution": "unresolved", "receiver_class": "Other"}),
        ),
    ];
    for owner in ["P", "Q", "R", "S"] {
        nodes.push(node(
            &format!("method:{owner}::save"),
            "method",
            "save",
            "src/P.php",
            2,
            json!({}),
        ));
    }
    let edges = vec![
        contains("method:Caller::run", "call_site:rare"),
        contains("method:Caller::run", "call_site:save"),
        contains("method:Caller::run", "call_site:typed"),
    ];
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes,
            edges,
            diagnostics: vec![],
        },
        vec![
            "src/A.php".into(),
            "src/Caller.php".into(),
            "src/P.php".into(),
        ],
        vec![],
    )
    .unwrap();
    s
}

#[test]
fn rare_name_lists_untyped_sites_as_possible_callers() {
    let outline = service()
        .outline_with_usage("method:A::rareMethod", &["called_by".into()])
        .unwrap()
        .0;
    assert_eq!(
        outline.called_by,
        ["Caller::run L5: $x->rareMethod() [unresolved: name only]"]
    );
    let tree = service()
        .callers_tree(&CallersRequest {
            root_id: "method:A::rareMethod".into(),
            levels: 2,
            max_nodes: 20,
            exclude_paths: None,
            include_paths: None,
        })
        .unwrap();
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].resolutions, ["unresolved: name only"]);
}

#[test]
fn common_name_reports_only_a_count() {
    let outline = service()
        .outline_with_usage("method:P::save", &["called_by".into()])
        .unwrap()
        .0;
    assert!(outline.called_by.is_empty(), "{:?}", outline.called_by);
    assert_eq!(outline.name_only_callers, 1);
}
