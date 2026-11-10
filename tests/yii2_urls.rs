//! YII-URL-01..03: route nodes, Url::to references, js/html url_site.
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
    code_atlas::plugins::registry()
        .analyze(
            &files,
            &["php".into()],
            &["yii2".into()],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

#[test]
fn controller_action_creates_convention_route() {
    let g = analyze(&[(
        "app/controllers/GameController.php",
        r#"<?php
namespace app\controllers;
class GameController {
  public function actionView($slug) {}
}
"#,
    )]);
    let route = g
        .nodes
        .iter()
        .find(|n| n.kind == "route" && n.name == "game/view")
        .expect("route game/view");
    assert!(g.edges.iter().any(|e| {
        e.kind == "handles"
            && e.from == route.id
            && e.to.as_deref() == Some("method:app\\controllers\\GameController::actionView")
    }));
}

#[test]
fn url_to_references_route() {
    let g = analyze(&[
        (
            "app/controllers/GameController.php",
            r#"<?php
namespace app\controllers;
class GameController {
  public function actionView($slug) {}
}
"#,
        ),
        (
            "app/widgets/Link.php",
            r#"<?php
namespace app\widgets;
use yii\helpers\Url;
class Link {
  public function href($slug) {
    return Url::to(['game/view', 'slug' => $slug]);
  }
}
"#,
        ),
    ]);
    assert!(
        g.nodes.iter().any(|n| n.kind == "url_site"),
        "expected url_site from Url::to; nodes={:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "url_site" || n.kind == "route")
            .map(|n| (&n.kind, &n.name))
            .collect::<Vec<_>>()
    );
    assert!(g
        .edges
        .iter()
        .any(|e| { e.kind == "references" && e.to.as_deref() == Some("route:game/view") }));
}

#[test]
fn js_fetch_creates_url_site() {
    let g = analyze(&[
        (
            "app/controllers/GameController.php",
            r#"<?php
namespace app\controllers;
class GameController {
  public function actionView($slug) {}
}
"#,
        ),
        (
            "web/game.js",
            r#"
async function openGame() {
  const r = await fetch('/game/view');
  return r.json();
}
"#,
        ),
    ]);
    let site = g
        .nodes
        .iter()
        .find(|n| n.kind == "url_site" && n.span.path.ends_with("game.js"))
        .expect("js url_site");
    assert!(g.edges.iter().any(|e| {
        e.kind == "references" && e.from == site.id && e.to.as_deref() == Some("route:game/view")
    }));
}

#[test]
fn routes_php_rule_creates_route_node() {
    let g = analyze(&[(
        "api/config/routes.php",
        r#"<?php
return [
  'docs' => 'site/docs',
  'v1/page/<slug:\w+>' => 'v1/page/view',
];
"#,
    )]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.kind == "route" && n.name == "api:site/docs"),
        "{:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "route")
            .map(|n| &n.name)
            .collect::<Vec<_>>()
    );
    assert!(g
        .nodes
        .iter()
        .any(|n| n.kind == "route" && n.name == "api:v1/page/view"));
}

