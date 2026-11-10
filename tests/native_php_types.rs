//! php-type-inference: PHPDoc-derived types are used but marked as annotations
//! (PLUGIN-08, PLUGIN-09) and vendor declarations are indexed without flow (INDEX-08).
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::{Edge, GraphSnapshot, Node},
    plugins::php::PhpLanguagePlugin,
};

fn analyze_files(files: &[(&str, &str)], declaration_paths: &[&str]) -> GraphSnapshot {
    let files: Vec<_> = files
        .iter()
        .map(|(path, source)| SourceFile {
            path: (*path).into(),
            source: (*source).into(),
        })
        .collect();
    let options = AnalysisOptions {
        declaration_paths: declaration_paths.iter().map(|p| p.to_string()).collect(),
        ..AnalysisOptions::default()
    };
    PhpLanguagePlugin.analyze(&files, &options).unwrap()
}
fn node<'a>(g: &'a GraphSnapshot, id: &str) -> &'a Node {
    g.nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("missing {id}; diagnostics {:?}", g.diagnostics))
}
/// The single effect edge of `kind` from a site owned by `method` whose member is `member`.
fn effect<'a>(g: &'a GraphSnapshot, method: &str, kind: &str, member: &str) -> &'a Edge {
    let sites: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "contains" && e.from == method)
        .filter_map(|e| e.to.as_deref())
        .collect();
    let found: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == kind && sites.contains(&e.from.as_str()))
        .filter(|e| e.metadata["member_name"] == member)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "{kind} {member} in {method}: {found:#?}\ndiagnostics {:?}",
        g.diagnostics
    );
    found[0]
}
fn assert_annotated(edge: &Edge, target: &str) {
    assert_eq!(edge.to.as_deref(), Some(target), "{edge:#?}");
    assert_eq!(edge.metadata["resolution"], "annotated", "{edge:#?}");
    assert_eq!(edge.metadata["type_source"], "annotation", "{edge:#?}");
}
fn assert_code(edge: &Edge, target: &str) {
    assert_eq!(edge.to.as_deref(), Some(target), "{edge:#?}");
    assert_eq!(edge.metadata["resolution"], "resolved", "{edge:#?}");
    assert_eq!(edge.metadata["type_source"], "code", "{edge:#?}");
}

const MODELS: &str = r#"<?php
namespace common\models;
class UserBonus { function updateWagerByTime() {} }
class MaxWinUserBonus { function getAllowedAmountToDep($amount) { return $amount; } }
/**
 * @property MaxWinUserBonus $maxWinUserBonus
 * @property-read UserBonus $userBonus
 */
class Freespin {
  /** @var UserBonus */
  public $bonus;
}
class A { function run() {} }
class B { function run() {} function onlyB() {} }
"#;

#[test]
fn inline_var_annotation_resolves_call_and_is_marked() {
    let g = analyze_files(
        &[
            ("models.php", MODELS),
            (
                "process.php",
                r#"<?php
namespace common\models;
class Process {
  function run($freespin) {
    /** @var UserBonus $userBonus */
    $userBonus = $freespin->userBonus;
    $userBonus->updateWagerByTime();
  }
}
"#,
            ),
        ],
        &[],
    );
    let call = effect(
        &g,
        "method:common\\models\\Process::run",
        "calls",
        "updateWagerByTime",
    );
    assert_annotated(call, "method:common\\models\\UserBonus::updateWagerByTime");
    let variable = node(
        &g,
        "variable:method:common\\models\\Process::run:$userBonus",
    );
    assert_eq!(variable.metadata["type_source"], "annotation");
}

#[test]
fn class_property_tags_become_magic_property_nodes_and_resolve_chains() {
    let g = analyze_files(
        &[
            ("models.php", MODELS),
            (
                "process.php",
                r#"<?php
namespace common\models;
class Process {
  function run(Freespin $freespin) {
    $amount = $freespin->maxWinUserBonus->getAllowedAmountToDep(1);
    $freespin->bonus->updateWagerByTime();
  }
}
"#,
            ),
        ],
        &[],
    );
    let property = node(&g, "property:common\\models\\Freespin::$maxWinUserBonus");
    assert_eq!(property.metadata["magic"], true);
    assert_eq!(property.metadata["type_source"], "annotation");
    assert_eq!(property.metadata["type"], "common\\models\\MaxWinUserBonus");
    assert_eq!(property.metadata["access"], "read-write");
    let read_only = node(&g, "property:common\\models\\Freespin::$userBonus");
    assert_eq!(read_only.metadata["access"], "read");

    let method = "method:common\\models\\Process::run";
    let read = effect(&g, method, "reads", "maxWinUserBonus");
    assert_eq!(read.to.as_deref(), Some(property.id.as_str()));
    // The receiver $freespin is typed by code, but the property exists only in PHPDoc.
    assert_eq!(read.metadata["resolution"], "annotated");
    assert_annotated(
        effect(&g, method, "calls", "getAllowedAmountToDep"),
        "method:common\\models\\MaxWinUserBonus::getAllowedAmountToDep",
    );
    // A declared property typed only by @var is an annotation too.
    assert_annotated(
        effect(&g, method, "calls", "updateWagerByTime"),
        "method:common\\models\\UserBonus::updateWagerByTime",
    );
    let declared = node(&g, "property:common\\models\\Freespin::$bonus");
    assert_eq!(declared.metadata["type_source"], "annotation");
    assert!(declared.metadata.get("magic").is_none() || declared.metadata["magic"] == false);
}

