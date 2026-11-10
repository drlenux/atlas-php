//! The Yii-specific subset of PHP configuration expressions. No expression is evaluated.
use crate::analyzer::AnalysisOptions;
use crate::plugins::php::tokens::{tokenize, Token, TokenKind};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub(super) struct Names {
    pub namespace: String,
    pub imports: BTreeMap<String, String>,
}
impl Names {
    pub fn metadata(value: &Value) -> Self {
        Self {
            namespace: value["namespace"]
                .as_str()
                .unwrap_or_default()
                .trim_matches('\\')
                .into(),
            imports: value["imports"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            v.as_str()
                                .map(|v| (k.to_lowercase(), v.trim_start_matches('\\').to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
    pub fn resolve(&self, name: &str) -> Option<String> {
        if !valid_name(name) {
            return None;
        }
        if name.starts_with('\\') {
            return Some(name.trim_start_matches('\\').to_owned());
        }
        if let Some(relative) = name.strip_prefix("namespace\\") {
            return Some(self.qualify(relative));
        }
        let (first, tail) = name.split_once('\\').unwrap_or((name, ""));
        if let Some(import) = self.imports.get(&first.to_lowercase()) {
            return Some(if tail.is_empty() {
                import.clone()
            } else {
                format!("{import}\\{tail}")
            });
        }
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "self" | "static" | "parent"
        ) {
            return None;
        }
        Some(self.qualify(name))
    }
    fn qualify(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.into()
        } else {
            format!("{}\\{name}", self.namespace)
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Expression {
    Text(String),
    Class(String),
    Array(Vec<(Option<String>, Expression)>),
    Unknown,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Classes {
    pub names: BTreeSet<String>,
    pub unknown: bool,
}
impl Classes {
    pub fn merge(&mut self, other: Self) {
        self.names.extend(other.names);
        self.unknown |= other.unknown;
    }
}

pub(super) fn configured_class(value: &Expression) -> Classes {
    match value {
        Expression::Text(name) | Expression::Class(name) if valid_name(name) => Classes {
            names: BTreeSet::from([name.trim_start_matches('\\').into()]),
            unknown: false,
        },
        Expression::Array(items) => {
            let mut found = false;
            let mut result = Classes::default();
            for (key, value) in items {
                if key.as_deref() == Some("class") {
                    found = true;
                    result.merge(configured_class(value));
                } else if key.is_none() {
                    result.unknown = true;
                }
            }
            if !found {
                result.unknown = true;
            }
            result
        }
        _ => Classes {
            unknown: true,
            ..Classes::default()
        },
    }
}

/// A component entry: like `configured_class`, but an array without `class` only
/// configures the component (Yii merges it with the entry that names the class).
pub(super) fn component_class(value: &Expression) -> Classes {
    match value {
        Expression::Array(items)
            if items.iter().all(|(key, _)| key.is_some())
                && !items.iter().any(|(key, _)| key.as_deref() == Some("class")) =>
        {
            Classes::default()
        }
        _ => configured_class(value),
    }
}

pub(super) fn fragment(source: &str, options: &AnalysisOptions) -> Result<Vec<Token>, String> {
    tokenize(&format!("<?php {source}"), options)
}

pub(super) fn expression(
    tokens: &[Token],
    names: &Names,
    options: &AnalysisOptions,
) -> Result<Expression, String> {
    parse(tokens, names, options, 0)
}
fn parse(
    mut tokens: &[Token],
    names: &Names,
    options: &AnalysisOptions,
    depth: usize,
) -> Result<Expression, String> {
    options.check(0)?;
    if depth > 64 {
        return Err("Yii configuration nesting limit exceeded".into());
    }
    while tokens.first().is_some_and(|t| t.text == "(")
        && closing(tokens, 0) == Some(tokens.len() - 1)
    {
        tokens = &tokens[1..tokens.len() - 1];
    }
    if tokens.is_empty() {
        return Ok(Expression::Unknown);
    }
    if tokens.len() == 1 && tokens[0].kind == TokenKind::String {
        return Ok(string(&tokens[0].text)
            .map(Expression::Text)
            .unwrap_or(Expression::Unknown));
    }
    if tokens.len() >= 3
        && tokens[tokens.len() - 2].text == "::"
        && tokens.last().unwrap().is("class")
    {
        let name = tokens[..tokens.len() - 2]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<String>();
        return Ok(names
            .resolve(&name)
            .map(Expression::Class)
            .unwrap_or(Expression::Unknown));
    }
    let start = if tokens[0].text == "[" {
        Some(0)
    } else if tokens[0].is("array") && tokens.get(1).is_some_and(|t| t.text == "(") {
        Some(1)
    } else {
        None
    };
    if let Some(start) = start {
        if closing(tokens, start) != Some(tokens.len() - 1) {
            return Ok(Expression::Unknown);
        }
        let mut items = Vec::new();
        for item in split(&tokens[start + 1..tokens.len() - 1], ",") {
            if item.is_empty() {
                continue;
            }
            let parts = split(item, "=>");
            if parts.len() == 2 {
                let key = if parts[0].len() == 1 {
                    string(&parts[0][0].text)
                } else {
                    None
                };
                items.push((key, parse(parts[1], names, options, depth + 1)?));
            } else if parts.len() == 1 {
                items.push((None, parse(item, names, options, depth + 1)?));
            } else {
                items.push((None, Expression::Unknown));
            }
        }
        return Ok(Expression::Array(items));
    }
    Ok(Expression::Unknown)
}

pub(super) fn closing(tokens: &[Token], start: usize) -> Option<usize> {
    let mut stack = Vec::new();
    for (index, token) in tokens.iter().enumerate().skip(start) {
        if token.kind != TokenKind::Symbol {
            continue;
        }
        match token.text.as_str() {
            "[" => stack.push("]"),
            "(" => stack.push(")"),
            "{" => stack.push("}"),
            "]" | ")" | "}" => {
                if stack.pop() != Some(token.text.as_str()) {
                    return None;
                }
                if stack.is_empty() {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn split<'a>(tokens: &'a [Token], separator: &str) -> Vec<&'a [Token]> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "[" | "(" | "{" => depth += 1,
                "]" | ")" | "}" => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if depth == 0 && token.text == separator {
            result.push(&tokens[start..i]);
            start = i + 1;
        }
    }
    result.push(&tokens[start..]);
    result
}

pub(super) fn string(text: &str) -> Option<String> {
    let quote = text.chars().next()?;
    if !matches!(quote, '\'' | '"') || !text.ends_with(quote) || text.len() < 2 {
        return None;
    }
    let mut result = String::new();
    let mut chars = text[1..text.len() - 1].chars();
    while let Some(c) = chars.next() {
        if quote == '"' && c == '$' {
            return None;
        }
        if c == '\\' {
            let escaped = chars.next()?;
            if escaped == quote || escaped == '\\' {
                result.push(escaped);
            } else {
                result.push('\\');
                result.push(escaped);
            }
        } else {
            result.push(c);
        }
    }
    Some(result)
}

fn valid_name(name: &str) -> bool {
    let name = name.trim_start_matches('\\');
    !name.is_empty()
        && name.split('\\').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
                && chars.all(|c| c == '_' || c.is_alphanumeric())
        })
}

pub(super) fn names_at(tokens: &[Token], end: usize) -> Names {
    let mut names = Names::default();
    let mut at = 0;
    let mut depth = 0usize;
    let mut namespace_depth = 0usize;
    while at < end && at < tokens.len() {
        let token = &tokens[at];
        if token.is("namespace") && depth == 0 {
            let start = at + 1;
            let mut stop = start;
            while stop < end && !matches!(tokens[stop].text.as_str(), ";" | "{") {
                stop += 1;
            }
            names.namespace = tokens[start..stop]
                .iter()
                .map(|t| t.text.as_str())
                .collect::<String>()
                .trim_matches('\\')
                .to_owned();
            names.imports.clear();
            if tokens.get(stop).is_some_and(|t| t.text == "{") {
                depth = 1;
                namespace_depth = 1;
            } else {
                namespace_depth = 0;
            }
            at = stop + 1;
            continue;
        }
        if token.is("use") && depth == namespace_depth {
            let mut stop = at + 1;
            while stop < end && tokens[stop].text != ";" {
                stop += 1;
            }
            if stop < end {
                imports(&tokens[at + 1..stop], &mut names.imports);
                at = stop + 1;
                continue;
            }
        }
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "{" => depth += 1,
                "}" => {
                    depth = depth.saturating_sub(1);
                    if namespace_depth > 0 && depth < namespace_depth {
                        names = Names::default();
                        namespace_depth = 0;
                    }
                }
                _ => {}
            }
        }
        at += 1;
    }
    names
}

fn imports(tokens: &[Token], imports: &mut BTreeMap<String, String>) {
    if tokens
        .first()
        .is_some_and(|t| t.is("function") || t.is("const") || t.text == "(")
    {
        return;
    }
    if let Some(open) = tokens.iter().position(|t| t.text == "{") {
        if let Some(close) = closing(tokens, open) {
            let prefix = tokens[..open]
                .iter()
                .map(|t| t.text.as_str())
                .collect::<String>();
            for entry in split(&tokens[open + 1..close], ",") {
                import_entry(entry, &prefix, imports);
            }
        }
    } else {
        for entry in split(tokens, ",") {
            import_entry(entry, "", imports);
        }
    }
}
fn import_entry(tokens: &[Token], prefix: &str, imports: &mut BTreeMap<String, String>) {
    let alias_at = tokens.iter().position(|t| t.is("as"));
    let end = alias_at.unwrap_or(tokens.len());
    let name = format!(
        "{prefix}{}",
        tokens[..end]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<String>()
    )
    .trim_start_matches('\\')
    .to_owned();
    if !valid_name(&name) {
        return;
    }
    let alias = alias_at
        .and_then(|i| tokens.get(i + 1))
        .map(|t| t.text.as_str())
        .unwrap_or_else(|| name.rsplit('\\').next().unwrap());
    imports.insert(alias.to_lowercase(), name);
}
