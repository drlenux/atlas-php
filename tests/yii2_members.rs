//! yii2-framework-members: framework members (AR relations, app components) feed PHP
//! type inference before bodies are analyzed (PLUGIN-10).
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::{Edge, GraphSnapshot, Node},
};

const YII_BASE: &str = r#"<?php
namespace yii\base;
class BaseObject {}
class Component extends BaseObject {}
class Model extends Component {}
class Module extends Component {}
class Application extends Module {}
"#;
const YII_WEB: &str = r#"<?php
namespace yii\web;
class Application extends \yii\base\Application {}
"#;
const YII_DB: &str = r#"<?php
namespace yii\db;
class BaseActiveRecord extends \yii\base\Model {
  public function hasOne($class, $link) { return $this->createRelationQuery($class, $link, false); }
  public function hasMany($class, $link) { return $this->createRelationQuery($class, $link, true); }
}
class ActiveRecord extends BaseActiveRecord {}
"#;
const YII_ROOT: &str = r#"<?php
namespace yii;
class BaseYii {
  /** @var \yii\web\Application the application instance */
  public static $app;
}
"#;
const YII_GLOBAL: &str = "<?php\nclass Yii extends \\yii\\BaseYii {}\n";
const REDIS: &str = r#"<?php
namespace yii\redis;
class Connection { public function get($key) {} }
"#;

const MODELS: &str = r#"<?php
namespace common\models;
use yii\db\ActiveRecord;
class MaxWin extends ActiveRecord { public function getAllowed() {} }
class UserBonus extends ActiveRecord { public function update() {} }
/**
 * @property Wrong $maxWin annotation contradicted by the relation
 */
class Freespin extends ActiveRecord {
  public $declared;
  public function getMaxWin() {
    return $this->hasOne(MaxWin::class, ['freespin_id' => 'id']);
  }
  public function getBonuses() {
    return $this->hasMany('common\models\UserBonus', ['freespin_id' => 'id'])->inverseOf('freespin');
  }
  public function getDeclared() { return $this->hasOne(MaxWin::class, []); }
  public function getConditional() {
    if ($this->declared) { return $this->hasOne(MaxWin::class, []); }
    return $this->hasOne(UserBonus::class, []);
  }
  public function getDynamic() { $class = MaxWin::class; return $this->hasOne($class, []); }
}
class Process {
  public function run(Freespin $freespin) {
    $m = $freespin->maxWin;
    $m->getAllowed();
    foreach ($freespin->bonuses as $bonus) { $bonus->update(); }
  }
  public function cache() {
    $redis = \Yii::$app->redis;
    $redis->get('key');
  }
}
"#;

fn vendor() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vendor/yiisoft/yii2/base/All.php", YII_BASE),
        ("vendor/yiisoft/yii2/web/Application.php", YII_WEB),
        ("vendor/yiisoft/yii2/db/ActiveRecord.php", YII_DB),
        ("vendor/yiisoft/yii2/BaseYii.php", YII_ROOT),
        ("vendor/yiisoft/yii2/Yii.php", YII_GLOBAL),
        ("vendor/yiisoft/yii2-redis/src/Connection.php", REDIS),
    ]
}

fn analyze(extra: &[(&str, &str)]) -> GraphSnapshot {
    let files: Vec<_> = vendor()
        .into_iter()
        .chain(extra.iter().copied())
        .map(|(path, source)| SourceFile {
            path: path.into(),
            source: source.into(),
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
fn node<'a>(g: &'a GraphSnapshot, id: &str) -> &'a Node {
    g.nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("missing {id}; diagnostics {:?}", g.diagnostics))
}
fn calls<'a>(g: &'a GraphSnapshot, member: &str) -> Vec<&'a Edge> {
    g.edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == member)
        .collect()
}
fn single_call<'a>(g: &'a GraphSnapshot, member: &str) -> &'a Edge {
    let found = calls(g, member);
    assert_eq!(found.len(), 1, "{member}: {found:#?}\n{:?}", g.diagnostics);
    found[0]
}

const CONFIG: &str =
    "<?php return ['components' => ['redis' => ['class' => 'yii\\redis\\Connection']]];";

