use code_atlas::analyzer::{AnalysisOptions, FrameworkPlugin, LanguagePlugin, SourceFile};
use code_atlas::domain::{Edge, GraphSnapshot, Node, SourceSpan};
use code_atlas::plugins::php::PhpLanguagePlugin;
use code_atlas::plugins::yii2::Yii2Plugin;
use serde_json::json;

fn class(name: &str, parent: Option<&str>) -> Node {
    Node {
        id: format!("class:{name}"),
        kind: "class".into(),
        name: name.rsplit('\\').next().unwrap().into(),
        qualified_name: name.into(),
        span: SourceSpan {
            path: "app.php".into(),
            start_line: 1,
            end_line: 1,
        },
        metadata: json!({"parent":parent,"interfaces":[],"traits":[]}),
        trust: 1,
        revision: 0,
    }
}
fn method(owner: &str, name: &str) -> Node {
    Node {
        id: format!("method:{owner}::{name}"),
        kind: "method".into(),
        name: name.into(),
        qualified_name: format!("{owner}::{name}"),
        span: SourceSpan {
            path: "app.php".into(),
            start_line: 2,
            end_line: 2,
        },
        metadata: json!({}),
        trust: 1,
        revision: 0,
    }
}
fn access(id: &str, owner: &str, receiver: Option<&str>, member: &str, operation: &str) -> Edge {
    Edge {
        id: id.into(),
        kind: if operation == "call" {
            "calls"
        } else {
            "property_access"
        }
        .into(),
        from: format!("method:{owner}::run"),
        to: None,
        span: SourceSpan {
            path: "app.php".into(),
            start_line: 5,
            end_line: 5,
        },
        metadata: json!({"receiver_class":receiver,"member_name":member,"operation":operation,"resolution":"unresolved"}),
        trust: 1,
        revision: 0,
    }
}
fn enrich(graph: &mut GraphSnapshot, files: &[SourceFile]) {
    Yii2Plugin
        .enrich(files, graph, &AnalysisOptions::default())
        .unwrap();
}

#[test]
fn inherited_getters_and_setters_resolve_only_for_yii_classes() {
    let mut getter = method("yii\\base\\Component", "getTitle");
    getter.span.path = "vendor/yii/base/Component.php".into();
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("yii\\base\\Component", None),
            class("App\\Model", Some("yii\\base\\Component")),
            class("App\\Ordinary", None),
            getter,
            method("yii\\base\\Component", "setTitle"),
            method("App\\Ordinary", "getTitle"),
        ],
        edges: vec![
            access("read", "App\\Model", Some("App\\Model"), "title", "read"),
            access("write", "App\\Model", Some("App\\Model"), "title", "write"),
            access(
                "plain",
                "App\\Ordinary",
                Some("App\\Ordinary"),
                "title",
                "read",
            ),
        ],
        diagnostics: vec![],
    };
    enrich(&mut graph, &[]);
    assert_eq!(graph.edges[0].kind, "calls");
    assert_eq!(
        graph.edges[0].to.as_deref(),
        Some("method:yii\\base\\Component::getTitle")
    );
    assert_eq!(graph.edges[1].kind, "magic_set");
    assert_eq!(
        graph.edges[1].to.as_deref(),
        Some("method:yii\\base\\Component::setTitle")
    );
    assert_eq!(graph.edges[2].kind, "property_access");
    assert!(graph.edges[2].to.is_none());
}

#[test]
fn static_behaviors_resolve_aliases_and_do_not_apply_to_plain_php_classes() {
    let mut behaviors = method("App\\Model", "behaviors");
    behaviors.metadata = json!({"body":"return ['audit' => ['class' => Audit::class]];","namespace":"App","imports":{"Audit":"Extensions\\AuditBehavior"}});
    let mut plain_behaviors = behaviors.clone();
    plain_behaviors.id = "method:App\\Ordinary::behaviors".into();
    plain_behaviors.qualified_name = "App\\Ordinary::behaviors".into();
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("yii\\base\\Component", None),
            class("App\\Model", Some("yii\\base\\Component")),
            class("App\\Ordinary", None),
            class("Extensions\\AuditBehavior", None),
            behaviors,
            plain_behaviors,
            method("Extensions\\AuditBehavior", "audit"),
        ],
        edges: vec![
            access("call", "App\\Model", Some("App\\Model"), "audit", "call"),
            access(
                "plain",
                "App\\Ordinary",
                Some("App\\Ordinary"),
                "audit",
                "call",
            ),
        ],
        diagnostics: vec![],
    };
    enrich(&mut graph, &[]);
    assert_eq!(
        graph.edges[0].to.as_deref(),
        Some("method:Extensions\\AuditBehavior::audit")
    );
    assert_eq!(graph.edges[0].metadata["framework"], "yii2");
    assert!(graph.edges[1].to.is_none());
}

