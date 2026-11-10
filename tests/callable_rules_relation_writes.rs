//! Callable arrays, rules() validators, relation property→getter, mass-assignment writes.
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::GraphSnapshot,
};

fn analyze(files: &[(&str, &str)]) -> GraphSnapshot {
    let files: Vec<_> = files
        .iter()
        .map(|(path, source)| SourceFile {
            path: (*path).into(),
            source: (*source).into(),
        })
        .collect();
    let options = AnalysisOptions {
        declaration_paths: vec!["vendor/yiisoft".into()],
        ..AnalysisOptions::default()
    };
    code_atlas::plugins::registry()
        .analyze(&files, &["php".into()], &["yii2".into()], &options)
        .unwrap()
}

const AR: &str = r#"<?php
namespace yii\db;
class BaseActiveRecord {
  public function hasOne($class, $link) { return null; }
  public function updateAttributes($attrs) { return 1; }
  public function setAttributes($attrs) {}
  public function setAttribute($name, $value) {}
  public function load($data, $formName = null) { return true; }
  public static function updateAll($attrs, $condition = '') { return 1; }
}
class ActiveRecord extends BaseActiveRecord {}
class ActiveQuery {}
"#;

#[test]
fn callable_array_emits_calls() {
    let g = analyze(&[(
        "app/Catalog.php",
        r#"<?php
namespace app;
class Catalog {
  public static function slugOf($item) { return 'x'; }
  public function import($items) {
    return array_map([self::class, 'slugOf'], $items);
  }
  public function viaThis() {
    $cb = [$this, 'slugOf'];
    return $cb;
  }
}
"#,
    )]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:app\\Catalog::slugOf")
                && e.metadata["framework_evidence"] == "callable_array"
                && e.from.contains("import")
        }),
        "array_map([self::class,'slugOf']); {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls")
            .map(|e| (
                e.from.clone(),
                e.to.clone(),
                e.metadata["framework_evidence"].clone()
            ))
            .collect::<Vec<_>>()
    );
    assert!(g.edges.iter().any(|e| {
        e.kind == "calls"
            && e.to.as_deref() == Some("method:app\\Catalog::slugOf")
            && e.from.contains("viaThis")
            && e.metadata["framework_evidence"] == "callable_array"
    }));
}

#[test]
fn rules_string_validator_calls_method() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/BaseActiveRecord.php", AR),
        (
            "common/models/Bonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Bonus extends ActiveRecord {
  public function rules() {
    return [
      [['ladder'], 'validateLadder', 'skipOnEmpty' => false],
      ['name', 'required'],
    ];
  }
  public function validateLadder($attr) {}
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.from == "method:common\\models\\Bonus::rules"
                && e.to.as_deref() == Some("method:common\\models\\Bonus::validateLadder")
                && e.metadata["framework_evidence"] == "rules_validator"
        }),
        "rules → validateLadder; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls")
            .map(|e| (
                e.from.clone(),
                e.to.clone(),
                e.metadata["framework_evidence"].clone()
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn relation_property_read_calls_getter() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/BaseActiveRecord.php", AR),
        (
            "common/models/Bonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Bonus extends ActiveRecord {
  public function getPublicTitle() { return 't'; }
}
"#,
        ),
        (
            "common/models/UserBonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class UserBonus extends ActiveRecord {
  public function getBonus() {
    return $this->hasOne(Bonus::class, ['id' => 'bonus_id']);
  }
  public function title() {
    return $this->bonus->getPublicTitle();
  }
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\models\\UserBonus::getBonus")
                && e.from.contains("title")
                && e.metadata["framework_evidence"] == "relation_property"
        }),
        "$this->bonus → getBonus(); {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls" && e.to.as_deref().is_some_and(|t| t.contains("getBonus")))
            .map(|e| (e.from.clone(), e.to.clone(), e.metadata.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn update_attributes_creates_write_sites() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/BaseActiveRecord.php", AR),
        (
            "console/Job.php",
            r#"<?php
namespace console;
use yii\db\ActiveRecord;
class Row extends ActiveRecord {
  public $is_take_bonus;
  public function mark() {
    $this->updateAttributes(['is_take_bonus' => true]);
  }
  public function markOther(Row $questionnaire) {
    $questionnaire->updateAttributes(['is_take_bonus' => false]);
  }
}
"#,
        ),
    ]);
    assert!(
        g.nodes
            .iter()
            .any(|n| { n.kind == "write_site" && n.metadata["member_name"] == "is_take_bonus" }),
        "updateAttributes write_site; sites={:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "write_site")
            .map(|n| (&n.name, n.metadata["member_name"].clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.nodes.iter().any(|n| {
            n.kind == "write_site"
                && n.metadata["member_name"] == "is_take_bonus"
                && n.metadata["receiver_expression"] == "$questionnaire"
        }),
        "receiver_expression from call receiver; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "write_site")
            .map(|n| n.metadata["receiver_expression"].clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn conditional_has_one_null_is_relation() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/BaseActiveRecord.php", AR),
        (
            "common/models/Bonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Bonus extends ActiveRecord {}
"#,
        ),
        (
            "common/models/UserBonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class UserBonus extends ActiveRecord {
  public $is_promocod;
  public function getBonus() {
    if (!$this->is_promocod) {
      return $this->hasOne(Bonus::class, ['id' => 'bonus_id']);
    }
    return null;
  }
  public function title() {
    return $this->bonus;
  }
}
"#,
        ),
    ]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "property:common\\models\\UserBonus::$bonus"),
        "conditional hasOne/null must create relation property"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\models\\UserBonus::getBonus")
                && e.metadata["framework_evidence"] == "relation_property"
        }),
        "$this->bonus → getBonus"
    );
}

