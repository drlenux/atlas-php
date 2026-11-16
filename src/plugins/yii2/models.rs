//! ActiveRecord `tableName()` → `maps_table`; attributes from `rules()` /
//! `behaviors()` → virtual properties + `maps_column` (columns need not be
//! declared as PHP properties).
use super::syntax::{self, Expression, Names};
use super::{method_source, Index, RELATION_BASES};
use crate::analyzer::{AnalysisOptions, SourceFile};
use crate::domain::{Edge, GraphSnapshot, Node, SourceSpan};
use crate::plugins::php::tokens::TokenKind;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct AttrFact {
    name: String,
    evidence: &'static str,
    line: usize,
}

pub(super) struct TableLink {
    class_id: String,
    class_qn: String,
    class_path: String,
    class_line: usize,
    schema: String,
    table: String,
    attributes: Vec<AttrFact>,
}

/// Framework members so `$model->attr` resolves when the column is only in rules/behaviors.
pub(super) fn attribute_members(
    index: &Index<'_>,
    nodes: &[Node],
    files: &[SourceFile],
    options: &AnalysisOptions,
) -> Result<Vec<Node>, String> {
    options.progress.phase(
        "analysis",
        "yii2_model_attributes",
        Some(nodes.iter().filter(|n| n.kind == "class").count()),
        "classes",
    );
    let existing: BTreeSet<String> = nodes
        .iter()
        .filter(|n| n.kind == "property")
        .map(|n| n.id.clone())
        .collect();
    let mut out = Vec::new();
    for class in nodes.iter().filter(|n| n.kind == "class") {
        options.check(out.len())?;
        if options.declaration_only(&class.span.path) {
            continue;
        }
        if !is_active_record(index, &class.qualified_name) {
            continue;
        }
        for attr in collect_attributes(class, files, index, options)? {
            let id = format!("property:{}::${}", class.qualified_name, attr.name);
            if existing.contains(&id) || out.iter().any(|n: &Node| n.id == id) {
                continue;
            }
            out.push(Node {
                id,
                kind: "property".into(),
                name: format!("${}", attr.name),
                qualified_name: format!("{}::${}", class.qualified_name, attr.name),
                span: SourceSpan {
                    path: class.span.path.clone(),
                    start_line: attr.line.max(1),
                    end_line: attr.line.max(1),
                },
                metadata: json!({
                    "type_source": "framework",
                    "framework": "yii2",
                    "framework_evidence": attr.evidence,
                    "magic": true,
                    "access": "readwrite",
                    "ar_attribute": true,
                    "commented": false,
                }),
                trust: 1,
                revision: 0,
            });
        }
    }
    Ok(out)
}

pub(super) fn collect_table_links(
    files: &[SourceFile],
    graph: &GraphSnapshot,
    index: &Index<'_>,
    options: &AnalysisOptions,
) -> Result<Vec<TableLink>, String> {
    options.progress.phase(
        "analysis",
        "yii2_model_tables",
        Some(graph.nodes.iter().filter(|n| n.kind == "class").count()),
        "classes",
    );
    let mut links = Vec::new();
    for class in graph.nodes.iter().filter(|n| n.kind == "class") {
        options.check(graph.nodes.len() + graph.edges.len() + links.len())?;
        if options.declaration_only(&class.span.path) {
            continue;
        }
        if !is_active_record(index, &class.qualified_name) {
            continue;
        }
        let (schema, table) = table_name_for(class, files, index).unwrap_or_else(|| {
            let short = class
                .qualified_name
                .rsplit('\\')
                .next()
                .unwrap_or(&class.name);
            ("public".into(), camel2id(short))
        });
        let attributes = collect_attributes(class, files, index, options)?;
        links.push(TableLink {
            class_id: class.id.clone(),
            class_qn: class.qualified_name.clone(),
            class_path: class.span.path.clone(),
            class_line: class.span.start_line,
            schema,
            table,
            attributes,
        });
    }
    Ok(links)
}

