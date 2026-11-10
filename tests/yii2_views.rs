//! Yii2 render() → view file + params injected into the view.
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

const YII: &str = r#"<?php
namespace yii\base;
class Controller {
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
