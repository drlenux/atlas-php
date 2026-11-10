use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    plugins::php::PhpLanguagePlugin,
};

#[test]
fn partial_edits_and_unicode_never_panic_or_leave_dangling_graph_edges() {
    let valid="<?php /* пример */ namespace App; class Model { public function read($x) { if ($x) { return 'שלום'; } return $x; } }";
    let boundaries: Vec<_> = valid
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(valid.len()))
        .collect();
    let mut corpus = vec![valid.to_owned()];
    for end in &boundaries {
        corpus.push(valid[..*end].to_owned());
    }
    for pair in boundaries.windows(2) {
        let mut edit = valid.to_owned();
        edit.replace_range(pair[0]..pair[1], "");
        corpus.push(edit);
    }
    for source in corpus {
        let result = std::panic::catch_unwind(|| {
            PhpLanguagePlugin.analyze(
                &[SourceFile {
                    path: "partial.php".into(),
                    source: source.clone(),
                }],
                &AnalysisOptions::default(),
            )
        });
        assert!(result.is_ok(), "parser panic for {source:?}");
        if let Ok(graph) = result.unwrap() {
            let ids: std::collections::BTreeSet<_> =
                graph.nodes.iter().map(|n| n.id.as_str()).collect();
            for edge in graph.edges {
                assert!(
                    ids.contains(edge.from.as_str()),
                    "dangling source after partial edit: {source:?}"
                );
                assert!(
                    edge.to.as_ref().is_none_or(|id| ids.contains(id.as_str())),
                    "dangling target after partial edit: {source:?}"
                );
            }
        }
    }
}
