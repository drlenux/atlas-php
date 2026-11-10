//! lean-mcp-responses: search filters/pagination, projections and context budget.
use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const PATH: &str = "src/Wallet.php";

fn node(id: &str, kind: &str, name: &str, metadata: Value) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        qualified_name: id.into(),
        span: SourceSpan {
            path: PATH.into(),
            start_line: 1,
            end_line: 1,
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
        span: SourceSpan {
            path: PATH.into(),
            start_line: 1,
            end_line: 1,
        },
        metadata: json!({"fingerprint":"edge-fp","conditions":[]}),
        trust: 1,
        revision: 0,
    }
}
fn indexed(nodes: Vec<Node>, edges: Vec<Edge>, diagnostics: Vec<Diagnostic>) -> AtlasService {
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes,
            edges,
            diagnostics,
        },
        vec![PATH.into()],
        vec![],
    )
    .unwrap();
    s
}
fn search(text: &str, kinds: &[&str], after: Option<String>, limit: usize) -> NodeSearch {
    NodeSearch {
        text: text.into(),
        kinds: kinds.iter().map(|k| k.to_string()).collect(),
        after,
        limit,
        ..NodeSearch::default()
    }
}
fn ids(nodes: &[Node]) -> BTreeSet<String> {
    nodes.iter().map(|n| n.id.clone()).collect()
}

fn wallet_sites() -> AtlasService {
    let mut nodes = vec![];
    for i in 0..3 {
        nodes.push(node(
            &format!("write_site:m:{i}"),
            "write_site",
            "$wallet->dep_tax_control = 0",
            json!({"conditions":[]}),
        ));
    }
    for i in 0..5 {
        nodes.push(node(
            &format!("read_site:m:{i}"),
            "read_site",
            "$wallet->dep_tax_control",
            json!({"conditions":[]}),
        ));
    }
    indexed(nodes, vec![], vec![])
}

