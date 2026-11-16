//! AR attribute access: rules/behaviors attrs, load(), attributes[], query builder, WHERE cols.
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

const AR: &str = r#"<?php
namespace yii\db;
class ActiveRecord {
  public static function tableName() { return ''; }
  public static function find() { return new ActiveQuery(static::class); }
  public static function findBySql($sql, $params = []) { return null; }
  public static function updateAll($attrs, $condition = '') { return 1; }
  public function load($data, $formName = null) { return true; }
  public function updateAttributes($attrs) { return 1; }
  public function save($runValidation = true, $attributeNames = null) { return true; }
  public function insert($runValidation = true, $attributes = null) { return true; }
  public function upsert($runValidation = true, $attributes = null) { return true; }
  public function delete() { return 1; }
  public function hasOne($class, $link) { return null; }
  public function hasMany($class, $link) { return null; }
  public $attributes;
}
/** @template T of ActiveRecord|array */
class ActiveQuery {
  public function where($c) { return $this; }
  public function andWhere($c) { return $this; }
  public function select($c) { return $this; }
  public function joinWith($with, $eagerLoading = true, $joinType = 'LEFT JOIN') { return $this; }
  public function innerJoinWith($with, $eagerLoading = true) { return $this; }
  public function with($with) { return $this; }
  public function one() { return null; }
}
class Connection {
  public function createCommand($sql, $params = []) { return null; }
}
"#;

#[test]
fn attributes_bag_and_load_write_virtual_attrs() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/ActiveRecord.php", AR),
        (
            "common/models/Wallet.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Wallet extends ActiveRecord {
  public static function tableName() { return '{{%wallet}}'; }
  public function rules() {
    return [[['amount', 'status'], 'required'], ['note', 'string']];
  }
  public function attributeLabels() {
    return ['currency' => 'Currency'];
  }
  public function fill($data) {
    $this->load($data);
    $this->attributes['amount'] = 10;
    $x = $this->attributes['status'];
    return $x;
  }
  public function fromPost() {
    $this->load(['Wallet' => ['note' => 'x']]);
  }
}
"#,
        ),
    ]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "property:common\\models\\Wallet::$amount"),
        "rules attr amount"
    );
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "property:common\\models\\Wallet::$currency"),
        "attributeLabels currency"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "writes"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$amount")
                && e.metadata["framework_evidence"] == "attributes_bag"
        }),
        "attributes['amount'] write; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "writes")
            .map(|e| (e.to.clone(), e.metadata["framework_evidence"].clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "reads"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$status")
                && e.metadata["framework_evidence"] == "attributes_bag"
        }),
        "attributes['status'] read"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "writes"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$note")
                && e.metadata["framework_evidence"] == "mass_assignment"
        }),
        "load literal nested note"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "writes"
                && e.metadata["framework_evidence"] == "load"
                && e.metadata["mass_load"] == true
        }),
        "load($data) fans out to known attrs"
    );
}

#[test]
fn query_builder_and_where_sql_columns() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/ActiveRecord.php", AR),
        (
            "schema.sql",
            "CREATE TABLE wallet (id int, amount numeric, status text);\n",
        ),
        (
            "common/models/Wallet.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Wallet extends ActiveRecord {
  public static function tableName() { return '{{%wallet}}'; }
  public function rules() { return [[['amount','status'], 'safe']]; }
  public function pick($id) {
    return static::find()->where(['status' => 'ok'])->andWhere(['=', 'amount', 1])->one();
  }
  public function raw($db, $id) {
    return $db->createCommand(
      'SELECT amount FROM wallet WHERE id = :id AND status = :s',
      [':id' => $id, ':s' => 'x']
    )->queryOne();
  }
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "reads"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$status")
                && e.metadata["framework_evidence"] == "query_builder"
        }),
        "where hash status; {:?}",
        g.edges
            .iter()
            .filter(|e| e.metadata["framework_evidence"] == "query_builder")
            .map(|e| (e.kind.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "reads"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$amount")
                && e.metadata["framework_evidence"] == "query_builder"
        }),
        "andWhere operator amount"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "sql_reads" && e.to.as_deref() == Some("column:public.wallet.id")
        }),
        "WHERE id column read; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "sql_reads")
            .map(|e| e.to.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "sql_reads" && e.to.as_deref() == Some("column:public.wallet.status")
        }),
        "WHERE status column read"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "maps_column"
                && e.from == "property:common\\models\\Wallet::$id"
                && e.to.as_deref() == Some("column:public.wallet.id")
        }),
        "DDL column id mapped to model"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "sql_reads"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$amount")
                && e.metadata["framework_evidence"] == "maps_column"
        }),
        "sql_site also links property via maps_column"
    );
}

