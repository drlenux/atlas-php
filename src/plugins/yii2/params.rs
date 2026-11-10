//! Top-level keys in `**/config/params.php` → `config_key` nodes (searchable definitions).
use super::syntax::{self, Expression, Names};
use crate::analyzer::{AnalysisOptions, SourceFile};
use crate::domain::{GraphSnapshot, Node, SourceSpan};
use crate::plugins::php::tokens::tokenize;
use serde_json::json;

/// Index string keys of the top-level `return [ … ]` in Yii params config files.
pub(super) fn enrich_params(
    files: &[SourceFile],
    graph: &mut GraphSnapshot,
    options: &AnalysisOptions,
) -> Result<(), String> {
    for file in files {
        if !is_params_config(&file.path) {
            continue;
        }
        options.check(graph.nodes.len() + graph.edges.len())?;
        let tokens = tokenize(&file.source, options)?;
        let Some(array_tokens) = top_level_return_array(&tokens) else {
            continue;
        };
        let names = Names::default();
        let Ok(Expression::Array(items)) = syntax::expression(array_tokens, &names, options) else {
            continue;
        };
        for (key, value) in items {
            let Some(key) = key.filter(|k| !k.is_empty()) else {
                continue;
            };
            options.check(graph.nodes.len() + 1)?;
            let line = key_line(&tokens, &key).unwrap_or(1);
            let id = format!("config_key:params:{key}");
            if graph.nodes.iter().any(|n| n.id == id) {
                continue;
            }
            let (value_expression, value_constant, value_type) = value_meta(&value);
            graph.nodes.push(Node {
                id: id.clone(),
                kind: "config_key".into(),
                name: key.clone(),
                qualified_name: format!("params.{key}"),
                span: SourceSpan {
                    path: file.path.clone(),
                    start_line: line,
                    end_line: line,
                },
                metadata: json!({
                    "config": "params",
                    "value_expression": value_expression,
                    "value_constant": value_constant,
                    "value_type": value_type,
                }),
                trust: 1,
                revision: 0,
            });
        }
    }
    Ok(())
}

fn is_params_config(path: &str) -> bool {
    let path = path.replace('\\', "/");
    path.ends_with("/config/params.php")
        || path.ends_with("/config/params-local.php")
        || path == "config/params.php"
        || path == "config/params-local.php"
}

fn top_level_return_array(
    tokens: &[crate::plugins::php::tokens::Token],
) -> Option<&[crate::plugins::php::tokens::Token]> {
    let return_at = tokens.iter().position(|t| t.is("return"))?;
    let start = return_at + 1;
    let open_tok = tokens.get(start)?;
    let open = if open_tok.text == "[" {
        start
    } else if open_tok.is("array") {
        start + 1
    } else {
        return None;
    };
    let close = syntax::closing(tokens, open)?;
    Some(&tokens[open..=close])
}

fn key_line(tokens: &[crate::plugins::php::tokens::Token], key: &str) -> Option<usize> {
    tokens.iter().find_map(|t| {
        let text = syntax::string(&t.text)?;
        (text == key).then_some(t.line)
    })
}

fn value_meta(value: &Expression) -> (Option<String>, Option<String>, Option<String>) {
    match value {
        Expression::Text(text) => (Some(format!("'{text}'")), None, Some("string".into())),
        Expression::Class(name) => (
            Some(format!("{name}::class")),
            Some(name.clone()),
            Some("class-string".into()),
        ),
        Expression::Array(_) => (Some("[…]".into()), None, Some("array".into())),
        Expression::Unknown => (None, None, None),
    }
}
