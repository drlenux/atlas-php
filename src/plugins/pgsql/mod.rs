//! Native PostgreSQL/SQL analysis: `.sql` files and shared outline for SQL-in-PHP.
mod query;
mod syntax;
pub mod tokens;

pub use query::{looks_like_sql, outline, Effect, SqlOutline, TableRef};

use crate::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::{Edge, GraphSnapshot, Node},
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use syntax::Declaration;

pub struct PgsqlLanguagePlugin;

impl LanguagePlugin for PgsqlLanguagePlugin {
    fn id(&self) -> &'static str {
        "pgsql"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["sql", "pgsql"]
    }
    fn analyze(
        &self,
        files: &[SourceFile],
        options: &AnalysisOptions,
    ) -> Result<GraphSnapshot, String> {
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();
        let mut diagnostics = Vec::new();
        options
            .progress
            .phase("analysis", "pgsql_parse", Some(files.len()), "files");
        for file in files {
            options.progress.work_started(&file.path);
            match syntax::parse_file(file, options) {
                Ok(unit) => {
                    diagnostics.extend(unit.diagnostics.clone());
                    build_declarations(&unit, options, &mut nodes, &mut edges)?;
                    // DML statements that are not CREATE still contribute table refs.
                    if let Ok(outline) = outline(&file.source) {
                        for table in &outline.tables {
                            ensure_table(table, &file.path, 1, options, &mut nodes, &mut edges)?;
                        }
                    }
                }
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
            options.progress.work_finished(&file.path);
            options.check(nodes.len() + edges.len() + diagnostics.len())?;
        }
        Ok(GraphSnapshot {
            nodes: nodes.into_values().collect(),
            edges: edges.into_values().collect(),
            diagnostics,
        })
    }
}

/// Build graph fragments for one SQL string embedded in PHP (or elsewhere).
pub fn embed_sql(
    sql: &str,
    path: &str,
    start_line: usize,
    end_line: usize,
    container_id: &str,
    options: &AnalysisOptions,
) -> Result<GraphSnapshot, String> {
    if !looks_like_sql(sql) {
        return Ok(GraphSnapshot::default());
    }
    let parsed = outline(sql).unwrap_or_else(|_| SqlOutline {
        tables: Vec::new(),
        operation: None,
    });
    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let fp = tokens::fingerprint([container_id, sql, &start_line.to_string()]);
    let site_id = format!("sql_site:{container_id}:{fp}");
    nodes.insert(
        site_id.clone(),
        Node {
            id: site_id.clone(),
            kind: "sql_site".into(),
            name: parsed
                .operation
                .clone()
                .unwrap_or_else(|| "sql".into()),
            qualified_name: site_id.clone(),
            span: crate::domain::SourceSpan {
                path: path.into(),
                start_line,
                end_line,
            },
            metadata: json!({
                "fingerprint": fp,
                "sql": sql,
                "operation": parsed.operation,
                "commented": false
            }),
            trust: 1,
            revision: 0,
        },
    );
    let contains_id = format!(
        "edge:{}",
        tokens::fingerprint(["contains", container_id, &site_id, &fp])
    );
    edges.insert(
        contains_id.clone(),
        Edge {
            id: contains_id,
            kind: "contains".into(),
            from: container_id.into(),
            to: Some(site_id.clone()),
            span: crate::domain::SourceSpan {
                path: path.into(),
                start_line,
                end_line,
            },
            metadata: json!({"fingerprint": fp, "resolution": "resolved"}),
            trust: 1,
            revision: 0,
        },
    );
    for table in &parsed.tables {
        ensure_table(table, path, start_line, options, &mut nodes, &mut edges)?;
        let kind = match table.effect {
            Effect::Read => "sql_reads",
            Effect::Write => "sql_writes",
        };
        let edge_id = format!(
            "edge:{}",
            tokens::fingerprint([kind, &site_id, &table.id(), &fp])
        );
        edges.insert(
            edge_id.clone(),
            Edge {
                id: edge_id,
                kind: kind.into(),
                from: site_id.clone(),
                to: Some(table.id()),
                span: crate::domain::SourceSpan {
                    path: path.into(),
                    start_line,
                    end_line,
                },
                metadata: json!({
                    "fingerprint": fp,
                    "resolution": "resolved",
                    "table": table.qualified(),
                    "effect": match table.effect {
                        Effect::Read => "read",
                        Effect::Write => "write",
                    }
                }),
                trust: 1,
                revision: 0,
            },
        );
    }
    options.check(nodes.len() + edges.len())?;
    Ok(GraphSnapshot {
        nodes: nodes.into_values().collect(),
        edges: edges.into_values().collect(),
        diagnostics: Vec::new(),
    })
}

