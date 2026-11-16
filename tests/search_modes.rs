//! lean-search-and-class-context: text = whole identifiers, substring = partial.
use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::json;
use std::collections::BTreeSet;

fn node(id: &str, kind: &str, name: &str, metadata: serde_json::Value) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        qualified_name: String::new(),
        span: SourceSpan {
            path: "src/Bonus.php".into(),
            start_line: 1,
            end_line: 1,
        },
        metadata,
        trust: 1,
        revision: 0,
    }
}

fn bonus_index() -> AtlasService {
    let mut nodes = vec![
        node(
            "method:A::getUserBonus",
            "method",
            "getUserBonus",
            json!({}),
        ),
        node(
            "method:A::getUserBonusCod",
            "method",
            "getUserBonusCod",
            json!({}),
        ),
        node(
            "call_site:1",
            "call_site",
            "$this->getUserBonus()",
            json!({"expression":"$this->getUserBonus()","member_name":"getUserBonus"}),
        ),
        node(
            "read_site:1",
            "read_site",
            "$targetUserBonus",
            json!({"expression":"$targetUserBonus"}),
        ),
        node(
            "call_site:2",
            "call_site",
            "$w->pay()",
            json!({"expression":"$w->pay()","member_name":"pay"}),
        ),
    ];
    // A log call only mentions the name inside a string literal.
    nodes.push(node(
        "call_site:log",
        "call_site",
        "Logger::info('getUserBonus failed', \"getUserBonus\")",
        json!({"expression":"Logger::info('getUserBonus failed', \"getUserBonus\")","member_name":"info"}),
    ));
    // A method's `$this` is scoped by the method name; the scope is not a match.
    let mut this = node(
        "variable:method:A::getUserBonus:$this",
        "variable",
        "$this",
        json!({}),
    );
    this.qualified_name = "method:A::getUserBonus:$this".into();
    nodes.push(this);
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes,
            edges: vec![],
            diagnostics: vec![],
        },
        vec!["src/Bonus.php".into()],
        vec![],
    )
    .unwrap();
    s
}

fn found(text: &str, mode: SearchMode) -> BTreeSet<String> {
    bonus_index()
        .search_page(
            &NodeSearch {
                text: text.into(),
                limit: 50,
                mode,
                ..NodeSearch::default()
            },
            Detail::Summary,
        )
        .unwrap()
        .nodes
        .into_iter()
        .map(|n| n.id)
        .collect()
}

#[test]
fn text_mode_matches_whole_identifiers_only() {
    let ids = found("getUserBonus", SearchMode::Text);
    assert_eq!(
        ids,
        BTreeSet::from(["method:A::getUserBonus".into(), "call_site:1".into()]),
    );
}

#[test]
fn substring_mode_keeps_partial_matches() {
    let ids = found("UserBonus", SearchMode::Substring);
    for expected in [
        "method:A::getUserBonus",
        "method:A::getUserBonusCod",
        "read_site:1",
        "call_site:log",
    ] {
        assert!(ids.contains(expected), "missing {expected}: {ids:?}");
    }
}

#[test]
fn punctuation_edges_need_no_boundary() {
    assert!(found("->pay(", SearchMode::Text).contains("call_site:2"));
}

#[test]
fn exclude_paths_drops_matching_nodes() {
    let mut s = bonus_index();
    let mut test_site = node(
        "call_site:test",
        "call_site",
        "$this->getUserBonus()",
        json!({"expression":"$this->getUserBonus()","member_name":"getUserBonus"}),
    );
    test_site.span.path = "common/tests/BonusTest.php".into();
    s.index(
        GraphSnapshot {
            nodes: vec![test_site],
            edges: vec![],
            diagnostics: vec![],
        },
        vec!["common/tests/BonusTest.php".into()],
        vec![],
    )
    .unwrap();
    assert!(bonus_index_ids(&mut s, None).contains("call_site:test"));
    let without_tests = bonus_index_ids(&mut s, code_atlas::glob::any_regex(&["*/tests/*".into()]));
    assert!(
        !without_tests.contains("call_site:test"),
        "{without_tests:?}"
    );
    assert!(without_tests.contains("call_site:1"));
}

fn bonus_index_ids(s: &mut AtlasService, exclude_paths: Option<String>) -> BTreeSet<String> {
    s.search_page(
        &NodeSearch {
            text: "getUserBonus".into(),
            limit: 50,
            exclude_paths,
            ..NodeSearch::default()
        },
        Detail::Summary,
    )
    .unwrap()
    .nodes
    .into_iter()
    .map(|n| n.id)
    .collect()
}

/// Benchmark: `symbol` search for `UserBonus::closeBonus` failed with a storage error in
/// PostgreSQL. The same fixture runs against both repositories (see `core_postgres.rs`).
fn class_method_nodes() -> Vec<Node> {
    let mut declaration = node(
        "method:common\\models\\UserBonus::closeBonus",
        "method",
        "closeBonus",
        json!({}),
    );
    declaration.qualified_name = "common\\models\\UserBonus::closeBonus".into();
    let mut other_declaration = node(
        "method:common\\models\\BonusCodLog::closeBonus",
        "method",
        "closeBonus",
        json!({}),
    );
    other_declaration.qualified_name = "common\\models\\BonusCodLog::closeBonus".into();
    vec![
        declaration,
        other_declaration,
        node(
            "call_site:user-bonus",
            "call_site",
            "$bonus->closeBonus()",
            json!({"expression":"$bonus->closeBonus()","member_name":"closeBonus",
                   "receiver_class":"common\\models\\UserBonus"}),
        ),
        node(
            "call_site:cod-log",
            "call_site",
            "$userBonusCod->closeBonus()",
            json!({"expression":"$userBonusCod->closeBonus()","member_name":"closeBonus",
                   "receiver_class":"common\\models\\BonusCodLog"}),
        ),
    ]
}

fn class_method_ids(query: &str, mode: SearchMode) -> Vec<String> {
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes: class_method_nodes(),
            edges: vec![],
            diagnostics: vec![],
        },
        vec!["src/Bonus.php".into()],
        vec![],
    )
    .unwrap();
    s.search_page(
        &NodeSearch {
            text: query.into(),
            limit: 50,
            mode,
            ..NodeSearch::default()
        },
        Detail::Summary,
    )
    .unwrap()
    .nodes
    .into_iter()
    .map(|n| n.id)
    .collect()
}

#[test]
fn class_method_symbol_search_finds_declaration_and_only_that_receivers_calls() {
    for query in [
        "common\\models\\UserBonus::closeBonus",
        "UserBonus::closeBonus",
        "userbonus::closebonus",
    ] {
        let ids = class_method_ids(query, SearchMode::Symbol);
        assert_eq!(
            ids,
            [
                "method:common\\models\\UserBonus::closeBonus",
                "call_site:user-bonus"
            ],
            "{query}"
        );
    }
}

#[test]
fn class_method_text_and_substring_search_keep_the_owner() {
    for mode in [SearchMode::Text, SearchMode::Substring] {
        let ids = class_method_ids("UserBonus::closeBonus", mode);
        assert_eq!(
            ids,
            ["method:common\\models\\UserBonus::closeBonus"],
            "{mode:?}"
        );
    }
}
