//! R2: long calls keep named-argument names (and array keys) in `called_by` / callers tree.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::CallersRequest,
    mcp_text,
    plugins::php::PhpLanguagePlugin,
    repository::MemoryRepository,
    service::AtlasService,
};

const SRC: &str = r#"<?php
namespace App;
class FreeSpins {
  public function createFreeSpin($userId, $gameId, $count, $bonusType = null, $meta = []) {}
}
class MyBonusController {
  private FreeSpins $freeSpins;
  public function actionClaim($user, $game) {
    $this->freeSpins->createFreeSpin(
      userId: $user->id,
      gameId: $game->identifier->value,
      count: $game->settings->freeSpinsCountForCurrentUserSegment,
      bonusType: 'my_bonus_with_a_very_long_identifier_value',
      meta: ['campaign' => $user->campaign->code, 'source' => 'my_bonus_page'],
    );
  }
}
"#;

fn service() -> AtlasService {
    service_for(SRC)
}

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

const TARGET: &str = "method:App\\FreeSpins::createFreeSpin";

fn assert_names_visible(text: &str) {
    assert!(text.contains("MyBonusController::actionClaim"), "{text}");
    for name in ["bonusType:", "meta:", "campaign =>", "source =>"] {
        assert!(text.contains(name), "`{name}` lost in: {text}");
    }
}

#[test]
fn callers_tree_shows_named_args_of_long_call() {
    let mut s = service();
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
    assert_names_visible(&text);
}

#[test]
fn outline_called_by_shows_named_args_of_long_call() {
    let mut s = service();
    let outline = s.outline(TARGET).unwrap();
    assert_names_visible(&outline.called_by.join("\n"));
}

#[test]
fn short_named_call_keeps_raw_expression() {
    let src = "<?php\nnamespace App;\nclass FreeSpins {\n  public function createFreeSpin($a, $b) {}\n}\nclass C {\n  private FreeSpins $f;\n  public function run() { $this->f->createFreeSpin(a: 1, b: 2); }\n}\n";
    let mut s = service_for(src);
    let outline = s.outline(TARGET).unwrap();
    assert_eq!(
        outline.called_by,
        ["App\\C::run L8: $this->f->createFreeSpin(a: 1, b: 2) [resolved]"]
    );
}

/// Deep indent + static call: flatten stays under 100 chars while `clip_expr` still
/// truncates the stored expression (slotor partner/MyBonusController pattern).
#[test]
fn deep_indent_clipped_call_keeps_trailing_named_args() {
    let pad = "                        "; // 24 spaces, as in nested Yii action bodies
    let src = format!(
        r#"<?php
namespace partner\controllers;
class Freespin {{
  const BONUS_COD = 2;
  public static function createFreeSpin($userUuid, $currency, $gameId, $count, $denomination, $bonusId, $event, $bonus_type = 1) {{}}
}}
class MyBonusController {{
  public function actionActivateBonus($bonusCod, $game) {{
{pad}Freespin::createFreeSpin(
{pad}  userUuid: Yii::$app->user->identity->uid,
{pad}  currency: $bonusCod->currency,
{pad}  gameId: $game->id,
{pad}  count: $bonusCod->freespin_count,
{pad}  denomination: $game->denomination,
{pad}  bonusId: $bonusCod->id,
{pad}  event: $bonusCod->event,
{pad}  bonus_type: Freespin::BONUS_COD,
{pad});
  }}
}}
"#,
        pad = pad
    );
    let mut s = service_for(&src);
    let outline = s
        .outline("method:partner\\controllers\\Freespin::createFreeSpin")
        .unwrap();
    let text = outline.called_by.join("\n");
    assert!(
        text.contains("bonus_type:"),
        "trailing named arg lost (deep indent / clip): {text}"
    );
    assert!(
        text.contains("MyBonusController::actionActivateBonus"),
        "{text}"
    );
}