fn build_declarations(
    unit: &syntax::Unit,
    options: &AnalysisOptions,
    nodes: &mut BTreeMap<String, Node>,
    edges: &mut BTreeMap<String, Edge>,
) -> Result<(), String> {
    let mut schemas = BTreeSet::new();
    for decl in &unit.declarations {
        options.check(nodes.len() + edges.len())?;
        match decl {
            Declaration::Schema { name, start, end } => {
                ensure_schema(name, unit, *start, *end, nodes)?;
                schemas.insert(name.clone());
            }
            Declaration::Table {
                name,
                columns,
                start,
                end,
            } => {
                ensure_schema(&name.schema, unit, *start, *end, nodes)?;
                schemas.insert(name.schema.clone());
                let table_id = format!("table:{}", name.display());
                nodes.insert(
                    table_id.clone(),
                    Node {
                        id: table_id.clone(),
                        kind: "table".into(),
                        name: name.name.clone(),
                        qualified_name: name.display(),
                        span: unit.span(*start, *end),
                        metadata: json!({
                            "fingerprint": unit.fp(*start, *end),
                            "schema": name.schema,
                            "commented": false
                        }),
                        trust: 1,
                        revision: 0,
                    },
                );
                link(
                    edges,
                    "contains",
                    &format!("schema:{}", name.schema),
                    &table_id,
                    unit,
                    *start,
                    *end,
                )?;
                for column in columns {
                    let column_id = format!("column:{}.{}", name.display(), column.name);
                    nodes.insert(
                        column_id.clone(),
                        Node {
                            id: column_id.clone(),
                            kind: "column".into(),
                            name: column.name.clone(),
                            qualified_name: format!("{}.{}", name.display(), column.name),
                            span: unit.span(column.token, column.end.max(column.token + 1)),
                            metadata: json!({
                                "fingerprint": unit.fp(column.token, column.end.max(column.token + 1)),
                                "type": column.type_name,
                                "commented": false
                            }),
                            trust: 1,
                            revision: 0,
                        },
                    );
                    link(
                        edges,
                        "contains",
                        &table_id,
                        &column_id,
                        unit,
                        column.token,
                        column.end.max(column.token + 1),
                    )?;
                    if let Some(target) = &column.references {
                        let target_id = format!("table:{}", target.display());
                        if !nodes.contains_key(&target_id) {
                            nodes.insert(
                                target_id.clone(),
                                Node {
                                    id: target_id.clone(),
                                    kind: "table".into(),
                                    name: target.name.clone(),
                                    qualified_name: target.display(),
                                    span: unit.span(column.token, column.end.max(column.token + 1)),
                                    metadata: json!({
                                        "fingerprint": tokens::fingerprint([&target.display()]),
                                        "schema": target.schema,
                                        "external": true,
                                        "commented": false
                                    }),
                                    trust: 1,
                                    revision: 0,
                                },
                            );
                        }
                        link(
                            edges,
                            "references",
                            &column_id,
                            &target_id,
                            unit,
                            column.token,
                            column.end.max(column.token + 1),
                        )?;
                    }
                }
            }
            Declaration::View { name, start, end } => {
                ensure_schema(&name.schema, unit, *start, *end, nodes)?;
                let id = format!("view:{}", name.display());
                nodes.insert(
                    id.clone(),
                    Node {
                        id: id.clone(),
                        kind: "view".into(),
                        name: name.name.clone(),
                        qualified_name: name.display(),
                        span: unit.span(*start, *end),
                        metadata: json!({
                            "fingerprint": unit.fp(*start, *end),
                            "schema": name.schema,
                            "commented": false
                        }),
                        trust: 1,
                        revision: 0,
                    },
                );
                link(
                    edges,
                    "contains",
                    &format!("schema:{}", name.schema),
                    &id,
                    unit,
                    *start,
                    *end,
                )?;
            }
            Declaration::Function { name, start, end } => {
                ensure_schema(&name.schema, unit, *start, *end, nodes)?;
                let id = format!("function:{}", name.display());
                nodes.insert(
                    id.clone(),
                    Node {
                        id: id.clone(),
                        kind: "function".into(),
                        name: name.name.clone(),
                        qualified_name: name.display(),
                        span: unit.span(*start, *end),
                        metadata: json!({
                            "fingerprint": unit.fp(*start, *end),
                            "schema": name.schema,
                            "commented": false
                        }),
                        trust: 1,
                        revision: 0,
                    },
                );
                link(
                    edges,
                    "contains",
                    &format!("schema:{}", name.schema),
                    &id,
                    unit,
                    *start,
                    *end,
                )?;
            }
            Declaration::Index {
                name,
                table,
                start,
                end,
            } => {
                ensure_schema(&name.schema, unit, *start, *end, nodes)?;
                let id = format!("index:{}", name.display());
                nodes.insert(
                    id.clone(),
                    Node {
                        id: id.clone(),
                        kind: "index".into(),
                        name: name.name.clone(),
                        qualified_name: name.display(),
                        span: unit.span(*start, *end),
                        metadata: json!({
                            "fingerprint": unit.fp(*start, *end),
                            "schema": name.schema,
                            "table": table.as_ref().map(|t| t.display()),
                            "commented": false
                        }),
                        trust: 1,
                        revision: 0,
                    },
                );
                if let Some(table) = table {
                    let table_id = format!("table:{}", table.display());
                    link(edges, "indexes", &id, &table_id, unit, *start, *end)?;
                }
            }
        }
    }
    let _ = schemas;
    Ok(())
}

