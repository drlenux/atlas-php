use code_atlas::{
    analyzer::{AnalysisOptions, FrameworkPlugin, LanguagePlugin, PluginRegistry, SourceFile},
    domain::{Diagnostic, GraphSnapshot},
};

struct Language(&'static str, &'static [&'static str]);
impl LanguagePlugin for Language {
    fn id(&self) -> &'static str {
        self.0
    }
    fn extensions(&self) -> &'static [&'static str] {
        self.1
    }
    fn analyze(&self, files: &[SourceFile], _: &AnalysisOptions) -> Result<GraphSnapshot, String> {
        Ok(GraphSnapshot {
            diagnostics: files
                .iter()
                .map(|f| Diagnostic {
                    path: f.path.clone(),
                    line: 1,
                    code: self.id().into(),
                    message: "language visited".into(),
                })
                .collect(),
            ..Default::default()
        })
    }
}
struct Framework;
impl FrameworkPlugin for Framework {
    fn id(&self) -> &'static str {
        "yii2"
    }
    fn language(&self) -> &'static str {
        "php"
    }
    fn enrich(
        &self,
        files: &[SourceFile],
        graph: &mut GraphSnapshot,
        _: &AnalysisOptions,
    ) -> Result<(), String> {
        assert!(files.iter().all(|f| f.path.ends_with(".php")));
        graph.diagnostics.push(Diagnostic {
            code: "yii2".into(),
            message: "framework visited".into(),
            ..Default::default()
        });
        Ok(())
    }
}
fn registry() -> PluginRegistry {
    PluginRegistry::new(
        vec![
            Box::new(Language("php", &["php"])),
            Box::new(Language("fake", &["example"])),
        ],
        vec![Box::new(Framework)],
    )
    .unwrap()
}
fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
#[test]
fn registration_routes_future_language_without_changing_indexer() {
    let files = vec![
        SourceFile {
            path: "a.php".into(),
            source: "".into(),
        },
        SourceFile {
            path: "b.example".into(),
            source: "".into(),
        },
    ];
    let graph = registry()
        .analyze(
            &files,
            &ids(&["php", "fake"]),
            &ids(&["yii2"]),
            &AnalysisOptions::default(),
        )
        .unwrap();
    assert_eq!(
        graph
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>(),
        ["php", "yii2", "fake"]
    );
    assert_eq!(
        registry()
            .extensions(&ids(&["fake"]), &[])
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["example"]
    );
}
#[test]
fn framework_can_be_disabled_and_requires_enabled_language() {
    let files = vec![SourceFile {
        path: "a.php".into(),
        source: "".into(),
    }];
    let graph = registry()
        .analyze(&files, &ids(&["php"]), &[], &AnalysisOptions::default())
        .unwrap();
    assert_eq!(graph.diagnostics.len(), 1);
    assert!(registry()
        .validate(&ids(&["fake"]), &ids(&["yii2"]))
        .unwrap_err()
        .contains("requires php"));
}
#[test]
fn unknown_duplicate_or_conflicting_plugin_is_rejected() {
    assert!(registry()
        .validate(&ids(&["python"]), &[])
        .unwrap_err()
        .contains("unknown language"));
    assert!(registry()
        .validate(&ids(&["php"]), &ids(&["unknown"]))
        .unwrap_err()
        .contains("unknown framework"));
    assert!(registry().validate(&ids(&["php", "php"]), &[]).is_err());
    assert!(PluginRegistry::new(
        vec![
            Box::new(Language("php", &["php"])),
            Box::new(Language("php", &["other"]))
        ],
        vec![]
    )
    .is_err());
    let conflicting = PluginRegistry::new(
        vec![
            Box::new(Language("one", &["php"])),
            Box::new(Language("two", &["php"])),
        ],
        vec![],
    )
    .unwrap();
    assert!(conflicting
        .validate(&ids(&["one", "two"]), &[])
        .unwrap_err()
        .contains("extension"));
}
#[test]
fn expired_budget_rejects_the_whole_analysis() {
    let options = AnalysisOptions {
        deadline: std::time::Instant::now(),
        ..Default::default()
    };
    assert!(registry()
        .analyze(&[], &ids(&["php"]), &[], &options)
        .unwrap_err()
        .contains("time limit"));
}

#[test]
fn nodes_under_an_area_are_labelled_with_its_prefix() {
    let files = [
        code_atlas::analyzer::SourceFile {
            path: "partner/A.php".into(),
            source: "<?php namespace partner; class A { public function run() {} }".into(),
        },
        code_atlas::analyzer::SourceFile {
            path: "lib/B.php".into(),
            source: "<?php namespace lib; class B {}".into(),
        },
    ];
    let options = code_atlas::analyzer::AnalysisOptions {
        areas: vec![code_atlas::domain::Area {
            prefix: "partner".into(),
            paths: vec!["partner".into()],
            shared: false,
        }],
        ..code_atlas::analyzer::AnalysisOptions::default()
    };
    let graph = code_atlas::plugins::registry()
        .analyze(&files, &["php".into()], &[], &options)
        .unwrap();
    let area = |id: &str| graph.nodes.iter().find(|n| n.id == id).unwrap().metadata["area"].clone();
    assert_eq!(area("class:partner\\A"), "partner");
    assert_eq!(area("method:partner\\A::run"), "partner");
    assert!(area("class:lib\\B").is_null());
}
