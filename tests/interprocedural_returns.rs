//! interprocedural-return-types: body-inferred returns and relation getter query (PLUGIN-16/17).
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::{Edge, GraphSnapshot},
};

const YII: &str = r#"<?php
namespace yii\db;
class BaseActiveRecord extends \yii\base\Model {
  public function hasOne($class, $link) { return $this->createRelationQuery($class, $link, false); }
  public function hasMany($class, $link) { return $this->createRelationQuery($class, $link, true); }
}
class ActiveRecord extends BaseActiveRecord {
  /** @return static|null */
  public static function findOne($condition) { return null; }
}
class Query {
  /** @return $this */
  public function where($condition) { return $this; }
  /** @return $this */
  public function andWhere($condition) { return $this; }
}
/** @template T of ActiveRecord|array */
class ActiveQuery extends Query {
  /** @return T|null */
  public function one($db = null) { return null; }
}
"#;
const YII_BASE: &str = "<?php\nnamespace yii\\base;\nclass Model {}\n";

const APP: &str = r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
use yii\db\ActiveQuery;
class Wallet extends ActiveRecord {
  public function deposit($amount) {}
  public static function findOrCreate(int $userId, string $currency)
  {
    $wallet = Wallet::findOne(['user_id' => $userId, 'currency' => $currency]);
    if ($wallet) return $wallet;
    $wallet = new Wallet(['user_id' => $userId, 'currency' => $currency]);
    return $wallet;
  }
}
class User extends ActiveRecord {
  /**
   * @return \yii\db\ActiveQuery
   */
  public function getWallets()
  {
    return $this->hasMany(Wallet::class, ['user_id' => 'id']);
  }
  /**
   * @return object Wallet
   */
  public function getWallet($currency)
  {
    return $this->getWallets()->andWhere(['currency' => $currency])->one();
  }
}
class Process {
  public function fromFactory() {
    $w = Wallet::findOrCreate(1, 'CP');
    $w->deposit(1);
  }
  public function fromGetter(User $user) {
    $w = $user->getWallet('CP');
    $w->deposit(1);
  }
  public function fromRelation(User $user) {
    $w = $user->getWallets()->andWhere([])->one();
    $w->deposit(1);
  }
}
"#;

fn analyze() -> GraphSnapshot {
    let files: Vec<_> = [
        ("vendor/yiisoft/yii2/base/Model.php", YII_BASE),
        ("vendor/yiisoft/yii2/db/All.php", YII),
        ("common/models/All.php", APP),
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
fn find_or_create_without_return_doc_types_callers() {
    let g = analyze();
    let factory = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "deposit")
        .filter(|e| e.from.contains("fromFactory"))
        .collect::<Vec<_>>();
    assert_eq!(factory.len(), 1, "{factory:#?}");
    assert_eq!(
        factory[0].to.as_deref(),
        Some("method:common\\models\\Wallet::deposit"),
        "{:#?}",
        factory[0]
    );
}

#[test]
fn get_wallet_with_object_doc_types_callers_via_relation_query() {
    let g = analyze();
    let getter = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "deposit")
        .filter(|e| e.from.contains("fromGetter"))
        .collect::<Vec<_>>();
    assert_eq!(getter.len(), 1, "{getter:#?}");
    assert_eq!(
        getter[0].to.as_deref(),
        Some("method:common\\models\\Wallet::deposit"),
        "{:#?}",
        getter[0]
    );
}

#[test]
fn relation_getter_returns_active_query_of_related_model() {
    let g = analyze();
    let deposit = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "deposit")
        .filter(|e| e.from.contains("fromRelation"))
        .collect::<Vec<_>>();
    assert_eq!(deposit.len(), 1, "{deposit:#?}");
    assert_eq!(
        deposit[0].to.as_deref(),
        Some("method:common\\models\\Wallet::deposit"),
        "{:#?}",
        deposit[0]
    );
    let get_wallets = call(&g, "getWallets", "$user");
    assert_ne!(get_wallets.metadata["resolution"], "unresolved");
}