fn ensure_schema(
    name: &str,
    unit: &syntax::Unit,
    start: usize,
    end: usize,
    nodes: &mut BTreeMap<String, Node>,
) -> Result<(), String> {
    let id = format!("schema:{name}");
    if nodes.contains_key(&id) {
        return Ok(());
    }
    nodes.insert(
        id.clone(),
        Node {
            id,
            kind: "schema".into(),
            name: name.into(),
            qualified_name: name.into(),
            span: unit.span(start, end),
            metadata: json!({
                "fingerprint": tokens::fingerprint([name]),
                "commented": false
            }),
            trust: 1,
            revision: 0,
        },
    );
    Ok(())
}

fn ensure_table(
    table: &TableRef,
    path: &str,
    line: usize,
    options: &AnalysisOptions,
    nodes: &mut BTreeMap<String, Node>,
    edges: &mut BTreeMap<String, Edge>,
) -> Result<(), String> {
    let schema_id = format!("schema:{}", table.schema);
    if !nodes.contains_key(&schema_id) {
        nodes.insert(
            schema_id.clone(),
            Node {
                id: schema_id.clone(),
                kind: "schema".into(),
                name: table.schema.clone(),
                qualified_name: table.schema.clone(),
                span: crate::domain::SourceSpan {
                    path: path.into(),
                    start_line: line,
                    end_line: line,
                },
                metadata: json!({
                    "fingerprint": tokens::fingerprint([&table.schema]),
                    "commented": false
                }),
                trust: 1,
                revision: 0,
            },
        );
    }
    let table_id = table.id();
    if !nodes.contains_key(&table_id) {
        nodes.insert(
            table_id.clone(),
            Node {
                id: table_id.clone(),
                kind: "table".into(),
                name: table.name.clone(),
                qualified_name: table.qualified(),
                span: crate::domain::SourceSpan {
                    path: path.into(),
                    start_line: line,
                    end_line: line,
                },
                metadata: json!({
                    "fingerprint": tokens::fingerprint([&table.qualified()]),
                    "schema": table.schema,
                    "commented": false
                }),
                trust: 1,
                revision: 0,
            },
        );
        let edge_id = format!(
            "edge:{}",
            tokens::fingerprint(["contains", &schema_id, &table_id, ""])
        );
        edges.insert(
            edge_id.clone(),
            Edge {
                id: edge_id,
                kind: "contains".into(),
                from: schema_id,
                to: Some(table_id),
                span: crate::domain::SourceSpan {
                    path: path.into(),
                    start_line: line,
                    end_line: line,
                },
                metadata: json!({"fingerprint": tokens::fingerprint([&table.qualified()]), "resolution": "resolved"}),
                trust: 1,
                revision: 0,
            },
        );
    }
    options.check(nodes.len() + edges.len())?;
    Ok(())
}

fn link(
    edges: &mut BTreeMap<String, Edge>,
    kind: &str,
    from: &str,
    to: &str,
    unit: &syntax::Unit,
    start: usize,
    end: usize,
) -> Result<(), String> {
    let fp = unit.fp(start, end);
    let id = format!("edge:{}", tokens::fingerprint([kind, from, to, &fp]));
    edges.insert(
        id.clone(),
        Edge {
            id,
            kind: kind.into(),
            from: from.into(),
            to: Some(to.into()),
            span: unit.span(start, end),
            metadata: json!({"fingerprint": fp, "resolution": "resolved"}),
            trust: 1,
            revision: 0,
        },
    );
    Ok(())
}