#[test]
fn component_calls_resolve_static_config_and_global_yii_only() {
    let files=[SourceFile{path:"config/main.php".into(),source:"<?php use Services\\Mailer as Mail; return ['components' => ['mailer' => ['class' => Mail::class]]];".into()}];
    let mut call = access("send", "App\\Caller", None, "send", "call");
    call.metadata["receiver_expression"] = json!("\\Yii::$app->mailer");
    call.metadata["receiver_root_class"] = json!("Yii");
    let mut ordinary = call.clone();
    ordinary.id = "ordinary".into();
    ordinary.metadata["receiver_root_class"] = json!("App\\Yii");
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("Services\\Mailer", None),
            method("Services\\Mailer", "send"),
        ],
        edges: vec![call, ordinary],
        diagnostics: vec![],
    };
    enrich(&mut graph, &files);
    assert_eq!(
        graph.edges[0].to.as_deref(),
        Some("method:Services\\Mailer::send")
    );
    assert!(graph.edges[1].to.is_none());
}

#[test]
fn conflicting_or_dynamic_component_definitions_remain_candidates() {
    let files = [
        SourceFile {
            path: "config/a.php".into(),
            source: "<?php return ['components'=>['mailer'=>\\Services\\First::class]];".into(),
        },
        SourceFile {
            path: "config/b.php".into(),
            source:
                "<?php return ['components'=>['mailer'=>['class'=>\\Services\\Second::class]]];"
                    .into(),
        },
    ];
    let mut call = access("send", "App\\Caller", None, "send", "call");
    call.metadata["receiver_expression"] = json!("Yii::$app->mailer");
    call.metadata["receiver_root_class"] = json!("Yii");
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("Services\\First", None),
            class("Services\\Second", None),
            method("Services\\First", "send"),
            method("Services\\Second", "send"),
        ],
        edges: vec![call],
        diagnostics: vec![],
    };
    enrich(&mut graph, &files);
    assert!(graph.edges[0].to.is_none());
    assert_eq!(graph.edges[0].metadata["resolution"], "candidate");
    assert_eq!(
        graph.edges[0].metadata["candidates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(graph
        .diagnostics
        .iter()
        .any(|d| d.code == "ambiguous_component"));
    let dynamic = SourceFile {
        path: "config/runtime.php".into(),
        source: "<?php return ['components'=>['mailer'=>$runtime]];".into(),
    };
    enrich(&mut graph, &[files[0].clone(), dynamic]);
    assert!(graph.edges[0].to.is_none());
}

#[test]
fn conditional_behavior_and_dynamic_member_do_not_guess_targets() {
    let mut behaviors = method("App\\Model", "behaviors");
    behaviors.metadata = json!({"body":"if ($enabled) { return [\\Extensions\\Audit::class]; } return [];","namespace":"App","imports":{}});
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("yii\\base\\Component", None),
            class("App\\Model", Some("yii\\base\\Component")),
            class("Extensions\\Audit", None),
            behaviors,
            method("Extensions\\Audit", "audit"),
        ],
        edges: vec![
            access(
                "conditional",
                "App\\Model",
                Some("App\\Model"),
                "audit",
                "call",
            ),
            access(
                "dynamic",
                "App\\Model",
                Some("App\\Model"),
                "$method",
                "call",
            ),
        ],
        diagnostics: vec![],
    };
    enrich(&mut graph, &[]);
    assert!(graph.edges.iter().all(|e| e.to.is_none()));
}

#[test]
fn framework_contract_and_analysis_deadline_are_enforced() {
    assert_eq!(Yii2Plugin.id(), "yii2");
    assert_eq!(Yii2Plugin.language(), "php");
    let options = AnalysisOptions {
        deadline: std::time::Instant::now() - std::time::Duration::from_secs(1),
        ..AnalysisOptions::default()
    };
    assert!(Yii2Plugin
        .enrich(&[], &mut GraphSnapshot::default(), &options)
        .is_err());
}

