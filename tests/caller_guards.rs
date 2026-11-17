//! R3: `called_by` and the callers tree show the nearest early-exit guards of the
//! calling method that precede the call site (e.g. `isLockTakeBonuses`).
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::CallersRequest,
    mcp_text,
    plugins::php::PhpLanguagePlugin,
    repository::MemoryRepository,
    service::AtlasService,
};

const TARGET: &str = "method:App\\FreeSpins::createFreeSpin";

const SRC: &str = r#"<?php
namespace App;
class FreeSpins {
  public function createFreeSpin($userId) {}
}
class BonusController {
  private FreeSpins $freeSpins;
  public function actionActivateBonus($user) {
    if ($user->isLockTakeBonuses()) {
      return;
    }
    if (!$user->isActive()) {
      throw new \Exception('inactive');
    }
    if ($user->isVip()) {
      $this->log('vip');
    }
    $this->freeSpins->createFreeSpin($user->id);
    if ($user->isBanned()) {
      return;
    }
  }
  public function actionPlain($user) {
    $this->freeSpins->createFreeSpin($user->id);
  }
}
"#;

fn service_for(source: &str) -> AtlasService {
    let snapshot = PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "App.php".into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap();
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(snapshot, vec!["App.php".into()], vec![], &std::collections::BTreeMap::new()).unwrap();
    s
}

fn called_by(source: &str) -> Vec<String> {
    service_for(source).outline(TARGET).unwrap().called_by
}

fn caller_line<'a>(lines: &'a [String], owner: &str) -> &'a String {
    lines
        .iter()
        .find(|l| l.contains(owner))
        .unwrap_or_else(|| panic!("no caller {owner} in {lines:?}"))
}

#[test]
fn called_by_shows_guards_preceding_the_call() {
    let lines = called_by(SRC);
    let line = caller_line(&lines, "actionActivateBonus");
    assert!(
        line.contains(
            "guards:[if($user->isLockTakeBonuses()) return, if(!$user->isActive()) throw]"
        ),
        "{line}"
    );
}

#[test]
fn guards_after_the_call_and_non_exiting_ifs_are_left_out() {
    let lines = called_by(SRC);
    let line = caller_line(&lines, "actionActivateBonus");
    assert!(!line.contains("isBanned"), "{line}");
    assert!(!line.contains("isVip"), "{line}");
}

#[test]
fn caller_without_guards_keeps_the_plain_line() {
    let lines = called_by(SRC);
    let line = caller_line(&lines, "actionPlain");
    assert!(!line.contains("guards:"), "{line}");
    assert_eq!(
        line,
        "App\\BonusController::actionPlain L24: $this->freeSpins->createFreeSpin($user->id) [resolved]"
    );
}

#[test]
fn only_the_nearest_guards_are_listed_with_short_conditions() {
    let long = "$user->isSomethingWithAVeryLongName() && $user->anotherVeryLongConditionName() && $user->yetAnotherCondition()";
    let src = format!(
        "<?php\nnamespace App;\nclass FreeSpins {{\n  public function createFreeSpin($u) {{}}\n}}\nclass C {{\n  private FreeSpins $f;\n  public function run($user) {{\n    if ($a) return;\n    if ($b) return;\n    if ($c) return;\n    if ({long}) return;\n    $this->f->createFreeSpin($user);\n  }}\n}}\n"
    );
    let lines = called_by(&src);
    let line = caller_line(&lines, "C::run");
    assert!(!line.contains("if($a)"), "oldest guard dropped: {line}");
    assert!(line.contains("if($b)") && line.contains("if($c)"), "{line}");
    assert!(line.contains('…'), "long condition clipped: {line}");
    assert!(!line.contains("yetAnotherCondition"), "{line}");
}

#[test]
fn guard_in_a_sibling_block_does_not_apply() {
    let src = "<?php\nnamespace App;\nclass FreeSpins {\n  public function createFreeSpin($u) {}\n}\nclass C {\n  private FreeSpins $f;\n  public function run($user, $mode) {\n    if ($mode) {\n      if ($user->locked) { return; }\n      $this->log();\n    } else {\n      $this->f->createFreeSpin($user);\n    }\n  }\n}\n";
    let lines = called_by(src);
    let line = caller_line(&lines, "C::run");
    assert!(!line.contains("guards:"), "{line}");
}

#[test]
fn callers_tree_shows_guards() {
    let mut s = service_for(SRC);
    let tree = s
        .callers_tree(&CallersRequest {
            root_id: TARGET.into(),
            levels: 1,
            max_nodes: 50,
            exclude_paths: None,
            include_paths: None,
        })
        .unwrap();
    let text = mcp_text::callers(&s.freshness().unwrap(), &tree);
    let line = text
        .lines()
        .find(|l| l.contains("actionActivateBonus"))
        .unwrap();
    assert!(line.contains("isLockTakeBonuses"), "{text}");
    assert!(line.contains("guards:["), "{text}");
    let plain = text.lines().find(|l| l.contains("actionPlain")).unwrap();
    assert!(!plain.contains("guards:"), "{text}");
}
