//! Extract table references from PostgreSQL SQL text (DDL and DML).
//! Used for `.sql` files and for SQL string literals inside PHP.
use super::tokens::{self, Token, TokenKind};
use crate::analyzer::AnalysisOptions;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    Read,
    Write,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRef {
    pub schema: String,
    pub name: String,
    pub effect: Effect,
}
impl TableRef {
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
    pub fn id(&self) -> String {
        format!("table:{}", self.qualified())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlOutline {
    pub tables: Vec<TableRef>,
    pub operation: Option<String>,
}

/// True when the string looks like a SQL statement worth outlining.
pub fn looks_like_sql(source: &str) -> bool {
    let trimmed = source
        .trim_start()
        .trim_start_matches('(')
        .trim_start()
        .to_ascii_lowercase();
    [
        "select", "insert", "update", "delete", "with", "create", "alter", "truncate", "drop",
    ]
    .iter()
    .any(|kw| trimmed.starts_with(kw))
}

pub fn outline(source: &str) -> Result<SqlOutline, String> {
    let options = AnalysisOptions::default();
    let tokens = tokens::tokenize(source, &options)?;
    Ok(outline_tokens(&tokens))
}

fn outline_tokens(tokens: &[Token]) -> SqlOutline {
    let mut tables = Vec::new();
    let mut ctes = std::collections::BTreeSet::new();
    let mut operation = None;
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].is("with") {
            i = collect_ctes(tokens, i + 1, &mut ctes);
            continue;
        }
        if tokens[i].is("select") {
            operation.get_or_insert_with(|| "select".into());
            i = scan_from_join(tokens, i + 1, Effect::Read, &ctes, &mut tables);
            continue;
        }
        if tokens[i].is("insert") {
            operation.get_or_insert_with(|| "insert".into());
            i += 1;
            if tokens.get(i).is_some_and(|t| t.is("into")) {
                i += 1;
            }
            if let Some(name) = take_qualified(tokens, &mut i) {
                push_table(&mut tables, name, Effect::Write, &ctes);
            }
            // INSERT … SELECT …
            continue;
        }
        if tokens[i].is("update") {
            operation.get_or_insert_with(|| "update".into());
            i += 1;
            // UPDATE ONLY name
            if tokens.get(i).is_some_and(|t| t.is("only")) {
                i += 1;
            }
            if let Some(name) = take_qualified(tokens, &mut i) {
                push_table(&mut tables, name, Effect::Write, &ctes);
            }
            // UPDATE … FROM other
            i = scan_from_join(tokens, i, Effect::Read, &ctes, &mut tables);
            continue;
        }
        if tokens[i].is("delete") {
            operation.get_or_insert_with(|| "delete".into());
            i += 1;
            if tokens.get(i).is_some_and(|t| t.is("from")) {
                i += 1;
            }
            if tokens.get(i).is_some_and(|t| t.is("only")) {
                i += 1;
            }
            if let Some(name) = take_qualified(tokens, &mut i) {
                push_table(&mut tables, name, Effect::Write, &ctes);
            }
            i = scan_from_join(tokens, i, Effect::Read, &ctes, &mut tables);
            continue;
        }
        if tokens[i].is("truncate") {
            operation.get_or_insert_with(|| "truncate".into());
            i += 1;
            if tokens.get(i).is_some_and(|t| t.is("table")) {
                i += 1;
            }
            while i < tokens.len() {
                if tokens[i].is(";") {
                    break;
                }
                if let Some(name) = take_qualified(tokens, &mut i) {
                    push_table(&mut tables, name, Effect::Write, &ctes);
                } else {
                    i += 1;
                }
                if tokens.get(i).is_some_and(|t| t.text == ",") {
                    i += 1;
                } else if !tokens.get(i).is_some_and(|t| t.is("only")) {
                    break;
                }
            }
            continue;
        }
        if tokens[i].is("create")
            || tokens[i].is("alter")
            || tokens[i].is("drop")
            || tokens[i].is("from")
            || tokens[i].is("join")
        {
            // FROM/JOIN also appear mid-statement after we jumped; handled in scan.
            if tokens[i].is("from") || tokens[i].is("join") {
                let effect = Effect::Read;
                i = scan_relation_list(tokens, i + 1, effect, &ctes, &mut tables);
                continue;
            }
            if operation.is_none() {
                operation = tokens[i].as_ident().map(str::to_owned);
            }
        }
        i += 1;
    }
    tables.sort_by(|a, b| (&a.schema, &a.name, &a.effect).cmp(&(&b.schema, &b.name, &b.effect)));
    tables.dedup();
    SqlOutline { tables, operation }
}

