//! R2b: `called_by` / callers tree list callee parameters a call site did not pass
//! (slotor MaxWin: 7 positional args of 8, `$bonus_type` silently defaulted).
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::CallersRequest,
    mcp_text,
    plugins::php::PhpLanguagePlugin,
    repository::MemoryRepository,
    service::AtlasService,
};

const TARGET: &str = "method:App\\FreeSpins::createFreeSpin";

fn service_for(source: &str) -> AtlasService {
    let snapshot = PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "App.php".into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap();
    let mut s = AtlasService::new(Box::new(MemoryRepository::new()));
    s.index(snapshot, vec!["App.php".into()], vec![]).unwrap();
    s
}

/// `createFreeSpin($a, $b, $c, $bonus_type = 1)` called by one `run()` body.
fn source_with_call(call: &str) -> String {
    format!(
        "<?php\nnamespace App;\nclass FreeSpins {{\n  public function createFreeSpin($a, $b, $c, $bonus_type = 1) {{}}\n}}\nclass C {{\n  private FreeSpins $f;\n  public function run() {{\n    {call}\n  }}\n}}\n"
    )
}

fn called_by(call: &str) -> String {
    let mut s = service_for(&source_with_call(call));
    s.outline(TARGET).unwrap().called_by.join("\n")
}

fn tree_text(call: &str) -> String {
    let mut s = service_for(&source_with_call(call));
    let tree = s
        .callers_tree(&CallersRequest {
            root_id: TARGET.into(),
            levels: 1,
            max_nodes: 50,
            exclude_paths: None,
            include_paths: None,
        })
        .unwrap();
    mcp_text::callers(&s.freshness().unwrap(), &tree)
}

#[test]
fn positional_call_lists_unpassed_parameter() {
    assert_eq!(
        called_by("$this->f->createFreeSpin(1, 2, 3);"),
        "App\\C::run L9 missing:[bonus_type]: $this->f->createFreeSpin(1, 2, 3) [resolved]"
    );
}

#[test]
fn callers_tree_lists_unpassed_parameter() {
    let text = tree_text("$this->f->createFreeSpin(1, 2, 3);");
    assert!(text.contains("missing:[bonus_type]"), "{text}");
}

#[test]
fn named_call_lists_parameters_without_named_argument() {
    let text = called_by("$this->f->createFreeSpin(a: 1, c: 3);");
    assert!(text.contains("missing:[b, bonus_type]"), "{text}");
}

#[test]
fn positional_then_named_counts_both() {
    let text = called_by("$this->f->createFreeSpin(1, 2, c: 3);");
    assert!(text.contains("missing:[bonus_type]"), "{text}");
}

#[test]
fn complete_call_has_no_missing() {
    let text = called_by("$this->f->createFreeSpin(1, 2, 3, 4);");
    assert!(!text.contains("missing"), "{text}");
}

#[test]
fn spread_argument_hides_missing() {
    let text = called_by("$this->f->createFreeSpin(...$args);");
    assert!(!text.contains("missing"), "{text}");
}

#[test]
fn variadic_parameter_is_never_missing() {
    let src = "<?php\nnamespace App;\nclass FreeSpins {\n  public function createFreeSpin($a, ...$rest) {}\n}\nclass C {\n  private FreeSpins $f;\n  public function run() { $this->f->createFreeSpin(1); }\n}\n";
    let mut s = service_for(src);
    let text = s.outline(TARGET).unwrap().called_by.join("\n");
    assert!(!text.contains("missing"), "{text}");
}

#[test]
fn static_call_lists_only_names_not_defaults() {
    let src = "<?php\nnamespace App;\nclass FreeSpins {\n  public static function createFreeSpin($a, $bonus_type = 1, $meta = ['x' => 'secret-default']) {}\n}\nclass C {\n  public function run() { FreeSpins::createFreeSpin(1); }\n}\n";
    let mut s = service_for(src);
    let text = s.outline(TARGET).unwrap().called_by.join("\n");
    assert!(text.contains("missing:[bonus_type, meta]"), "{text}");
    assert!(!text.contains("secret-default"), "{text}");
}
