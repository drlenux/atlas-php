use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::json;

#[test]
fn source_paths_are_complete_and_deduplicated_without_search_limit() {
    let mut s = service();
    index(&mut s, vec![node("a", "A", 1), node("b", "B", 4)], vec![]);
    let mut other = node("c", "C", 1);
    other.span.path = "other/File.php".into();
    s.index(
        GraphSnapshot {
            nodes: vec![other],
            edges: vec![],
            diagnostics: vec![],
        },
        vec!["other/File.php".into()],
        vec![],
    )
    .unwrap();
    assert_eq!(
        s.source_paths().unwrap(),
        vec!["other/File.php", "src/Test.php"]
    );
}

fn node(id: &str, name: &str, line: usize) -> Node {
    Node {
        id: id.into(),
        kind: "method".into(),
        name: name.into(),
        qualified_name: name.into(),
        span: SourceSpan {
            path: "src/Test.php".into(),
            start_line: line,
            end_line: line + 2,
        },
        metadata: json!({"type":"int", "fingerprint":format!("body:{id}")}),
        trust: 1,
        revision: 0,
    }
}
fn service() -> AtlasService {
    AtlasService::new(Box::new(MemoryRepository::new()))
}
fn index(s: &mut AtlasService, nodes: Vec<Node>, edges: Vec<Edge>) {
    s.index(
        GraphSnapshot {
            nodes,
            edges,
            diagnostics: vec![],
        },
        vec!["src/Test.php".into()],
        vec![],
    )
    .unwrap();
}
fn current(s: &mut AtlasService, id: &str) -> Node {
    s.context(id, 0, 10)
        .unwrap()
        .nodes
        .into_iter()
        .find(|n| n.id == id)
        .unwrap()
}

#[test]
fn agent_changes_take_effect_and_reindex_preserves_trust_but_updates_span() {
    let mut s = service();
    index(&mut s, vec![node("a", "A::value", 1)], vec![]);
    let n = current(&mut s, "a");
    let c = s
        .correct(
            "node",
            "a",
            n.revision,
            json!({"metadata":{"type":"string"}}),
            "observed return",
        )
        .unwrap();
    assert_eq!(c.status, "pending");
    assert_eq!(current(&mut s, "a").metadata["type"], "string");
    index(&mut s, vec![node("a", "A::value", 20)], vec![]);
    let n = current(&mut s, "a");
    assert_eq!(n.trust, 2);
    assert_eq!(n.span.start_line, 20);
    assert_eq!(n.metadata["type"], "string");
}

#[test]
fn approval_protects_from_agent_and_stale_decisions_conflict() {
    let mut s = service();
    index(&mut s, vec![node("a", "A::value", 1)], vec![]);
    let rev = current(&mut s, "a").revision;
    let c = s
        .correct(
            "node",
            "a",
            rev,
            json!({"metadata":{"type":"string"}}),
            "fix",
        )
        .unwrap();
    assert_eq!(
        s.review(&c.id, rev, true).unwrap_err().kind,
        ErrorKind::Conflict
    );
    let rev = current(&mut s, "a").revision;
    s.review(&c.id, rev, true).unwrap();
    let n = current(&mut s, "a");
    assert_eq!(n.trust, 3);
    assert_eq!(
        s.correct(
            "node",
            "a",
            n.revision,
            json!({"metadata":{"type":"bool"}}),
            "fix"
        )
        .unwrap_err()
        .kind,
        ErrorKind::Forbidden
    );
}

