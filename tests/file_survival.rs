//! Declarations survive method-body limitations; named args and case `;` parse.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    plugins::php::PhpLanguagePlugin,
};

fn analyze(source: &str) -> code_atlas::domain::GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "sample.php".into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

#[test]
fn named_argument_class_does_not_abort_file_parse() {
    let g = analyze(
        r#"<?php
namespace common\events;
class HandleWelcomeBonus {
  public static function register(): void {
    Event::on(
      class: HoldDeposit::class,
      name: HoldDeposit::EVENT_APPROVED_DEPOSIT,
      handler: static fn($event) => self::go($event),
    );
  }
  public static function go($event) {}
}
"#,
    );
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "class:common\\events\\HandleWelcomeBonus"),
        "{:?}",
        g.diagnostics
    );
    assert!(g.diagnostics.iter().all(|d| d.code != "parse_error"));
}

#[test]
fn case_with_semicolon_keeps_class_and_analyzes_switch() {
    let g = analyze(
        r#"<?php
class PaymentSystem {
  public function check($name) {
    switch ($name) {
      case self::VERIFICATION_USER;
        return false;
      default:
        return true;
    }
  }
}
"#,
    );
    assert!(g.nodes.iter().any(|n| n.id == "class:PaymentSystem"));
    assert!(g
        .nodes
        .iter()
        .any(|n| n.id == "method:PaymentSystem::check"));
    assert!(
        g.diagnostics
            .iter()
            .all(|d| !d.message.contains("case requires")),
        "{:?}",
        g.diagnostics
    );
}

#[test]
fn sibling_method_survives_alternative_syntax_in_another_method() {
    let g = analyze(
        r#"<?php
class BonusCod {
  public function deposit($amount) { return $amount > 0; }
  public function legacy($x) { if ($x): return 1; endif; }
}
class Caller {
  public function run(BonusCod $b) { $b->deposit(1); }
}
"#,
    );
    assert!(g.nodes.iter().any(|n| n.id == "class:BonusCod"));
    let deposit = g
        .edges
        .iter()
        .find(|e| e.kind == "calls" && e.metadata["member_name"] == "deposit");
    assert_eq!(
        deposit.and_then(|e| e.to.as_deref()),
        Some("method:BonusCod::deposit"),
        "{deposit:?} diagnostics={:?}",
        g.diagnostics
    );
}
