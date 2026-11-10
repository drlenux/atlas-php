use code_atlas::{
    domain::*,
    progress::{IndexingProgress, ProgressSnapshot},
    repository::{MemoryRepository, Repository},
    service::AtlasService,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

fn node(id: &str) -> Node {
    Node {
        id: id.into(),
        kind: "method".into(),
        name: id.into(),
        qualified_name: id.into(),
        span: SourceSpan {
            path: "app.php".into(),
            start_line: 1,
            end_line: 2,
        },
        metadata: json!({}),
        trust: 1,
        revision: 0,
    }
}

fn edge(id: &str, fingerprint: &str) -> Edge {
    Edge {
        id: id.into(),
        kind: "calls".into(),
        from: "caller".into(),
        to: Some("callee".into()),
        span: node("caller").span,
        metadata: json!({"fingerprint":fingerprint}),
        trust: 1,
        revision: 0,
    }
}

fn publish(service: &mut AtlasService, edges: Vec<Edge>) -> IndexReport {
    service
        .index(
            GraphSnapshot {
                nodes: vec![node("caller"), node("callee")],
                edges,
                diagnostics: vec![],
            },
            vec!["app.php".into()],
            vec![],
        )
        .unwrap()
}

#[derive(Default)]
struct Probe {
    progress: Option<IndexingProgress>,
    events: Vec<(&'static str, ProgressSnapshot)>,
}
struct ObservedRepository {
    inner: MemoryRepository,
    probe: Arc<Mutex<Probe>>,
}
impl ObservedRepository {
    fn observe(&self, operation: &'static str) {
        let mut probe = self.probe.lock().unwrap();
        if let Some(progress) = &probe.progress {
            let snapshot = progress.snapshot();
            probe.events.push((operation, snapshot));
        }
    }
}
macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $out:ty;)*) => { $(
        fn $name(&mut self, $($arg: $ty),*) -> Result<$out> {
            self.observe(stringify!($name));
            self.inner.$name($($arg),*)
        }
    )* };
}
impl Repository for ObservedRepository {
    forward! {
        begin() -> ();
        commit() -> ();
        rollback() -> ();
        nodes() -> Vec<Node>;
        edges() -> Vec<Edge>;
        node(id: &str) -> Option<Node>;
        edge(id: &str) -> Option<Edge>;
        put_node(node: &Node) -> ();
        put_edge(edge: &Edge) -> ();
        delete_node(id: &str) -> ();
        delete_edge(id: &str) -> ();
        corrections() -> Vec<StoredCorrection>;
        put_correction(correction: &StoredCorrection) -> ();
        diagnostics() -> Vec<Diagnostic>;
        set_diagnostics(diagnostics: &[Diagnostic]) -> ();
        delivery(id: &str) -> Option<Delivery>;
        deliveries(from: i64, to: i64) -> Vec<Delivery>;
        put_delivery(delivery: &Delivery) -> ();
        meta(key: &str) -> Option<String>;
        set_meta(key: &str, value: &str) -> ();
        reset() -> ();
    }
}

fn observed_publication() -> Arc<Mutex<Probe>> {
    let probe = Arc::new(Mutex::new(Probe::default()));
    let mut service = AtlasService::new(Box::new(ObservedRepository {
        inner: MemoryRepository::new(),
        probe: probe.clone(),
    }));
    let progress = service.indexing_progress();
    probe.lock().unwrap().progress = Some(progress.clone());
    progress.start(1000);
    publish(
        &mut service,
        (0..12)
            .map(|i| edge(&format!("edge:{i}"), &i.to_string()))
            .collect(),
    );
    probe
}

#[test]
fn publication_checks_endpoints_without_per_edge_queries() {
    let probe = observed_publication();
    let probe = probe.lock().unwrap();
    assert_eq!(
        probe.events.iter().filter(|(op, _)| *op == "node").count(),
        0,
        "publication must validate endpoints from its loaded node set, without N+1 queries"
    );
}

#[test]
fn publication_reports_writes_before_commit() {
    let probe = observed_publication();
    let probe = probe.lock().unwrap();
    // MemoryRepository batches via default put_nodes/put_edges → per-row put_*.
    for (operation, detail, total) in [
        ("put_node", "write_nodes", Some(2)),
        ("put_edge", "write_edges", Some(12)),
        ("commit", "commit", None),
    ] {
        let snapshots: Vec<_> = probe
            .events
            .iter()
            .filter(|(op, _)| *op == operation)
            .map(|(_, s)| s)
            .collect();
        assert!(!snapshots.is_empty());
        for snapshot in snapshots {
            assert_eq!(
                snapshot.state, "running",
                "must not announce success before DB commit"
            );
            assert_eq!(snapshot.stage, "publishing");
            assert_eq!(snapshot.detail, detail);
            assert_eq!(snapshot.total, total);
        }
    }
}

#[test]
fn call_site_matching_excludes_direct_matches_and_keeps_duplicate_candidates_distinct() {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    publish(
        &mut service,
        vec![
            edge("direct", "shared"),
            edge("moved", "shared"),
            edge("ambiguous", "duplicate"),
        ],
    );
    let report = publish(
        &mut service,
        vec![
            edge("direct", "shared"),
            edge("new-position", "shared"),
            edge("duplicate-a", "duplicate"),
            edge("duplicate-b", "duplicate"),
        ],
    );
    assert_eq!(report.deleted_edges, 1);
    let edges = service.context("caller", 1, 100).unwrap().edges;
    let mut ids: Vec<_> = edges.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["direct", "duplicate-a", "duplicate-b", "moved"]);
    assert_eq!(edges.iter().find(|e| e.id == "moved").unwrap().revision, 2);
}