#[test]
fn nullable_receiver_links_relation_getter() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/BaseActiveRecord.php", AR),
        (
            "common/models/PaySystem.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class PaySystem extends ActiveRecord {}
"#,
        ),
        (
            "common/models/Payment.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Payment extends ActiveRecord {
  public function getPaymentSystem() {
    return $this->hasOne(PaySystem::class, ['id' => 'ps_id']);
  }
}
"#,
        ),
        (
            "admin/controllers/FinanceController.php",
            r#"<?php
namespace admin\controllers;
use common\models\Payment;
class FinanceController {
  public function actionShow($id) {
    /** @var Payment|null $payment */
    $payment = Payment::findOne($id);
    return $payment->paymentSystem;
  }
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\models\\Payment::getPaymentSystem")
                && e.metadata["framework_evidence"] == "relation_property"
        }),
        "Payment|null → paymentSystem → getPaymentSystem; {:?}",
        g.edges
            .iter()
            .filter(|e| e.metadata["member_name"] == "paymentSystem"
                || e.to
                    .as_deref()
                    .is_some_and(|t| t.contains("getPaymentSystem")))
            .map(|e| (
                e.kind.clone(),
                e.to.clone(),
                e.metadata["resolution"].clone(),
                e.metadata["receiver_class"].clone()
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn write_site_records_assigned_constant_for_value_search() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/BaseActiveRecord.php", AR),
        (
            "common/models/UserBonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class UserBonus extends ActiveRecord {
  public $status;
  public const STATUS_REFUSE = 3;
  public function refuseBonus() {
    $this->status = self::STATUS_REFUSE;
    return $this->save(false);
  }
  public static function bulkRefuse() {
    self::updateAll(['status' => self::STATUS_REFUSE], ['status' => 1]);
  }
}
"#,
        ),
    ]);
    let assign = g
        .nodes
        .iter()
        .find(|n| {
            n.kind == "write_site"
                && n.metadata["member_name"] == "status"
                && n.metadata["value_constant"]
                    .as_str()
                    .is_some_and(|c| c.contains("STATUS_REFUSE"))
        })
        .expect("assignment write_site with value_constant");
    assert!(
        assign.metadata["value_expression"]
            .as_str()
            .is_some_and(|e| e.contains("STATUS_REFUSE")),
        "{:?}",
        assign.metadata
    );
    assert!(
        g.nodes.iter().any(|n| {
            n.kind == "write_site"
                && n.metadata["member_name"] == "status"
                && n.metadata["expression"]
                    .as_str()
                    .is_some_and(|e| e.contains("updateAll"))
                && n.metadata["value_constant"]
                    .as_str()
                    .is_some_and(|c| c.contains("STATUS_REFUSE"))
        }),
        "updateAll write_site value; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "write_site")
            .map(|n| n.metadata.clone())
            .collect::<Vec<_>>()
    );
}
