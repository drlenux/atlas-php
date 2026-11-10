//! Bounded DDL outline for PostgreSQL SQL. Source is data, never executed.
use super::tokens::{self, Token, TokenKind};
use crate::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::{Diagnostic, SourceSpan},
};

#[derive(Clone, Debug)]
pub struct QualifiedName {
    pub schema: String,
    pub name: String,
}
impl QualifiedName {
    pub fn display(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
}

#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub token: usize,
    pub end: usize,
    pub references: Option<QualifiedName>,
}

#[derive(Clone, Debug)]
pub enum Declaration {
    Schema {
        name: String,
        start: usize,
        end: usize,
    },
    Table {
        name: QualifiedName,
        columns: Vec<Column>,
        start: usize,
        end: usize,
    },
    View {
        name: QualifiedName,
        start: usize,
        end: usize,
    },
    Function {
        name: QualifiedName,
        start: usize,
        end: usize,
    },
    Index {
        name: QualifiedName,
        table: Option<QualifiedName>,
        start: usize,
        end: usize,
    },
}

#[derive(Clone, Debug)]
pub struct Unit {
    pub file: SourceFile,
    pub tokens: Vec<Token>,
    pub declarations: Vec<Declaration>,
    pub diagnostics: Vec<Diagnostic>,
}
impl Unit {
    pub fn span(&self, start: usize, end: usize) -> SourceSpan {
        let start_line = self.tokens.get(start).map(|t| t.line).unwrap_or(1);
        let end_line = self
            .tokens
            .get(end.saturating_sub(1))
            .map(|t| t.line)
            .unwrap_or(start_line);
        SourceSpan {
            path: self.file.path.clone(),
            start_line,
            end_line,
        }
    }
    pub fn fp(&self, start: usize, end: usize) -> String {
        let slice: String = self.tokens[start..end.min(self.tokens.len())]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        tokens::fingerprint([&self.file.path, &slice])
    }
}