#[test]
fn decline_restores_previous_correction_then_baseline_with_current_positions() {
    let mut s = service();
    index(&mut s, vec![node("a", "A::value", 1)], vec![]);
    let rev = current(&mut s, "a").revision;
    let first = s
        .correct(
            "node",
            "a",
            rev,
            json!({"metadata":{"type":"string"}}),
            "first",
        )
        .unwrap();
    let rev = current(&mut s, "a").revision;
    let second = s
        .correct(
            "node",
            "a",
            rev,
            json!({"metadata":{"type":"bool"}}),
            "second",
        )
        .unwrap();
    assert_eq!(
        s.review(&first.id, rev, false).unwrap_err().kind,
        ErrorKind::Conflict
    );
    index(&mut s, vec![node("a", "A::value", 30)], vec![]);
    let rev = current(&mut s, "a").revision;
    s.review(&second.id, rev, false).unwrap();
    let n = current(&mut s, "a");
    assert_eq!(n.metadata["type"], "string");
    assert_eq!(n.trust, 2);
    assert_eq!(n.span.start_line, 30);
    let pending = s.pending_reviews(10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, first.id);
    s.review(&first.id, n.revision, false).unwrap();
    let n = current(&mut s, "a");
    assert_eq!(n.metadata["type"], "int");
    assert_eq!(n.trust, 1);
    assert_eq!(n.span.start_line, 30);
}

