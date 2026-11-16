//! ActiveRecord tableName() → maps_table → table, so SQL sites match models.
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
        .analyze(
            &files,
            &["php".into(), "pgsql".into()],
            &["yii2".into()],
            &options,
        )
        .unwrap()
}

#[test]
fn table_name_maps_model_and_sql_columns_link() {
    let g = analyze(&[
        (
            "vendor/yiisoft/yii2/db/ActiveRecord.php",
            r#"<?php
namespace yii\db;
class ActiveRecord {
  public static function tableName() { return ''; }
  public static function findBySql($sql, $params = []) { return null; }
}
"#,
        ),
        (
            "common/models/Wallet.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Wallet extends ActiveRecord {
  public static function tableName() {
    return '{{%wallet}}';
  }
  public function debit($amount, $id) {
    return static::getDb()->createCommand(
      'UPDATE wallet SET amount = amount - :amount WHERE id = :id',
      [':amount' => $amount, ':id' => $id]
    )->execute();
  }
  public function lock($id) {
    return static::findBySql(
      'SELECT balance FROM wallet WHERE id = :id FOR UPDATE',
      [':id' => $id]
    )->one();
  }
}
"#,
        ),
        (
            "common/models/UserBonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class UserBonus extends ActiveRecord {
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "maps_table"
                && e.from == "class:common\\models\\Wallet"
                && e.to.as_deref() == Some("table:public.wallet")
        }),
        "Wallet::tableName maps to table; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "maps_table")
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "maps_table"
                && e.from == "class:common\\models\\UserBonus"
                && e.to.as_deref() == Some("table:public.user_bonus")
        }),
        "default camel2id tableName for UserBonus; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "maps_table")
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "sql_writes" && e.to.as_deref() == Some("column:public.wallet.amount")
        }),
        "UPDATE SET amount → column write"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "sql_reads" && e.to.as_deref() == Some("column:public.wallet.balance")
        }),
        "SELECT balance → column read"
    );
}

#[test]
fn rules_and_behaviors_declare_undeclared_columns() {
    let g = analyze(&[
        (
            "vendor/yiisoft/yii2/db/ActiveRecord.php",
            r#"<?php
namespace yii\db;
class ActiveRecord {
  public static function tableName() { return ''; }
}
"#,
        ),
        (
            "vendor/yiisoft/yii2/behaviors/TimestampBehavior.php",
            r#"<?php
namespace yii\behaviors;
class TimestampBehavior {}
"#,
        ),
        (
            "vendor/yiisoft/yii2/behaviors/AttributeBehavior.php",
            r#"<?php
namespace yii\behaviors;
class AttributeBehavior {}
"#,
        ),
        (
            "common/models/Bonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
use yii\behaviors\TimestampBehavior;
use yii\behaviors\AttributeBehavior;
class Bonus extends ActiveRecord {
  public static function tableName() { return '{{%bonus}}'; }
  public function behaviors() {
    return [
      TimestampBehavior::class,
      [
        'class' => AttributeBehavior::class,
        'attributes' => [
          ActiveRecord::EVENT_BEFORE_INSERT => 'status',
        ],
        'value' => 1,
      ],
    ];
  }
  public function rules() {
    return [
      [['ladder', 'amount'], 'required'],
      ['name', 'string'],
      ['amount', 'number'],
    ];
  }
}
"#,
        ),
    ]);
    for attr in [
        "ladder",
        "amount",
        "name",
        "created_at",
        "updated_at",
        "status",
    ] {
        let prop = format!("property:common\\models\\Bonus::${attr}");
        let col = format!("column:public.bonus.{attr}");
        assert!(
            g.nodes.iter().any(|n| n.id == prop),
            "missing virtual property {prop}; props={:?}",
            g.nodes
                .iter()
                .filter(|n| n.kind == "property" && n.id.contains("Bonus"))
                .map(|n| &n.id)
                .collect::<Vec<_>>()
        );
        assert!(
            g.edges.iter().any(|e| {
                e.kind == "maps_column" && e.from == prop && e.to.as_deref() == Some(col.as_str())
            }),
            "missing maps_column {prop} → {col}; {:?}",
            g.edges
                .iter()
                .filter(|e| e.kind == "maps_column")
                .map(|e| (
                    e.from.clone(),
                    e.to.clone(),
                    e.metadata["framework_evidence"].clone()
                ))
                .collect::<Vec<_>>()
        );
    }
    assert!(g
        .nodes
        .iter()
        .find(|n| n.id == "property:common\\models\\Bonus::$ladder")
        .is_some_and(|n| n.metadata["framework_evidence"] == "rules"));
    assert!(g
        .nodes
        .iter()
        .find(|n| n.id == "property:common\\models\\Bonus::$created_at")
        .is_some_and(|n| n.metadata["framework_evidence"] == "behaviors"));
}
