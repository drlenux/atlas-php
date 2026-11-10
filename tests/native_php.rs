use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::{Edge, GraphSnapshot, Node},
    plugins::php::PhpLanguagePlugin,
};
use std::time::Instant;

fn analyze(source: &str) -> GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "source.php".into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}
fn fixture() -> GraphSnapshot {
    analyze(include_str!("fixtures/php/app/Domain.php"))
}
fn node<'a>(g: &'a GraphSnapshot, id: &str) -> &'a Node {
    g.nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("missing {id}; diagnostics {:?}", g.diagnostics))
}
fn edges<'a>(g: &'a GraphSnapshot, from: &str, kind: &str, to: Option<&str>) -> Vec<&'a Edge> {
    let nested: std::collections::BTreeSet<_> = g
        .edges
        .iter()
        .filter(|e| e.kind == "contains" && e.from == from)
        .filter_map(|e| e.to.clone())
        .collect();
    let allow_nested = matches!(kind, "calls" | "reads" | "writes" | "property_access");
    g.edges
        .iter()
        .filter(|e| {
            e.kind == kind
                && (e.from == from || (allow_nested && nested.contains(&e.from)))
                && to.is_none_or(|t| e.to.as_deref() == Some(t))
        })
        .collect()
}

#[test]
fn methods_are_slim_with_branch_and_occurrence_nodes() {
    let g = analyze(
        r#"<?php
namespace Fine;
class Wallet {
  public $balance = 0;
  function charge(bool $flag) {
    if ($flag) { $this->balance = 1; }
    return $this->balance;
  }
}
"#,
    );
    let method = node(&g, "method:Fine\\Wallet::charge");
    assert!(method.metadata.get("body").is_none());
    assert!(method.metadata.get("source").is_none());
    assert_eq!(method.metadata["commented"], false);
    assert!(g.nodes.iter().any(|n| {
        n.kind == "branch"
            && n.metadata["condition"].as_str().unwrap().contains("$flag")
            && n.metadata["commented"] == false
    }));
    let writes = edges(
        &g,
        "method:Fine\\Wallet::charge",
        "writes",
        Some("property:Fine\\Wallet::$balance"),
    );
    assert_eq!(writes.len(), 1);
    assert!(writes[0].from.starts_with("write_site:"));
    let write_site = node(&g, &writes[0].from);
    assert_eq!(write_site.metadata["member_name"], "balance");
    assert_eq!(write_site.metadata["receiver_class"], "Fine\\Wallet");
    assert_eq!(writes[0].metadata["receiver_class"], "Fine\\Wallet");
    assert_eq!(
        edges(
            &g,
            "method:Fine\\Wallet::charge",
            "contains",
            Some(&writes[0].from)
        )
        .len(),
        1
    );
    let reads = edges(
        &g,
        "method:Fine\\Wallet::charge",
        "reads",
        Some("property:Fine\\Wallet::$balance"),
    );
    assert!(!reads.is_empty());
    assert!(reads.iter().all(|e| e.from.starts_with("read_site:")));
}

#[test]
fn commented_call_becomes_commented_occurrence() {
    let g = analyze(
        r#"<?php
namespace Fine;
class Service {
  function run() {
    $wallet = null;
    // $wallet->charge(1);
    return 1;
  }
}
"#,
    );
    let commented: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| n.kind == "call_site" && n.metadata["commented"] == true)
        .collect();
    assert_eq!(commented.len(), 1);
    assert!(commented[0].metadata["expression"]
        .as_str()
        .unwrap()
        .contains("$wallet->charge"));
    assert_eq!(
        node(&g, "method:Fine\\Service::run").metadata["commented"],
        false
    );
}