#[test]
fn has_one_relation_types_a_variable_and_links_the_getter() {
    let g = analyze(&[
        ("common/models/All.php", MODELS),
        ("config/main.php", CONFIG),
    ]);
    let property = node(&g, "property:common\\models\\Freespin::$maxWin");
    assert_eq!(property.metadata["type"], "common\\models\\MaxWin|null");
    assert_eq!(property.metadata["type_source"], "framework");
    assert_eq!(property.metadata["framework"], "yii2");
    assert_eq!(property.metadata["framework_evidence"], "relation");
    assert_eq!(property.metadata["relation"], "hasOne");
    assert!(g.edges.iter().any(|e| e.kind == "implemented_by"
        && e.from == property.id
        && e.to.as_deref() == Some("method:common\\models\\Freespin::getMaxWin")));
    assert!(g.edges.iter().any(|e| e.kind == "contains"
        && e.from == "class:common\\models\\Freespin"
        && e.to.as_deref() == Some(property.id.as_str())));

    let call = single_call(&g, "getAllowed");
    assert_eq!(
        call.to.as_deref(),
        Some("method:common\\models\\MaxWin::getAllowed")
    );
    assert_eq!(call.metadata["resolution"], "resolved");
    let read = g
        .edges
        .iter()
        .find(|e| e.kind == "reads" && e.to.as_deref() == Some(property.id.as_str()))
        .unwrap();
    assert_eq!(read.metadata["type_source"], "framework");
    assert_eq!(read.metadata["framework_evidence"], "relation");
}

#[test]
fn has_many_relation_types_foreach_elements() {
    let g = analyze(&[
        ("common/models/All.php", MODELS),
        ("config/main.php", CONFIG),
    ]);
    let property = node(&g, "property:common\\models\\Freespin::$bonuses");
    assert_eq!(property.metadata["type"], "common\\models\\UserBonus[]");
    assert_eq!(property.metadata["relation"], "hasMany");
    let call = single_call(&g, "update");
    assert_eq!(
        call.to.as_deref(),
        Some("method:common\\models\\UserBonus::update")
    );
}

#[test]
fn declared_properties_and_unprovable_getters_produce_no_relation() {
    let g = analyze(&[
        ("common/models/All.php", MODELS),
        ("config/main.php", CONFIG),
    ]);
    let declared = node(&g, "property:common\\models\\Freespin::$declared");
    assert!(declared.metadata.get("framework").is_none(), "{declared:?}");
    for name in ["conditional", "dynamic"] {
        let id = format!("property:common\\models\\Freespin::${name}");
        assert!(!g.nodes.iter().any(|n| n.id == id), "{id} must not exist");
    }
}

#[test]
fn configured_component_types_a_variable() {
    let g = analyze(&[
        ("common/models/All.php", MODELS),
        ("config/main.php", CONFIG),
    ]);
    let property = node(&g, "property:yii\\base\\Application::$redis");
    assert_eq!(property.metadata["type"], "yii\\redis\\Connection");
    assert_eq!(property.metadata["framework_evidence"], "component");
    assert_eq!(property.span.path, "config/main.php");
    let call = single_call(&g, "get");
    assert_eq!(
        call.to.as_deref(),
        Some("method:yii\\redis\\Connection::get")
    );
    // Yii::$app is typed by vendor PHPDoc, so the chain is an annotation result.
    assert_eq!(call.metadata["resolution"], "annotated");
}

#[test]
fn dynamic_component_map_creates_no_component_members() {
    let g = analyze(&[
        ("common/models/All.php", MODELS),
        ("config/main.php", CONFIG),
        ("config/web.php", "<?php return ['components' => $runtime];"),
    ]);
    assert!(!g
        .nodes
        .iter()
        .any(|n| n.id == "property:yii\\base\\Application::$redis"));
}