#[test]
fn search_kinds_filter_and_cursor_walk_every_match_once() {
    let mut s = wallet_sites();
    let first = s
        .search_page(
            &search("dep_tax_control", &["write_site"], None, 2),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(first.nodes.len(), 2);
    assert!(first.nodes.iter().all(|n| n.kind == "write_site"));
    let cursor = first.next_cursor.clone().expect("more write sites remain");
    let second = s
        .search_page(
            &search("dep_tax_control", &["write_site"], Some(cursor), 2),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(second.nodes.len(), 1);
    assert_eq!(second.next_cursor, None);
    let mut seen = ids(&first.nodes);
    seen.extend(ids(&second.nodes));
    assert_eq!(seen.len(), 3);
}

#[test]
fn search_without_kinds_returns_every_kind_and_exact_last_page_has_no_cursor() {
    let mut s = wallet_sites();
    let page = s
        .search_page(&search("dep_tax_control", &[], None, 8), Detail::Summary)
        .unwrap();
    assert_eq!(page.nodes.len(), 8);
    assert_eq!(page.next_cursor, None);
}

#[test]
fn search_rejects_malformed_kinds() {
    let mut s = wallet_sites();
    for kinds in [vec!["Write-Site"], vec!["write_site'"], vec![""]] {
        let error = s
            .search_page(&search("x", &kinds, None, 10), Detail::Summary)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput, "{kinds:?}");
    }
    let too_many: Vec<String> = (0..21).map(|i| format!("k{i}")).collect();
    let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert!(s
        .search_page(&search("x", &refs, None, 10), Detail::Summary)
        .is_err());
}

fn heavy_method() -> Node {
    node(
        "method:Wallet::pay",
        "method",
        "pay",
        json!({
            "fingerprint":"fp",
            "returns":[{"snippet":"return $this->amount;","origins":[]}],
            "imports":{"Setting":"common\\models\\Setting"},
            "assignments":[{"expression":"$x = 1"}],
            "usages":[{"access":"read"}],
            "body":"{ ... }",
            "source":"function pay() { ... }",
            "parameters":[{"name":"amount","type":"int"}],
            "declared_return_type":"int",
            "note":"agent correction"
        }),
    )
}

#[test]
fn summary_projection_drops_heavy_metadata_and_keeps_correction_fields() {
    let mut s = indexed(vec![heavy_method()], vec![], vec![]);
    let page = s
        .search_page(&search("pay", &[], None, 10), Detail::Summary)
        .unwrap();
    let method = &page.nodes[0];
    for key in [
        "fingerprint",
        "returns",
        "imports",
        "assignments",
        "usages",
        "body",
        "source",
    ] {
        assert!(method.metadata.get(key).is_none(), "{key} leaked");
    }
    assert_eq!(method.metadata["parameters"][0]["name"], "amount");
    assert_eq!(method.metadata["declared_return_type"], "int");
    assert_eq!(method.metadata["note"], "agent correction");
    assert_eq!(method.trust, 1);
    assert_eq!(method.revision, 1);
}

/// C contains M; M reads P through many read sites and calls T through one call
/// site; K1 and K2 call M through their own call sites.
fn call_graph(read_sites: usize) -> AtlasService {
    build_call_graph(read_sites, false)
}
fn call_graph_with_late_caller_ids(read_sites: usize) -> AtlasService {
    build_call_graph(read_sites, true)
}
fn build_call_graph(read_sites: usize, late_caller_ids: bool) -> AtlasService {
    let mut nodes = vec![
        node("class:C", "class", "C", json!({})),
        heavy_method(),
        node("property:P", "property", "$p", json!({})),
        node("method:T", "method", "target", json!({})),
        node("call_site:M:out", "call_site", "$t->target()", json!({})),
    ];
    let m = "method:Wallet::pay";
    let mut edges = vec![
        edge("contains", "class:C", m),
        edge("contains", m, "call_site:M:out"),
        edge("calls", "call_site:M:out", "method:T"),
    ];
    for i in 0..read_sites {
        let site = format!("read_site:M:{i:02}");
        nodes.push(node(&site, "read_site", "$this->p", json!({})));
        edges.push(edge("contains", m, &site));
        edges.push(edge("reads", &site, "property:P"));
    }
    for caller in ["K1", "K2"] {
        let method = format!("method:{caller}");
        let site = format!("call_site:{caller}:pay");
        nodes.push(node(&method, "method", caller, json!({})));
        nodes.push(node(&site, "call_site", "$w->pay()", json!({})));
        edges.push(edge("contains", &method, &site));
        let mut call = edge("calls", &site, m);
        if late_caller_ids {
            call.id = format!("zz-{}", call.id);
        }
        edges.push(call);
    }
    indexed(
        nodes,
        edges,
        vec![Diagnostic {
            path: PATH.into(),
            line: 3,
            code: "unresolved_call".into(),
            message: "cannot resolve".into(),
        }],
    )
}
fn request(direction: Direction, max_nodes: usize) -> ContextRequest {
    ContextRequest {
        root_id: "method:Wallet::pay".into(),
        depth: 1,
        max_nodes,
        direction,
        detail: Detail::Summary,
        include_diagnostics: false,
        expansion: Expansion::Full,
    }
}

#[test]
fn incoming_context_returns_callers_without_own_effects() {
    let mut s = call_graph(3);
    let context = s.context_view(&request(Direction::In, 50)).unwrap();
    let found = ids(&context.nodes);
    for expected in [
        "class:C",
        "method:K1",
        "method:K2",
        "call_site:K1:pay",
        "call_site:K2:pay",
    ] {
        assert!(found.contains(expected), "missing {expected}");
    }
    for unexpected in [
        "read_site:M:00",
        "call_site:M:out",
        "property:P",
        "method:T",
    ] {
        assert!(!found.contains(unexpected), "unexpected {unexpected}");
    }
}

#[test]
fn outgoing_context_returns_effects_without_callers() {
    let mut s = call_graph(3);
    let context = s.context_view(&request(Direction::Out, 50)).unwrap();
    let found = ids(&context.nodes);
    for expected in [
        "read_site:M:00",
        "call_site:M:out",
        "property:P",
        "method:T",
    ] {
        assert!(found.contains(expected), "missing {expected}");
    }
    for unexpected in ["class:C", "method:K1", "call_site:K1:pay"] {
        assert!(!found.contains(unexpected), "unexpected {unexpected}");
    }
}

#[test]
fn occurrence_sites_have_their_own_budget_and_callers_come_first() {
    // Six declarations (C, M, P, T, K1, K2) fit max_nodes = 6; twenty read sites
    // exceed the separate occurrence budget of the same size.
    let mut s = call_graph(20);
    let context = s.context_view(&request(Direction::Both, 6)).unwrap();
    let found = ids(&context.nodes);
    assert!(found.contains("method:K1") && found.contains("method:K2"));
    let occurrences = context
        .nodes
        .iter()
        .filter(|n| n.kind.ends_with("_site") || n.kind == "branch")
        .count();
    assert_eq!(occurrences, 6);
    assert!(context.truncated);
}

#[test]
fn incoming_edges_are_admitted_before_outgoing_regardless_of_edge_ids() {
    // Rename caller edges so they sort after the method's own edges by id.
    let mut s = call_graph_with_late_caller_ids(20);
    let context = s.context_view(&request(Direction::Both, 6)).unwrap();
    let found = ids(&context.nodes);
    for caller in [
        "call_site:K1:pay",
        "call_site:K2:pay",
        "method:K1",
        "method:K2",
    ] {
        assert!(found.contains(caller), "missing {caller}");
    }
}

#[test]
fn node_budget_still_truncates_declarations() {
    let mut s = call_graph(20);
    let context = s.context_view(&request(Direction::Both, 3)).unwrap();
    let declarations = context
        .nodes
        .iter()
        .filter(|n| !n.kind.ends_with("_site") && n.kind != "branch")
        .count();
    assert!(declarations <= 3);
    assert!(context.truncated);
}

#[test]
fn diagnostics_are_opt_in_and_detail_full_restores_heavy_metadata() {
    let mut s = call_graph(1);
    let lean = s.context_view(&request(Direction::Both, 50)).unwrap();
    assert!(lean.diagnostics.is_empty());
    let root = lean.nodes.iter().find(|n| n.id == lean.root_id).unwrap();
    assert!(root.metadata.get("returns").is_none());
    assert!(lean
        .edges
        .iter()
        .all(|e| e.metadata.get("fingerprint").is_none()));

    let mut full_request = request(Direction::Both, 50);
    full_request.detail = Detail::Full;
    full_request.include_diagnostics = true;
    let full = s.context_view(&full_request).unwrap();
    assert_eq!(full.diagnostics.len(), 1);
    let root = full.nodes.iter().find(|n| n.id == full.root_id).unwrap();
    assert!(root.metadata.get("returns").is_some());
}

#[test]
fn legacy_context_and_search_keep_full_data_for_the_reviewer() {
    let mut s = call_graph(1);
    let context = s.context("method:Wallet::pay", 1, 50).unwrap();
    assert_eq!(context.diagnostics.len(), 1);
    let root = context
        .nodes
        .iter()
        .find(|n| n.id == "method:Wallet::pay")
        .unwrap();
    assert!(root.metadata.get("returns").is_some());
    let found = s.search("pay", 10).unwrap();
    assert!(found.iter().any(|n| n.metadata.get("returns").is_some()));
}

#[test]
fn summary_drops_qualified_name_derivable_from_kind_prefixed_id() {
    let mut method = node("method:Wallet::refund", "method", "refund", json!({}));
    method.qualified_name = "Wallet::refund".into();
    let mut s = indexed(vec![method], vec![], vec![]);
    let page = s
        .search_page(&search("refund", &[], None, 10), Detail::Summary)
        .unwrap();
    assert_eq!(page.nodes[0].qualified_name, "");
    let full = s
        .search_page(&search("refund", &[], None, 10), Detail::Full)
        .unwrap();
    assert_eq!(full.nodes[0].qualified_name, "Wallet::refund");
}