pub fn parse_file(file: &SourceFile, options: &AnalysisOptions) -> Result<Unit, Diagnostic> {
    let tokens = tokens::tokenize(&file.source, options).map_err(|message| Diagnostic {
        path: file.path.clone(),
        line: 1,
        code: "parse_error".into(),
        message,
    })?;
    let mut unit = Unit {
        file: file.clone(),
        tokens,
        declarations: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut at = 0;
    while at < unit.tokens.len() {
        options
            .check(unit.declarations.len())
            .map_err(|message| Diagnostic {
                path: file.path.clone(),
                line: unit.tokens[at].line,
                code: "parse_error".into(),
                message,
            })?;
        if unit.tokens[at].is(";") {
            at += 1;
            continue;
        }
        match parse_statement(&unit.tokens, at) {
            Ok((decl, next)) => {
                if let Some(decl) = decl {
                    unit.declarations.push(decl);
                }
                at = next;
            }
            Err(message) => {
                unit.diagnostics.push(Diagnostic {
                    path: file.path.clone(),
                    line: unit.tokens[at].line,
                    code: "parse_error".into(),
                    message,
                });
                // Ignore paren depth: a broken `(;` must not swallow the next statement.
                at = skip_to_semicolon(&unit.tokens, at);
            }
        }
    }
    Ok(unit)
}

fn parse_statement(
    tokens: &[Token],
    start: usize,
) -> Result<(Option<Declaration>, usize), String> {
    if !tokens[start].is("create") {
        return Ok((None, skip_statement(tokens, start)));
    }
    let mut i = start + 1;
    skip_create_noise(tokens, &mut i);
    let keyword = tokens
        .get(i)
        .and_then(Token::as_ident)
        .ok_or_else(|| {
            format!(
                "expected object kind after CREATE at line {}",
                tokens[start].line
            )
        })?;
    match keyword {
        "schema" => {
            i += 1;
            skip_if_not_exists(tokens, &mut i);
            let name = expect_ident(tokens, &mut i)?;
            let end = statement_end(tokens, i);
            Ok((Some(Declaration::Schema { name, start, end }), end))
        }
        "table" => {
            i += 1;
            skip_if_not_exists(tokens, &mut i);
            let name = expect_qualified(tokens, &mut i)?;
            let columns = if tokens.get(i).is_some_and(|t| t.text == "(") {
                parse_columns(tokens, &mut i)?
            } else {
                Vec::new()
            };
            let end = statement_end(tokens, i);
            Ok((
                Some(Declaration::Table {
                    name,
                    columns,
                    start,
                    end,
                }),
                end,
            ))
        }
        "view" | "materialized" => {
            if keyword == "materialized" {
                i += 1;
                if !tokens.get(i).is_some_and(|t| t.is("view")) {
                    return Err(format!(
                        "expected VIEW after MATERIALIZED at line {}",
                        tokens[i.min(tokens.len().saturating_sub(1))].line
                    ));
                }
            }
            i += 1;
            skip_if_not_exists(tokens, &mut i);
            let name = expect_qualified(tokens, &mut i)?;
            let end = statement_end(tokens, i);
            Ok((Some(Declaration::View { name, start, end }), end))
        }
        "function" | "procedure" => {
            i += 1;
            let name = expect_qualified(tokens, &mut i)?;
            let end = statement_end(tokens, i);
            Ok((Some(Declaration::Function { name, start, end }), end))
        }
        "unique" | "index" => {
            if keyword == "unique" {
                i += 1;
                if !tokens.get(i).is_some_and(|t| t.is("index")) {
                    return Err(format!(
                        "expected INDEX after UNIQUE at line {}",
                        tokens[start].line
                    ));
                }
            }
            i += 1;
            skip_if_not_exists(tokens, &mut i);
            if tokens.get(i).is_some_and(|t| t.is("concurrently")) {
                i += 1;
            }
            let name = expect_qualified(tokens, &mut i)?;
            let mut table = None;
            if tokens.get(i).is_some_and(|t| t.is("on")) {
                i += 1;
                table = Some(expect_qualified(tokens, &mut i)?);
            }
            let end = statement_end(tokens, i);
            Ok((
                Some(Declaration::Index {
                    name,
                    table,
                    start,
                    end,
                }),
                end,
            ))
        }
        _ => Ok((None, skip_statement(tokens, start))),
    }
}

fn skip_create_noise(tokens: &[Token], i: &mut usize) {
    loop {
        let Some(t) = tokens.get(*i) else {
            return;
        };
        if t.is("or") {
            *i += 1;
            if tokens.get(*i).is_some_and(|t| t.is("replace")) {
                *i += 1;
            }
            continue;
        }
        if t.is("temp") || t.is("temporary") || t.is("unlogged") || t.is("global") || t.is("local")
        {
            *i += 1;
            continue;
        }
        break;
    }
}

fn skip_if_not_exists(tokens: &[Token], i: &mut usize) {
    if tokens.get(*i).is_some_and(|t| t.is("if"))
        && tokens.get(*i + 1).is_some_and(|t| t.is("not"))
        && tokens.get(*i + 2).is_some_and(|t| t.is("exists"))
    {
        *i += 3;
    }
}

fn parse_columns(tokens: &[Token], i: &mut usize) -> Result<Vec<Column>, String> {
    expect_symbol(tokens, i, "(")?;
    let mut columns = Vec::new();
    while *i < tokens.len() && tokens[*i].text != ")" {
        if tokens[*i].text == "," {
            *i += 1;
            continue;
        }
        if tokens[*i].is("constraint")
            || tokens[*i].is("primary")
            || tokens[*i].is("unique")
            || tokens[*i].is("foreign")
            || tokens[*i].is("check")
            || tokens[*i].is("exclude")
        {
            skip_balanced_item(tokens, i);
            continue;
        }
        let name_token = *i;
        let name = expect_ident(tokens, i)?;
        let type_name = parse_type_name(tokens, i)?;
        let mut references = None;
        while *i < tokens.len() && !matches!(tokens[*i].text.as_str(), "," | ")") {
            if tokens[*i].is("references") {
                *i += 1;
                references = Some(expect_qualified(tokens, i)?);
                if tokens.get(*i).is_some_and(|t| t.text == "(") {
                    skip_balanced(tokens, i)?;
                }
                continue;
            }
            if tokens[*i].text == "(" {
                skip_balanced(tokens, i)?;
            } else {
                *i += 1;
            }
        }
        let end = *i;
        columns.push(Column {
            name,
            type_name,
            token: name_token,
            end,
            references,
        });
    }
    expect_symbol(tokens, i, ")")?;
    Ok(columns)
}

fn parse_type_name(tokens: &[Token], i: &mut usize) -> Result<String, String> {
    let mut parts = Vec::new();
    while *i < tokens.len() {
        let t = &tokens[*i];
        if matches!(t.text.as_str(), "," | ")" | ";") {
            break;
        }
        if constraint_keyword(t) {
            break;
        }
        if t.kind == TokenKind::Ident {
            if !parts.is_empty() && !continues_type(t, &parts) {
                // schema-qualified type: name . name
                if t.text == "." || parts.last().is_some_and(|p| p == ".") {
                    parts.push(t.text.clone());
                    *i += 1;
                    continue;
                }
                break;
            }
            parts.push(t.text.clone());
            *i += 1;
            continue;
        }
        if t.kind == TokenKind::Symbol && matches!(t.text.as_str(), "." | "[" | "]" | "(" | ")") {
            if t.text == "(" && !parts.is_empty() {
                // typmod: numeric(10,2)
                parts.push(t.text.clone());
                *i += 1;
                while *i < tokens.len() && tokens[*i].text != ")" {
                    parts.push(tokens[*i].text.clone());
                    *i += 1;
                }
                if *i < tokens.len() {
                    parts.push(")".into());
                    *i += 1;
                }
                continue;
            }
            if t.text == "[" {
                parts.push("[".into());
                *i += 1;
                while *i < tokens.len() && tokens[*i].text != "]" {
                    parts.push(tokens[*i].text.clone());
                    *i += 1;
                }
                if *i < tokens.len() {
                    parts.push("]".into());
                    *i += 1;
                }
                continue;
            }
            if t.text == "." {
                parts.push(".".into());
                *i += 1;
                continue;
            }
        }
        if t.kind == TokenKind::Number && !parts.is_empty() {
            parts.push(t.text.clone());
            *i += 1;
            continue;
        }
        break;
    }
    if parts.is_empty() {
        return Err(format!(
            "expected type name at line {}",
            tokens.get(*i).map(|t| t.line).unwrap_or(1)
        ));
    }
    Ok(parts.join(""))
}

fn constraint_keyword(t: &Token) -> bool {
    t.is("default")
        || t.is("not")
        || t.is("null")
        || t.is("primary")
        || t.is("unique")
        || t.is("check")
        || t.is("references")
        || t.is("constraint")
        || t.is("collate")
        || t.is("generated")
}

fn continues_type(t: &Token, parts: &[String]) -> bool {
    if t.quoted {
        return false;
    }
    matches!(
        t.text.as_str(),
        "precision" | "varying" | "with" | "without" | "time" | "zone" | "large" | "object"
    ) || matches!(
        parts.last().map(String::as_str),
        Some("double" | "character" | "bit" | "timestamp" | "time" | "interval")
    )
}

fn skip_balanced_item(tokens: &[Token], i: &mut usize) {
    let mut depth = 0usize;
    while *i < tokens.len() {
        match tokens[*i].text.as_str() {
            "(" => depth += 1,
            ")" if depth == 0 => return,
            ")" => depth -= 1,
            "," if depth == 0 => return,
            _ => {}
        }
        *i += 1;
    }
}

fn skip_balanced(tokens: &[Token], i: &mut usize) -> Result<(), String> {
    expect_symbol(tokens, i, "(")?;
    let mut depth = 1usize;
    while *i < tokens.len() {
        match tokens[*i].text.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                *i += 1;
                if depth == 0 {
                    return Ok(());
                }
                continue;
            }
            _ => {}
        }
        *i += 1;
    }
    Err("unclosed parenthesis in column definition".into())
}

