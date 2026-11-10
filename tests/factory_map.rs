//! PLUGIN-20/21: class-const MAP keys as read_site; new $class from ::class values.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::GraphSnapshot,
    plugins::php::PhpLanguagePlugin,
};

const SRC: &str = r#"<?php
namespace App;
class Keys {
  const GAMINATOR = 'gaminator';
  const AGGREGATORUA = 'aggregatorua';
}
class GaminatorProvider {
  public function __construct($provider) {}
}
class AggregatoruaProvider {
  public function __construct($provider) {}
}
class Factory {
  private const MAP = [
    Keys::GAMINATOR => GaminatorProvider::class,
    Keys::AGGREGATORUA => AggregatoruaProvider::class,
  ];
  public static function forProvider($provider, $aggregator) {
    $class = self::MAP[$aggregator] ?? null;
    if ($class === null) {
      return null;
    }
    return new $class($provider);
  }
}
"#;

fn analyze() -> GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "Factory.php".into(),
                source: SRC.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

#[test]
fn map_key_constant_emits_read_site() {
    let g = analyze();
    let sites: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| {
            n.kind == "read_site"
                && n.metadata["member_name"] == "AGGREGATORUA"
                && n.span.path.contains("Factory")
        })
        .collect();
    assert!(
        !sites.is_empty(),
        "expected read_site for Keys::AGGREGATORUA in MAP; nodes={:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "read_site")
            .map(|n| (&n.id, n.metadata.get("expression")))
            .collect::<Vec<_>>()
    );
    assert!(g.edges.iter().any(|e| {
        e.kind == "reads"
            && e.from.starts_with("read_site:")
            && e.to.as_deref() == Some("constant:App\\Keys::AGGREGATORUA")
    }));
}

#[test]
fn map_class_values_type_dynamic_new() {
    let g = analyze();
    let map = g
        .nodes
        .iter()
        .find(|n| n.id == "constant:App\\Factory::MAP")
        .expect("MAP constant");
    let values = map.metadata["class_string_values"]
        .as_array()
        .expect("class_string_values");
    assert!(
        values
            .iter()
            .any(|v| v.as_str() == Some("App\\AggregatoruaProvider")),
        "{values:?}"
    );
    let constructs: Vec<_> = g
        .edges
        .iter()
        .filter(|e| {
            e.kind == "calls"
                && e.metadata["construction"] == true
                && e.from.contains("forProvider")
        })
        .collect();
    assert!(
        constructs
            .iter()
            .any(|e| { e.to.as_deref() == Some("method:App\\AggregatoruaProvider::__construct") }),
        "expected construct of AggregatoruaProvider; got {:?}",
        constructs
            .iter()
            .map(|e| (&e.to, e.metadata["receiver_class"].clone()))
            .collect::<Vec<_>>()
    );
    assert!(constructs
        .iter()
        .any(|e| { e.to.as_deref() == Some("method:App\\GaminatorProvider::__construct") }));
}