#[test]
fn component_with_conflicting_classes_resolves_to_candidates() {
    let services = r#"<?php
namespace Services;
class First { public function send() {} }
class Second { public function send() {} }
class Notifier {
  public function notify() {
    $mailer = \Yii::$app->mailer;
    $mailer->send();
  }
}
"#;
    let g = analyze(&[
        ("app/Services.php", services),
        (
            "config/a.php",
            "<?php return ['components'=>['mailer'=>\\Services\\First::class]];",
        ),
        (
            "config/b.php",
            "<?php return ['components'=>['mailer'=>['class'=>\\Services\\Second::class]]];",
        ),
    ]);
    let property = node(&g, "property:yii\\base\\Application::$mailer");
    assert_eq!(
        property.metadata["type"],
        "Services\\First|Services\\Second"
    );
    let found = calls(&g, "send");
    assert_eq!(found.len(), 2, "one calls edge per candidate: {found:#?}");
    for call in &found {
        assert_eq!(call.metadata["resolution"], "candidate");
        let candidates = call.metadata["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2, "{call:#?}");
    }
    let targets: std::collections::BTreeSet<_> =
        found.iter().filter_map(|e| e.to.as_deref()).collect();
    assert_eq!(
        targets,
        [
            "method:Services\\First::send",
            "method:Services\\Second::send"
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn classless_local_config_only_configures_the_component() {
    let g = analyze(&[
        ("common/models/All.php", MODELS),
        ("config/main.php", CONFIG),
        (
            "config/main-local.php",
            "<?php return ['components' => ['redis' => ['hostname' => 'localhost', 'port' => 6379]]];",
        ),
    ]);
    let property = node(&g, "property:yii\\base\\Application::$redis");
    assert_eq!(property.metadata["type"], "yii\\redis\\Connection");
}

#[test]
fn dynamic_component_map_is_reported_with_its_location() {
    let g = analyze(&[
        ("common/models/All.php", MODELS),
        ("config/main.php", CONFIG),
        (
            "scripts/stand.php",
            "<?php\n$queues = [];\nreturn ['components' => [...$queues, 'db' => ['class' => 'yii\\db\\Connection']]];",
        ),
    ]);
    assert!(!g
        .nodes
        .iter()
        .any(|n| n.id == "property:yii\\base\\Application::$redis"));
    let diagnostic = g
        .diagnostics
        .iter()
        .find(|d| d.code == "dynamic_component_map")
        .unwrap_or_else(|| panic!("{:?}", g.diagnostics));
    assert_eq!(
        (diagnostic.path.as_str(), diagnostic.line),
        ("scripts/stand.php", 3)
    );
}

#[test]
fn occurrence_nodes_mirror_the_resolution_of_their_effect_edge() {
    let mailer = r#"<?php
namespace Services;
class Mailer { public function send() {} }
class Notifier {
  public function notify() {
    \Yii::$app->mailer->send();
    $m = new Mailer();
  }
}
"#;
    // No Yii vendor: only the post-analysis component rule can resolve `send`.
    let files: Vec<_> = [
        ("app/Services.php", mailer),
        (
            "config/main.php",
            "<?php return ['components'=>['mailer'=>['class'=>\\Services\\Mailer::class]]];",
        ),
    ]
    .iter()
    .map(|(path, source)| SourceFile {
        path: (*path).into(),
        source: (*source).into(),
    })
    .collect();
    let g = code_atlas::plugins::registry()
        .analyze(
            &files,
            &["php".into()],
            &["yii2".into()],
            &AnalysisOptions::default(),
        )
        .unwrap();
    let effects: Vec<_> = g
        .edges
        .iter()
        .filter(|e| {
            matches!(
                e.kind.as_str(),
                "calls" | "reads" | "writes" | "property_access"
            )
        })
        .collect();
    assert!(
        effects.iter().any(|e| e.metadata["framework"] == "yii2"),
        "{effects:#?}"
    );
    for edge in effects {
        let site = node(&g, &edge.from);
        assert_eq!(
            site.metadata["resolution"], edge.metadata["resolution"],
            "{} {}",
            site.name, edge.kind
        );
    }
}

fn analyze_in_areas(extra: &[(&str, &str)], areas: Vec<code_atlas::domain::Area>) -> GraphSnapshot {
    let files: Vec<_> = vendor()
        .into_iter()
        .chain(extra.iter().copied())
        .map(|(path, source)| SourceFile {
            path: path.into(),
            source: source.into(),
        })
        .collect();
    let options = AnalysisOptions {
        declaration_paths: vec!["vendor/yiisoft".into()],
        areas,
        ..AnalysisOptions::default()
    };
    code_atlas::plugins::registry()
        .analyze(&files, &["php".into()], &["yii2".into()], &options)
        .unwrap()
}

#[test]
fn components_follow_the_configs_of_their_area() {
    const MAILERS: &str = r#"<?php
namespace app\mail;
class PartnerMailer { public function send() {} }
class AdminMailer { public function send() {} }
"#;
    const CALLER: &str = r#"<?php
namespace partner\controllers;
class SiteController { public function run() { \Yii::$app->mailer->send(); } }
"#;
    let files = [
        ("lib/Mailers.php", MAILERS),
        (
            "partner/config/main.php",
            "<?php return ['components' => ['mailer' => ['class' => 'app\\mail\\PartnerMailer']]];",
        ),
        (
            "admin/config/main.php",
            "<?php return ['components' => ['mailer' => ['class' => 'app\\mail\\AdminMailer']]];",
        ),
        ("partner/controllers/SiteController.php", CALLER),
    ];
    let area = |prefix: &str| code_atlas::domain::Area {
        prefix: prefix.into(),
        paths: vec![prefix.into()],
        shared: false,
    };
    let graph = analyze_in_areas(&files, vec![area("partner"), area("admin")]);
    let send = single_call(&graph, "send");
    assert_eq!(
        send.to.as_deref(),
        Some("method:app\\mail\\PartnerMailer::send"),
        "{:?}",
        send.metadata
    );
    // `Yii::$app` itself is typed by the BaseYii `@var` annotation.
    assert_eq!(send.metadata["resolution"], "annotated");
    // Without areas both configs apply: two candidates.
    let union = analyze_in_areas(&files, vec![]);
    assert!(
        calls(&union, "send").len() >= 2
            || calls(&union, "send")[0].metadata["resolution"] == "candidate"
    );
}