fn expect_ident(tokens: &[Token], i: &mut usize) -> Result<String, String> {
    let t = tokens
        .get(*i)
        .ok_or_else(|| "expected identifier".to_string())?;
    let name = t
        .as_ident()
        .ok_or_else(|| format!("expected identifier at line {}", t.line))?
        .to_string();
    *i += 1;
    Ok(name)
}

fn expect_qualified(tokens: &[Token], i: &mut usize) -> Result<QualifiedName, String> {
    let first = expect_ident(tokens, i)?;
    if tokens.get(*i).is_some_and(|t| t.text == ".") {
        *i += 1;
        let second = expect_ident(tokens, i)?;
        Ok(QualifiedName {
            schema: first,
            name: second,
        })
    } else {
        Ok(QualifiedName {
            schema: "public".into(),
            name: first,
        })
    }
}

fn expect_symbol(tokens: &[Token], i: &mut usize, symbol: &str) -> Result<(), String> {
    let t = tokens
        .get(*i)
        .ok_or_else(|| format!("expected {symbol}"))?;
    if t.text != symbol {
        return Err(format!("expected {symbol} at line {}", t.line));
    }
    *i += 1;
    Ok(())
}

fn statement_end(tokens: &[Token], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < tokens.len() {
        match tokens[i].text.as_str() {
            "(" => depth += 1,
            ")" => depth = depth.saturating_sub(1),
            ";" if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    i
}

fn skip_statement(tokens: &[Token], start: usize) -> usize {
    statement_end(tokens, start)
}

fn skip_to_semicolon(tokens: &[Token], start: usize) -> usize {
    let mut i = start;
    while i < tokens.len() {
        if tokens[i].text == ";" {
            return i + 1;
        }
        i += 1;
    }
    i
}
