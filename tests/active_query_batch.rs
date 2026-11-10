//! ActiveQuery::batch() → T[][] so nested foreach types model writes.
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::GraphSnapshot,
};

const YII: &str = r#"<?php
namespace yii\db;
class BaseActiveRecord {}
class ActiveRecord extends BaseActiveRecord {
  /** @return ActiveQuery<static> */
  public static function find() { return null; }
}
class Query {
  /** @return $this */
  public function where($condition) { return $this; }
  /** @return $this */
  public function andWhere($condition) { return $this; }
  public function batch($batchSize = 100, $db = null) { return null; }
  public function each($batchSize = 100, $db = null) { return null; }
}
/** @template T of ActiveRecord|array */
class ActiveQuery extends Query {
  /** @return T|null */
  public function one($db = null) { return null; }
  /** @return T[] */
  public function all($db = null) { return []; }
}
"#;

const APP: &str = r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class UserBonus extends ActiveRecord {
  public $status;
  const STATUS_ACTIVE = 1;
}
namespace console\controllers;
use common\models\UserBonus;
class ServantController {
  public function actionActiveEndedFSWagerBonus($id) {
    $usersBonuses = UserBonus::find()->where(['bonus_id' => $id]);
    foreach ($usersBonuses->batch() as $userBonus) {
      foreach ($userBonus as $bonus) {
        $bonus->status = UserBonus::STATUS_ACTIVE;
      }
    }
  }
  public function actionEach($id) {
    $q = UserBonus::find()->where(['bonus_id' => $id]);
    foreach ($q->each() as $bonus) {
      $bonus->status = UserBonus::STATUS_ACTIVE;
    }
  }
}
"#;

fn analyze() -> GraphSnapshot {
    let files = [
        ("vendor/yiisoft/yii2/db/All.php", YII),
        ("common/models/UserBonus.php", APP),
    ]
    .map(|(path, source)| SourceFile {
        path: path.into(),
        source: source.into(),
    });
    let options = AnalysisOptions {
        declaration_paths: vec!["vendor/yiisoft".into()],
        ..AnalysisOptions::default()
    };
    code_atlas::plugins::registry()
        .analyze(&files, &["php".into()], &["yii2".into()], &options)
        .unwrap()
}

#[test]
fn batch_nested_foreach_types_status_write() {
    let g = analyze();
    let writes: Vec<_> = g
        .edges
        .iter()
        .filter(|e| {
            e.kind == "writes"
                && e.metadata["member_name"] == "status"
                && e.from.contains("actionActiveEndedFSWagerBonus")
        })
        .collect();
    assert!(
        !writes.is_empty(),
        "expected write to UserBonus::$status; edges={:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "writes")
            .map(|e| (
                e.to.clone(),
                e.metadata["receiver_class"].clone(),
                e.metadata["resolution"].clone()
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        writes.iter().any(|e| {
            e.to.as_deref() == Some("property:common\\models\\UserBonus::$status")
                && e.metadata["receiver_class"] == "common\\models\\UserBonus"
        }),
        "{writes:#?}"
    );
}

#[test]
fn each_foreach_types_status_write() {
    let g = analyze();
    let write = g
        .edges
        .iter()
        .find(|e| {
            e.kind == "writes"
                && e.metadata["member_name"] == "status"
                && e.from.contains("actionEach")
        })
        .expect("each() write");
    assert_eq!(
        write.to.as_deref(),
        Some("property:common\\models\\UserBonus::$status")
    );
}
