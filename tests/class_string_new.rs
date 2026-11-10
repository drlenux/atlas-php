//! PLUGIN-19: `@return class-string<Base>` → `new $class` → calls resolve to Base.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::{Edge, GraphSnapshot},
    plugins::php::PhpLanguagePlugin,
};

const SRC: &str = r#"<?php
namespace App;
class AbstractAgg {
  public function initGame($game) {}
}
class Gaminator extends AbstractAgg {
  public function initGame($game) {}
  public function refresh() {}
}
class Aggregatorua extends Gaminator {
  public function refresh() {}
}
class Resolver {
  /**
   * @return class-string<Gaminator>
   */
  public static function familyClass($provider): string {
    return Gaminator::class;
  }
}
class Keys {
  const GAMINATOR = 'gaminator';
  const AGGREGATORUA = 'aggregatorua';
}
class Factory {
  private const MAP = [
    Keys::GAMINATOR => Gaminator::class,
    Keys::AGGREGATORUA => Aggregatorua::class,
  ];
  public function ignored() {}
}
class Champion extends AbstractAgg {
  public function initGame($game) {}
}
class GameController {
  public function actionPlay($game) {
    $aggregatorClass = Resolver::familyClass($game);
    $aggregator = new $aggregatorClass(['demo' => false]);
    $aggregator->initGame($game);
  }
  public function actionPlayBranched($game, $kind) {
    if ($kind === 'champion') {
      $aggregator = new Champion([]);
    } else {
      $aggregatorClass = Resolver::familyClass($game);
      $aggregator = new $aggregatorClass([]);
    }
    $aggregator->initGame($game);
  }
}
"#;

fn analyze() -> GraphSnapshot {
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

fn calls_to<'a>(g: &'a GraphSnapshot, to: &str) -> Vec<&'a Edge> {
    g.edges
        .iter()
        .filter(|e| e.kind == "calls" && e.to.as_deref() == Some(to))
        .collect()
}

#[test]
fn class_string_dynamic_new_types_init_game_call() {
    let g = analyze();
    let to_gaminator = calls_to(&g, "method:App\\Gaminator::initGame");
    assert!(
        !to_gaminator.is_empty(),
        "expected call to Gaminator::initGame; initGame edges: {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "initGame")
            .map(|e| (
                e.to.clone(),
                e.metadata["receiver_class"].clone(),
                e.metadata["resolution"].clone()
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        to_gaminator
            .iter()
            .any(|e| e.from.contains("GameController::actionPlay")),
        "{to_gaminator:?}"
    );
    let bogus = g.edges.iter().any(|e| {
        e.kind == "calls"
            && e.metadata["member_name"] == "initGame"
            && e.metadata["receiver_class"]
                .as_str()
                .is_some_and(|c| c.contains("$aggregatorClass"))
    });
    assert!(
        !bogus,
        "must not treat $aggregatorClass as a literal class name"
    );
}

#[test]
fn branched_receiver_union_emits_called_by_for_each_candidate() {
    let g = analyze();
    let to_gaminator = calls_to(&g, "method:App\\Gaminator::initGame");
    let to_champion = calls_to(&g, "method:App\\Champion::initGame");
    assert!(
        to_gaminator
            .iter()
            .any(|e| e.from.contains("actionPlayBranched")),
        "{to_gaminator:?}"
    );
    assert!(
        to_champion
            .iter()
            .any(|e| e.from.contains("actionPlayBranched")),
        "{to_champion:?}"
    );
}

#[test]
fn class_const_initializer_reads_referenced_constants() {
    let g = analyze();
    let reads: Vec<_> = g
        .edges
        .iter()
        .filter(|e| {
            e.kind == "reads" && e.to.as_deref() == Some("constant:App\\Keys::AGGREGATORUA")
        })
        .collect();
    assert!(
        reads.iter().any(|e| e.from.contains("Factory::MAP")
            || e.metadata["expression"]
                .as_str()
                .is_some_and(|s| s.contains("AGGREGATORUA"))),
        "expected MAP initializer to read Keys::AGGREGATORUA; got {:?}",
        g.edges
            .iter()
            .filter(|e| e.to.as_deref() == Some("constant:App\\Keys::AGGREGATORUA"))
            .map(|e| (&e.kind, &e.from))
            .collect::<Vec<_>>()
    );
}
