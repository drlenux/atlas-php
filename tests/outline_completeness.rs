//! agent-efficiency: outline states requested empty sections instead of omitting them.
use code_atlas::domain::*;
use code_atlas::repository::MemoryRepository;
use code_atlas::service::AtlasService;
use serde_json::json;

fn node(id: &str, kind: &str) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: id.rsplit("::").next().unwrap().into(),
        qualified_name: String::new(),
        span: SourceSpan {
            path: "src/A.php".into(),
            start_line: 3,
            end_line: 5,
        },
        metadata: json!({}),
        trust: 1,
        revision: 0,
    }
}

fn service() -> AtlasService {
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(
        GraphSnapshot {
            nodes: vec![
                node("interface:I", "interface"),
                node("method:I::run", "method"),
            ],
            edges: vec![Edge {
                id: "e".into(),
                kind: "contains".into(),
                from: "interface:I".into(),
                to: Some("method:I::run".into()),
                span: SourceSpan::default(),
                metadata: json!({}),
                trust: 1,
                revision: 0,
            }],
            diagnostics: vec![],
        },
        vec!["src/A.php".into()],
        vec![],
    )
    .unwrap();
    s
}

#[test]
fn explicitly_requested_empty_sections_are_reported() {
    let outline = service()
        .outline_with_usage(
            "method:I::run",
            &["called_by".into(), "overridden_by".into()],
        )
        .unwrap()
        .0;
    assert_eq!(outline.none, ["called_by", "overridden_by"]);
}

#[test]
fn default_method_outline_reports_empty_calls_and_callers_only() {
    let outline = service().outline("method:I::run").unwrap();
    assert_eq!(outline.none, ["calls", "called_by"]);
}
