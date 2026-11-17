//! Yii2 render() → view file + params injected into the view.
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::GraphSnapshot,
    repository::MemoryRepository,
    service::AtlasService,
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

const YII: &str = r#"<?php
namespace yii\base;
class Controller {
  public $layout = 'main';
  public function render($view, $params = []) { return ''; }
  public function renderPartial($view, $params = []) { return ''; }
  public function renderAjax($view, $params = []) { return ''; }
  public function renderFile($file, $params = []) { return ''; }
}
"#;

#[test]
fn render_links_controller_view_and_provides_params() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/base/Controller.php", YII),
        (
            "partner/controllers/MyBonusController.php",
            r#"<?php
namespace partner\controllers;
use yii\base\Controller;
use common\models\User;
class MyBonusController extends Controller {
  public function actionIndex(User $user) {
    return $this->render('index', [
      'user' => $user,
      'issetWager' => true,
    ]);
  }
}
"#,
        ),
        (
            "partner/views/my-bonus/index.php",
            r#"<?php
/** @var yii\web\View $this */
echo $user->id;
echo $issetWager;
"#,
        ),
    ]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "view:partner/views/my-bonus/index.php"),
        "view node; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "view")
            .map(|n| &n.id)
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "renders"
                && e.to.as_deref() == Some("view:partner/views/my-bonus/index.php")
                && e.from.contains("actionIndex")
        }),
        "renders edge; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "renders")
            .map(|e| (e.from.clone(), e.to.clone(), e.metadata.clone()))
            .collect::<Vec<_>>()
    );
    let params = g
        .edges
        .iter()
        .find(|e| e.kind == "renders")
        .and_then(|e| e.metadata["params"].as_array().cloned())
        .unwrap_or_default();
    assert!(
        params.iter().any(|p| p["name"] == "user"),
        "params include user: {params:?}"
    );
    assert!(
        g.nodes.iter().any(|n| {
            n.id == "variable:view:partner/views/my-bonus/index.php:$user"
                && n.metadata["type"]
                    .as_str()
                    .is_some_and(|t| t.contains("User"))
        }),
        "injected $user variable; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "variable")
            .map(|n| (&n.id, n.metadata["type"].clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "provides"
                && e.to.as_deref() == Some("variable:view:partner/views/my-bonus/index.php:$user")
        }),
        "provides param edge"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "contains"
                && e.from == "class:partner\\controllers\\MyBonusController"
                && e.to.as_deref() == Some("view:partner/views/my-bonus/index.php")
        }),
        "view belongs to controller via contains; {:?}",
        g.edges
            .iter()
            .filter(
                |e| e.kind == "contains" && e.to.as_deref().is_some_and(|t| t.starts_with("view:"))
            )
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn render_partial_resolves_relative_view() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/base/Controller.php", YII),
        (
            "partner/controllers/SiteController.php",
            r#"<?php
namespace partner\controllers;
use yii\base\Controller;
class SiteController extends Controller {
  public function actionCard() {
    return $this->renderPartial('_card', ['title' => 'x']);
  }
}
"#,
        ),
        ("partner/views/site/_card.php", "<?php echo $title;\n"),
    ]);
    assert!(g.edges.iter().any(|e| {
        e.kind == "renders" && e.to.as_deref() == Some("view:partner/views/site/_card.php")
    }));
}

#[test]
fn theme_override_is_candidate_when_present() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/base/Controller.php", YII),
        (
            "partner/controllers/SiteController.php",
            r#"<?php
namespace partner\controllers;
use yii\base\Controller;
class SiteController extends Controller {
  public function actionIndex() {
    return $this->render('index', []);
  }
}
"#,
        ),
        ("partner/views/site/index.php", "<?php echo 1;\n"),
        ("partner/themes/slot/site/index.php", "<?php echo 2;\n"),
    ]);
    let targets: Vec<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "renders")
        .filter_map(|e| e.to.clone())
        .collect();
    assert!(
        targets.contains(&"view:partner/views/site/index.php".into()),
        "{targets:?}"
    );
    assert!(
        targets.contains(&"view:partner/themes/slot/site/index.php".into()),
        "BaseController theme pathMap candidate; {targets:?}"
    );
}

