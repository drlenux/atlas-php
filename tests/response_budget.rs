//! R5 response budget: magic members collapse, name-only guesses are hidden by default,
//! full detail (web/reviewer) keeps everything.
use code_atlas::domain::*;
use code_atlas::mcp_text;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::{json, Value};

const PATH: &str = "app/Wallet.php";

fn node(id: &str, kind: &str, name: &str, line: usize, metadata: Value) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        qualified_name: String::new(),
        span: SourceSpan {
            path: PATH.into(),
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
            path: PATH.into(),
            start_line: 1,
            end_line: 1,
        },
        metadata: json!({}),
        trust: 1,
        revision: 0,
    }
}

fn ar_column(name: &str, line: usize) -> Node {
    node(
        &format!("property:app\\Wallet::${name}"),
        "property",
        &format!("${name}"),
        line,
        json!({
            "type": "int",
            "type_source": "framework",
            "framework": "yii2",
            "framework_evidence": "attributes",
            "magic": true,
            "access": "readwrite",
            "ar_attribute": true,
            "commented": false,
        }),
    )
}

const COLUMNS: [&str; 12] = [
    "id",
    "user_id",
    "balance",
    "bonus_balance",
    "currency",
    "status",
    "tax",
    "created_at",
    "updated_at",
    "locked_until",
    "limit_daily",
    "limit_monthly",
];

/// Class `Wallet` with one real method, twelve magic AR columns, and `Wallet::pay`
/// that reads one of them and is called by name only from an untyped site.
fn wallet() -> AtlasService {
    let mut nodes = vec![
        node("class:app\\Wallet", "class", "Wallet", 5, json!({})),
        node(
            "method:app\\Wallet::pay",
            "method",
            "pay",
            20,
            json!({"parameters":[{"name":"amount","type":"int"}],"declared_return_type":"bool"}),
        ),
        node(
            "method:app\\Wallet::exists",
            "method",
            "exists",
            6,
            json!({"parameters":[],"annotated_return_type":"mixed","magic":true,"type_source":"annotation"}),
        ),
        node(
            "read_site:pay:0",
            "read_site",
            "$this->balance",
            21,
            json!({"member_name":"balance"}),
        ),
        node("method:app\\Caller::run", "method", "run", 3, json!({})),
        node(
            "call_site:name_only",
            "call_site",
            "$x->pay()",
            4,
            json!({"member_name":"pay","resolution":"unresolved","receiver_class":"unknown","expression":"$x->pay()"}),
        ),
    ];
    let mut edges = vec![
        edge("contains", "class:app\\Wallet", "method:app\\Wallet::pay"),
        edge(
            "contains",
            "class:app\\Wallet",
            "method:app\\Wallet::exists",
        ),
        edge("contains", "method:app\\Wallet::pay", "read_site:pay:0"),
        edge("contains", "method:app\\Caller::run", "call_site:name_only"),
        edge("reads", "read_site:pay:0", "property:app\\Wallet::$balance"),
    ];
    for (i, name) in COLUMNS.iter().enumerate() {
        let column = ar_column(name, 7 + i);
        edges.push(edge("contains", "class:app\\Wallet", &column.id));
        nodes.push(column);
    }
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    service
        .index(
            GraphSnapshot {
                nodes,
                edges,
                diagnostics: vec![],
            },
            vec![PATH.into()],
            vec![],
        )
        .unwrap();
    service
}

fn context_text(detail: Detail) -> String {
    let mut s = wallet();
    let context = s
        .context_view(&ContextRequest {
            root_id: "method:app\\Wallet::pay".into(),
            depth: 2,
            max_nodes: 50,
            direction: Direction::Both,
            detail,
            include_diagnostics: false,
            expansion: Expansion::Focused,
        })
        .unwrap();
    mcp_text::context(&s.freshness().unwrap(), &context)
}

fn class_outline() -> (Outline, String) {
    let mut s = wallet();
    let outline = s.outline("class:app\\Wallet").unwrap();
    let freshness = s.freshness().unwrap();
    let text = mcp_text::outline(&freshness, &outline);
    (outline, text)
}

#[test]
fn class_outline_folds_magic_members_into_one_summary_line() {
    let (outline, text) = class_outline();
    assert!(text.contains("L20 method pay(int $amount): bool"), "{text}");
    let summary: Vec<&str> = text.lines().filter(|l| l.contains("magic (")).collect();
    assert_eq!(summary.len(), 1, "{text}");
    assert!(summary[0].contains("magic (13):"), "{text}");
    for name in ["$balance", "$limit_monthly", "exists()"] {
        assert!(summary[0].contains(name), "{name} missing: {text}");
    }
    assert!(!text.contains("[magic"), "per-member flags leaked: {text}");
    assert!(!text.contains("L7 property"), "{text}");
    // The service/web outline keeps one full line per member.
    assert_eq!(outline.members.len(), 14, "{:?}", outline.members);
    assert!(outline.members.iter().any(|m| m.contains("$balance: int")));
}