pub(super) fn apply_table_links(graph: &mut GraphSnapshot, links: Vec<TableLink>) {
    for link in links {
        let table_id = format!("table:{}.{}", link.schema, link.table);
        ensure_table_node(graph, &link, &table_id);
        let maps_table = format!("edge:maps_table:{}:{table_id}", link.class_id);
        if !graph.edges.iter().any(|e| e.id == maps_table) {
            graph.edges.push(Edge {
                id: maps_table,
                kind: "maps_table".into(),
                from: link.class_id.clone(),
                to: Some(table_id.clone()),
                span: span(&link),
                metadata: json!({
                    "framework": "yii2",
                    "framework_evidence": "tableName",
                    "schema": link.schema,
                    "table": link.table,
                    "resolution": "resolved",
                }),
                trust: 1,
                revision: 0,
            });
        }
        for attr in &link.attributes {
            let prop_id = format!("property:{}::${}", link.class_qn, attr.name);
            if !graph.nodes.iter().any(|n| n.id == prop_id) {
                graph.nodes.push(Node {
                    id: prop_id.clone(),
                    kind: "property".into(),
                    name: format!("${}", attr.name),
                    qualified_name: format!("{}::${}", link.class_qn, attr.name),
                    span: SourceSpan {
                        path: link.class_path.clone(),
                        start_line: attr.line.max(1),
                        end_line: attr.line.max(1),
                    },
                    metadata: json!({
                        "type_source": "framework",
                        "framework": "yii2",
                        "framework_evidence": attr.evidence,
                        "magic": true,
                        "access": "readwrite",
                        "ar_attribute": true,
                        "commented": false,
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
            let contains_prop = format!("edge:contains:{}:{prop_id}", link.class_id);
            if !graph.edges.iter().any(|e| e.id == contains_prop) {
                graph.edges.push(Edge {
                    id: contains_prop,
                    kind: "contains".into(),
                    from: link.class_id.clone(),
                    to: Some(prop_id.clone()),
                    span: SourceSpan {
                        path: link.class_path.clone(),
                        start_line: attr.line.max(1),
                        end_line: attr.line.max(1),
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "framework_evidence": attr.evidence,
                        "resolution": "resolved",
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
            let column_id = format!("column:{}.{}.{}", link.schema, link.table, attr.name);
            if !graph.nodes.iter().any(|n| n.id == column_id) {
                graph.nodes.push(Node {
                    id: column_id.clone(),
                    kind: "column".into(),
                    name: attr.name.clone(),
                    qualified_name: format!("{}.{}.{}", link.schema, link.table, attr.name),
                    span: SourceSpan {
                        path: link.class_path.clone(),
                        start_line: attr.line.max(1),
                        end_line: attr.line.max(1),
                    },
                    metadata: json!({
                        "schema": link.schema,
                        "table": link.table,
                        "framework": "yii2",
                        "framework_evidence": attr.evidence,
                        "commented": false,
                    }),
                    trust: 1,
                    revision: 0,
                });
                let contains_col = format!("edge:contains:{table_id}:{column_id}");
                if !graph.edges.iter().any(|e| e.id == contains_col) {
                    graph.edges.push(Edge {
                        id: contains_col,
                        kind: "contains".into(),
                        from: table_id.clone(),
                        to: Some(column_id.clone()),
                        span: SourceSpan {
                            path: link.class_path.clone(),
                            start_line: attr.line.max(1),
                            end_line: attr.line.max(1),
                        },
                        metadata: json!({
                            "framework": "yii2",
                            "resolution": "resolved",
                        }),
                        trust: 1,
                        revision: 0,
                    });
                }
            }
            let maps_col = format!("edge:maps_column:{prop_id}:{column_id}");
            if !graph.edges.iter().any(|e| e.id == maps_col) {
                graph.edges.push(Edge {
                    id: maps_col,
                    kind: "maps_column".into(),
                    from: prop_id,
                    to: Some(column_id),
                    span: SourceSpan {
                        path: link.class_path.clone(),
                        start_line: attr.line.max(1),
                        end_line: attr.line.max(1),
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "framework_evidence": attr.evidence,
                        "schema": link.schema,
                        "table": link.table,
                        "column": attr.name,
                        "resolution": "resolved",
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
        }
    }
}

fn ensure_table_node(graph: &mut GraphSnapshot, link: &TableLink, table_id: &str) {
    if graph.nodes.iter().any(|n| n.id == table_id) {
        return;
    }
    let schema_id = format!("schema:{}", link.schema);
    if !graph.nodes.iter().any(|n| n.id == schema_id) {
        graph.nodes.push(Node {
            id: schema_id.clone(),
            kind: "schema".into(),
            name: link.schema.clone(),
            qualified_name: link.schema.clone(),
            span: span(link),
            metadata: json!({
                "framework": "yii2",
                "commented": false,
            }),
            trust: 1,
            revision: 0,
        });
    }
    graph.nodes.push(Node {
        id: table_id.to_owned(),
        kind: "table".into(),
        name: link.table.clone(),
        qualified_name: format!("{}.{}", link.schema, link.table),
        span: span(link),
        metadata: json!({
            "schema": link.schema,
            "framework": "yii2",
            "framework_evidence": "tableName",
            "commented": false,
        }),
        trust: 1,
        revision: 0,
    });
    let contains_id = format!("edge:contains:{schema_id}:{table_id}");
    if !graph.edges.iter().any(|e| e.id == contains_id) {
        graph.edges.push(Edge {
            id: contains_id,
            kind: "contains".into(),
            from: schema_id,
            to: Some(table_id.to_owned()),
            span: span(link),
            metadata: json!({
                "framework": "yii2",
                "resolution": "resolved",
            }),
            trust: 1,
            revision: 0,
        });
    }
}

fn span(link: &TableLink) -> SourceSpan {
    SourceSpan {
        path: link.class_path.clone(),
        start_line: link.class_line,
        end_line: link.class_line,
    }
}

fn is_active_record(index: &Index<'_>, class: &str) -> bool {
    RELATION_BASES.iter().any(|base| index.is_a(class, base)) || index.parent_unindexed(class)
}

fn collect_attributes(
    class: &Node,
    files: &[SourceFile],
    index: &Index<'_>,
    options: &AnalysisOptions,
) -> Result<Vec<AttrFact>, String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let push = |name: String,
                evidence: &'static str,
                line: usize,
                seen: &mut BTreeSet<String>,
                out: &mut Vec<AttrFact>| {
        if !is_attr_name(&name) || !seen.insert(name.clone()) {
            return;
        }
        out.push(AttrFact {
            name,
            evidence,
            line,
        });
    };
    if let Some(rules) = own_method(index, &class.qualified_name, "rules") {
        let body = method_source(rules, files);
        let base_line = method_body_line(rules, files);
        for (name, rel_line) in parse_rules_attributes(&body, &rules.metadata, options) {
            push(
                name,
                "rules",
                base_line.saturating_add(rel_line.saturating_sub(1)),
                &mut seen,
                &mut out,
            );
        }
    }
    if let Some(behaviors) = own_method(index, &class.qualified_name, "behaviors").or_else(|| {
        index
            .methods(&class.qualified_name, "behaviors")
            .into_iter()
            .next()
    }) {
        let body = method_source(behaviors, files);
        let base_line = method_body_line(behaviors, files);
        for (name, rel_line) in parse_behavior_attributes(&body, &behaviors.metadata, options) {
            push(
                name,
                "behaviors",
                base_line.saturating_add(rel_line.saturating_sub(1)),
                &mut seen,
                &mut out,
            );
        }
    }
    for (method_name, evidence) in [
        ("attributeLabels", "attribute_labels"),
        ("scenarios", "scenarios"),
    ] {
        if let Some(method) = own_method(index, &class.qualified_name, method_name) {
            let body = method_source(method, files);
            let base_line = method_body_line(method, files);
            let parsed = if method_name == "scenarios" {
                parse_scenario_attributes(&body, &method.metadata, options)
            } else {
                parse_label_attributes(&body, &method.metadata, options)
            };
            for (name, rel_line) in parsed {
                push(
                    name,
                    evidence,
                    base_line.saturating_add(rel_line.saturating_sub(1)),
                    &mut seen,
                    &mut out,
                );
            }
        }
    }
    Ok(out)
}

/// Mirror `sql_reads`/`sql_writes` onto mapped model properties (site → property).
pub(super) fn link_sql_sites_to_properties(graph: &mut GraphSnapshot) {
    let prop_for_column: BTreeMap<String, Vec<String>> = {
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in graph.edges.iter().filter(|e| e.kind == "maps_column") {
            if let Some(col) = edge.to.clone() {
                map.entry(col).or_default().push(edge.from.clone());
            }
        }
        map
    };
    let mut extras = Vec::new();
    for edge in graph
        .edges
        .iter()
        .filter(|e| matches!(e.kind.as_str(), "sql_reads" | "sql_writes"))
    {
        let Some(col) = edge.to.as_deref() else {
            continue;
        };
        if !col.starts_with("column:") {
            continue;
        }
        let Some(props) = prop_for_column.get(col) else {
            continue;
        };
        for prop in props {
            let id = format!("edge:{}:{}:{}:prop", edge.kind, edge.from, prop);
            if graph.edges.iter().any(|e| e.id == id) {
                continue;
            }
            let mut metadata = edge.metadata.clone();
            metadata["maps_column"] = json!(col);
            metadata["framework_evidence"] = json!("maps_column");
            extras.push(Edge {
                id,
                kind: edge.kind.clone(),
                from: edge.from.clone(),
                to: Some(prop.clone()),
                span: edge.span.clone(),
                metadata,
                trust: edge.trust,
                revision: edge.revision,
            });
        }
    }
    graph.edges.extend(extras);
}

/// After `maps_table`, promote DDL/`sql` columns on the table into model attrs.
pub(super) fn apply_ddl_column_links(graph: &mut GraphSnapshot) {
    let maps: Vec<(String, String, String, usize)> = graph
        .edges
        .iter()
        .filter(|e| e.kind == "maps_table")
        .filter_map(|e| {
            Some((
                e.from.clone(),
                e.to.clone()?,
                e.span.path.clone(),
                e.span.start_line,
            ))
        })
        .collect();
    for (class_id, table_id, path, line) in maps {
        let class_qn = class_id
            .strip_prefix("class:")
            .unwrap_or(class_id.as_str())
            .to_owned();
        let columns: Vec<(String, String)> = graph
            .edges
            .iter()
            .filter(|e| e.kind == "contains" && e.from == table_id)
            .filter_map(|e| {
                let col_id = e.to.as_deref()?;
                let node = graph.nodes.iter().find(|n| n.id == col_id)?;
                (node.kind == "column").then(|| (col_id.to_owned(), node.name.clone()))
            })
            .collect();
        for (column_id, attr) in columns {
            if !is_attr_name(&attr) {
                continue;
            }
            let prop_id = format!("property:{class_qn}::${attr}");
            if !graph.nodes.iter().any(|n| n.id == prop_id) {
                graph.nodes.push(Node {
                    id: prop_id.clone(),
                    kind: "property".into(),
                    name: format!("${attr}"),
                    qualified_name: format!("{class_qn}::${attr}"),
                    span: SourceSpan {
                        path: path.clone(),
                        start_line: line,
                        end_line: line,
                    },
                    metadata: json!({
                        "type_source": "framework",
                        "framework": "yii2",
                        "framework_evidence": "ddl_column",
                        "magic": true,
                        "access": "readwrite",
                        "ar_attribute": true,
                        "commented": false,
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
            let contains_prop = format!("edge:contains:{class_id}:{prop_id}");
            if !graph.edges.iter().any(|e| e.id == contains_prop) {
                graph.edges.push(Edge {
                    id: contains_prop,
                    kind: "contains".into(),
                    from: class_id.clone(),
                    to: Some(prop_id.clone()),
                    span: SourceSpan {
                        path: path.clone(),
                        start_line: line,
                        end_line: line,
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "framework_evidence": "ddl_column",
                        "resolution": "resolved",
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
            let maps_col = format!("edge:maps_column:{prop_id}:{column_id}");
            if !graph.edges.iter().any(|e| e.id == maps_col) {
                graph.edges.push(Edge {
                    id: maps_col,
                    kind: "maps_column".into(),
                    from: prop_id,
                    to: Some(column_id),
                    span: SourceSpan {
                        path: path.clone(),
                        start_line: line,
                        end_line: line,
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "framework_evidence": "ddl_column",
                        "column": attr,
                        "resolution": "resolved",
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
        }
    }
}

fn own_method<'a>(index: &'a Index<'_>, class: &str, name: &str) -> Option<&'a Node> {
    index.methods(class, name).into_iter().find(|m| {
        m.qualified_name
            .rsplit_once("::")
            .is_some_and(|(owner, _)| owner == class)
    })
}

fn method_body_line(method: &Node, files: &[SourceFile]) -> usize {
    // Prefer span start; absolute lines for AttrFact are approximate enough for outline.
    let _ = files;
    method.span.start_line.max(1)
}

fn parse_label_attributes(
    body: &str,
    metadata: &serde_json::Value,
    options: &AnalysisOptions,
) -> Vec<(String, usize)> {
    // attributeLabels(): keys are attribute names.
    parse_return_array_keys(body, metadata, options)
}

fn parse_scenario_attributes(
    body: &str,
    metadata: &serde_json::Value,
    options: &AnalysisOptions,
) -> Vec<(String, usize)> {
    // scenarios(): values are lists of attrs (optional leading '!').
    let Ok(tokens) = syntax::fragment(body, options) else {
        return Vec::new();
    };
    let names = Names::metadata(metadata);
    let mut out = Vec::new();
    let mut block_depth = 0usize;
    for (at, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "{" => block_depth += 1,
                "}" => block_depth = block_depth.saturating_sub(1),
                _ => {}
            }
        }
        if !token.is("return") || block_depth != 0 {
            continue;
        }
        let end = expression_end(&tokens, at + 1);
        let Ok(expr) = syntax::expression(&tokens[at + 1..end], &names, options) else {
            continue;
        };
        let Expression::Array(items) = expr else {
            continue;
        };
        for (_, value) in items {
            collect_scenario_attrs(&value, token.line, &mut out);
        }
    }
    out
}

fn collect_scenario_attrs(value: &Expression, line: usize, out: &mut Vec<(String, usize)>) {
    match value {
        Expression::Text(name) => {
            let name = name.trim_start_matches('!');
            if is_attr_name(name) {
                out.push((name.to_owned(), line));
            }
        }
        Expression::Array(items) => {
            for (_, v) in items {
                collect_scenario_attrs(v, line, out);
            }
        }
        _ => {}
    }
}

fn parse_return_array_keys(
    body: &str,
    metadata: &serde_json::Value,
    options: &AnalysisOptions,
) -> Vec<(String, usize)> {
    let Ok(tokens) = syntax::fragment(body, options) else {
        return Vec::new();
    };
    let names = Names::metadata(metadata);
    let mut out = Vec::new();
    let mut block_depth = 0usize;
    for (at, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "{" => block_depth += 1,
                "}" => block_depth = block_depth.saturating_sub(1),
                _ => {}
            }
        }
        if !token.is("return") || block_depth != 0 {
            continue;
        }
        let end = expression_end(&tokens, at + 1);
        let Ok(expr) = syntax::expression(&tokens[at + 1..end], &names, options) else {
            continue;
        };
        let Expression::Array(items) = expr else {
            continue;
        };
        for (key, _) in items {
            if let Some(name) = key {
                if is_attr_name(&name) {
                    out.push((name, token.line));
                }
            }
        }
    }
    out
}

fn parse_rules_attributes(
    body: &str,
    metadata: &serde_json::Value,
    options: &AnalysisOptions,
) -> Vec<(String, usize)> {
    let Ok(tokens) = syntax::fragment(body, options) else {
        return Vec::new();
    };
    let names = Names::metadata(metadata);
    let mut out = Vec::new();
    let mut block_depth = 0usize;
    for (at, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "{" => block_depth += 1,
                "}" => block_depth = block_depth.saturating_sub(1),
                _ => {}
            }
        }
        if !token.is("return") || block_depth != 0 {
            continue;
        }
        let end = expression_end(&tokens, at + 1);
        let Ok(expr) = syntax::expression(&tokens[at + 1..end], &names, options) else {
            continue;
        };
        let Expression::Array(rules) = expr else {
            continue;
        };
        for (_, rule) in rules {
            let Expression::Array(parts) = rule else {
                continue;
            };
            let Some((_, first)) = parts.iter().find(|(k, _)| k.is_none()) else {
                continue;
            };
            match first {
                Expression::Text(name) if is_attr_name(name) => {
                    out.push((name.clone(), token.line));
                }
                Expression::Array(attrs) => {
                    for (k, v) in attrs {
                        if k.is_some() {
                            continue;
                        }
                        if let Expression::Text(name) = v {
                            if is_attr_name(name) {
                                out.push((name.clone(), token.line));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

const BEHAVIOR_ATTR_KEYS: &[&str] = &[
    "createdAtAttribute",
    "updatedAtAttribute",
    "createdByAttribute",
    "updatedByAttribute",
    "deletedAtAttribute",
    "softDeleteAttribute",
    "attribute",
    "slugAttribute",
    "valueAttribute",
    "attributes",
];

fn parse_behavior_attributes(
    body: &str,
    metadata: &serde_json::Value,
    options: &AnalysisOptions,
) -> Vec<(String, usize)> {
    let Ok(tokens) = syntax::fragment(body, options) else {
        return Vec::new();
    };
    let names = Names::metadata(metadata);
    let mut out = Vec::new();
    let mut block_depth = 0usize;
    for (at, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "{" => block_depth += 1,
                "}" => block_depth = block_depth.saturating_sub(1),
                _ => {}
            }
        }
        if !token.is("return") || block_depth != 0 {
            continue;
        }
        let end = expression_end(&tokens, at + 1);
        let Ok(expr) = syntax::expression(&tokens[at + 1..end], &names, options) else {
            continue;
        };
        let Expression::Array(items) = expr else {
            continue;
        };
        for (_, item) in items {
            collect_behavior_item(&item, token.line, &mut out);
        }
    }
    out
}

fn collect_behavior_item(item: &Expression, line: usize, out: &mut Vec<(String, usize)>) {
    match item {
        Expression::Class(name) | Expression::Text(name) => {
            for def in behavior_defaults(name) {
                out.push(((*def).to_owned(), line));
            }
        }
        Expression::Array(cfg) => {
            let class_names = super::syntax::configured_class(item).names;
            let mut explicit = false;
            for (key, value) in cfg {
                let Some(key) = key.as_deref() else {
                    continue;
                };
                if !BEHAVIOR_ATTR_KEYS
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(key))
                {
                    continue;
                }
                explicit = true;
                push_attr_values(value, line, out);
            }
            if !explicit {
                for class in &class_names {
                    for def in behavior_defaults(class) {
                        out.push(((*def).to_owned(), line));
                    }
                }
            }
        }
        _ => {}
    }
}

fn push_attr_values(value: &Expression, line: usize, out: &mut Vec<(String, usize)>) {
    match value {
        Expression::Text(name) if is_attr_name(name) => out.push((name.clone(), line)),
        Expression::Array(items) => {
            for (key, v) in items {
                // event => 'attr' | ['a','b'] or list of attrs
                if key.is_some() {
                    push_attr_values(v, line, out);
                } else if let Expression::Text(name) = v {
                    if is_attr_name(name) {
                        out.push((name.clone(), line));
                    }
                } else {
                    push_attr_values(v, line, out);
                }
            }
        }
        _ => {}
    }
}

fn behavior_defaults(class: &str) -> &'static [&'static str] {
    let short = class.rsplit('\\').next().unwrap_or(class);
    let lower = short.to_ascii_lowercase();
    if lower.contains("timestampbehavior") {
        &["created_at", "updated_at"]
    } else if lower.contains("blameablebehavior") {
        &["created_by", "updated_by"]
    } else if lower.contains("softdelete") {
        &["deleted_at"]
    } else {
        &[]
    }
}

fn expression_end(tokens: &[crate::plugins::php::tokens::Token], mut at: usize) -> usize {
    let mut depth = 0usize;
    while at < tokens.len() {
        match tokens[at].text.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => {
                if depth == 0 {
                    return at;
                }
                depth = depth.saturating_sub(1);
            }
            "," | ";" if depth == 0 => return at,
            _ => {}
        }
        at += 1;
    }
    tokens.len()
}

fn table_name_for(
    class: &Node,
    files: &[SourceFile],
    index: &Index<'_>,
) -> Option<(String, String)> {
    let method = own_method(index, &class.qualified_name, "tableName")?;
    let body = method_source(method, files);
    parse_table_name_return(&body)
}

fn parse_table_name_return(body: &str) -> Option<(String, String)> {
    let trimmed = body.trim();
    let after_return = trimmed
        .find("return")
        .map(|i| trimmed[i + "return".len()..].trim_start())?;
    let lit = string_literal(after_return)?;
    normalize_yii_table(&lit)
}

fn string_literal(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let quote = *bytes.first()?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                out.push(quote as char);
                i += 2;
                continue;
            }
            return Some(out);
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    None
}

fn normalize_yii_table(raw: &str) -> Option<(String, String)> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains("{{") {
        let parts: Vec<String> = s
            .split('.')
            .map(|part| {
                part.trim()
                    .trim_start_matches("{{")
                    .trim_end_matches("}}")
                    .trim_start_matches('%')
                    .to_owned()
            })
            .filter(|p| !p.is_empty())
            .collect();
        return match parts.as_slice() {
            [table] => Some(("public".into(), table.clone())),
            [schema, table] => Some((schema.clone(), table.clone())),
            _ => None,
        };
    }
    if let Some((schema, table)) = s.split_once('.') {
        return Some((schema.to_owned(), table.to_owned()));
    }
    Some(("public".into(), s.to_owned()))
}

fn camel2id(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            for lower in c.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn is_attr_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !matches!(
            name.to_ascii_lowercase().as_str(),
            "required"
                | "string"
                | "integer"
                | "number"
                | "boolean"
                | "email"
                | "url"
                | "date"
                | "datetime"
                | "time"
                | "safe"
                | "unique"
                | "exist"
                | "compare"
                | "in"
                | "match"
                | "filter"
                | "trim"
                | "default"
                | "captcha"
                | "file"
                | "image"
                | "each"
                | "ip"
                | "class"
                | "on"
                | "except"
                | "when"
                | "skiponempty"
                | "skiponerror"
                | "message"
                | "targetclass"
                | "targetattribute"
        )
}