#[test]
fn code_types_win_over_contradicting_annotations() {
    let g = analyze_files(
        &[
            ("models.php", MODELS),
            (
                "process.php",
                r#"<?php
namespace common\models;
class Process {
  /** @param B $typed */
  function run(A $typed, ?A $nullable) {
    /** @var B $x */
    $x = new A();
    $x->run();
    $typed->run();
    $nullable->run();
  }
}
"#,
            ),
        ],
        &[],
    );
    let method = "method:common\\models\\Process::run";
    let sites: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "run")
        .filter(|e| e.from.contains("Process::run"))
        .collect();
    assert_eq!(sites.len(), 3, "{sites:#?}");
    for edge in sites {
        assert_code(edge, "method:common\\models\\A::run");
    }
    let _ = method;
}

#[test]
fn param_annotation_fills_untyped_and_array_parameters_and_foreach_elements() {
    let g = analyze_files(
        &[
            ("models.php", MODELS),
            (
                "process.php",
                r#"<?php
namespace common\models;
class Process {
  /**
   * @param B $loose
   * @param A[] $items
   * @return UserBonus|null
   */
  function run($loose, array $items) {
    $loose->onlyB();
    foreach ($items as $key => $item) { $item->run(); }
    return null;
  }
  function caller() {
    $this->run(null, [])->updateWagerByTime();
  }
}
"#,
            ),
        ],
        &[],
    );
    let method = "method:common\\models\\Process::run";
    assert_annotated(
        effect(&g, method, "calls", "onlyB"),
        "method:common\\models\\B::onlyB",
    );
    assert_annotated(
        effect(&g, method, "calls", "run"),
        "method:common\\models\\A::run",
    );
    assert_annotated(
        effect(
            &g,
            "method:common\\models\\Process::caller",
            "calls",
            "updateWagerByTime",
        ),
        "method:common\\models\\UserBonus::updateWagerByTime",
    );
    let declaration = node(&g, method);
    assert_eq!(
        declaration.metadata["annotated_return_type"],
        "common\\models\\UserBonus|null"
    );
    let parameters = declaration.metadata["parameters"].as_array().unwrap();
    assert_eq!(parameters[0]["type"], "common\\models\\B");
    assert_eq!(parameters[0]["type_source"], "annotation");
    assert_eq!(parameters[1]["type"], "common\\models\\A[]");
}

const YII: &str = r#"<?php
namespace yii\db;
class Query {
  /** @return $this the query object itself */
  public function where($condition) { $this->where = $condition; return $this; }
}
/**
 * @template T of ActiveRecord|array = ActiveRecord
 */
class ActiveQuery extends Query {
  /** @return T|null a single row */
  public function one($db = null) { return parent::one($db); }
  /** @return T[] the query results */
  public function all($db = null) { return parent::all($db); }
}
class ActiveRecord {
  /** @return ActiveQuery<static> the newly created instance */
  public static function find() { return \Yii::createObject(ActiveQuery::class, [get_called_class()]); }
}
"#;

#[test]
fn active_query_chain_substitutes_static_and_templates() {
    let g = analyze_files(
        &[
            ("vendor/yiisoft/yii2/db/ActiveRecord.php", YII),
            (
                "common/models/Wallet.php",
                r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class Wallet extends ActiveRecord { function deposit($amount) {} }
class Service {
  function pay() {
    $wallet = Wallet::find()->where(['user_id' => 1])->one();
    $wallet->deposit(1);
    foreach (Wallet::find()->all() as $each) { $each->deposit(2); }
  }
}
"#,
            ),
        ],
        &["vendor/yiisoft"],
    );
    let method = "method:common\\models\\Service::pay";
    let deposits: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "deposit")
        .collect();
    assert_eq!(deposits.len(), 2, "{deposits:#?}");
    for edge in deposits {
        assert_annotated(edge, "method:common\\models\\Wallet::deposit");
    }
    // Static call on a code-known class: the target needs no annotation;
    // only what follows from find()'s PHPDoc return type is annotated.
    let finds: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "find")
        .collect();
    assert_eq!(finds.len(), 2);
    for edge in finds {
        assert_code(edge, "method:yii\\db\\ActiveRecord::find");
    }
    let _ = method;
}