#[test]
fn explicit_members_section_keeps_every_magic_member_line() {
    let (outline, _) = class_outline();
    let mut s = wallet();
    let text = mcp_text::outline_expanded(&s.freshness().unwrap(), &outline);
    assert!(
        text.contains("L7 property $id: int [magic, framework]"),
        "{text}"
    );
    assert!(!text.contains("magic ("), "{text}");
}

#[test]
fn summary_context_drops_redundant_framework_fields_of_magic_nodes() {
    let lean = context_text(Detail::Summary);
    let line = lean
        .lines()
        .find(|l| l.contains("property $balance"))
        .unwrap_or_else(|| panic!("{lean}"));
    for noise in ["framework=yii2", "framework_evidence", "access=readwrite"] {
        assert!(!line.contains(noise), "{noise} in {line}");
    }
    assert!(line.contains("magic"), "{line}");
    assert!(line.contains("type_source=framework"), "{line}");
    let full = context_text(Detail::Full);
    for kept in [
        "framework=yii2",
        "framework_evidence=attributes",
        "access=readwrite",
    ] {
        assert!(full.contains(kept), "{kept} missing in full detail: {full}");
    }
}

fn pay_called_by_text() -> String {
    let mut s = wallet();
    let (outline, _, _) = s
        .outline_view("method:app\\Wallet::pay", &["called_by".into()], None, None)
        .unwrap();
    mcp_text::outline(&s.freshness().unwrap(), &outline)
}

#[test]
fn name_only_callers_are_hidden_by_name_or_by_unresolved() {
    let text = pay_called_by_text();
    assert!(text.contains("[unresolved: name only]"), "{text}");
    for token in ["name_only", "unresolved"] {
        let filtered = mcp_text::omit_resolutions(&text, &[token.to_owned()]);
        assert!(!filtered.contains("name only"), "{token}: {filtered}");
        assert!(
            filtered.contains(&format!("# omitted {token}=1")),
            "{token}: {filtered}"
        );
        // An emptied section must not read as an unexplained empty header.
        assert!(filtered.contains("called_by: none"), "{token}: {filtered}");
        assert!(
            filtered.contains("[proof=weak]"),
            "omitted callers are weak, not complete: {token}: {filtered}"
        );
    }
}

#[test]
fn omitting_name_only_keeps_plain_unresolved_lines() {
    let text = "# index state=completed\nA::a L1 [unresolved]: $a->a()\nB::b L2 [unresolved: name only]: $b->b()";
    let filtered = mcp_text::omit_resolutions(text, &["name_only".to_owned()]);
    assert!(filtered.contains("A::a L1 [unresolved]"), "{filtered}");
    assert!(!filtered.contains("B::b"), "{filtered}");
}

#[test]
fn hidden_tree_line_takes_its_subtree_along() {
    let text = [
        "# callers method:A::a levels=2 complete",
        "A::a a.php:1",
        "├ B::b L2 [unresolved: name only]: $x->a()",
        "│ └ C::c L3 [resolved]: $b->b()",
        "└ D::d L4 [resolved]: $d->a()",
        "  └ E::e L5 [resolved]: $d->d()",
    ]
    .join("\n");
    let filtered = mcp_text::omit_resolutions(&text, &["name_only".to_owned()]);
    assert!(
        !filtered.contains("B::b") && !filtered.contains("C::c"),
        "{filtered}"
    );
    assert!(
        filtered.contains("D::d") && filtered.contains("E::e"),
        "{filtered}"
    );
    assert!(filtered.ends_with("# omitted name_only=1"), "{filtered}");
}

#[test]
fn nothing_hidden_means_no_omitted_line() {
    let text = "# index state=completed\nA::a L1 [resolved]: $a->a()";
    assert_eq!(
        mcp_text::omit_resolutions(text, &["name_only".to_owned()]),
        text
    );
}

/// Metric: bytes of the agent texts before (nothing folded/hidden) vs after.
#[test]
fn compact_responses_are_smaller_on_the_fixture() {
    let (outline, compact_outline) = class_outline();
    let mut s = wallet();
    let full_outline = mcp_text::outline_expanded(&s.freshness().unwrap(), &outline);
    let compact_context = context_text(Detail::Summary);
    let full_context = context_text(Detail::Full);
    let called_by = pay_called_by_text();
    let compact_called_by = mcp_text::omit_resolutions(&called_by, &["name_only".to_owned()]);
    let before = full_outline.len() + full_context.len() + called_by.len();
    let after = compact_outline.len() + compact_context.len() + compact_called_by.len();
    eprintln!(
        "bytes outline {}->{} context {}->{} called_by {}->{} total {before}->{after}",
        full_outline.len(),
        compact_outline.len(),
        full_context.len(),
        compact_context.len(),
        called_by.len(),
        compact_called_by.len(),
    );
    assert!(compact_outline.len() * 2 <= full_outline.len(), "outline");
    assert!(compact_context.len() < full_context.len(), "context");
    // Omitting name-only may replace a long caller line with a short
    // `called_by: none [proof=weak]…` — still must state weakness, not silent none.
    assert!(
        compact_called_by.contains("[proof=weak]") || compact_called_by.len() < called_by.len(),
        "called_by: {compact_called_by}"
    );
    assert!(after * 5 <= before * 4, "{before} -> {after}");
}
