//! agent-navigation: symbol search, honest truncation, outline, freshness (MCP-08..11).
use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::{json, Value};

fn node(id: &str, kind: &str, name: &str, qualified: &str, line: usize, metadata: Value) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        qualified_name: qualified.into(),
        span: SourceSpan {
            path: "app/Model.php".into(),
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
        span: SourceSpan {
            path: "app/Model.php".into(),
            start_line: 1,
            end_line: 1,
        },
        metadata: json!({}),
        trust: 1,
        revision: 0,
    }
}
fn indexed(nodes: Vec<Node>, edges: Vec<Edge>) -> AtlasService {
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes,
            edges,
            diagnostics: vec![],
        },
        vec!["app/Model.php".into()],
        vec![],
    )
    .unwrap();
    s
}

fn call_sites() -> AtlasService {
    let site = |id: &str, name: &str, member: &str| {
        node(id, "call_site", name, id, 5, json!({"member_name": member}))
    };
    indexed(
        vec![
            node(
                "method:A::processingGameTransaction",
                "method",
                "processingGameTransaction",
                "A::processingGameTransaction",
                3,
                json!({}),
            ),
            site(
                "call_site:1",
                "$this->processingGameTransaction($a)",
                "processingGameTransaction",
            ),
            site(
                "call_site:2",
                "BackgroundGameProcess::processingGameTransaction($b)",
                "processingGameTransaction",
            ),
            site(
                "call_site:3",
                "Logger::writeInfo('processingGameTransaction start')",
                "writeInfo",
            ),
            node(
                "property:A::$tax",
                "property",
                "$tax",
                "A::$tax",
                4,
                json!({}),
            ),
            node(
                "write_site:1",
                "write_site",
                "$wallet->tax = 1",
                "write_site:1",
                5,
                json!({"member_name": "tax", "receiver_class": "common\\models\\Wallet"}),
            ),
            node(
                "write_site:status",
                "write_site",
                "$this->status",
                "write_site:status",
                6,
                json!({
                    "member_name": "status",
                    "receiver_class": "common\\models\\Withdraw",
                    "value_expression": "self::STATUS_REFUSE",
                    "value_constant": "self::STATUS_REFUSE"
                }),
            ),
            node(
                "write_site:bonus-refuse",
                "write_site",
                "$this->status",
                "write_site:bonus-refuse",
                8,
                json!({
                    "member_name": "status",
                    "receiver_class": "common\\models\\UserBonus",
                    "value_expression": "self::STATUS_REFUSE",
                    "value_constant": "self::STATUS_REFUSE"
                }),
            ),
            node(
                "write_site:status-nullable",
                "write_site",
                "$withdraw->status = 1",
                "write_site:status-nullable",
                7,
                json!({
                    "member_name": "status",
                    "receiver_class": "common\\models\\Withdraw|null",
                    "receiver_root_class": ""
                }),
            ),
        ],
        vec![],
    )
}
fn search(mode: SearchMode, text: &str, kinds: &[&str]) -> NodeSearch {
    NodeSearch {
        text: text.into(),
        kinds: kinds.iter().map(|k| k.to_string()).collect(),
        mode,
        limit: 50,
        ..NodeSearch::default()
    }
}
fn ids(page: &SearchPage) -> Vec<&str> {
    page.nodes.iter().map(|n| n.id.as_str()).collect()
}