#[test]
fn nested_render_and_layout_belong_to_controller() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/base/Controller.php", YII),
        (
            "vendor/yiisoft/yii2/base/View.php",
            r#"<?php
namespace yii\base;
class View {
  public function render($view, $params = []) { return ''; }
  public function renderPartial($view, $params = []) { return ''; }
  public function renderFile($file, $params = []) { return ''; }
}
"#,
        ),
        (
            "partner/controllers/SiteController.php",
            r#"<?php
namespace partner\controllers;
use yii\base\Controller;
class SiteController extends Controller {
  public function actionIndex() {
    $this->layout = 'main';
    return $this->render('index', []);
  }
}
"#,
        ),
        (
            "partner/views/site/index.php",
            r#"<?php
/** @var yii\base\View $this */
echo $this->render('_card', []);
"#,
        ),
        ("partner/views/site/_card.php", "<?php echo 1;\n"),
        ("partner/views/layouts/main.php", "<?php echo $content;\n"),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "renders"
                && e.span.path.contains("views/site/index.php")
                && e.to.as_deref() == Some("view:partner/views/site/_card.php")
        }),
        "nested renders edge; renders={:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "renders")
            .map(|e| (e.from.clone(), e.to.clone(), e.span.path.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "contains"
                && e.from == "class:partner\\controllers\\SiteController"
                && e.to.as_deref() == Some("view:partner/views/site/_card.php")
        }),
        "nested view owned by controller; contains-views={:?}",
        g.edges
            .iter()
            .filter(
                |e| e.kind == "contains" && e.to.as_deref().is_some_and(|t| t.starts_with("view:"))
            )
            .map(|e| (
                e.from.clone(),
                e.to.clone(),
                e.metadata["framework_evidence"].clone()
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "contains"
                && e.from == "class:partner\\controllers\\SiteController"
                && e.to.as_deref() == Some("view:partner/views/layouts/main.php")
        }),
        "layout owned by controller; layout-edges={:?}",
        g.edges
            .iter()
            .filter(|e| { e.to.as_deref().is_some_and(|t| t.contains("layouts/main")) })
            .map(|e| (e.kind.clone(), e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn short_echo_nested_render_creates_source_view_node() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/base/Controller.php", YII),
        (
            "vendor/yiisoft/yii2/base/View.php",
            r#"<?php
namespace yii\base;
class View {
  public function render($view, $params = []) { return ''; }
}
"#,
        ),
        (
            "partner/controllers/SiteController.php",
            r#"<?php
namespace partner\controllers;
use yii\base\Controller;
class SiteController extends Controller {
  public function actionUpdate() {
    return $this->render('update', []);
  }
}
"#,
        ),
        (
            "partner/views/site/update.php",
            r#"<?php
/** @var yii\base\View $this */
?>
<?= $this->render('_form', []) ?>
"#,
        ),
        ("partner/views/site/_form.php", "<?php echo 1;\n"),
    ]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "view:partner/views/site/update.php"),
        "source view node must exist for nested render"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "renders" && e.to.as_deref() == Some("view:partner/views/site/_form.php")
        }),
        "short-echo nested render; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "renders")
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn top_level_view_reads_sql_and_nested_render_via_file_scope() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/base/Controller.php", YII),
        (
            "vendor/yiisoft/yii2/base/View.php",
            r#"<?php
namespace yii\base;
class View {
  public function render($view, $params = []) { return ''; }
}
"#,
        ),
        (
            "vendor/yiisoft/yii2/db/Connection.php",
            r#"<?php
namespace yii\db;
class Connection {
  public function createCommand($sql, $params = []) { return null; }
}
"#,
        ),
        (
            "common/models/User.php",
            r#"<?php
namespace common\models;
class User {
  /** @var int */
  public $id;
}
"#,
        ),
        (
            "schema.sql",
            "CREATE TABLE wallet (id int, amount numeric);\n",
        ),
        (
            "partner/controllers/SiteController.php",
            r#"<?php
namespace partner\controllers;
use yii\base\Controller;
use common\models\User;
class SiteController extends Controller {
  public function actionIndex(User $user) {
    return $this->render('index', ['user' => $user]);
  }
}
"#,
        ),
        (
            "partner/views/site/index.php",
            r#"<?php
/** @var yii\base\View $this */
/** @var common\models\User $user */
/** @var yii\db\Connection $db */
echo $user->id;
$db->createCommand('SELECT amount FROM wallet')->queryOne();
echo $this->render('_card', []);
"#,
        ),
        ("partner/views/site/_card.php", "<?php echo 1;\n"),
    ]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "function:file:partner/views/site/index.php"),
        "file-scope function; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.id.starts_with("function:file:"))
            .map(|n| &n.id)
            .collect::<Vec<_>>()
    );
    let file_fn = "function:file:partner/views/site/index.php";
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "reads"
                && e.from.starts_with(&format!("read_site:{file_fn}"))
                && e.to.as_deref() == Some("property:common\\models\\User::$id")
        }),
        "top-level $user->id → property; {:?}",
        g.edges
            .iter()
            .filter(|e| e.metadata["member_name"] == "id")
            .map(|e| (
                e.kind.clone(),
                e.to.clone(),
                e.metadata["receiver_class"].clone()
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        g.nodes
            .iter()
            .any(|n| n.kind == "sql_site" && n.span.path.contains("views/site/index")),
        "top-level createCommand sql_site"
    );
    let render_calls: Vec<_> = g
        .edges
        .iter()
        .filter(|e| {
            e.kind == "calls"
                && e.from.starts_with(&format!("call_site:{file_fn}"))
                && e.metadata["member_name"] == "render"
        })
        .map(|e| {
            (
                e.to.clone(),
                e.metadata["resolution"].clone(),
                e.metadata["receiver_class"].clone(),
            )
        })
        .collect();
    assert!(
        render_calls.iter().any(|(to, res, recv)| {
            to.as_deref() == Some("method:yii\\base\\View::render")
                && (*res == "resolved" || *res == "annotated")
                && recv.as_str().is_some_and(|c| c.contains("View"))
        }),
        "file-scope $this->render via @var View; {render_calls:?}"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "renders"
                && e.from.starts_with(&format!("call_site:{file_fn}"))
                && e.to.as_deref() == Some("view:partner/views/site/_card.php")
        }),
        "nested render from file-scope call_site; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "renders")
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn view_outline_and_callers_tree_list_renderers() {
    let g = analyze(&[
        ("vendor/yiisoft/yii2/base/Controller.php", YII),
        (
            "partner/controllers/MyBonusController.php",
            r#"<?php
namespace partner\controllers;
use yii\base\Controller;
use common\models\User;
class MyBonusController extends Controller {
  public function actionIndex(User $user) {
    return $this->render('index', [
      'user' => $user,
      'issetWager' => true,
    ]);
  }
}
"#,
        ),
        (
            "partner/views/my-bonus/index.php",
            r#"<?php
/** @var yii\web\View $this */
echo $user->id;
echo $issetWager;
"#,
        ),
    ]);
    let paths: Vec<String> = g.nodes.iter().map(|n| n.span.path.clone()).collect();
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    service
        .index(g, paths, vec![], &std::collections::BTreeMap::new())
        .unwrap();
    let view = "view:partner/views/my-bonus/index.php";
    let outline = service
        .outline_with_usage(
            view,
            &["owner".into(), "called_by".into(), "members".into()],
        )
        .unwrap()
        .0;
    assert_eq!(
        outline.owner.as_deref(),
        Some("class:partner\\controllers\\MyBonusController"),
        "owner should be the rendering controller: {:?}",
        outline.owner
    );
    assert!(
        outline
            .called_by
            .iter()
            .any(|line| line.contains("actionIndex") && line.contains("renders")),
        "called_by should list the controller render site: {:?}",
        outline.called_by
    );
    assert!(
        outline.members.iter().any(|m| m.contains("$user")),
        "members should list injected view params: {:?}",
        outline.members
    );
    let tree = service
        .callers_tree(&code_atlas::domain::CallersRequest {
            root_id: view.into(),
            levels: 1,
            max_nodes: 50,
            exclude_paths: None,
            include_paths: None,
        })
        .unwrap();
    assert!(
        !tree.children.is_empty(),
        "callers tree on a view must walk renders edges: {tree:?}"
    );
}