#[test]
fn dynamic_component_map_prevents_false_certainty_from_another_config() {
    let files = [
        SourceFile {
            path: "config/static.php".into(),
            source: "<?php return ['components'=>['mailer'=>\\Services\\Mailer::class]];".into(),
        },
        SourceFile {
            path: "config/dynamic.php".into(),
            source: "<?php return ['components'=>$runtimeComponents];".into(),
        },
    ];
    let mut call = access("send", "App\\Caller", None, "send", "call");
    call.metadata["receiver_expression"] = json!("Yii::$app->mailer");
    call.metadata["receiver_root_class"] = json!("Yii");
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("Services\\Mailer", None),
            method("Services\\Mailer", "send"),
        ],
        edges: vec![call],
        diagnostics: vec![],
    };
    enrich(&mut graph, &files);
    assert!(
        graph.edges[0].to.is_none(),
        "an unknown component map can override the static mailer"
    );
    assert_eq!(
        graph.edges[0].metadata["candidates"],
        json!(["method:Services\\Mailer::send"])
    );
}

#[test]
fn runtime_spread_cannot_confirm_the_static_component_class() {
    let files = [SourceFile {
        path: "config/main.php".into(),
        source: "<?php return ['components'=>['mailer'=>['class'=>\\Services\\Mailer::class, ...$override]]];".into(),
    }];
    let mut call = access("send", "App\\Caller", None, "send", "call");
    call.metadata["receiver_expression"] = json!("Yii::$app->mailer");
    call.metadata["receiver_root_class"] = json!("Yii");
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("Services\\Mailer", None),
            method("Services\\Mailer", "send"),
        ],
        edges: vec![call],
        diagnostics: vec![],
    };
    enrich(&mut graph, &files);
    assert!(
        graph.edges[0].to.is_none(),
        "runtime array spread may replace class"
    );
    assert_eq!(graph.edges[0].metadata["resolution"], "candidate");
}

#[test]
fn return_inside_a_closure_does_not_register_a_behavior() {
    let mut behaviors = method("App\\Model", "behaviors");
    behaviors.metadata = json!({"body":"$unused = function () { return [\\Extensions\\Audit::class]; };","namespace":"App","imports":{}});
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("yii\\base\\Component", None),
            class("App\\Model", Some("yii\\base\\Component")),
            class("Extensions\\Audit", None),
            behaviors,
            method("Extensions\\Audit", "audit"),
        ],
        edges: vec![access(
            "call",
            "App\\Model",
            Some("App\\Model"),
            "audit",
            "call",
        )],
        diagnostics: vec![],
    };
    enrich(&mut graph, &[]);
    assert!(
        graph.edges[0].to.is_none(),
        "nested closure return is not the behaviors() return"
    );
}

#[test]
fn real_php_sources_enrich_vendor_properties_behavior_and_component_calls() {
    let files = [
        SourceFile {
            path: "app/YiiModel.php".into(),
            source: include_str!("fixtures/php/app/YiiModel.php").into(),
        },
        SourceFile {
            path: "vendor/yii/base/Component.php".into(),
            source: include_str!("fixtures/php/vendor/yii/base/Component.php").into(),
        },
        SourceFile {
            path: "config/main.php".into(),
            source: include_str!("fixtures/php/config/main.php").into(),
        },
    ];
    let mut graph = PhpLanguagePlugin
        .analyze(&files, &AnalysisOptions::default())
        .unwrap();
    assert!(!graph
        .edges
        .iter()
        .any(|e| e.kind == "magic_get" || e.kind == "magic_set"));
    enrich(&mut graph, &files);
    let sites = |owner: &str| -> std::collections::BTreeSet<String> {
        let method = format!("method:{owner}");
        graph
            .edges
            .iter()
            .filter(|e| e.kind == "contains" && e.from == method)
            .filter_map(|e| e.to.clone())
            .collect()
    };
    for (owner, kind, target) in [
        (
            "App\\Model::read",
            "calls",
            "yii\\base\\Component::getTitle",
        ),
        (
            "App\\Model::write",
            "magic_set",
            "yii\\base\\Component::setTitle",
        ),
        ("App\\Model::invoke", "calls", "App\\AuditBehavior::audit"),
        ("App\\Model::mail", "calls", "App\\Mailer::send"),
    ] {
        let nested = sites(owner);
        assert!(
            graph.edges.iter().any(|e| {
                (e.from == format!("method:{owner}") || nested.contains(&e.from))
                    && e.kind == kind
                    && e.to.as_deref() == Some(format!("method:{target}").as_str())
            }),
            "missing {owner} -> {target}: {:?}",
            graph.edges
        );
    }
    let dynamic_sites = sites("App\\Model::dynamic");
    assert!(graph
        .edges
        .iter()
        .filter(|e| {
            e.kind == "calls"
                && (e.from == "method:App\\Model::dynamic" || dynamic_sites.contains(&e.from))
        })
        .all(|e| e.to.is_none()));
}

