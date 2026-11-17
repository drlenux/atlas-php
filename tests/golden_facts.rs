//! Deterministic golden facts for cycle-2 product gate (no LLM).
//! Fixture facts always run. Live slotor checks are `#[ignore]` and need a running Atlas.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    mcp_text,
    plugins::php::PhpLanguagePlugin,
    repository::MemoryRepository,
    service::AtlasService,
};

fn index_php(source: &str, path: &str) -> AtlasService {
    let snapshot = PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: path.into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap();
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(snapshot, vec![path.into()], vec![], &std::collections::BTreeMap::new()).unwrap();
    s
}

#[test]
fn dead_method_called_by_is_proof_complete() {
    let src = r#"<?php
namespace App;
class Bonus {
  public function updateWager($x) {}
  public function updateWagerByTime($x) { $this->updateWagerByTime($x); }
}
"#;
    let mut s = index_php(src, "Bonus.php");
    let outline = s.outline("method:App\\Bonus::updateWager").unwrap();
    let text = mcp_text::outline(&s.freshness().unwrap(), &outline);
    assert!(
        text.contains("called_by: none [proof=complete]"),
        "dead method must not look like a soft miss: {text}"
    );
}

#[test]
fn create_freespin_missing_bonus_type_in_called_by() {
    let src = r#"<?php
namespace App;
class Freespin {
  const BONUS = 1;
  public static function createFreeSpin($a, $b, $c, $d, $e, $f, $event = false, $bonus_type = self::BONUS) {}
}
class MyBonusController {
  public function actionActivateBonus() {
    Freespin::createFreeSpin(1, 2, 3, 4, 5, 6);
  }
  public function actionOk() {
    Freespin::createFreeSpin(1, 2, 3, 4, 5, 6, false, Freespin::BONUS);
  }
}
"#;
    let mut s = index_php(src, "App.php");
    let outline = s.outline("method:App\\Freespin::createFreeSpin").unwrap();
    let text = outline.called_by.join("\n");
    assert!(
        text.contains("actionActivateBonus")
            && text.contains("missing:[")
            && text.contains("bonus_type"),
        "MaxWin API signal must appear in called_by: {text}"
    );
    assert!(
        !text
            .lines()
            .find(|l| l.contains("actionOk"))
            .is_some_and(|l| l.contains("missing:[") && l.contains("bonus_type")),
        "complete call must not mark bonus_type missing: {text}"
    );
}

#[test]
fn guards_before_create_freespin_appear_in_called_by() {
    let src = r#"<?php
namespace App;
class Freespin {
  public static function createFreeSpin($uid) {}
}
class BonusController {
  public function actionActivateBonus($user) {
    if ($user->isLockTakeBonuses()) {
      return;
    }
    if (!$user->isActive()) {
      throw new \RuntimeException('no');
    }
    Freespin::createFreeSpin($user->id);
  }
}
"#;
    let mut s = index_php(src, "Bonus.php");
    let outline = s.outline("method:App\\Freespin::createFreeSpin").unwrap();
    let text = outline.called_by.join("\n");
    assert!(
        text.contains("isLockTakeBonuses"),
        "DailyFreespin guard signal missing: {text}"
    );
}