#[test]
fn join_with_links_relation_property_and_getter() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/ActiveRecord.php", AR),
        (
            "common/models/Bonus.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Bonus extends ActiveRecord {
  public static function tableName() { return '{{%bonus}}'; }
}
"#,
        ),
        (
            "common/models/Wallet.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Wallet extends ActiveRecord {
  public static function tableName() { return '{{%wallet}}'; }
  public function getBonus() {
    return $this->hasOne(Bonus::class, ['id' => 'bonus_id']);
  }
  public function withBonus() {
    return static::find()->joinWith('bonus')->with(['bonus'])->one();
  }
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "reads"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$bonus")
                && e.metadata["framework_evidence"] == "join_with"
        }),
        "joinWith/with → relation property; {:?}",
        g.edges
            .iter()
            .filter(|e| e.metadata["framework_evidence"] == "join_with")
            .map(|e| (e.kind.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\models\\Wallet::getBonus")
                && e.metadata["framework_evidence"] == "join_with"
        }),
        "joinWith → getBonus call"
    );
}

#[test]
fn save_insert_delete_touch_known_attrs() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/ActiveRecord.php", AR),
        (
            "common/models/Wallet.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Wallet extends ActiveRecord {
  public static function tableName() { return '{{%wallet}}'; }
  public function rules() { return [[['amount', 'status'], 'safe']]; }
  public function persist() {
    $this->save();
    $this->insert();
    $this->delete();
  }
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "writes"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$amount")
                && e.metadata["framework_evidence"] == "save"
                && e.metadata["mass_load"] == true
        }),
        "save → known attr writes; {:?}",
        g.edges
            .iter()
            .filter(|e| e.metadata["framework_evidence"] == "save")
            .map(|e| e.to.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "writes"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$status")
                && e.metadata["framework_evidence"] == "insert"
        }),
        "insert → known attrs"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "reads"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$amount")
                && e.metadata["framework_evidence"] == "delete"
        }),
        "delete → known attr reads"
    );
}

#[test]
fn fluent_active_query_keeps_model_after_where() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/ActiveRecord.php", AR),
        (
            "common/models/Wallet.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Wallet extends ActiveRecord {
  public static function tableName() { return '{{%wallet}}'; }
  public function rules() { return [[['amount', 'status'], 'safe']]; }
}
"#,
        ),
        (
            "common/services/ReportService.php",
            r#"<?php
namespace common\services;
use common\models\Wallet;
class ReportService {
  public function statusOk() {
    $q = Wallet::find();
    return $q->where(['amount' => 1])->andWhere(['status' => 'ok'])->one();
  }
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "reads"
                && e.to.as_deref() == Some("property:common\\models\\Wallet::$status")
                && e.metadata["framework_evidence"] == "query_builder"
                && e.from.contains("ReportService")
        }),
        "fluent andWhere keeps Wallet attrs outside model class; {:?}",
        g.edges
            .iter()
            .filter(|e| e.metadata["framework_evidence"] == "query_builder")
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn sql_literal_concat_embeds() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/db/ActiveRecord.php", AR),
        (
            "schema.sql",
            "CREATE TABLE wallet (id int, amount numeric, status text);\n",
        ),
        (
            "common/models/Wallet.php",
            r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
use yii\db\Connection;
class Wallet extends ActiveRecord {
  public static function tableName() { return '{{%wallet}}'; }
  public function raw(Connection $db) {
    return $db->createCommand('SELECT amount FROM wallet WHERE id = ' . '1')->queryOne();
  }
}
"#,
        ),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "sql_reads" && e.to.as_deref() == Some("column:public.wallet.amount")
        }),
        "concat literals → sql_site; sql_reads={:?}",
        g.edges
            .iter()
            .filter(|e| e.kind.starts_with("sql_"))
            .map(|e| (e.kind.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
}