#[test]
fn component_calls_missing_everywhere_including_behaviors_are_undeclared() {
    let mut behaviors = method("App\\Model", "behaviors");
    behaviors.metadata = json!({"body":"return ['audit' => ['class' => Audit::class]];","namespace":"App","imports":{"Audit":"Extensions\\AuditBehavior"}});
    let mut site = method("App\\Model", "site");
    site.id = "call_site:gone".into();
    site.kind = "call_site".into();
    site.metadata = json!({"member_name":"gone","resolution":"unresolved"});
    let call = |id: &str, member: &str, dispatcher: &str| {
        let mut edge = access(id, "App\\Model", Some("App\\Model"), member, "call");
        edge.metadata["dispatcher"] = json!(dispatcher);
        edge
    };
    let mut gone = call("gone", "gone", "method:yii\\base\\Component::__call");
    gone.from = "call_site:gone".into();
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("yii\\base\\Component", None),
            method("yii\\base\\Component", "__call"),
            class("App\\Model", Some("yii\\base\\Component")),
            class("App\\Child", Some("App\\Model")),
            method("App\\Child", "onlyBelow"),
            class("App\\Magic", Some("yii\\base\\Component")),
            method("App\\Magic", "__call"),
            class("Extensions\\AuditBehavior", None),
            method("Extensions\\AuditBehavior", "audit"),
            behaviors,
            site,
        ],
        edges: vec![
            call("audit", "audit", "method:yii\\base\\Component::__call"),
            gone,
            call("below", "onlyBelow", "method:yii\\base\\Component::__call"),
            {
                let mut magic = access(
                    "magic",
                    "App\\Magic",
                    Some("App\\Magic"),
                    "anything",
                    "call",
                );
                magic.metadata["dispatcher"] = json!("method:App\\Magic::__call");
                magic
            },
        ],
        diagnostics: vec![code_atlas::domain::Diagnostic {
            path: "app.php".into(),
            line: 5,
            code: "unresolved_call".into(),
            message: "Cannot resolve call $m->gone()".into(),
        }],
    };
    enrich(&mut graph, &[]);
    let edge = |id: &str| graph.edges.iter().find(|e| e.id == id).unwrap();
    assert_eq!(
        edge("gone").metadata["undeclared"],
        true,
        "{:?}",
        edge("gone").metadata
    );
    let site = graph
        .nodes
        .iter()
        .find(|n| n.id == "call_site:gone")
        .unwrap();
    assert_eq!(site.metadata["undeclared"], true);
    for id in ["audit", "below", "magic"] {
        assert_ne!(edge(id).metadata["undeclared"], true, "{id}");
    }
    assert!(graph
        .diagnostics
        .iter()
        .any(|d| d.code == "undeclared_member"));
    assert!(!graph
        .diagnostics
        .iter()
        .any(|d| d.code == "unresolved_call"));
}

#[test]
fn undeclared_needs_every_possible_declarer_to_be_ruled_out() {
    // A subclass whose ancestry leaves the index may be a hidden descendant; a descendant
    // may take the method from a trait.
    let mut filsh = class("App\\FilshModule", Some("Vendor\\Oauth\\Module"));
    filsh.metadata["traits"] = json!([]);
    let mut active = class("App\\ActiveQuery", Some("App\\Query"));
    active.metadata["traits"] = json!(["App\\ActiveQueryTrait"]);
    let mut query_trait = class("App\\ActiveQueryTrait", None);
    query_trait.kind = "trait".into();
    query_trait.id = "trait:App\\ActiveQueryTrait".into();
    let call = |id: &str, receiver: &str, member: &str| {
        let mut edge = access(id, "App\\Caller", Some(receiver), member, "call");
        edge.metadata["dispatcher"] = json!("method:yii\\base\\Component::__call");
        edge
    };
    let mut graph = GraphSnapshot {
        nodes: vec![
            class("yii\\base\\Component", None),
            method("yii\\base\\Component", "__call"),
            class("yii\\base\\Module", Some("yii\\base\\Component")),
            filsh,
            method("App\\FilshModule", "getServer"),
            class("App\\Query", Some("yii\\base\\Component")),
            active,
            query_trait,
            method("App\\ActiveQueryTrait", "with"),
        ],
        edges: vec![
            call("server", "yii\\base\\Module", "getServer"),
            call("with", "App\\Query", "with"),
        ],
        diagnostics: vec![],
    };
    enrich(&mut graph, &[]);
    for edge in &graph.edges {
        assert_ne!(edge.metadata["undeclared"], true, "{}", edge.id);
    }
}
