//! PLUGIN-18: mixed/callable wrappers (useMaster) take the closure's return type.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::GraphSnapshot,
    plugins::php::PhpLanguagePlugin,
};

const SRC: &str = r#"<?php
namespace App;
class Withdraw {
  public $status;
  /** @return static|null */
  public static function findOne($id) { return null; }
}
class Connection {
  /**
   * @param callable $callback
   * @return mixed
   */
  public function useMaster(callable $callback) {
    return call_user_func($callback, $this);
  }
}
class Pay {
  public function withUse(Connection $db, $id) {
    $withdraw = $db->useMaster(function () use ($id) {
      return Withdraw::findOne($id);
    });
    $withdraw->status = 1;
  }
  public function withoutUse(Connection $db) {
    $withdraw = $db->useMaster(function () {
      return Withdraw::findOne(1);
    });
    $withdraw->status = 2;
  }
}
"#;

fn analyze() -> GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "Pay.php".into(),
                source: SRC.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

fn writes_to_status<'a>(
    g: &'a GraphSnapshot,
    from_method: &str,
) -> Vec<&'a code_atlas::domain::Edge> {
    g.edges
        .iter()
        .filter(|e| {
            e.kind == "writes"
                && e.to.as_deref() == Some("property:App\\Withdraw::$status")
                && e.from.contains(from_method)
        })
        .collect()
}

#[test]
fn use_master_closure_with_use_types_assignment() {
    let g = analyze();
    let writes = writes_to_status(&g, "Pay::withUse");
    assert_eq!(
        writes.len(),
        1,
        "expected writes edge; got {:?}",
        g.edges
            .iter()
            .filter(|e| e.from.contains("withUse") && e.metadata["member_name"] == "status")
            .map(|e| (&e.kind, e.metadata["receiver_class"].clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        writes[0].metadata["receiver_class"]
            .as_str()
            .unwrap()
            .contains("Withdraw"),
        "{}",
        writes[0].metadata["receiver_class"]
    );
}

#[test]
fn use_master_closure_without_use_types_assignment() {
    let g = analyze();
    assert_eq!(writes_to_status(&g, "Pay::withoutUse").len(), 1);
}
