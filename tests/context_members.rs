//! lean-grep-and-context / lean-search-and-class-context: agent context omits
//! declaration members (counted for the root) but keeps a root's own sites.
use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::json;
use std::collections::BTreeSet;

const PATH: &str = "src/Bonus.php";

fn node(id: &str, kind: &str) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: id.into(),
        qualified_name: String::new(),
        span: SourceSpan {
            path: PATH.into(),
            start_line: 1,
            end_line: 1,
        },
        metadata: json!({}),
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
        span: SourceSpan {
            path: PATH.into(),
            start_line: 1,
            end_line: 1,
        },
        metadata: json!({}),
        trust: 1,
        revision: 0,
    }
}

/// Class C contains the root method M, siblings S1/S2 and property Q, and
/// exposes inherited method I; M contains its own read site.
fn bonus_class() -> AtlasService {
    let nodes = vec![
        node("class:C", "class"),
        node("method:M", "method"),
        node("method:S1", "method"),
        node("method:S2", "method"),
        node("property:Q", "property"),
        node("method:I", "method"),
        node("read_site:M:0", "read_site"),
        node("class:P", "class"),
        node("class:Sibling", "class"),
        node("trait:T", "trait"),
        node("class:TraitUser", "class"),
    ];
    let edges = vec![
        edge("contains", "class:C", "method:M"),
        edge("contains", "class:C", "method:S1"),
        edge("contains", "class:C", "method:S2"),
        edge("contains", "class:C", "property:Q"),
        edge("available_via", "class:C", "method:I"),
        edge("contains", "method:M", "read_site:M:0"),
        edge("reads", "read_site:M:0", "property:Q"),
        edge("inherits", "class:C", "class:P"),
        edge("inherits", "class:Sibling", "class:P"),
        edge("uses_trait", "class:C", "trait:T"),
        edge("uses_trait", "class:TraitUser", "trait:T"),
    ];
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes,
            edges,
            diagnostics: vec![],
        },
        vec![PATH.into()],
        vec![],
    )
    .unwrap();
    s
}

fn context(root: &str, expansion: Expansion) -> GraphContext {
    bonus_class()
        .context_view(&ContextRequest {
            root_id: root.into(),
            depth: 2,
            max_nodes: 50,
            direction: Direction::Both,
            detail: Detail::Summary,
            include_diagnostics: false,
            expansion,
        })
        .unwrap()
}

fn ids(context: &GraphContext) -> BTreeSet<String> {
    context.nodes.iter().map(|n| n.id.clone()).collect()
}

#[test]
fn omit_method_context_returns_owner_and_own_sites_without_sibling_members() {
    let lean = context("method:M", Expansion::Focused);
    let found = ids(&lean);
    for expected in ["class:C", "read_site:M:0", "property:Q"] {
        assert!(found.contains(expected), "missing {expected}: {found:?}");
    }
    for sibling in ["method:S1", "method:S2", "method:I"] {
        assert!(!found.contains(sibling), "sibling {sibling} listed");
    }
    assert!(!lean.truncated, "omitted members are not a budget cut");
}

#[test]
fn all_members_keeps_full_owner_expansion() {
    let found = ids(&context("method:M", Expansion::Full));
    for sibling in ["method:S1", "method:S2", "method:I"] {
        assert!(found.contains(sibling), "missing {sibling}");
    }
}

#[test]
fn omit_class_context_counts_members_instead_of_listing_them() {
    let lean = context("class:C", Expansion::Focused);
    let found = ids(&lean);
    for member in [
        "method:M",
        "method:S1",
        "method:S2",
        "property:Q",
        "method:I",
    ] {
        assert!(!found.contains(member), "member {member} listed");
    }
    assert_eq!(lean.omitted_members, 5);
    assert!(!lean.truncated);
}

#[test]
fn focused_class_context_returns_parent_without_its_other_children() {
    let found = ids(&context("class:C", Expansion::Focused));
    assert!(found.contains("class:P"), "{found:?}");
    assert!(!found.contains("class:Sibling"), "{found:?}");
    assert!(found.contains("trait:T"), "{found:?}");
    assert!(!found.contains("class:TraitUser"), "{found:?}");
    assert!(ids(&context("class:C", Expansion::Full)).contains("class:Sibling"));
}