#[test]
fn declaration_paths_index_declarations_without_flow() {
    let g = analyze_files(
        &[
            ("vendor/yiisoft/yii2/db/ActiveRecord.php", YII),
            (
                "common/models/Wallet.php",
                "<?php\nnamespace common\\models;\nclass Wallet extends \\yii\\db\\ActiveRecord {}\n",
            ),
        ],
        &["vendor/yiisoft"],
    );
    node(&g, "class:yii\\db\\ActiveRecord");
    node(&g, "method:yii\\db\\ActiveQuery::one");
    let inherits = g
        .edges
        .iter()
        .find(|e| e.kind == "inherits" && e.from == "class:common\\models\\Wallet")
        .unwrap();
    assert_eq!(inherits.to.as_deref(), Some("class:yii\\db\\ActiveRecord"));
    assert!(!g.diagnostics.iter().any(|d| d
        .message
        .contains("Definition not indexed: yii\\db\\ActiveRecord")));
    let flow_kinds = [
        "call_site",
        "read_site",
        "write_site",
        "return_site",
        "variable",
        "branch",
    ];
    let vendor_flow: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| n.span.path.starts_with("vendor/") && flow_kinds.contains(&n.kind.as_str()))
        .map(|n| n.id.as_str())
        .collect();
    assert!(vendor_flow.is_empty(), "{vendor_flow:?}");
    assert!(
        !g.diagnostics.iter().any(|d| d.path.starts_with("vendor/")),
        "{:?}",
        g.diagnostics
    );
}

#[test]
fn library_methods_are_not_materialized_as_available_via_edges() {
    let g = analyze_files(
        &[
            ("vendor/yiisoft/yii2/db/ActiveRecord.php", YII),
            (
                "common/models/Wallet.php",
                "<?php\nnamespace common\\models;\nclass Base { function own() {} }\nclass Wallet extends \\yii\\db\\ActiveRecord {}\nclass Child extends Base {}\n",
            ),
        ],
        &["vendor/yiisoft"],
    );
    let available: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "available_via")
        .map(|e| (e.from.as_str(), e.to.as_deref().unwrap_or("")))
        .collect();
    assert!(available.contains(&(
        "class:common\\models\\Child",
        "method:common\\models\\Base::own"
    )));
    assert!(
        !available
            .iter()
            .any(|(_, to)| to.starts_with("method:yii\\")),
        "{available:?}"
    );
}

#[test]
fn unresolved_call_names_the_unindexed_ancestor_without_guessing_a_cause() {
    let g = analyze_files(
        &[(
            "common/models/User.php",
            r#"<?php
namespace common\models;
use Da\User\Model\User as BaseUser;
class User extends BaseUser {}
class Service {
  function run(User $user) { $user->getWallet(); }
}
"#,
        )],
        &[],
    );
    let call = effect(
        &g,
        "method:common\\models\\Service::run",
        "calls",
        "getWallet",
    );
    assert_eq!(call.metadata["resolution"], "unresolved");
    assert_eq!(call.metadata["unindexed_ancestor"], "Da\\User\\Model\\User");
    let diagnostic = g
        .diagnostics
        .iter()
        .find(|d| d.code == "unresolved_call" && d.message.contains("getWallet"))
        .unwrap();
    assert!(
        diagnostic.message.contains("Da\\User\\Model\\User"),
        "{diagnostic:?}"
    );
}

#[test]
fn class_method_tags_become_annotated_magic_methods_and_real_methods_win() {
    let g = analyze_files(
        &[(
            "app/Redis.php",
            r#"<?php
namespace app;
class Foo { public function bar() {} }
/**
 * @method mixed exists($key, ...$keys) Determine if a key exists. <https://redis.io/commands/exists>
 * @method static Foo make()
 * @method int real()
 */
class Conn {
  public function real(): int { return 1; }
  public function __call($name, $params) {}
}
class User {
  public function run(Conn $conn) {
    $conn->exists('k');
    Conn::make()->bar();
    $conn->real();
  }
}
"#,
        )],
        &[],
    );
    let magic = node(&g, "method:app\\Conn::exists");
    assert_eq!(magic.metadata["magic"], true);
    assert_eq!(magic.metadata["type_source"], "annotation");
    assert_eq!(magic.metadata["annotated_return_type"], "mixed");
    let method = "method:app\\User::run";
    assert_annotated(
        effect(&g, method, "calls", "exists"),
        "method:app\\Conn::exists",
    );
    assert_annotated(effect(&g, method, "calls", "bar"), "method:app\\Foo::bar");
    assert_code(
        effect(&g, method, "calls", "real"),
        "method:app\\Conn::real",
    );
    let real = node(&g, "method:app\\Conn::real");
    assert!(real.metadata.get("magic").is_none() || real.metadata["magic"] == false);
}

