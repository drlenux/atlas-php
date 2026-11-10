//! Method outline includes calls inside nested closures; ($this->fn)() invokes Closure.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::GraphSnapshot,
    plugins::php::PhpLanguagePlugin,
    repository::MemoryRepository,
    service::AtlasService,
};

const SRC: &str = r#"<?php
namespace App;
class Wallet {
  public function deposit($amount) {}
}
class Host {
  /** @var \Closure */
  private $credit;
  public function __construct(\Closure $credit) { $this->credit = $credit; }
  public function pay($amount) {
    ($this->credit)($amount);
  }
  public function activate(Wallet $wallet) {
    $credit = function ($amount) use ($wallet) {
      $wallet->deposit($amount);
    };
    $credit(10);
    return $credit;
  }
}
"#;

fn graph() -> GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "App.php".into(),
                source: SRC.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

#[test]
fn outline_lists_calls_inside_nested_closure() {
    let g = graph();
    let paths: Vec<String> = g
        .nodes
        .iter()
        .map(|n| n.span.path.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    service.index(g, paths, vec![]).expect("index");
    let outline = service
        .outline("method:App\\Host::activate")
        .expect("outline");
    assert!(
        outline
            .calls
            .iter()
            .any(|line| line.contains("deposit") && line.contains("Wallet::deposit")),
        "expected nested closure deposit in outline calls: {:#?}",
        outline.calls
    );
}

#[test]
fn property_closure_invoke_emits_call() {
    let g = graph();
    let invoke = g
        .edges
        .iter()
        .find(|e| {
            e.kind == "calls"
                && e.from.contains("Host::pay")
                && e.metadata["expression"]
                    .as_str()
                    .is_some_and(|s| s.contains("credit"))
        })
        .expect("invoke call edge");
    assert_ne!(invoke.metadata["resolution"], "unresolved", "{invoke:#?}");
}