fn collect_ctes(
    tokens: &[Token],
    mut i: usize,
    ctes: &mut std::collections::BTreeSet<String>,
) -> usize {
    loop {
        let Some(name) = tokens.get(i).and_then(Token::as_ident).map(str::to_owned) else {
            return i;
        };
        ctes.insert(name.to_ascii_lowercase());
        i += 1;
        if tokens.get(i).is_some_and(|t| t.text == "(") {
            i = skip_parens(tokens, i);
        }
        if tokens.get(i).is_some_and(|t| t.is("as")) {
            i += 1;
        }
        if tokens.get(i).is_some_and(|t| t.text == "(") {
            i = skip_parens(tokens, i);
        }
        if tokens.get(i).is_some_and(|t| t.text == ",") {
            i += 1;
            continue;
        }
        return i;
    }
}

fn scan_from_join(
    tokens: &[Token],
    mut i: usize,
    effect: Effect,
    ctes: &std::collections::BTreeSet<String>,
    tables: &mut Vec<TableRef>,
) -> usize {
    while i < tokens.len() {
        if tokens[i].is(";") {
            return i;
        }
        if tokens[i].is("from")
            || tokens[i].is("join")
            || (tokens[i].is("inner")
                || tokens[i].is("left")
                || tokens[i].is("right")
                || tokens[i].is("full")
                || tokens[i].is("cross"))
                && tokens.get(i + 1).is_some_and(|t| t.is("join"))
        {
            while i < tokens.len()
                && (tokens[i].is("inner")
                    || tokens[i].is("left")
                    || tokens[i].is("right")
                    || tokens[i].is("full")
                    || tokens[i].is("cross")
                    || tokens[i].is("outer"))
            {
                i += 1;
            }
            if tokens.get(i).is_some_and(|t| t.is("join") || t.is("from")) {
                i += 1;
            }
            i = scan_relation_list(tokens, i, effect, ctes, tables);
            continue;
        }
        i += 1;
    }
    i
}

fn scan_relation_list(
    tokens: &[Token],
    mut i: usize,
    effect: Effect,
    ctes: &std::collections::BTreeSet<String>,
    tables: &mut Vec<TableRef>,
) -> usize {
    if tokens.get(i).is_some_and(|t| t.text == "(") {
        // Subquery in FROM: descend for nested FROM/JOIN only.
        let end = skip_parens(tokens, i);
        let _ = scan_from_join(tokens, i + 1, effect, ctes, tables);
        return end;
    }
    if let Some(name) = take_qualified(tokens, &mut i) {
        push_table(tables, name, effect, ctes);
        // optional alias
        if tokens.get(i).is_some_and(|t| t.is("as")) {
            i += 1;
        }
        if tokens
            .get(i)
            .is_some_and(|t| t.kind == TokenKind::Ident && !join_stop(t))
        {
            i += 1;
        }
    }
    i
}

fn join_stop(t: &Token) -> bool {
    t.is("on")
        || t.is("using")
        || t.is("where")
        || t.is("group")
        || t.is("order")
        || t.is("limit")
        || t.is("offset")
        || t.is("returning")
        || t.is("set")
        || t.is("join")
        || t.is("inner")
        || t.is("left")
        || t.is("right")
        || t.is("full")
        || t.is("cross")
        || t.is("union")
        || t.is("except")
        || t.is("intersect")
        || t.is("having")
        || t.is(";")
        || t.text == ","
        || t.text == ")"
}

fn take_qualified(tokens: &[Token], i: &mut usize) -> Option<(String, String)> {
    let first = tokens.get(*i).and_then(Token::as_ident)?.to_string();
    *i += 1;
    if tokens.get(*i).is_some_and(|t| t.text == ".") {
        *i += 1;
        let second = tokens.get(*i).and_then(Token::as_ident)?.to_string();
        *i += 1;
        Some((first, second))
    } else {
        Some(("public".into(), first))
    }
}

fn push_table(
    tables: &mut Vec<TableRef>,
    (schema, name): (String, String),
    effect: Effect,
    ctes: &std::collections::BTreeSet<String>,
) {
    if ctes.contains(&name.to_ascii_lowercase()) {
        return;
    }
    // Skip obvious non-tables (SELECT pg_stat… / functions without FROM).
    if schema == "public" && name.contains('(') {
        return;
    }
    tables.push(TableRef {
        schema,
        name,
        effect,
    });
}

fn skip_parens(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < tokens.len() {
        match tokens[i].text.as_str() {
            "(" => depth += 1,
            ")" => {
                depth = depth.saturating_sub(1);
                i += 1;
                if depth == 0 {
                    return i;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    i
}