#[test]
fn closures_inherit_captured_variable_types_from_the_enclosing_scope() {
    let g = analyze_files(
        &[(
            "app/Pay.php",
            r#"<?php
namespace app;
class Wallet { public function deposit($amount) { return true; } }
class Controller {
  public function run(Wallet $wallet, $plain) {
    $credit = function (int $x) use ($wallet, &$plain): void {
      if ($x > 0 && $wallet->deposit($x)) { $plain = 1; }
    };
    $arrow = fn() => $wallet->deposit(2);
    $nested = function () use ($wallet) {
      return function () use ($wallet) { $wallet->deposit(3); };
    };
  }
}
"#,
        )],
        &[],
    );
    let deposits: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "deposit")
        .collect();
    assert_eq!(deposits.len(), 3, "{deposits:#?}");
    for edge in deposits {
        assert_code(edge, "method:app\\Wallet::deposit");
    }
}

#[test]
fn class_constants_are_declarations_and_their_reads_are_not_property_access() {
    let g = analyze_files(
        &[(
            "app/Status.php",
            r#"<?php
namespace app;
interface HasKey { const KEY = 'jackpot:white'; }
class UserBonus {
  const STATUS_END = 3;
  public const STATUS_ACTIVE = 1, STATUS_NEW = 0;
  final public const int STATUS_REFUSE = 4;
}
class Sub extends UserBonus {}
class Jackpot implements HasKey {}
enum Kind { case Tax; case Bonus; }
class Process {
  public function run() {
    $a = UserBonus::STATUS_END;
    $b = \app\Sub::STATUS_NEW;
    $c = Jackpot::KEY;
    $d = UserBonus::MISSING;
    $e = UserBonus::class;
    $f = Kind::Tax;
    $g = static::class;
  }
}
"#,
        )],
        &[],
    );
    let constant = node(&g, "constant:app\\UserBonus::STATUS_END");
    assert_eq!(constant.kind, "constant");
    assert_eq!(constant.metadata["value"], "3");
    node(&g, "constant:app\\UserBonus::STATUS_REFUSE");
    node(&g, "constant:app\\Kind::Tax");
    assert!(g.edges.iter().any(|e| e.kind == "contains"
        && e.from == "class:app\\UserBonus"
        && e.to.as_deref() == Some("constant:app\\UserBonus::STATUS_ACTIVE")));
    let method = "method:app\\Process::run";
    for (member, target) in [
        ("STATUS_END", "constant:app\\UserBonus::STATUS_END"),
        ("STATUS_NEW", "constant:app\\UserBonus::STATUS_NEW"),
        ("KEY", "constant:app\\HasKey::KEY"),
        ("Tax", "constant:app\\Kind::Tax"),
    ] {
        assert_code(effect(&g, method, "reads", member), target);
    }
    assert_eq!(
        effect(&g, method, "constant_access", "MISSING").metadata["resolution"],
        "unresolved"
    );
    let noise: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "property_access" || e.metadata["member_name"] == "class")
        .collect();
    assert!(noise.is_empty(), "{noise:#?}");
}

#[test]
fn builtin_php_methods_are_external_with_their_return_types() {
    let g = analyze_files(
        &[
            (
                "vendor/yiisoft/yii2/base/Exception.php",
                "<?php\nnamespace yii\\base;\nclass Exception extends \\Exception {}\n",
            ),
            (
                "app/E.php",
                r#"<?php
namespace app;
use yii\base\Exception as YiiException;
class Logger { public function info(string $text) {} }
class E {
  public function run(Logger $log) {
    try {
      $log->info('x');
    } catch (YiiException $ex) {
      $message = $ex->getMessage();
      $log->info($message);
      $ex->noSuchMethod();
    } catch (\Throwable $t) {
      $t->getTraceAsString();
    }
  }
}
"#,
            ),
        ],
        &["vendor/yiisoft"],
    );
    let method = "method:app\\E::run";
    let message = effect(&g, method, "calls", "getMessage");
    assert_eq!(message.to, None);
    assert_eq!(message.metadata["resolution"], "external");
    assert_eq!(message.metadata["builtin"], "Exception::getMessage");
    let trace = effect(&g, method, "calls", "getTraceAsString");
    assert_eq!(trace.metadata["builtin"], "Throwable::getTraceAsString");
    let variable = node(&g, "variable:method:app\\E::run:$message");
    assert_eq!(variable.metadata["type"], "string");
    assert_eq!(
        effect(&g, method, "calls", "noSuchMethod").metadata["resolution"],
        "unresolved"
    );
    assert!(!g
        .diagnostics
        .iter()
        .any(|d| d.code == "unresolved_call" && d.message.contains("getMessage")));
}