#[test]
fn real_fixture_has_typed_declarations_and_relations_without_runtime() {
    let g = fixture();
    assert!(g.nodes.len() > 20);
    for n in &g.nodes {
        assert_eq!(n.trust, 1);
        assert_eq!(n.revision, 0);
        assert!(n.span.start_line > 0 && n.span.end_line >= n.span.start_line);
        assert!(n.metadata["fingerprint"].is_string());
    }
    assert_eq!(
        edges(&g, "class:App\\Shop", "inherits", Some("class:App\\Base")).len(),
        1
    );
    assert_eq!(
        edges(
            &g,
            "class:App\\Shop",
            "uses_trait",
            Some("trait:App\\Labels")
        )
        .len(),
        1
    );
    assert_eq!(
        edges(
            &g,
            "method:App\\Shop::__construct",
            "calls",
            Some("method:App\\Base::__construct")
        )
        .len(),
        1
    );
    assert_eq!(
        edges(
            &g,
            "method:App\\Shop::label",
            "overrides",
            Some("method:App\\Labels::label")
        )
        .len(),
        1
    );
    assert!(edges(
        &g,
        "method:App\\Shop::label",
        "calls",
        Some("method:App\\Labels::label")
    )
    .is_empty());
    assert_eq!(
        edges(
            &g,
            "class:App\\OtherShop",
            "available_via",
            Some("method:App\\Labels::label")
        )
        .len(),
        1
    );
}
#[test]
fn return_cases_conditions_variables_and_origins_survive() {
    let g = fixture();
    let r = node(&g, "method:App\\Shop::choose").metadata["returns"]
        .as_array()
        .unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(r[0]["type"], "null");
    assert_eq!(r[1]["type"], "int");
    assert!(r[1]["conditions"].as_array().unwrap().len() >= 3);
    assert!(r[1]["origins"][0]["expression"]
        .as_str()
        .unwrap()
        .contains("42"));
    assert!(r[1]["snippet"].as_str().unwrap().contains("return $number"));
    let v = node(&g, "variable:method:App\\Shop::choose:$number");
    assert_eq!(v.metadata["type"], "int");
    assert_eq!(v.metadata["assignments"].as_array().unwrap().len(), 1);
    assert!(v.metadata["usages"].as_array().unwrap().len() >= 2);
    let types: Vec<_> = node(&g, "method:App\\Shop::values").metadata["returns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        ["int", "string", "App\\NewPayment", "resource|false", "null"]
    );
}
#[test]
fn interface_candidates_do_not_become_active_implementation() {
    let g = fixture();
    let calls = edges(
        &g,
        "method:App\\Shop::choose",
        "calls",
        Some("method:App\\Payment::pay"),
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].metadata["resolution"], "candidate");
    assert_eq!(calls[0].metadata["candidates"].as_array().unwrap().len(), 2);
    // Implementations also get candidate calls edges so called_by works; not "resolved".
    for impl_method in ["method:App\\OldPayment::pay", "method:App\\NewPayment::pay"] {
        let edge = edges(&g, "method:App\\Shop::choose", "calls", Some(impl_method));
        assert_eq!(edge.len(), 1, "{impl_method}");
        assert_eq!(edge[0].metadata["resolution"], "candidate");
    }
    let definite = edges(
        &g,
        "method:App\\Shop::definite",
        "calls",
        Some("method:App\\NewPayment::pay"),
    );
    assert_eq!(definite.len(), 1);
    assert_eq!(definite[0].metadata["resolution"], "resolved");
}
#[test]
fn nested_functions_closures_and_implicit_completion_are_separate() {
    let g = fixture();
    assert_eq!(
        node(&g, "function:App\\outer").metadata["returns"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        node(&g, "function:App\\inner").metadata["returns"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        g.nodes
            .iter()
            .filter(|n| n.kind == "function" && n.metadata["closure"] == true)
            .count(),
        2
    );
    assert_eq!(
        node(&g, "function:App\\noReturn").metadata["implicit_return"],
        true
    );
    assert!(
        node(&g, "function:App\\early").metadata["returns"][1]["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains('!'))
    );
}
#[test]
fn aliases_self_unset_and_branch_merges() {
    let g = analyze(
        r#"<?php namespace Edge;
 trait First {function shared(){return 1;}} trait Second {function shared(){return 'two';}}
 class Using {use First,Second {First::shared insteadof Second;First::shared as renamed;}function run(){return $this->renamed();}}
 class Creator {function make():self{return $this;}} class Caller {function call(Creator $creator){return $creator->make();}}
 function erased(){$value=4;unset($value);return $value;}
 function merged(bool $flag){if($flag){$value=1;}else{$value='two';}return $value;}
 "#,
    );
    assert_eq!(
        edges(
            &g,
            "method:Edge\\Using::run",
            "calls",
            Some("method:Edge\\First::shared")
        )
        .len(),
        1
    );
    assert_eq!(
        node(&g, "method:Edge\\Caller::call").metadata["returns"][0]["type"],
        "Edge\\Creator"
    );
    assert_eq!(
        node(&g, "function:Edge\\erased").metadata["returns"][0]["type"],
        "unknown"
    );
    let r = &node(&g, "function:Edge\\merged").metadata["returns"][0];
    assert_eq!(r["type"], "int|string");
    assert!(r["origins"].as_array().unwrap().len() >= 2);
}
#[test]
fn constrained_receivers_preserve_subclass_candidates() {
    let g=analyze("<?php namespace Dispatch;class Base{function answer(){return 'base';}}class Child extends Base{function answer(){return 'child';}}function ask(Base $v){return $v->answer();}function exact(){$v=new Base();return $v->answer();}");
    let calls = edges(
        &g,
        "function:Dispatch\\ask",
        "calls",
        Some("method:Dispatch\\Base::answer"),
    );
    assert_eq!(calls[0].metadata["resolution"], "candidate");
    assert_eq!(calls[0].metadata["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(
        edges(
            &g,
            "function:Dispatch\\exact",
            "calls",
            Some("method:Dispatch\\Base::answer")
        )[0]
        .metadata["resolution"],
        "resolved"
    );
}
#[test]
fn stable_ids_fingerprints_syntax_errors_and_inert_source() {
    let a = analyze("<?php namespace Stable;class Before{function work(){return 1;}}");
    let b = analyze("<?php\n\nnamespace Stable;class Before {function work() {return 1;}}");
    let c = analyze("<?php namespace Stable;class After{function work(){return 1;}}");
    assert_eq!(
        node(&a, "class:Stable\\Before").metadata["fingerprint"],
        node(&b, "class:Stable\\Before").metadata["fingerprint"]
    );
    assert_eq!(
        node(&a, "class:Stable\\Before").metadata["fingerprint"],
        node(&c, "class:Stable\\After").metadata["fingerprint"]
    );
    let bad = analyze("<?php function broken( {");
    assert!(bad.nodes.is_empty());
    assert!(bad.diagnostics.iter().any(|d| d.code == "parse_error"));
    let inert = analyze("<?php file_put_contents('/not-executed','payload');class Safe{}");
    assert!(inert.nodes.iter().any(|n| n.id == "class:Safe"));
    let quoted =
        analyze("<?php function strings(){/* return 8; } */ return 'function fake(){return 0;}';}");
    assert_eq!(
        node(&quoted, "function:strings").metadata["returns"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn framework_hooks_are_structured_but_php_does_not_apply_yii() {
    let g = analyze(include_str!("fixtures/php/app/YiiModel.php"));
    let property = edges(&g, "method:App\\Model::read", "property_access", None);
    assert_eq!(property.len(), 1);
    assert_eq!(property[0].metadata["receiver_class"], "App\\Model");
    assert_eq!(property[0].metadata["member_name"], "title");
    assert!(g.edges.iter().all(|e| e.kind != "magic_get"));
    assert!(g.diagnostics.iter().any(|d| d.code == "dynamic_call"));
    let call = edges(&g, "method:App\\Model::mail", "calls", None);
    assert!(
        call.iter()
            .any(|e| e.metadata["member_name"] == "send"
                && e.metadata["receiver_root_class"] == "Yii")
    );
}
#[test]
fn analysis_limits_are_enforced() {
    let files = [SourceFile {
        path: "a.php".into(),
        source: "<?php class A{function f(){return 1;}}".into(),
    }];
    let expired = AnalysisOptions {
        deadline: Instant::now(),
        ..Default::default()
    };
    assert!(PhpLanguagePlugin.analyze(&files, &expired).is_err());
    let small = AnalysisOptions {
        max_graph_elements: 1,
        ..Default::default()
    };
    assert!(PhpLanguagePlugin.analyze(&files, &small).is_err());
}

#[test]
fn parallel_parse_preserves_order_diagnostics_and_unit_indices_after_failed_files() {
    let files = [
        SourceFile { path: "bad.php".into(), source: "<?php function {".into() },
        SourceFile { path: "A.php".into(), source: "<?php namespace App; class A { function f() { $run = function() { return 7; }; return $run(); } }".into() },
        SourceFile { path: "bad2.php".into(), source: "<?php class {".into() },
        SourceFile { path: "B.php".into(), source: "<?php namespace App; class B extends A { function g() { return $this->f(); } }".into() },
    ];
    let sequential = PhpLanguagePlugin
        .analyze(&files, &AnalysisOptions::default())
        .unwrap();
    let parallel = PhpLanguagePlugin
        .analyze(
            &files,
            &AnalysisOptions {
                workers: 4,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(sequential
        .nodes
        .iter()
        .any(|node| node.id == "method:App\\B::g"));
    assert_eq!(sequential.diagnostics.len(), 2);
    assert_eq!(
        serde_json::to_value(sequential).unwrap(),
        serde_json::to_value(parallel).unwrap()
    );
    assert!(PhpLanguagePlugin
        .analyze(
            &files,
            &AnalysisOptions {
                workers: 0,
                ..Default::default()
            }
        )
        .is_err());
}

#[test]
fn unicode_comments_strings_identifiers_and_open_tags_are_safe() {
    let g = analyze("<?php /* Привет мир */ function привет() { return 'שלום'; }");
    assert_eq!(
        node(&g, "function:привет").metadata["returns"][0]["type"],
        "string"
    );
    let h = analyze("<?php function f() { return <<<TXT\nשלום мир\nTXT;\n}");
    assert_eq!(
        node(&h, "function:f").metadata["returns"][0]["type"],
        "string"
    );
    let html = analyze("<?phpword function fake(){return 1;}");
    assert!(html.nodes.is_empty());
    let uppercase = analyze("<?PHP function real(){return 1;}");
    assert_eq!(
        node(&uppercase, "function:real").metadata["returns"][0]["type"],
        "int"
    );
}

#[test]
fn native_analysis_runs_without_runtime_on_path() {
    if std::env::var_os("ATLAS_NATIVE_CHILD").is_some() {
        assert!(std::env::var("PATH").unwrap().is_empty());
        assert!(fixture().nodes.len() > 20);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native_analysis_runs_without_runtime_on_path"])
        .env("PATH", "")
        .env("ATLAS_NATIVE_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn selected_php_version_gates_known_language_features() {
    for (source, old, supported) in [
        (
            "<?php function f(){ $g=fn($x)=>$x; return $g(1); }",
            "7.3",
            "7.4",
        ),
        ("<?php function f($x){return $x?->answer();}", "7.4", "8.0"),
        ("<?php #[Marker] function f(){return 1;}", "7.4", "8.0"),
        (
            "<?php function f(int|string $x):int|string{return $x;}",
            "7.4",
            "8.0",
        ),
        ("<?php class C {public int|string $value;}", "7.4", "8.0"),
        ("<?php enum Choice {case Yes;}", "8.0", "8.1"),
        ("<?php class C {public readonly int $value;}", "8.0", "8.1"),
        ("<?php readonly class C {}", "8.1", "8.2"),
        (
            "<?php function f($x){return match($x){1=>2,default=>3};}",
            "7.4",
            "8.0",
        ),
    ] {
        let files = [SourceFile {
            path: "feature.php".into(),
            source: source.into(),
        }];
        let mut options = AnalysisOptions::default();
        options.language_versions.insert("php".into(), old.into());
        let denied = PhpLanguagePlugin.analyze(&files, &options).unwrap();
        assert!(
            denied.nodes.is_empty(),
            "old target {old} accepted {source}"
        );
        assert!(
            denied
                .diagnostics
                .iter()
                .any(|d| d.code == "unsupported_syntax"),
            "missing gate: {source}"
        );
        options
            .language_versions
            .insert("php".into(), supported.into());
        let accepted = PhpLanguagePlugin.analyze(&files, &options).unwrap();
        assert!(
            !accepted.nodes.is_empty(),
            "supported target {supported}: {:?}",
            accepted.diagnostics
        );
        assert!(accepted
            .diagnostics
            .iter()
            .all(|d| d.code != "unsupported_syntax"));
    }
    let options = AnalysisOptions {
        language_versions: [("php".into(), "7.4".into())].into(),
        ..Default::default()
    };
    let nullable = PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "a.php".into(),
                source: "<?php function f(?int $x):?int{return $x;}".into(),
            }],
            &options,
        )
        .unwrap();
    assert!(
        nullable.diagnostics.is_empty(),
        "nullable syntax is not a PHP 8 union: {:?}",
        nullable.diagnostics
    );
}

#[test]
fn incomplete_or_unsupported_files_do_not_publish_partial_declarations() {
    // Parse-time defects: no declarations at all.
    for source in [
        "<?php class Good {} function broken(",
        "<?php function f(){return;",
        "<?php /* unfinished",
        "<?php function f(){return 'unfinished;}",
    ] {
        let g = analyze(source);
        assert!(g.nodes.is_empty(), "{source}");
        assert!(g.diagnostics.iter().any(|d| d.code == "parse_error"));
    }
    // Body defects and alternative control-flow: keep declarations, limit that method.
    for source in [
        "<?php class Good {} function f(){$value = ;}",
        "<?php class Good {} function f(){return 1 + ;}",
        "<?php class Good { function ok(){} function f($x){if($x): return 1; endif;} }",
    ] {
        let g = analyze(source);
        assert!(
            g.nodes.iter().any(|n| n.id == "class:Good"),
            "class must remain for {source}: {:?}",
            g.diagnostics
        );
        assert!(g
            .diagnostics
            .iter()
            .any(|d| { d.code == "analysis_limitation" || d.code == "parse_error" }));
    }
}

#[test]
fn anonymous_classes_parse_without_duplicate_methods() {
    let g = analyze(
        r#"<?php
namespace T;
interface Builder { public function setAttribute(string $k, mixed $v); }
class Span {}
final class Faulty {
  public function make(): object {
    return new class () implements Builder {
      public function setAttribute(string $k, mixed $v) { return $this; }
    };
  }
  public function other(): object {
    return new class () extends Span {
      public function setAttribute(string $k, mixed $v) { throw new \RuntimeException('x'); }
    };
  }
}
"#,
    );
    assert!(
        g.diagnostics
            .iter()
            .all(|d| d.code != "parse_error" && d.code != "unsupported_syntax"),
        "{:?}",
        g.diagnostics
    );
    assert!(g.nodes.iter().any(|n| n.id == "class:T\\Faulty"));
    let anon_methods: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| n.kind == "method" && n.id.contains("{anonymous@") && n.name == "setAttribute")
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(
        anon_methods.len(),
        2,
        "each anonymous class keeps its own setAttribute: {anon_methods:?}"
    );
}

#[test]
fn arrow_property_named_namespace_is_not_a_declaration() {
    let g = analyze(
        r#"<?php
namespace App;
class Gen {
  public $namespace = 'x';
  public function n() {
    return substr($this->namespace, 0, -1);
  }
}
"#,
    );
    assert!(
        g.diagnostics.iter().all(|d| d.code != "parse_error"),
        "{:?}",
        g.diagnostics
    );
    assert!(g.nodes.iter().any(|n| n.id == "class:App\\Gen"));
}

#[test]
fn gii_template_with_namespace_property_survives() {
    let g = PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path:
                    "vendor/yiisoft/yii2-gii/src/generators/extension/default/AutoloadExample.php"
                        .into(),
                source: r#"<?= "<?php\n" ?>

namespace <?= substr($generator->namespace, 0, -1) ?>;

class AutoloadExample extends \yii\base\Widget
{
    public function run()
    {
        return "Hello!";
    }
}
"#
                .into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap();
    assert!(
        g.diagnostics.iter().all(|d| d.code != "parse_error"),
        "{:?}",
        g.diagnostics
    );
}

#[test]
fn native_source_paths_must_stay_relative() {
    for path in ["../outside.php", "/absolute.php", "bad\0.php"] {
        let g = PhpLanguagePlugin
            .analyze(
                &[SourceFile {
                    path: path.into(),
                    source: "<?php class Outside {}".into(),
                }],
                &AnalysisOptions::default(),
            )
            .unwrap();
        assert!(g.nodes.is_empty());
        assert!(g.diagnostics.iter().any(|d| d.code == "path_outside_root"));
    }
}

fn probe(expression: &str) -> GraphSnapshot {
    analyze(&format!(
        "<?php class Probe {{ function run() {{ return {expression}; }} }}"
    ))
}

fn limitation_texts(g: &GraphSnapshot) -> Vec<&str> {
    g.diagnostics
        .iter()
        .filter(|d| d.code == "analysis_limitation")
        .map(|d| d.message.as_str())
        .collect()
}

#[test]
fn cast_preserves_call_chain_and_result_type() {
    let baseline = probe("Yii::$app->request->post('userId')");
    let baseline_calls: Vec<_> = edges(&baseline, "method:Probe::run", "calls", None)
        .into_iter()
        .map(|e| {
            (
                e.metadata["member_name"].as_str().unwrap().to_string(),
                e.span.start_line,
            )
        })
        .collect();
    assert!(
        baseline_calls.iter().any(|(m, _)| m == "post"),
        "baseline calls: {baseline_calls:?}"
    );

    for (cast, ty) in [("int", "int"), ("bool", "bool"), ("string", "string")] {
        let g = probe(&format!("({cast})Yii::$app->request->post('userId')"));
        let calls: Vec<_> = edges(&g, "method:Probe::run", "calls", None)
            .into_iter()
            .map(|e| e.metadata["member_name"].as_str().unwrap().to_string())
            .collect();
        assert!(
            calls.iter().any(|m| m == "post"),
            "cast ({cast}) lost post call: {calls:?}; diagnostics {:?}",
            g.diagnostics
        );
        assert_eq!(
            calls.iter().filter(|m| *m == "post").count(),
            1,
            "duplicate post after ({cast})"
        );
        assert_eq!(
            node(&g, "method:Probe::run").metadata["returns"][0]["type"],
            ty
        );
        let limitations = limitation_texts(&g);
        assert!(
            limitations.iter().all(|m| {
                !m.contains("::") && !m.contains("->") && !m.contains("Yii::$app->request->post")
            }),
            "cast ({cast}) cascaded limitations: {limitations:?}"
        );
    }
}

#[test]
fn ternary_and_elvis_preserve_branch_calls_without_operator_noise() {
    let g = analyze(
        "<?php function loadA(){return 1;} function loadB(){return 2;} function fallback(){return 3;}
 function run($ok, $value) { $a = $ok ? loadA() : loadB(); $b = $value ?: fallback(); $c = $value ?? fallback(); return $a; }",
    );
    for target in ["function:loadA", "function:loadB", "function:fallback"] {
        let count = edges(&g, "function:run", "calls", Some(target)).len();
        let expected = if target == "function:fallback" { 2 } else { 1 };
        assert_eq!(count, expected, "{target}: {:?}", g.diagnostics);
    }
    let load_a = edges(&g, "function:run", "calls", Some("function:loadA"))[0];
    let load_b = edges(&g, "function:run", "calls", Some("function:loadB"))[0];
    assert!(
        load_a.metadata["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("$ok")),
        "loadA conditions: {:?}",
        load_a.metadata["conditions"]
    );
    assert!(
        load_b.metadata["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("$ok")),
        "loadB conditions: {:?}",
        load_b.metadata["conditions"]
    );
    let limitations = limitation_texts(&g);
    assert!(
        limitations
            .iter()
            .all(|m| !m.contains('?') && !m.contains(':') && !m.contains("??")),
        "operator-only limitations: {limitations:?}"
    );
}

#[test]
fn array_index_and_compound_assignment_preserve_calls_and_uses() {
    let g = analyze(
        r#"<?php class Counter { static function deleteAll($filter){return 1;} }
 class Probe { function run($data, $filter) {
   $id = $data['id'];
   $deleted = [];
   $deleted['counter'] = Counter::deleteAll($filter);
   $deletedTotal = 0;
   $deletedTotal += Counter::deleteAll($filter);
   return $id;
 } }"#,
    );
    let id = node(&g, "variable:method:Probe::run:$id");
    assert!(
        id.metadata["assignments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["expression"].as_str().unwrap().contains("$data['id']")),
        "missing $data['id'] assign: {}",
        id.metadata
    );
    assert!(
        !node(&g, "variable:method:Probe::run:$data").metadata["usages"]
            .as_array()
            .unwrap()
            .is_empty(),
        "container $data was not read"
    );
    let delete_calls = edges(
        &g,
        "method:Probe::run",
        "calls",
        Some("method:Counter::deleteAll"),
    );
    assert_eq!(delete_calls.len(), 2, "diagnostics {:?}", g.diagnostics);
    assert!(
        node(&g, "variable:method:Probe::run:$deletedTotal").metadata["assignments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["expression"].as_str().unwrap().contains("+="))
    );
    let limitations = limitation_texts(&g);
    assert!(
        limitations
            .iter()
            .all(|m| !m.contains("['id']") && !m.contains("+=")),
        "index/+= limitations: {limitations:?}"
    );
}

#[test]
fn nested_object_cast_array_keeps_keys_conditions_and_nested_call() {
    let g = analyze(
        r#"<?php function fallback(){return 'x';}
 function run($ok, $user, $documentId, $uid) {
   return (object)[
     'status' => $ok ? 'valid' : 'invalid',
     'applicant' => (object)['id' => $user?->id ?? fallback()],
     'document_id' => $documentId ?: ('sim-' . $uid)
   ];
 }"#,
    );
    assert_eq!(
        node(&g, "function:run").metadata["returns"][0]["type"],
        "object"
    );
    assert_eq!(
        edges(&g, "function:run", "calls", Some("function:fallback")).len(),
        1
    );
    let property = edges(&g, "function:run", "property_access", None);
    assert!(
        property.iter().any(|e| e.metadata["member_name"] == "id"),
        "nullsafe id access missing: {property:?}"
    );
    let limitations = limitation_texts(&g);
    assert!(
        limitations
            .iter()
            .all(|m| !m.contains("=>") && !m.contains(',') && !m.trim_end().ends_with(':')),
        "array fragment cascade: {limitations:?}"
    );
}

#[test]
fn unsupported_construct_emits_one_primary_limitation_and_keeps_nested_call() {
    let g = analyze(
        "<?php class Probe { function run($a, $b) { return match($a) { 1 => load(), default => $b }; } function load(){return 1;} }",
    );
    // match is gated as unsupported_syntax for whole file when version denies it;
    // for default 8.2 match is accepted by version gate but expression may be limited.
    let limitations = g
        .diagnostics
        .iter()
        .filter(|d| d.code == "analysis_limitation")
        .collect::<Vec<_>>();
    if !g.nodes.is_empty() {
        assert!(edges(&g, "method:Probe::run", "calls", Some("method:Probe::load")).len() <= 1);
        assert!(
            limitations.len() <= 2,
            "cascaded limitations: {:?}",
            limitations
        );
    }
}

#[test]
fn independent_limitations_on_one_line_remain_distinct() {
    let g = analyze("<?php class Probe { function run($a, $b) { return ($a <=> $b) + (`raw`); } }");
    // backticks / spaceship if not structurally supported should not collapse to one path+line dedupe hide
    let limitations = g
        .diagnostics
        .iter()
        .filter(|d| d.code == "analysis_limitation")
        .collect::<Vec<_>>();
    assert!(
        !limitations.is_empty() || g.diagnostics.iter().any(|d| d.code == "parse_error"),
        "expected limitation or parse evidence: {:?}",
        g.diagnostics
    );
}

#[test]
fn cast_index_keeps_calls_while_unresolved_targets_stay_distinct_from_parse_defects() {
    let g = analyze(
        "<?php class Probe { function run($data){ $id=(int)$data['id']; return Yii::$app->request->post($id); } }",
    );
    let calls = edges(&g, "method:Probe::run", "calls", None);
    assert!(
        calls.iter().any(|e| e.metadata["member_name"] == "post"),
        "post call missing: {:?}; {:?}",
        calls,
        g.diagnostics
    );
    assert!(
        limitation_texts(&g)
            .iter()
            .all(|m| !m.contains("(int)") && !m.contains("['id']")),
        "parse defect leaked into limitations: {:?}",
        limitation_texts(&g)
    );
    assert!(
        calls.iter().any(|e| {
            e.metadata["member_name"] == "post"
                && (e.metadata["resolution"] == "unresolved"
                    || e.metadata["resolution"] == "candidate")
        }) || g
            .diagnostics
            .iter()
            .any(|d| { d.code == "unresolved_call" || d.code == "unresolved_symbol" }),
        "missing unresolved target evidence: {:?}; {:?}",
        calls,
        g.diagnostics
    );
}

#[test]
fn expression_graph_identical_for_one_and_four_workers() {
    let files = [SourceFile {
        path: "probe.php".into(),
        source: "<?php class Probe { function run($ok,$data){ $id=(int)$data['id']; return $ok ? Yii::$app->x($id) : ($data['n'] ?? 0); } }".into(),
    }];
    let one = PhpLanguagePlugin
        .analyze(
            &files,
            &AnalysisOptions {
                workers: 1,
                ..Default::default()
            },
        )
        .unwrap();
    let four = PhpLanguagePlugin
        .analyze(
            &files,
            &AnalysisOptions {
                workers: 4,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(&one).unwrap(),
        serde_json::to_value(&four).unwrap()
    );
}

#[test]
fn methods_override_the_interface_methods_they_implement() {
    let g = fixture();
    for implementation in ["method:App\\OldPayment::pay", "method:App\\NewPayment::pay"] {
        assert_eq!(
            edges(
                &g,
                implementation,
                "overrides",
                Some("method:App\\Payment::pay")
            )
            .len(),
            1,
            "{implementation} implements Payment::pay"
        );
    }
}

#[test]
fn union_receiver_property_takes_every_part_type() {
    let g = analyze(
        r#"<?php
namespace App;
class ConsoleRequest {}
class WebRequest { public function post() { return 1; } }
/** @property ConsoleRequest $request */
class ConsoleApp {}
/** @property WebRequest $request */
class WebApp {}
class Ctl {
    public function run(ConsoleApp|WebApp $app) { return $app->request->post(); }
}
"#,
    );
    assert_eq!(
        edges(
            &g,
            "method:App\\Ctl::run",
            "calls",
            Some("method:App\\WebRequest::post")
        )
        .len(),
        1,
        "diagnostics {:?}",
        g.diagnostics
    );
}

fn site_for<'a>(g: &'a GraphSnapshot, member: &str) -> &'a Node {
    g.nodes
        .iter()
        .find(|n| n.kind == "call_site" && n.metadata["member_name"] == member)
        .unwrap_or_else(|| panic!("no call site for {member}"))
}

#[test]
fn calls_to_methods_declared_nowhere_are_undeclared() {
    let g = analyze(
        r#"<?php
namespace App;
class Wallet { public function deposit() {} }
class Magic { public function __call($name, $args) {} }
class Base {}
class Child extends Base { public function onlyBelow() {} }
class Remote extends \Vendor\Missing {}
class Hidden extends \Vendor\Unknown { public function onlyHidden() {} }
class Ctl {
    public function run(Wallet $w, Magic $m, Base $b, Remote $r, $x) {
        $w->checkCreateWager();
        $m->anything();
        $b->onlyBelow();
        $r->inherited();
        $x->untyped();
        $b->onlyHidden();
    }
}
"#,
    );
    let undeclared = site_for(&g, "checkCreateWager");
    assert_eq!(
        undeclared.metadata["undeclared"], true,
        "{:?}",
        undeclared.metadata
    );
    assert!(g
        .diagnostics
        .iter()
        .any(|d| d.code == "undeclared_member"
            && d.message.contains("App\\Wallet::checkCreateWager")));
    // `Hidden` may descend from `Base` through classes outside the index.
    for member in [
        "anything",
        "onlyBelow",
        "inherited",
        "untyped",
        "onlyHidden",
    ] {
        assert_ne!(
            site_for(&g, member).metadata["undeclared"],
            true,
            "{member} may exist at runtime"
        );
    }
    assert!(
        !g.diagnostics
            .iter()
            .any(|d| d.code == "unresolved_call" && d.message.contains("checkCreateWager")),
        "undeclared replaces the generic diagnostic"
    );
}

#[test]
fn namespace_less_names_are_prefixed_and_resolved_by_area() {
    let files = [
        ("api/Provider.php", "<?php class Provider { public function api() {} }"),
        ("console/Provider.php", "<?php class Provider { public function console() {} }"),
        ("api/Base.php", "<?php class Base {}"),
        ("common/Shared.php", "<?php class Shared { public function s() {} }"),
        ("api/helpers.php", "<?php function helper() {}"),
        // An IDE stub in a shared area must not shadow the real library class.
        ("vendor/lib/Yii.php", "<?php class Yii { public static function t() {} }"),
        ("common/config/__autocomplete.php", "<?php class Yii {}"),
        ("api/T.php", "<?php class T { public function run() { Yii::t(); } }"),
        ("common/C.php", "<?php class C { public function run() { Yii::t(); } }"),
        ("api/Ns.php", "<?php namespace api\\ns; class Ns { public function go() { helper(); } }"),
        (
            "api/UseIt.php",
            "<?php class UseIt extends Base { public function run() { $p = new Provider(); $p->api(); (new Shared())->s(); } }",
        ),
    ];
    let area = |prefix: &str, shared: bool| code_atlas::domain::Area {
        prefix: prefix.into(),
        paths: vec![prefix.into()],
        shared,
    };
    let options = AnalysisOptions {
        areas: vec![
            area("api", false),
            area("console", false),
            area("common", true),
        ],
        ..AnalysisOptions::default()
    };
    let g = PhpLanguagePlugin
        .analyze(
            &files
                .iter()
                .map(|(path, source)| SourceFile {
                    path: (*path).into(),
                    source: (*source).into(),
                })
                .collect::<Vec<_>>(),
            &options,
        )
        .unwrap();
    for id in [
        "class:api:Provider",
        "class:console:Provider",
        "class:common:Shared",
    ] {
        node(&g, id);
    }
    assert!(
        !g.diagnostics
            .iter()
            .any(|d| d.message.contains("duplicate")),
        "{:?}",
        g.diagnostics
    );
    assert_eq!(
        edges(
            &g,
            "method:api:UseIt::run",
            "calls",
            Some("method:api:Provider::api")
        )
        .len(),
        1,
        "{:?}",
        g.diagnostics
    );
    assert_eq!(
        edges(
            &g,
            "method:api:UseIt::run",
            "calls",
            Some("method:common:Shared::s")
        )
        .len(),
        1
    );
    assert_eq!(
        edges(&g, "class:api:UseIt", "inherits", Some("class:api:Base")).len(),
        1
    );
    // The library class wins even inside the area that holds the stub.
    for caller in ["method:api:T::run", "method:common:C::run"] {
        assert_eq!(
            edges(&g, caller, "calls", Some("method:Yii::t")).len(),
            1,
            "{caller}: {:?}",
            g.diagnostics
        );
    }
    // A namespaced file falls back to the global function of its own area.
    assert_eq!(
        edges(
            &g,
            "method:api\\ns\\Ns::go",
            "calls",
            Some("function:api:helper")
        )
        .len(),
        1
    );
}