#[test]
fn search_ranks_exact_declarations_before_occurrence_sites() {
    let mut s = call_sites();
    let page = s
        .search_page(
            &search(SearchMode::Text, "processingGameTransaction", &[]),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(
        page.nodes.first().map(|n| n.kind.as_str()),
        Some("method"),
        "declaration before call_site/log hits: {:?}",
        ids(&page)
    );
    assert!(
        page.next_cursor
            .as_deref()
            .is_none_or(|c| c.contains('|') || page.nodes.len() < 20),
        "ranked cursor uses score|id when more pages exist"
    );
}

#[test]
fn symbol_mode_matches_names_not_expression_text() {
    let mut s = call_sites();
    let symbol = s
        .search_page(
            &search(
                SearchMode::Symbol,
                "processingGameTransaction",
                &["call_site"],
            ),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(ids(&symbol), ["call_site:1", "call_site:2"]);
    let text = s
        .search_page(
            &search(
                SearchMode::Text,
                "processingGameTransaction",
                &["call_site"],
            ),
            Detail::Summary,
        )
        .unwrap();
    // The log call names the method only inside a string literal: text skips it.
    assert_eq!(ids(&text), ["call_site:1", "call_site:2"]);
    let substring = s
        .search_page(
            &search(
                SearchMode::Substring,
                "processingGameTransaction",
                &["call_site"],
            ),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(substring.nodes.len(), 3);
    let all = s
        .search_page(
            &search(SearchMode::Symbol, "PROCESSINGgameTransaction", &[]),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(all.nodes.len(), 3, "declaration and both call sites");
    let qualified = s
        .search_page(
            &search(SearchMode::Symbol, "A::processingGameTransaction", &[]),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(ids(&qualified), ["method:A::processingGameTransaction"]);
    for query in ["tax", "$tax"] {
        let page = s
            .search_page(&search(SearchMode::Symbol, query, &[]), Detail::Summary)
            .unwrap();
        assert_eq!(ids(&page), ["property:A::$tax", "write_site:1"], "{query}");
    }
    for query in [
        "Withdraw::$status",
        "common\\models\\Withdraw::status",
        "common\\models\\Withdraw::$status",
    ] {
        let page = s
            .search_page(
                &search(SearchMode::Symbol, query, &["write_site"]),
                Detail::Summary,
            )
            .unwrap();
        assert_eq!(
            ids(&page),
            ["write_site:status", "write_site:status-nullable"],
            "{query}"
        );
    }
    let wallet_tax = s
        .search_page(
            &search(SearchMode::Symbol, "Wallet::$tax", &["write_site"]),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(ids(&wallet_tax), ["write_site:1"]);
    let partial = s
        .search_page(
            &search(SearchMode::Symbol, "processing", &[]),
            Detail::Summary,
        )
        .unwrap();
    assert!(partial.nodes.is_empty(), "symbol mode is exact");
    // BL-008 style: find writes by assigned constant, not only by field name.
    let by_value = s
        .search_page(
            &search(SearchMode::Symbol, "STATUS_REFUSE", &["write_site"]),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(
        ids(&by_value),
        ["write_site:bonus-refuse", "write_site:status"],
        "value_constant symbol search"
    );
    let by_value_text = s
        .search_page(
            &search(SearchMode::Text, "STATUS_REFUSE", &["write_site"]),
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(by_value_text.nodes.len(), 2);
}

#[test]
fn depth_boundary_is_reported_separately_from_truncation() {
    let mut s = indexed(
        vec![
            node("class:A", "class", "A", "A", 1, json!({})),
            node("method:A::run", "method", "run", "A::run", 2, json!({})),
            node("class:Lonely", "class", "Lonely", "Lonely", 9, json!({})),
        ],
        vec![edge("contains", "class:A", "method:A::run")],
    );
    let lone = s.context("method:A::run", 0, 10).unwrap();
    assert!(!lone.truncated);
    assert!(lone.depth_limited);
    let full = s.context("method:A::run", 1, 10).unwrap();
    assert!(!full.truncated && !full.depth_limited);
    let isolated = s.context("class:Lonely", 0, 10).unwrap();
    assert!(!isolated.truncated && !isolated.depth_limited);
    let cut = s.context("class:A", 1, 1).unwrap();
    assert!(cut.truncated);
}

fn hierarchy() -> AtlasService {
    indexed(
        vec![
            node(
                "class:app\\Base",
                "class",
                "Base",
                "app\\Base",
                1,
                json!({}),
            ),
            node(
                "class:app\\Model",
                "class",
                "Model",
                "app\\Model",
                2,
                json!({"parent": "app\\Base"}),
            ),
            node(
                "class:app\\Child",
                "class",
                "Child",
                "app\\Child",
                30,
                json!({"parent": "app\\Model"}),
            ),
            node(
                "interface:app\\I",
                "interface",
                "I",
                "app\\I",
                40,
                json!({}),
            ),
            node(
                "class:app\\Impl1",
                "class",
                "Impl1",
                "app\\Impl1",
                41,
                json!({}),
            ),
            node(
                "class:app\\Impl2",
                "class",
                "Impl2",
                "app\\Impl2",
                42,
                json!({}),
            ),
            node(
                "method:app\\Model::save",
                "method",
                "save",
                "app\\Model::save",
                3,
                json!({"parameters":[{"name":"x","type":"int"},{"name":"y","type":"unknown"}],"declared_return_type":"bool","returns":[{"snippet":"return true;"}]}),
            ),
            node(
                "method:app\\Model::find",
                "method",
                "find",
                "app\\Model::find",
                9,
                json!({"parameters":[],"declared_return_type":"unknown","annotated_return_type":"yii\\db\\ActiveQuery<static>"}),
            ),
            node(
                "method:app\\Model::exists",
                "method",
                "exists",
                "app\\Model::exists",
                2,
                json!({"parameters":[],"declared_return_type":"unknown","annotated_return_type":"mixed","magic":true,"type_source":"annotation"}),
            ),
            node(
                "property:app\\Model::$tax",
                "property",
                "$tax",
                "app\\Model::$tax",
                2,
                json!({"type":"float","magic":true,"type_source":"annotation"}),
            ),
            node(
                "method:app\\Child::save",
                "method",
                "save",
                "app\\Child::save",
                31,
                json!({"parameters":[],"declared_return_type":"bool"}),
            ),
        ],
        vec![
            edge("contains", "class:app\\Model", "method:app\\Model::save"),
            edge("contains", "class:app\\Model", "method:app\\Model::find"),
            edge("contains", "class:app\\Model", "method:app\\Model::exists"),
            edge("contains", "class:app\\Model", "property:app\\Model::$tax"),
            edge("contains", "class:app\\Child", "method:app\\Child::save"),
            edge("inherits", "class:app\\Model", "class:app\\Base"),
            edge("inherits", "class:app\\Child", "class:app\\Model"),
            edge("implements", "class:app\\Impl1", "interface:app\\I"),
            edge("implements", "class:app\\Impl2", "interface:app\\I"),
            edge(
                "overrides",
                "method:app\\Child::save",
                "method:app\\Model::save",
            ),
        ],
    )
}

#[test]
fn class_outline_lists_members_compactly_with_hierarchy() {
    let mut s = hierarchy();
    let outline = s.outline("class:app\\Model").unwrap();
    assert_eq!(outline.name, "app\\Model");
    assert_eq!(outline.parents, ["class:app\\Base"]);
    assert_eq!(outline.children, ["class:app\\Child"]);
    assert_eq!(
        outline.members,
        [
            "L2 method exists(): mixed [magic, annotation]",
            "L2 property $tax: float [magic, annotation]",
            "L3 method save(int $x, $y): bool",
            "L9 method find(): yii\\db\\ActiveQuery<static> [annotation]",
        ]
    );
    let json = serde_json::to_string(&outline).unwrap();
    assert!(
        !json.contains("return true"),
        "no bodies or returns: {json}"
    );
}

#[test]
fn interface_outline_lists_implementations_and_method_outline_lists_overrides() {
    let mut s = hierarchy();
    let interface = s.outline("interface:app\\I").unwrap();
    assert_eq!(
        interface.implementations,
        ["class:app\\Impl1", "class:app\\Impl2"]
    );
    let base = s.outline("method:app\\Model::save").unwrap();
    assert_eq!(base.owner.as_deref(), Some("class:app\\Model"));
    assert_eq!(base.overridden_by, ["method:app\\Child::save"]);
    assert!(
        base.members.is_empty(),
        "default method outline omits members"
    );
    let with_members = s
        .outline_with_usage(
            "method:app\\Model::save",
            &["members".into(), "owner".into()],
        )
        .unwrap()
        .0;
    assert_eq!(with_members.members, ["L3 method save(int $x, $y): bool"]);
    let child = s.outline("method:app\\Child::save").unwrap();
    assert_eq!(child.overrides, ["method:app\\Model::save"]);
    assert_eq!(s.outline("missing").unwrap_err().kind, ErrorKind::NotFound);
}

#[test]
fn freshness_reports_publication_time_and_pending_changes() {
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    assert_eq!(s.freshness().unwrap().indexed_at, None);
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    s.index(GraphSnapshot::default(), vec![], vec![]).unwrap();
    let fresh = s.freshness().unwrap();
    assert!(fresh
        .indexed_at
        .is_some_and(|t| t >= before && t <= before + 5));
    assert!(!fresh.changes_pending);
    s.indexing_progress().set_changes_pending(true);
    assert!(s.freshness().unwrap().changes_pending);
}

#[test]
fn method_outline_lists_outgoing_and_incoming_calls_one_line_each() {
    let site = |id: &str, name: &str, line: usize| node(id, "call_site", name, id, line, json!({}));
    let mut call_a = edge("calls", "call_site:a1", "method:W::pay");
    call_a.metadata = json!({"resolution":"resolved"});
    let mut call_b = Edge {
        to: None,
        ..edge("calls", "call_site:a2", "none")
    };
    call_b.metadata = json!({"resolution":"unresolved"});
    let mut call_c = edge("calls", "call_site:b1", "method:A::run");
    call_c.metadata = json!({"resolution":"annotated"});
    let mut push = edge("dispatches", "call_site:a3", "method:Job::execute");
    push.metadata = json!({"resolution":"annotated","queue":"queueGame"});
    let mut s = indexed(
        vec![
            node(
                "method:A::run",
                "method",
                "run",
                "A::run",
                4,
                json!({"parameters":[]}),
            ),
            node(
                "method:B::go",
                "method",
                "go",
                "B::go",
                8,
                json!({"parameters":[]}),
            ),
            node("method:W::pay", "method", "pay", "W::pay", 1, json!({})),
            node(
                "method:Job::execute",
                "method",
                "execute",
                "Job::execute",
                1,
                json!({}),
            ),
            site("call_site:a1", "$w->pay(1)", 5),
            site("call_site:a2", "Logger::info(\n  'x'\n)", 6),
            site("call_site:a3", "Yii::$app->queueGame->push(new Job())", 7),
            site("call_site:b1", "$a->run()", 9),
        ],
        vec![
            edge("contains", "method:A::run", "call_site:a1"),
            edge("contains", "method:A::run", "call_site:a2"),
            edge("contains", "method:A::run", "call_site:a3"),
            edge("contains", "method:B::go", "call_site:b1"),
            call_a,
            call_b,
            call_c,
            push,
        ],
    );
    let outline = s.outline("method:A::run").unwrap();
    assert_eq!(
        outline.calls,
        [
            "L5 $w->pay(1) → W::pay [resolved]",
            "L6 Logger::info( 'x' ) → ? [unresolved]",
            "L7 Yii::$app->queueGame->push(new Job()) ⇒ Job::execute [annotated, queue queueGame]",
        ]
    );
    assert_eq!(outline.called_by, ["B::go L9: $a->run() [annotated]"]);
    let job = s.outline("method:Job::execute").unwrap();
    assert_eq!(
        job.called_by,
        ["A::run L7: Yii::$app->queueGame->push(new Job()) [annotated, queue queueGame]"]
    );
}

#[test]
fn freshness_lists_pending_paths_and_derives_changes_pending_from_them() {
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    let progress = s.indexing_progress();
    progress.set_pending_paths(vec!["app/A.php".into(), "app/B.php".into()]);
    let fresh = s.freshness().unwrap();
    assert_eq!(fresh.pending_paths, ["app/A.php", "app/B.php"]);
    assert!(fresh.changes_pending);
    let many: Vec<String> = (0..150).map(|i| format!("f{i}.php")).collect();
    progress.set_pending_paths(many);
    let fresh = s.freshness().unwrap();
    assert_eq!(fresh.pending_paths.len(), 100, "capped");
    progress.set_pending_paths(vec![]);
    assert!(!s.freshness().unwrap().changes_pending);
}
