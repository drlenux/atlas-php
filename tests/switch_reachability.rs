//! A `return` in one `case` must not mark the next `case` as unreachable
//! (Champion-style provider callbacks: credit returns, then bet handles stake).
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    plugins::php::PhpLanguagePlugin,
};

fn analyze(source: &str) -> code_atlas::domain::GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "Champion.php".into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

#[test]
fn return_in_one_case_does_not_make_next_case_unreachable() {
    let g = analyze(
        r#"<?php
namespace App;
class Champion {
  function handle($type, Wallet $wallet) {
    switch ($type) {
      case 'credit':
        return ['credit' => $wallet->balance()];
      case 'bet':
        $wallet->debit(1);
        return ['ok' => true];
    }
  }
}
class Wallet {
  function balance() { return 0; }
  function debit($amount) { return $amount; }
}
"#,
    );
    let debit = g
        .edges
        .iter()
        .find(|e| e.kind == "calls" && e.metadata["member_name"] == "debit")
        .expect("debit call in case bet");
    let conditions = debit.metadata["conditions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !conditions
            .iter()
            .any(|c| c.as_str() == Some("unreachable_after_exit")),
        "case bet must stay reachable after return in case credit: {conditions:?}"
    );
    assert_eq!(
        debit.to.as_deref(),
        Some("method:App\\Wallet::debit"),
        "{debit:?}"
    );
}

#[test]
fn break_stops_fallthrough_into_following_case() {
    let g = analyze(
        r#"<?php
namespace App;
class S {
  function run($type, $w) {
    switch ($type) {
      case 'a':
        $w->touchA();
        break;
      case 'b':
        $w->touchB();
        break;
    }
  }
}
class W {
  function touchA() {}
  function touchB() {}
}
"#,
    );
    let touch_b = g
        .edges
        .iter()
        .find(|e| e.kind == "calls" && e.metadata["member_name"] == "touchB")
        .expect("touchB");
    let conditions = touch_b.metadata["conditions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !conditions
            .iter()
            .any(|c| c.as_str() == Some("unreachable_after_exit")),
        "{conditions:?}"
    );
}
