//! active-query-and-builtins: Yii query class of a model (PLUGIN-14).
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::{Edge, GraphSnapshot},
};

const YII: &str = r#"<?php
namespace yii\db;
class BaseActiveRecord {}
class ActiveRecord extends BaseActiveRecord {
  /** @return ActiveQuery<static> */
  public static function find() { return \Yii::createObject(ActiveQuery::class, [get_called_class()]); }
}
class Query {
  /** @return $this */
  public function where($condition) { return $this; }
}
/** @template T of ActiveRecord|array */
class ActiveQuery extends Query {
  /** @return T|null */
  public function one($db = null) { return null; }
}
"#;
const USUARIO: &str = r#"<?php
namespace Da\User\Model;
use Da\User\Query\UserQuery;
class User extends \yii\db\ActiveRecord {
  /** @return UserQuery */
  public static function find() { return new UserQuery(static::class); }
}
"#;
const USUARIO_QUERY: &str =
    "<?php\nnamespace Da\\User\\Query;\nclass UserQuery extends \\yii\\db\\ActiveQuery {}\n";
const APP: &str = r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
use yii\db\ActiveQuery;
class Wallet { public function deposit($amount) {} }
class Game extends ActiveRecord { public function play() {} }
class BonusCod extends ActiveRecord {
  /** @return BonusCodQuery */
  public static function find() { return new BonusCodQuery(get_called_class()); }
  public function apply() {}
}
class BonusCodQuery extends ActiveQuery {
  /** @return BonusCod|null */
  public function one($db = null) { return parent::one($db); }
}
class User extends \Da\User\Model\User {
  public function getWallet(): Wallet { return new Wallet(); }
}
class Process {
  public function run() {
    $g = Game::find()->where([])->one();
    $g->play();
    $b = BonusCod::find()->where([])->one();
    $b->apply();
    $u = User::find()->where(['uid' => 1])->one();
    $wallet = $u->getWallet();
    $wallet->deposit(1);
  }
}
"#;

fn analyze() -> GraphSnapshot {
    let files: Vec<_> = [
        ("vendor/yiisoft/yii2/db/All.php", YII),
        (
            "vendor/2amigos/yii2-usuario/src/User/Model/User.php",
            USUARIO,
        ),
        (
            "vendor/2amigos/yii2-usuario/src/User/Query/UserQuery.php",
            USUARIO_QUERY,
        ),
        ("common/models/All.php", APP),
    ]
    .iter()
    .map(|(path, source)| SourceFile {
        path: (*path).into(),
        source: (*source).into(),
    })
    .collect();
    let options = AnalysisOptions {
        declaration_paths: vec!["vendor/yiisoft".into(), "vendor/2amigos".into()],
        ..AnalysisOptions::default()
    };
    code_atlas::plugins::registry()
        .analyze(&files, &["php".into()], &["yii2".into()], &options)
        .unwrap()
}
fn call<'a>(g: &'a GraphSnapshot, member: &str, receiver_starts: &str) -> &'a Edge {
    let found: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == member)
        .filter(|e| {
            e.metadata["receiver_expression"]
                .as_str()
                .is_some_and(|r| r.starts_with(receiver_starts))
        })
        .collect();
    assert_eq!(found.len(), 1, "{member} on {receiver_starts}: {found:#?}");
    found[0]
}

#[test]
fn default_query_class_is_exact_so_one_has_no_candidates() {
    let g = analyze();
    let one = call(&g, "one", "Game::find()");
    assert_eq!(one.to.as_deref(), Some("method:yii\\db\\ActiveQuery::one"));
    assert_ne!(one.metadata["resolution"], "candidate", "{one:#?}");
    assert_eq!(one.metadata["candidates"], serde_json::json!([]));
    let play = call(&g, "play", "$g");
    assert_eq!(
        play.to.as_deref(),
        Some("method:common\\models\\Game::play")
    );
    // The target of find() stays the real method.
    let find = call(&g, "find", "Game");
    assert_eq!(
        find.to.as_deref(),
        Some("method:yii\\db\\ActiveRecord::find")
    );
    assert!(!g
        .nodes
        .iter()
        .any(|n| n.id == "method:common\\models\\Game::find"));
}

#[test]
fn model_specific_query_class_is_used() {
    let g = analyze();
    let one = call(&g, "one", "BonusCod::find()");
    assert_eq!(
        one.to.as_deref(),
        Some("method:common\\models\\BonusCodQuery::one")
    );
    assert_eq!(
        call(&g, "apply", "$b").to.as_deref(),
        Some("method:common\\models\\BonusCod::apply")
    );
}

#[test]
fn library_query_without_model_parameter_is_bound_to_the_called_model() {
    let g = analyze();
    assert_eq!(
        call(&g, "getWallet", "$u").to.as_deref(),
        Some("method:common\\models\\User::getWallet")
    );
    assert_eq!(
        call(&g, "deposit", "$wallet").to.as_deref(),
        Some("method:common\\models\\Wallet::deposit")
    );
}

#[test]
fn find_still_binds_when_library_parent_is_outside_declaration_paths() {
    // Default declaration_paths cover only yiisoft; Da\User is missing — common\models\User
    // still needs User::find()->one() typed like ActiveRecord (slotor BaseUser gap).
    let files: Vec<_> = [
        ("vendor/yiisoft/yii2/db/All.php", YII),
        ("common/models/User.php", APP_USER_ONLY),
    ]
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
    let g = code_atlas::plugins::registry()
        .analyze(&files, &["php".into()], &["yii2".into()], &options)
        .unwrap();
    assert_eq!(
        call(&g, "getWallet", "$u").to.as_deref(),
        Some("method:common\\models\\User::getWallet"),
        "User::find()->one() must type as User even when Da\\User\\Model\\User is unindexed"
    );
}

const APP_USER_ONLY: &str = r#"<?php
namespace common\models;
class Wallet { public function deposit($amount) {} }
class User extends \Da\User\Model\User {
  public function getWallet(): Wallet { return new Wallet(); }
}
class Process {
  public function run() {
    $u = User::find()->where(['uid' => 1])->one();
    $wallet = $u->getWallet();
    $wallet->deposit(1);
  }
}
"#;