#[test]
fn corrections_reject_structural_changes_and_missing_targets_atomically() {
    let mut s = service();
    index(
        &mut s,
        vec![node("a", "A", 1), node("b", "B", 4)],
        vec![Edge {
            id: "e".into(),
            kind: "calls".into(),
            from: "a".into(),
            to: Some("b".into()),
            span: SourceSpan {
                path: "src/Test.php".into(),
                start_line: 2,
                end_line: 2,
            },
            metadata: json!({}),
            trust: 1,
            revision: 0,
        }],
    );
    let n = current(&mut s, "a");
    assert_eq!(
        s.correct("node", "a", n.revision, json!({"id":"b"}), "bad")
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    let edge = s.context("a", 1, 10).unwrap().edges[0].clone();
    assert_eq!(
        s.correct("edge", "e", edge.revision, json!({"to":"missing"}), "bad")
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert!(s.pending_reviews(10).unwrap().is_empty());
    s.correct("edge", "e", edge.revision, json!({"to":"a"}), "target")
        .unwrap();
    let ctx = s.context("a", 1, 10).unwrap();
    assert_eq!(ctx.edges[0].to.as_deref(), Some("a"));
    assert_eq!(ctx.edges[0].trust, 2);
}

#[test]
fn unique_fingerprint_preserves_identity_and_confirmed_deletion_removes_protected_node() {
    let mut s = service();
    let old = node("old", "A::old", 1);
    index(&mut s, vec![old.clone()], vec![]);
    let rev = current(&mut s, "old").revision;
    s.correct(
        "node",
        "old",
        rev,
        json!({"metadata":{"type":"string"}}),
        "fix",
    )
    .unwrap();
    let mut renamed = node("new", "A::renamed", 20);
    renamed.metadata["fingerprint"] = old.metadata["fingerprint"].clone();
    index(&mut s, vec![renamed], vec![]);
    let n = current(&mut s, "old");
    assert_eq!(n.qualified_name, "A::renamed");
    assert_eq!(n.trust, 2);
    index(&mut s, vec![], vec![]);
    assert_eq!(
        s.context("old", 0, 10).unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn failed_index_batch_rolls_back_earlier_objects() {
    let mut s = service();
    index(&mut s, vec![node("a", "A", 1)], vec![]);
    let mut changed = node("a", "A", 9);
    changed.metadata["type"] = json!("bool");
    let result = s.index(
        GraphSnapshot {
            nodes: vec![changed],
            edges: vec![Edge {
                id: "bad".into(),
                kind: "calls".into(),
                from: "absent".into(),
                to: Some("a".into()),
                span: SourceSpan::default(),
                metadata: json!({}),
                trust: 1,
                revision: 0,
            }],
            diagnostics: vec![],
        },
        vec!["src/Test.php".into()],
        vec![],
    );
    assert!(result.is_err());
    assert_eq!(current(&mut s, "a").span.start_line, 1);
}

#[test]
fn analytics_deduplicate_response_and_elements_and_preserve_deleted_labels() {
    let mut s = service();
    index(&mut s, vec![node("a", "A", 1), node("b", "B", 4)], vec![]);
    s.record_delivery("r1", &["a".into(), "a".into(), "b".into()], &[], 100)
        .unwrap();
    s.record_delivery("r1", &["a".into()], &[], 101).unwrap();
    s.record_delivery("r2", &["a".into()], &[], 200).unwrap();
    let stats = s.analytics(100, 200, 60).unwrap();
    assert_eq!(stats.total_responses, 1);
    assert_eq!(stats.unique_nodes, 2);
    assert!(stats.elements.iter().all(|e| e.count == 1));
    index(&mut s, vec![], vec![]);
    let stats = s.analytics(0, 300, 60).unwrap();
    assert_eq!(stats.total_responses, 2);
    assert_eq!(
        stats.elements.iter().find(|e| e.id == "a").unwrap().count,
        2
    );
    assert_eq!(
        stats.elements.iter().find(|e| e.id == "a").unwrap().label,
        "A"
    );
}

#[test]
fn context_is_bounded_and_web_reads_do_not_count_as_delivery() {
    let mut s = service();
    index(
        &mut s,
        vec![node("a", "A", 1), node("b", "B", 4)],
        vec![Edge {
            id: "e".into(),
            kind: "calls".into(),
            from: "a".into(),
            to: Some("b".into()),
            span: SourceSpan::default(),
            metadata: json!({}),
            trust: 1,
            revision: 0,
        }],
    );
    let ctx = s.context("a", 1, 1).unwrap();
    assert_eq!(ctx.nodes.len(), 1);
    assert!(ctx.truncated);
    assert_eq!(s.analytics(0, 1000, 60).unwrap().total_responses, 0);
}

#[test]
fn deleting_previous_target_does_not_remove_a_retargeted_live_edge() {
    let mut s = service();
    let mut edge = Edge {
        id: "call".into(),
        kind: "calls".into(),
        from: "a".into(),
        to: Some("b".into()),
        span: SourceSpan {
            path: "src/Test.php".into(),
            start_line: 2,
            end_line: 2,
        },
        metadata: json!({}),
        trust: 1,
        revision: 0,
    };
    index(
        &mut s,
        vec![node("a", "A", 1), node("b", "B", 4), node("c", "C", 7)],
        vec![edge.clone()],
    );
    edge.to = Some("c".into());
    index(
        &mut s,
        vec![node("a", "A", 1), node("c", "C", 7)],
        vec![edge],
    );
    let ctx = s.context("a", 1, 10).unwrap();
    assert_eq!(
        ctx.edges.len(),
        1,
        "live updated edge must survive deletion of its former target"
    );
    assert_eq!(ctx.edges[0].to.as_deref(), Some("c"));
}

#[test]
fn ambiguous_rename_does_not_bypass_protected_metadata() {
    let mut s = service();
    index(&mut s, vec![node("old", "A::old", 1)], vec![]);
    let rev = current(&mut s, "old").revision;
    s.correct(
        "node",
        "old",
        rev,
        json!({"metadata":{"type":"string"}}),
        "fix",
    )
    .unwrap();
    index(&mut s, vec![node("new", "A::different", 9)], vec![]);
    let ctx = s.context("old", 0, 10).unwrap();
    assert_eq!(ctx.nodes[0].trust, 2);
    assert!(ctx
        .diagnostics
        .iter()
        .any(|d| d.code == "identity_ambiguous"));
    assert!(s.context("new", 0, 10).is_err());
}

#[test]
fn rename_remaps_structural_metadata_references_but_not_source_text() {
    let mut s = service();
    let original = node("old", "A::old", 1);
    index(&mut s, vec![original.clone()], vec![]);
    let mut renamed = node("new", "A::new", 10);
    renamed.metadata["fingerprint"] = original.metadata["fingerprint"].clone();
    renamed.metadata["returns"] = json!([{"origins":[{"symbol_id":"new","candidates":["new"],"definition":{"id":"new","expression":"new"}}]}]);
    let mut variable = node("variable", "$x", 11);
    variable.kind = "variable".into();
    variable.metadata = json!({"scope":"new","assignments":[{"origins":[{"symbol_id":"new","candidates":["new"]}]}],"expression":"new"});
    let edge = Edge {
        id: "edge".into(),
        kind: "calls".into(),
        from: "variable".into(),
        to: Some("new".into()),
        span: variable.span.clone(),
        metadata: json!({"candidates":["new"]}),
        trust: 1,
        revision: 0,
    };
    index(&mut s, vec![renamed, variable], vec![edge]);
    let method = current(&mut s, "old");
    assert_eq!(
        method.metadata["returns"][0]["origins"][0]["symbol_id"],
        "old"
    );
    assert_eq!(
        method.metadata["returns"][0]["origins"][0]["definition"]["id"],
        "old"
    );
    assert_eq!(
        method.metadata["returns"][0]["origins"][0]["definition"]["expression"],
        "new"
    );
    let variable = current(&mut s, "variable");
    assert_eq!(variable.metadata["scope"], "old");
    assert_eq!(variable.metadata["expression"], "new");
    assert_eq!(
        variable.metadata["assignments"][0]["origins"][0]["candidates"][0],
        "old"
    );
    assert_eq!(
        s.context("old", 1, 10).unwrap().edges[0].metadata["candidates"][0],
        "old"
    );
}

#[test]
fn changed_call_identity_cannot_replace_a_protected_edge_with_lower_trust() {
    let mut s = service();
    let original = Edge {
        id: "old-call".into(),
        kind: "calls".into(),
        from: "a".into(),
        to: Some("b".into()),
        span: SourceSpan {
            path: "src/Test.php".into(),
            start_line: 2,
            end_line: 2,
        },
        metadata: json!({"fingerprint":"old-expression"}),
        trust: 1,
        revision: 0,
    };
    index(
        &mut s,
        vec![node("a", "A", 1), node("b", "B", 4), node("c", "C", 7)],
        vec![original],
    );
    let edge = s.context("a", 1, 10).unwrap().edges.remove(0);
    s.correct(
        "edge",
        "old-call",
        edge.revision,
        json!({"to":"c"}),
        "runtime evidence",
    )
    .unwrap();
    let changed = Edge {
        id: "new-call".into(),
        kind: "calls".into(),
        from: "a".into(),
        to: Some("b".into()),
        span: SourceSpan {
            path: "src/Test.php".into(),
            start_line: 2,
            end_line: 2,
        },
        metadata: json!({"fingerprint":"changed-expression"}),
        trust: 1,
        revision: 0,
    };
    index(
        &mut s,
        vec![node("a", "A", 1), node("b", "B", 4), node("c", "C", 7)],
        vec![changed],
    );
    let context = s.context("a", 1, 10).unwrap();
    assert!(
        context
            .edges
            .iter()
            .any(|e| e.id == "old-call" && e.trust == 2 && e.to.as_deref() == Some("c")),
        "protected edge must survive ambiguous source edit"
    );
    assert!(!context.edges.iter().any(|e| e.id == "new-call"));
    assert!(context
        .diagnostics
        .iter()
        .any(|d| d.code == "identity_ambiguous"));
    index(
        &mut s,
        vec![node("a", "A", 1), node("b", "B", 4), node("c", "C", 7)],
        vec![],
    );
    assert!(
        s.context("a", 1, 10).unwrap().edges.is_empty(),
        "actual call deletion still removes protected edge"
    );
}

#[test]
fn ambiguous_endpoint_rename_cannot_cascade_delete_a_protected_edge() {
    let mut s = service();
    let edge = Edge {
        id: "call".into(),
        kind: "calls".into(),
        from: "a".into(),
        to: Some("b".into()),
        span: SourceSpan {
            path: "src/Test.php".into(),
            start_line: 2,
            end_line: 2,
        },
        metadata: json!({"fingerprint":"call"}),
        trust: 1,
        revision: 0,
    };
    index(
        &mut s,
        vec![node("a", "A", 1), node("b", "B", 4)],
        vec![edge.clone()],
    );
    let edge_revision = s.context("a", 1, 10).unwrap().edges[0].revision;
    s.correct(
        "edge",
        "call",
        edge_revision,
        json!({"metadata":{"note":"verified"}}),
        "fix",
    )
    .unwrap();
    let mut changed = edge;
    changed.id = "new-call".into();
    changed.from = "new-a".into();
    index(
        &mut s,
        vec![node("new-a", "RenamedAndEdited", 1), node("b", "B", 4)],
        vec![changed],
    );
    let ctx = s
        .context("a", 1, 10)
        .expect("endpoint identity must survive ambiguity when an edge is protected");
    assert!(ctx.nodes.iter().any(|n| n.id == "a" && n.trust == 1));
    assert!(ctx.edges.iter().any(|e| e.id == "call" && e.trust == 2));
}

#[test]
fn renamed_symbol_keeps_identity_when_its_body_changes_on_a_later_scan() {
    for protected in [false, true] {
        let mut s = service();
        let method = node("old", "A::before", 1);
        let mut variable = node("old-variable", "$x", 2);
        variable.kind = "variable".into();
        variable.metadata = json!({"scope":"old","fingerprint":"variable-original","type":"int"});
        index(&mut s, vec![method.clone(), variable.clone()], vec![]);
        if protected {
            let revision = current(&mut s, "old").revision;
            s.correct(
                "node",
                "old",
                revision,
                json!({"metadata":{"type":"verified"}}),
                "verified",
            )
            .unwrap();
        }
        let mut renamed = method;
        renamed.id = "new".into();
        renamed.name = "A::after".into();
        renamed.qualified_name = "A::after".into();
        renamed.span.start_line = 10;
        let mut renamed_variable = variable;
        renamed_variable.id = "new-variable".into();
        renamed_variable.metadata["scope"] = json!("new");
        index(
            &mut s,
            vec![renamed.clone(), renamed_variable.clone()],
            vec![],
        );
        assert_eq!(current(&mut s, "old").qualified_name, "A::after");
        renamed.span.start_line = 30;
        renamed.metadata["fingerprint"] = json!("method-body-edited");
        renamed.metadata["type"] = json!("string");
        renamed_variable.metadata["fingerprint"] = json!("variable-value-edited");
        renamed_variable.metadata["type"] = json!("string");
        // Variables deliberately precede their owning method: matching cannot depend on input order.
        index(&mut s, vec![renamed_variable, renamed], vec![]);
        let method = current(&mut s, "old");
        assert_eq!(method.span.start_line, 30);
        assert_eq!(method.trust, if protected { 2 } else { 1 });
        assert_eq!(
            method.metadata["type"],
            if protected { "verified" } else { "string" }
        );
        let variable = current(&mut s, "old-variable");
        assert_eq!(variable.metadata["scope"], "old");
        assert_eq!(variable.metadata["type"], "string");
        assert!(!s
            .context("old", 0, 10)
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code == "identity_ambiguous"));
    }
}

#[test]
fn reintroduced_old_name_cannot_steal_a_renamed_approved_identity() {
    let mut s = service();
    let original = node("method:A", "A::work", 1);
    index(&mut s, vec![original.clone()], vec![]);
    let revision = current(&mut s, "method:A").revision;
    let correction = s
        .correct(
            "node",
            "method:A",
            revision,
            json!({"metadata":{"note":"approved B"}}),
            "verified",
        )
        .unwrap();
    s.review(&correction.id, correction.revision, true).unwrap();
    let mut renamed = original;
    renamed.id = "method:B".into();
    renamed.name = "B::work".into();
    renamed.qualified_name = "B::work".into();
    index(&mut s, vec![renamed.clone()], vec![]);
    let mut reintroduced = node("method:A", "A::work", 20);
    reintroduced.metadata["fingerprint"] = json!("new A body");
    reintroduced.metadata["returns"] = json!([{"origins":[{"symbol_id":"method:A"}]}]);
    let call = Edge {
        id: "A-to-B".into(),
        kind: "calls".into(),
        from: "method:A".into(),
        to: Some("method:B".into()),
        span: reintroduced.span.clone(),
        metadata: json!({"candidates":["method:B"]}),
        trust: 1,
        revision: 0,
    };
    index(
        &mut s,
        vec![reintroduced.clone(), renamed.clone()],
        vec![call.clone()],
    );
    let b = current(&mut s, "method:A");
    assert_eq!(b.qualified_name, "B::work");
    assert_eq!(b.trust, 3);
    assert_eq!(b.metadata["note"], "approved B");
    let a = s.search("A::work", 10).unwrap().remove(0);
    assert_ne!(a.id, b.id);
    assert_eq!(a.trust, 1);
    assert_eq!(a.metadata["returns"][0]["origins"][0]["symbol_id"], a.id);
    let context = s.context(&a.id, 1, 10).unwrap();
    let edge = context.edges.iter().find(|e| e.id == "A-to-B").unwrap();
    assert_eq!(edge.from, a.id);
    assert_eq!(edge.to.as_deref(), Some(b.id.as_str()));
    assert_eq!(edge.metadata["candidates"][0], b.id);
    index(&mut s, vec![renamed, reintroduced], vec![call]);
    let b_again = current(&mut s, "method:A");
    assert_eq!(b_again.qualified_name, "B::work");
    assert_eq!(b_again.trust, 3);
    let a_again = s.search("A::work", 10).unwrap().remove(0);
    assert_eq!(a_again.id, a.id);
    assert_eq!(a_again.trust, 1);
}

#[test]
fn republishing_unchanged_code_updates_analyzer_facts_of_nodes_and_edges() {
    let mut s = service();
    let mut site = node("call_site:a", "pay", 3);
    site.kind = "call_site".into();
    site.metadata = json!({"fingerprint":"fp-site","resolution":"unresolved","type_source":"code"});
    let mut call = Edge {
        id: "e".into(),
        kind: "calls".into(),
        from: "call_site:a".into(),
        to: Some("b".into()),
        span: SourceSpan::default(),
        metadata: json!({"fingerprint":"fp-edge","resolution":"resolved"}),
        trust: 1,
        revision: 0,
    };
    index(
        &mut s,
        vec![site.clone(), node("b", "B", 4)],
        vec![call.clone()],
    );
    // Same source text (same fingerprints and spans); a newer analyzer knows more.
    site.metadata =
        json!({"fingerprint":"fp-site","resolution":"annotated","type_source":"annotation"});
    call.metadata =
        json!({"fingerprint":"fp-edge","resolution":"annotated","type_source":"annotation"});
    index(&mut s, vec![site, node("b", "B", 4)], vec![call]);
    let context = s.context("call_site:a", 1, 10).unwrap();
    let node = context
        .nodes
        .iter()
        .find(|n| n.id == "call_site:a")
        .unwrap();
    assert_eq!(node.metadata["resolution"], "annotated");
    assert_eq!(node.metadata["type_source"], "annotation");
    assert_eq!(context.edges[0].metadata["resolution"], "annotated");
}

#[test]
fn repeated_diagnostics_for_unpublished_files_do_not_accumulate() {
    let mut s = service();
    let mut broken = node("b", "B", 1);
    broken.span.path = "src/Broken.php".into();
    s.index(
        GraphSnapshot {
            nodes: vec![broken],
            edges: vec![],
            diagnostics: vec![],
        },
        vec!["src/Broken.php".into()],
        vec![],
    )
    .unwrap();
    let parse_error = Diagnostic {
        path: "src/Broken.php".into(),
        line: 7,
        code: "parse_error".into(),
        message: "unexpected token".into(),
    };
    for _ in 0..3 {
        // The file now fails to parse: its previous graph stays, it is not in scope.
        s.index(
            GraphSnapshot {
                nodes: vec![node("a", "A", 1)],
                edges: vec![],
                diagnostics: vec![parse_error.clone()],
            },
            vec!["src/Test.php".into()],
            vec![],
        )
        .unwrap();
    }
    let reported = s.context("b", 0, 10).unwrap().diagnostics;
    assert_eq!(reported, vec![parse_error]);
}

#[test]
fn reset_removes_graph_corrections_analytics_and_index_time() {
    let mut s = service();
    index(&mut s, vec![node("a", "A", 1), node("b", "B", 4)], vec![]);
    s.correct(
        "node",
        "a",
        1,
        json!({"metadata":{"note":"agent"}}),
        "checked",
    )
    .unwrap();
    s.record_delivery("r1", &["a".into()], &[], 10).unwrap();
    assert!(s.freshness().unwrap().indexed_at.is_some());

    s.reset().unwrap();

    assert_eq!(s.context("a", 0, 10).unwrap_err().kind, ErrorKind::NotFound);
    assert!(s.search("A", 10).unwrap().is_empty());
    assert!(s.pending_reviews(10).unwrap().is_empty());
    assert_eq!(s.analytics(0, 1000, 60).unwrap().total_responses, 0);
    assert_eq!(s.freshness().unwrap().indexed_at, None);
    // The atlas works normally afterwards.
    index(&mut s, vec![node("a", "A", 1)], vec![]);
    assert_eq!(current(&mut s, "a").trust, 1);
}

#[test]
fn partial_publication_accepts_edges_to_stored_nodes_of_other_paths() {
    let mut s = service();
    let mut target = node("target", "T", 1);
    target.span.path = "src/Other.php".into();
    s.index(
        GraphSnapshot {
            nodes: vec![target],
            edges: vec![],
            diagnostics: vec![],
        },
        vec!["src/Other.php".into()],
        vec![],
    )
    .unwrap();
    let caller = node("caller", "C", 1);
    let call = Edge {
        id: "call".into(),
        kind: "calls".into(),
        from: "caller".into(),
        to: Some("target".into()),
        span: SourceSpan {
            path: "src/Test.php".into(),
            start_line: 1,
            end_line: 1,
        },
        metadata: json!({"fingerprint":"call"}),
        trust: 1,
        revision: 0,
    };
    // Only src/Test.php is published; the target lives in an unchanged path.
    index(&mut s, vec![caller], vec![call]);
    assert_eq!(s.context("caller", 1, 10).unwrap().edges.len(), 1);
    let missing = Edge {
        id: "dangling".into(),
        to: Some("nowhere".into()),
        ..s.context("caller", 1, 10).unwrap().edges[0].clone()
    };
    let error = s
        .index(
            GraphSnapshot {
                nodes: vec![node("caller", "C", 1)],
                edges: vec![missing],
                diagnostics: vec![],
            },
            vec!["src/Test.php".into()],
            vec![],
        )
        .unwrap_err();
    assert!(error.message.contains("nowhere"), "{error:?}");
}

#[test]
fn area_prefix_rekeys_unprotected_entities_and_keeps_protected_ids() {
    for protected in [false, true] {
        let mut s = service();
        let mut class = node("class:Foo", "Foo", 1);
        class.kind = "class".into();
        class.metadata["fingerprint"] = json!("fp-class");
        let mut method = node("method:Foo::run", "Foo::run", 2);
        method.metadata["fingerprint"] = json!("fp-run");
        index(&mut s, vec![class.clone(), method.clone()], vec![]);
        if protected {
            let revision = current(&mut s, "method:Foo::run").revision;
            s.correct(
                "node",
                "method:Foo::run",
                revision,
                json!({"metadata":{"note":"verified"}}),
                "verified",
            )
            .unwrap();
        }
        // Same source, now inside area `console`: namespace-less names gain the prefix.
        let mut prefixed_class = class;
        prefixed_class.id = "class:console:Foo".into();
        prefixed_class.qualified_name = "console:Foo".into();
        prefixed_class.metadata["area"] = json!("console");
        let mut prefixed_method = method;
        prefixed_method.id = "method:console:Foo::run".into();
        prefixed_method.qualified_name = "console:Foo::run".into();
        prefixed_method.metadata["area"] = json!("console");
        index(&mut s, vec![prefixed_class, prefixed_method], vec![]);
        assert!(
            s.context("class:console:Foo", 0, 1).is_ok(),
            "protected={protected}"
        );
        assert!(
            s.context("class:Foo", 0, 1).is_err(),
            "the old spelling is gone"
        );
        let (kept, gone) = if protected {
            ("method:Foo::run", "method:console:Foo::run")
        } else {
            ("method:console:Foo::run", "method:Foo::run")
        };
        assert!(
            s.context(kept, 0, 1).is_ok(),
            "protected={protected}: {kept}"
        );
        assert!(
            s.context(gone, 0, 1).is_err(),
            "protected={protected}: {gone}"
        );
    }
}