#[test]
#[ignore = "explicit large-graph performance regression; run separately"]
fn moved_call_sites_publish_within_a_bounded_time() {
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    const COUNT: usize = 20_000;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
        publish(
            &mut service,
            (0..COUNT)
                .map(|i| edge(&format!("old:{i}"), &i.to_string()))
                .collect(),
        );
        let started = Instant::now();
        let report = publish(
            &mut service,
            (0..COUNT)
                .map(|i| edge(&format!("new:{i}"), &i.to_string()))
                .collect(),
        );
        sender.send((started.elapsed(), report)).unwrap();
    });
    let (elapsed, report) = receiver.recv_timeout(Duration::from_secs(15)).expect(
        "publishing 20,000 moved call sites exceeded 15 seconds without SQL: repeated full graph scans",
    );
    eprintln!("20,000 moved call sites published in {elapsed:?}");
    assert_eq!(report.edges, COUNT);
    assert_eq!(
        report.deleted_edges, 0,
        "unique identities must survive movement"
    );
}

#[test]
fn node_fingerprint_matching_scales_for_large_graphs() {
    use std::time::Instant;
    const COUNT: usize = 25_000;
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let old_nodes: Vec<_> = (0..COUNT)
        .map(|i| {
            let mut n = node(&format!("old:{i}"));
            n.metadata = json!({"fingerprint": format!("fp:{i}")});
            n
        })
        .collect();
    service
        .index(
            GraphSnapshot {
                nodes: old_nodes,
                edges: vec![],
                diagnostics: vec![],
            },
            vec!["app.php".into()],
            vec![],
        )
        .unwrap();
    let new_nodes: Vec<_> = (0..COUNT)
        .map(|i| {
            let mut n = node(&format!("new:{i}"));
            n.metadata = json!({"fingerprint": format!("fp:{i}")});
            n
        })
        .collect();
    let started = Instant::now();
    let report = service
        .index(
            GraphSnapshot {
                nodes: new_nodes,
                edges: vec![],
                diagnostics: vec![],
            },
            vec!["app.php".into()],
            vec![],
        )
        .unwrap();
    assert!(
        started.elapsed().as_secs() < 5,
        "fingerprint rematch of {COUNT} nodes took {:?}",
        started.elapsed()
    );
    assert_eq!(report.nodes, COUNT);
    assert_eq!(report.deleted_nodes, 0);
    assert_eq!(current_revision(&mut service, "old:0"), 2);
}

fn current_revision(service: &mut AtlasService, id: &str) -> u64 {
    service.context(id, 0, 1).unwrap().nodes[0].revision
}

#[test]
fn call_site_matching_respects_scope_kind_source_and_old_ambiguity() {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let mut outside = edge("outside", "outside");
    outside.span.path = "other.php".into();
    let mut different_kind = edge("different-kind", "kind");
    different_kind.kind = "reads".into();
    let mut different_source = edge("different-source", "source");
    different_source.from = "callee".into();
    service
        .index(
            GraphSnapshot {
                nodes: vec![node("caller"), node("callee")],
                edges: vec![
                    edge("amb-a", "duplicate"),
                    edge("amb-b", "duplicate"),
                    outside,
                    different_kind,
                    different_source,
                ],
                diagnostics: vec![],
            },
            vec!["app.php".into(), "other.php".into()],
            vec![],
        )
        .unwrap();
    let report = publish(
        &mut service,
        vec![
            edge("new-amb", "duplicate"),
            edge("new-outside", "outside"),
            edge("new-kind", "kind"),
            edge("new-source", "source"),
        ],
    );
    assert_eq!(report.deleted_edges, 4);
    let edges = service.context("caller", 1, 100).unwrap().edges;
    let mut ids: Vec<_> = edges.iter().map(|e| e.id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "new-amb",
            "new-kind",
            "new-outside",
            "new-source",
            "outside"
        ]
    );
    assert!(edges.iter().all(|edge| edge.revision == 1));
}

#[test]
fn multiple_protected_edges_survive_one_ambiguous_replacement_group() {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    publish(
        &mut service,
        (0..3)
            .map(|i| edge(&format!("old:{i}"), &format!("old-fp:{i}")))
            .collect(),
    );
    for i in 0..3 {
        service
            .correct(
                "edge",
                &format!("old:{i}"),
                1,
                json!({"metadata":{"note":"protected"}}),
                "reviewed relationship",
            )
            .unwrap();
    }
    publish(
        &mut service,
        (0..3)
            .map(|i| edge(&format!("new:{i}"), &format!("new-fp:{i}")))
            .collect(),
    );
    let context = service.context("caller", 1, 100).unwrap();
    assert_eq!(context.edges.len(), 3);
    assert!(context.edges.iter().all(|edge| edge.id.starts_with("old:")
        && edge.trust == 2
        && edge.metadata["note"] == "protected"));
    assert_eq!(
        context
            .diagnostics
            .iter()
            .filter(|d| d.code == "identity_ambiguous")
            .count(),
        3
    );
}
