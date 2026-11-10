//! Yii params.php top-level keys → config_key nodes.
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::{Detail, GraphSnapshot, NodeSearch, SearchMode},
    repository::MemoryRepository,
    service::AtlasService,
};

fn analyze(files: &[(&str, &str)]) -> GraphSnapshot {
    let files: Vec<_> = files
        .iter()
        .map(|(path, source)| SourceFile {
            path: (*path).into(),
            source: (*source).into(),
        })
        .collect();
    code_atlas::plugins::registry()
        .analyze(
            &files,
            &["php".into()],
            &["yii2".into()],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

#[test]
fn params_php_keys_become_config_key_nodes() {
    let g = analyze(&[(
        "common/config/params.php",
        r#"<?php
return [
    'piastrixInvoiceRefund' => 'https://core.example/refund',
    'paySoftUrl' => 'https://pay.example/',
    'nested' => ['a' => 1],
];
"#,
    )]);
    let keys: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| n.kind == "config_key")
        .map(|n| n.name.as_str())
        .collect();
    assert!(
        keys.contains(&"piastrixInvoiceRefund"),
        "missing refund key in {keys:?}"
    );
    assert!(keys.contains(&"paySoftUrl"), "{keys:?}");
    assert!(keys.contains(&"nested"), "{keys:?}");
    let refund = g
        .nodes
        .iter()
        .find(|n| n.id == "config_key:params:piastrixInvoiceRefund")
        .unwrap();
    assert_eq!(refund.span.path, "common/config/params.php");
    assert_eq!(
        refund.metadata["value_expression"].as_str(),
        Some("'https://core.example/refund'")
    );
}

#[test]
fn config_key_is_findable_via_symbol_search() {
    let g = analyze(&[(
        "common/config/params.php",
        "<?php\nreturn [\n  'piastrixInvoiceRefund' => 'https://x',\n];\n",
    )]);
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(g, vec!["common/config/params.php".into()], vec![])
        .unwrap();
    let page = s
        .search_page(
            &NodeSearch {
                text: "piastrixInvoiceRefund".into(),
                kinds: vec!["config_key".into()],
                mode: SearchMode::Symbol,
                limit: 5,
                ..NodeSearch::default()
            },
            Detail::Summary,
        )
        .unwrap();
    assert_eq!(page.nodes.len(), 1);
    assert_eq!(page.nodes[0].id, "config_key:params:piastrixInvoiceRefund");
}