#[test]
fn array_pattern_route_rule_creates_route() {
    let g = analyze(&[(
        "partner/config/routes.php",
        r#"<?php
return [
  [
    'pattern' => 'mock-payplace/payout-invoices',
    'route' => 'mock/pay-place-send-withdraw',
    'verb' => 'POST',
  ],
];
"#,
    )]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.kind == "route" && n.name == "partner:mock/pay-place-send-withdraw"),
        "array pattern/route; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "route")
            .map(|n| &n.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn keyed_placeholder_rule_is_indexed() {
    let g = analyze(&[(
        "partner/config/routes.php",
        r#"<?php
return [
  'mock-payplace/<action>' => 'mock-payplace/<action>',
  'mock-payplace/payout' => [
    'class' => 'yii\web\UrlRule',
    'pattern' => 'mock-payplace/payout',
    'route' => 'mock-payplace/payout',
  ],
];
"#,
    )]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.kind == "route" && n.name == "partner:mock-payplace/<action>"),
        "keyed placeholder rule; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "route")
            .map(|n| &n.name)
            .collect::<Vec<_>>()
    );
    assert!(
        g.nodes
            .iter()
            .any(|n| n.kind == "route" && n.name == "partner:mock-payplace/payout"),
        "keyed pattern => array rule; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "route")
            .map(|n| &n.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn rest_extra_patterns_create_prefixed_routes() {
    let g = analyze(&[
        (
            "api/config/routes.php",
            r#"<?php
return [
  [
    'class' => 'yii\rest\UrlRule',
    'controller' => ['v1/my-bonus'],
    'pluralize' => false,
    'extraPatterns' => [
      'POST activate-bonus' => 'activate-bonus',
    ],
  ],
];
"#,
        ),
        (
            "api/modules/v1/controllers/MyBonusController.php",
            r#"<?php
namespace api\modules\v1\controllers;
class MyBonusController {
  public function actionActivateBonus() {}
}
"#,
        ),
    ]);
    assert!(
        g.nodes
            .iter()
            .any(|n| n.kind == "route" && n.name == "api:v1/my-bonus/activate-bonus"),
        "REST extraPatterns; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "route")
            .map(|n| &n.name)
            .collect::<Vec<_>>()
    );
    let route = g
        .nodes
        .iter()
        .find(|n| n.kind == "route" && n.name == "api:v1/my-bonus/activate-bonus")
        .unwrap();
    assert!(g.edges.iter().any(|e| {
        e.kind == "handles"
            && e.from == route.id
            && e.to.as_deref()
                == Some(
                    "method:api\\modules\\v1\\controllers\\MyBonusController::actionActivateBonus",
                )
    }));
    // No bare or module-less api:my-bonus/… — only api:v1/my-bonus/…
    assert!(!g
        .nodes
        .iter()
        .any(|n| n.id == "route:my-bonus/activate-bonus"));
    assert!(!g
        .nodes
        .iter()
        .any(|n| n.id == "route:api:my-bonus/activate-bonus"));
}

#[test]
fn partner_url_to_prefers_app_route_not_module() {
    let g = analyze(&[
        (
            "api/config/main.php",
            r#"<?php
return ['modules' => ['game' => ['class' => 'common\modules\game\Module']]];
"#,
        ),
        (
            "common/modules/game/controllers/GameController.php",
            r#"<?php
namespace common\modules\game\controllers;
class GameController {
  public function actionView($slug) {}
}
"#,
        ),
        (
            "partner/controllers/GameController.php",
            r#"<?php
namespace partner\controllers;
class GameController {
  public function actionView($slug) {}
}
"#,
        ),
        (
            "partner/widgets/Link.php",
            r#"<?php
namespace partner\widgets;
use yii\helpers\Url;
class Link {
  public function href() {
    return Url::to(['/game/view']);
  }
}
"#,
        ),
    ]);
    assert!(
        g.nodes.iter().any(|n| n.id == "route:partner:game/view"),
        "partner convention route; {:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "route")
            .map(|n| &n.id)
            .collect::<Vec<_>>()
    );
    assert!(
        g.nodes.iter().any(|n| n.id == "route:api:game/game/view"),
        "module only under apps that register it"
    );
    assert!(
        !g.nodes.iter().any(|n| n.id == "route:game/view"),
        "bare module route must not steal partner Url::to"
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "references" && e.to.as_deref() == Some("route:partner:game/view")
        }),
        "Url::to from partner → partner:game/view; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "references")
            .map(|e| e.to.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn js_comment_apostrophes_do_not_swallow_url_strings() {
    let g = analyze(&[
        (
            "partner/controllers/MyBonusController.php",
            r#"<?php
namespace partner\controllers;
class MyBonusController {
  public function actionActivateBonus() {}
}
"#,
        ),
        (
            "partner/web/js/JQ/mybonus.js",
            r#"
$(document).ready(function (e) {
    // Єдина точка прив'язки обробників бонус-попапів.
    // Усі обробники делеговані на document, тож ловлять і динамічно вставлені
    // (AJAX) кнопки. Guard гарантує одноразовий біндінг навіть якщо файл
    // випадково завантажиться повторно у AJAX-фрагменті.
    $(document).on("click", ".btn", function () {
        var url = lang_slug +'/my-bonus/activate-bonus';
        $.post(url);
    });
});
"#,
        ),
    ]);
    assert!(
        g.nodes.iter().any(|n| {
            n.kind == "url_site"
                && n.span.path.ends_with("mybonus.js")
                && n.metadata["route"] == "my-bonus/activate-bonus"
        }),
        "UA apostrophes in // comments must not hide url; sites={:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "url_site")
            .map(|n| (&n.span.path, n.metadata["route"].clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.nodes
            .iter()
            .any(|n| n.kind == "route" && n.name == "partner:my-bonus/activate-bonus"),
        "app-prefixed convention route"
    );
}
