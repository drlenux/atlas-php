//! Extract table and column references from PostgreSQL SQL text (DDL and DML).
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
pub struct ColumnRef {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub effect: Effect,
}
impl ColumnRef {
    pub fn qualified_table(&self) -> String {
        format!("{}.{}", self.schema, self.table)
    }
    pub fn id(&self) -> String {
        format!("column:{}.{}", self.qualified_table(), self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlOutline {
    pub tables: Vec<TableRef>,
    pub columns: Vec<ColumnRef>,
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
    let mut columns = Vec::new();
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
            let select_at = i;
            i = scan_from_join(tokens, i + 1, Effect::Read, &ctes, &mut tables);
            let primary = primary_table(&tables, Effect::Read);
            collect_select_columns(tokens, select_at + 1, primary.as_ref(), &mut columns);
            collect_predicate_columns(tokens, select_at + 1, primary.as_ref(), &mut columns);
            continue;
        }
        if tokens[i].is("insert") {
            operation.get_or_insert_with(|| "insert".into());
            i += 1;
            if tokens.get(i).is_some_and(|t| t.is("into")) {
                i += 1;
            }
            if let Some(name) = take_qualified(tokens, &mut i) {
                push_table(&mut tables, name.clone(), Effect::Write, &ctes);
                if tokens.get(i).is_some_and(|t| t.text == "(") {
                    i = collect_insert_columns(tokens, i + 1, &name, &mut columns);
                }
            }
            continue;
        }
        if tokens[i].is("update") {
            operation.get_or_insert_with(|| "update".into());
            let update_at = i;
            i += 1;
            if tokens.get(i).is_some_and(|t| t.is("only")) {
                i += 1;
            }
            let target = take_qualified(tokens, &mut i);
            if let Some(ref name) = target {
                push_table(&mut tables, name.clone(), Effect::Write, &ctes);
            }
            if tokens.get(i).is_some_and(|t| t.is("set")) {
                i += 1;
                if let Some(ref name) = target {
                    i = collect_update_set(tokens, i, name, &mut columns);
                }
            }
            i = scan_from_join(tokens, i, Effect::Read, &ctes, &mut tables);
            let primary = target.as_ref().map(|(schema, name)| TableRef {
                schema: schema.clone(),
                name: name.clone(),
                effect: Effect::Write,
            });
            collect_predicate_columns(tokens, update_at + 1, primary.as_ref(), &mut columns);
            continue;
        }
        if tokens[i].is("delete") {
            operation.get_or_insert_with(|| "delete".into());
            let delete_at = i;
            i += 1;
            if tokens.get(i).is_some_and(|t| t.is("from")) {
                i += 1;
            }
            if tokens.get(i).is_some_and(|t| t.is("only")) {
                i += 1;
            }
            let target = take_qualified(tokens, &mut i);
            if let Some(ref name) = target {
                push_table(&mut tables, name.clone(), Effect::Write, &ctes);
            }
            i = scan_from_join(tokens, i, Effect::Read, &ctes, &mut tables);
            let primary = target.as_ref().map(|(schema, name)| TableRef {
                schema: schema.clone(),
                name: name.clone(),
                effect: Effect::Write,
            });
            collect_predicate_columns(tokens, delete_at + 1, primary.as_ref(), &mut columns);
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
    columns.sort_by(|a, b| {
        (&a.schema, &a.table, &a.name, &a.effect).cmp(&(&b.schema, &b.table, &b.name, &b.effect))
    });
    columns.dedup();
    SqlOutline {
        tables,
        columns,
        operation,
    }
}

fn primary_table(tables: &[TableRef], effect: Effect) -> Option<TableRef> {
    let matching: Vec<_> = tables.iter().filter(|t| t.effect == effect).collect();
    if matching.len() == 1 {
        Some(matching[0].clone())
    } else {
        tables.first().cloned()
    }
}

/// WHERE / JOIN ON / USING — column reads (in addition to SELECT list writes/reads).
fn collect_predicate_columns(
    tokens: &[Token],
    mut i: usize,
    primary: Option<&TableRef>,
    columns: &mut Vec<ColumnRef>,
) {
    while i < tokens.len() {
        if tokens[i].is(";") {
            return;
        }
        if tokens[i].is("where") || tokens[i].is("on") || tokens[i].is("having") {
            i += 1;
            i = collect_condition_columns(tokens, i, primary, columns);
            continue;
        }
        if tokens[i].is("using") && tokens.get(i + 1).is_some_and(|t| t.text == "(") {
            i += 2;
            while i < tokens.len() && tokens[i].text != ")" {
                if let Some(name) = tokens[i].as_ident().map(str::to_owned) {
                    let (schema, table) = resolve_column_table(None, None, primary);
                    if !table.is_empty() {
                        columns.push(ColumnRef {
                            schema,
                            table,
                            name,
                            effect: Effect::Read,
                        });
                    }
                }
                i += 1;
            }
            if tokens.get(i).is_some_and(|t| t.text == ")") {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
}

fn collect_condition_columns(
    tokens: &[Token],
    mut i: usize,
    primary: Option<&TableRef>,
    columns: &mut Vec<ColumnRef>,
) -> usize {
    while i < tokens.len() {
        if tokens[i].is(";")
            || tokens[i].is("group")
            || tokens[i].is("order")
            || tokens[i].is("limit")
            || tokens[i].is("offset")
            || tokens[i].is("returning")
            || tokens[i].is("union")
            || tokens[i].is("except")
            || tokens[i].is("intersect")
            || tokens[i].is("set")
            || (tokens[i].is("join")
                || tokens[i].is("inner")
                || tokens[i].is("left")
                || tokens[i].is("right")
                || tokens[i].is("full")
                || tokens[i].is("cross"))
        {
            return i;
        }
        if tokens[i].text == "(" {
            i = skip_parens(tokens, i);
            continue;
        }
        // Skip string/number literals so bind params and values are not columns.
        if matches!(tokens[i].kind, TokenKind::String | TokenKind::Number) {
            i += 1;
            continue;
        }
        if tokens[i].is("and")
            || tokens[i].is("or")
            || tokens[i].is("not")
            || tokens[i].is("in")
            || tokens[i].is("is")
            || tokens[i].is("null")
            || tokens[i].is("like")
            || tokens[i].is("ilike")
            || tokens[i].is("between")
            || tokens[i].is("exists")
            || tokens[i].is("true")
            || tokens[i].is("false")
        {
            i += 1;
            continue;
        }
        let saved = i;
        if let Some((schema, table, name)) = take_column_ref(tokens, &mut i) {
            // Avoid treating multi-arg function names as columns: ident (
            if tokens.get(i).is_some_and(|t| t.text == "(") {
                i = skip_parens(tokens, i);
                continue;
            }
            let (schema, table) = resolve_column_table(schema, table, primary);
            if !table.is_empty() && is_column_ident(&name) {
                columns.push(ColumnRef {
                    schema,
                    table,
                    name,
                    effect: Effect::Read,
                });
            }
            continue;
        }
        i = saved + 1;
    }
    i
}

fn is_column_ident(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        "and"
            | "or"
            | "not"
            | "in"
            | "is"
            | "null"
            | "like"
            | "ilike"
            | "between"
            | "exists"
            | "case"
            | "when"
            | "then"
            | "else"
            | "end"
            | "as"
            | "on"
            | "using"
            | "true"
            | "false"
            | "asc"
            | "desc"
            | "nulls"
            | "first"
            | "last"
    )
}

fn collect_select_columns(
    tokens: &[Token],
    mut i: usize,
    primary: Option<&TableRef>,
    columns: &mut Vec<ColumnRef>,
) {
    while i < tokens.len() {
        if tokens[i].is("from") || tokens[i].is(";") {
            return;
        }
        if tokens[i].text == "*" || tokens[i].is("distinct") || tokens[i].is("all") {
            i += 1;
            continue;
        }
        if tokens[i].text == "(" {
            i = skip_parens(tokens, i);
            i = skip_select_item_tail(tokens, i);
            continue;
        }
        if let Some((schema, table, name)) = take_column_ref(tokens, &mut i) {
            let (schema, table) = resolve_column_table(schema, table, primary);
            if !table.is_empty() {
                columns.push(ColumnRef {
                    schema,
                    table,
                    name,
                    effect: Effect::Read,
                });
            }
            i = skip_select_item_tail(tokens, i);
            continue;
        }
        i += 1;
    }
}

fn skip_select_item_tail(tokens: &[Token], mut i: usize) -> usize {
    while i < tokens.len() {
        if tokens[i].is("from") || tokens[i].is(";") {
            return i;
        }
        if tokens[i].text == "," {
            return i + 1;
        }
        if tokens[i].is("as") {
            i += 1;
            if tokens
                .get(i)
                .is_some_and(|t| t.kind == TokenKind::Ident || t.kind == TokenKind::String)
            {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    i
}

fn collect_update_set(
    tokens: &[Token],
    mut i: usize,
    table: &(String, String),
    columns: &mut Vec<ColumnRef>,
) -> usize {
    while i < tokens.len() {
        if tokens[i].is("where")
            || tokens[i].is("from")
            || tokens[i].is("returning")
            || tokens[i].is(";")
        {
            return i;
        }
        let Some(name) = tokens.get(i).and_then(Token::as_ident).map(str::to_owned) else {
            i += 1;
            continue;
        };
        i += 1;
        if tokens.get(i).is_some_and(|t| t.text == "=") {
            columns.push(ColumnRef {
                schema: table.0.clone(),
                table: table.1.clone(),
                name,
                effect: Effect::Write,
            });
            i += 1;
            // Skip RHS until comma / clause keyword.
            while i < tokens.len() {
                if tokens[i].text == "," {
                    i += 1;
                    break;
                }
                if tokens[i].is("where")
                    || tokens[i].is("from")
                    || tokens[i].is("returning")
                    || tokens[i].is(";")
                {
                    return i;
                }
                if tokens[i].text == "(" {
                    i = skip_parens(tokens, i);
                    continue;
                }
                i += 1;
            }
        }
    }
    i
}

fn collect_insert_columns(
    tokens: &[Token],
    mut i: usize,
    table: &(String, String),
    columns: &mut Vec<ColumnRef>,
) -> usize {
    while i < tokens.len() {
        if tokens[i].text == ")" {
            return i + 1;
        }
        if let Some(name) = tokens[i].as_ident().map(str::to_owned) {
            columns.push(ColumnRef {
                schema: table.0.clone(),
                table: table.1.clone(),
                name,
                effect: Effect::Write,
            });
            i += 1;
            if tokens.get(i).is_some_and(|t| t.text == ",") {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    i
}

fn take_column_ref(
    tokens: &[Token],
    i: &mut usize,
) -> Option<(Option<String>, Option<String>, String)> {
    let first = tokens.get(*i).and_then(Token::as_ident)?.to_string();
    *i += 1;
    if tokens.get(*i).is_some_and(|t| t.text == ".") {
        *i += 1;
        let second = tokens.get(*i).and_then(Token::as_ident)?.to_string();
        *i += 1;
        if tokens.get(*i).is_some_and(|t| t.text == ".") {
            *i += 1;
            let third = tokens.get(*i).and_then(Token::as_ident)?.to_string();
            *i += 1;
            Some((Some(first), Some(second), third))
        } else {
            Some((None, Some(first), second))
        }
    } else {
        Some((None, None, first))
    }
}

fn resolve_column_table(
    schema: Option<String>,
    table: Option<String>,
    primary: Option<&TableRef>,
) -> (String, String) {
    if let (Some(schema), Some(table)) = (schema, table.clone()) {
        return (schema, table);
    }
    if let Some(table) = table {
        return ("public".into(), table);
    }
    if let Some(primary) = primary {
        return (primary.schema.clone(), primary.name.clone());
    }
    ("public".into(), String::new())
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
        let end = skip_parens(tokens, i);
        let _ = scan_from_join(tokens, i + 1, effect, ctes, tables);
        return end;
    }
    if let Some(name) = take_qualified(tokens, &mut i) {
        push_table(tables, name, effect, ctes);
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
