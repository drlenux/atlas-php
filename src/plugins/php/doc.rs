//! PHPDoc parsing. Pure functions: tags of one docblock and canonical type strings.
//!
//! Canonical types match the analyzer's existing strings: `A|B|null` unions, `X[]` for
//! arrays of X, `Base<Arg>` for a generic with one (the last) type argument, `static`
//! for `$this`/`static`, and lowercase scalar keywords. Types from PHPDoc are
//! annotations: callers must mark anything derived from them as such.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    ReadWrite,
    Read,
    Write,
}
impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::ReadWrite => "read-write",
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocVar {
    pub name: Option<String>,
    pub type_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocProperty {
    pub name: String,
    pub type_name: String,
    pub access: Access,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocMethod {
    pub name: String,
    pub return_type: String,
    pub is_static: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocBlock {
    pub methods: Vec<DocMethod>,
    pub vars: Vec<DocVar>,
    pub params: BTreeMap<String, String>,
    pub return_type: Option<String>,
    pub properties: Vec<DocProperty>,
    pub templates: Vec<String>,
}

/// Tool-specific tags (`@phpstan-*`, `@psalm-*`) are more precise and win over plain ones.
fn tag_parts(tag: &str) -> (&str, bool) {
    for prefix in ["@phpstan-", "@psalm-"] {
        if let Some(rest) = tag.strip_prefix(prefix) {
            return (rest, true);
        }
    }
    (tag.trim_start_matches('@'), false)
}

impl DocBlock {
    /// `text` is the comment body without `/*` and `*/` (as the lexer stores it).
    pub fn parse(text: &str, resolve: &dyn Fn(&str) -> String) -> Self {
        let lines: Vec<(&str, &str)> = text
            .lines()
            .filter_map(|line| {
                let line = line.trim().trim_start_matches('*').trim();
                let tag_end = line.find(char::is_whitespace).unwrap_or(line.len());
                line.starts_with('@')
                    .then(|| (&line[..tag_end], line[tag_end..].trim()))
            })
            .collect();
        let mut doc = DocBlock::default();
        for (tag, rest) in &lines {
            if matches!(tag_parts(tag).0, "template" | "template-covariant") {
                if let Some(name) = rest.split_whitespace().next() {
                    if !doc.templates.iter().any(|t| t == name) {
                        doc.templates.push(name.to_owned());
                    }
                }
            }
        }
        let normalize = |raw: &str| doc_type(raw, resolve, &doc.templates);
        let mut precise_return = false;
        let mut precise_params = BTreeMap::new();
        let mut vars = Vec::new();
        let mut properties = Vec::new();
        for (tag, rest) in &lines {
            let (name, precise) = tag_parts(tag);
            match name {
                "var" => {
                    let (raw, variable) = type_and_variable(rest);
                    if !raw.is_empty() {
                        vars.push(DocVar {
                            name: variable,
                            type_name: normalize(raw),
                        });
                    }
                }
                "param" => {
                    let (raw, variable) = type_and_variable(rest);
                    if let (false, Some(variable)) = (raw.is_empty(), variable) {
                        let known: bool = *precise_params.get(&variable).unwrap_or(&false);
                        if precise || !known {
                            doc.params.insert(variable.clone(), normalize(raw));
                            precise_params.insert(variable, precise);
                        }
                    }
                }
                "return" => {
                    let raw = type_expression(rest).0;
                    if !raw.is_empty() && (precise || !precise_return) {
                        doc.return_type = Some(normalize(raw));
                        precise_return = precise;
                    }
                }
                "property" | "property-read" | "property-write" => {
                    let (raw, variable) = type_and_variable(rest);
                    if let (false, Some(variable)) = (raw.is_empty(), variable) {
                        properties.push(DocProperty {
                            name: variable,
                            type_name: normalize(raw),
                            access: match name {
                                "property-read" => Access::Read,
                                "property-write" => Access::Write,
                                _ => Access::ReadWrite,
                            },
                        });
                    }
                }
                _ => {}
            }
        }
        doc.vars = vars;
        doc.properties = properties;
        doc.methods = lines
            .iter()
            .filter(|(tag, _)| tag_parts(tag).0 == "method")
            .filter_map(|(_, rest)| method_tag(rest, &normalize))
            .collect();
        doc
    }

    pub fn is_empty(&self) -> bool {
        self.methods.is_empty()
            && self.vars.is_empty()
            && self.params.is_empty()
            && self.return_type.is_none()
            && self.properties.is_empty()
            && self.templates.is_empty()
    }
}

/// `@method [static] [Type] name(args) description`.
fn method_tag(rest: &str, normalize: &dyn Fn(&str) -> String) -> Option<DocMethod> {
    let mut rest = rest.trim();
    let is_static = rest
        .strip_prefix("static ")
        .map(|after| rest = after.trim_start())
        .is_some();
    let open = rest.find('(')?;
    let head = rest[..open].trim();
    let (raw_type, name) = match head.rsplit_once(char::is_whitespace) {
        Some((raw, name)) => (raw.trim(), name),
        None => ("", head),
    };
    let valid = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    valid.then(|| DocMethod {
        name: name.to_owned(),
        return_type: if raw_type.is_empty() {
            "unknown".into()
        } else {
            normalize(raw_type)
        },
        is_static,
    })
}

/// Splits `Type $name …` or `$name Type …` into the raw type and variable name.
fn type_and_variable(rest: &str) -> (&str, Option<String>) {
    let variable = |token: &str| {
        let name = token
            .trim_start_matches('&')
            .trim_start_matches("...")
            .strip_prefix('$')?;
        let end = name
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(name.len());
        (end > 0).then(|| name[..end].to_owned())
    };
    if rest.starts_with('$') {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = parts.next().and_then(variable);
        let raw = type_expression(parts.next().unwrap_or("").trim()).0;
        return (raw, name);
    }
    let (raw, after) = type_expression(rest);
    let name = after.split_whitespace().next().and_then(variable);
    (raw, name)
}

/// Reads one type expression, keeping brackets and `|` unions together.
fn type_expression(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    let mut depth = 0i32;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' | b'(' | b'{' | b'[' => depth += 1,
            b'>' | b')' | b'}' | b']' => depth -= 1,
            b if b.is_ascii_whitespace() && depth <= 0 => {
                // Allow `A | B` with spaces around the union bar.
                let next = text[i..].trim_start();
                let previous = text[..i].trim_end();
                if !(next.starts_with('|') || previous.ends_with('|')) {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (text[..i].trim(), &text[i..])
}

/// Splits on `separator` outside brackets.
pub fn split_top(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in text.char_indices() {
        match c {
            '<' | '(' | '{' | '[' => depth += 1,
            '>' | ')' | '}' | ']' => depth -= 1,
            c if c == separator && depth == 0 => {
                parts.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

const ARRAY_LIKE: [&str; 5] = [
    "array",
    "list",
    "iterable",
    "non-empty-array",
    "non-empty-list",
];

fn scalar(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "int" | "integer" => "int",
        "float" | "double" | "number" => "float",
        "string" => "string",
        "bool" | "boolean" => "bool",
        "array" => "array",
        "object" => "object",
        "mixed" => "mixed",
        "void" => "void",
        "never" => "never",
        "null" => "null",
        "false" => "false",
        "true" => "true",
        "callable" => "callable",
        "iterable" => "iterable",
        "resource" => "resource",
        "scalar" | "numeric" => "mixed",
        "self" => "self",
        "static" | "$this" => "static",
        "parent" => "parent",
        // `class-string` / `class-string<T>` handled in `single`, not as plain string.
        "class-string" => "class-string",
        _ if lower.contains('-') => {
            if lower.contains("int") {
                "int"
            } else if lower.starts_with("class-") {
                return None;
            } else if lower.contains("string") {
                "string"
            } else if lower.contains("array") || lower.contains("list") {
                "array"
            } else {
                "mixed"
            }
        }
        _ => return None,
    })
}

/// Canonical type string for a PHPDoc type expression.
pub fn doc_type(raw: &str, resolve: &dyn Fn(&str) -> String, templates: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in split_top(raw.trim(), '|') {
        for normalized in single(part.trim(), resolve, templates) {
            if !parts.contains(&normalized) {
                parts.push(normalized);
            }
        }
    }
    if parts.is_empty() {
        "unknown".into()
    } else {
        parts.join("|")
    }
}

fn single(part: &str, resolve: &dyn Fn(&str) -> String, templates: &[String]) -> Vec<String> {
    if part.is_empty() {
        return vec![];
    }
    if let Some(inner) = part.strip_prefix('?') {
        let mut types = single(inner, resolve, templates);
        types.push("null".into());
        return types;
    }
    if part.starts_with('(') && part.ends_with(')') {
        return split_top(&part[1..part.len() - 1], '|')
            .into_iter()
            .flat_map(|p| single(p.trim(), resolve, templates))
            .collect();
    }
    if let Some(inner) = part.strip_suffix("[]") {
        return single(inner, resolve, templates)
            .into_iter()
            .map(|t| format!("{t}[]"))
            .collect();
    }
    // Shapes and callable signatures carry no class we can follow.
    if let Some(open) = part.find(['{', '(']) {
        let base = &part[..open];
        return vec![scalar(base)
            .map(str::to_owned)
            .unwrap_or_else(|| name(base, resolve, templates))];
    }
    if let (Some(open), true) = (part.find('<'), part.ends_with('>')) {
        let base = part[..open].trim();
        let arguments = split_top(&part[open + 1..part.len() - 1], ',');
        let last = doc_type(arguments.last().copied().unwrap_or(""), resolve, templates);
        if ARRAY_LIKE.contains(&base.to_ascii_lowercase().as_str()) {
            return vec![format!("{last}[]")];
        }
        // Keep the type argument: `new $class` needs `class-string<Gaminator>`.
        if base.eq_ignore_ascii_case("class-string") {
            return vec![format!("class-string<{last}>")];
        }
        if let Some(scalar) = scalar(base) {
            return vec![scalar.into()];
        }
        return vec![format!("{}<{last}>", name(base, resolve, templates))];
    }
    vec![scalar(part)
        .map(str::to_owned)
        .unwrap_or_else(|| name(part, resolve, templates))]
}

fn name(raw: &str, resolve: &dyn Fn(&str) -> String, templates: &[String]) -> String {
    if templates.iter().any(|t| t == raw) {
        raw.to_owned()
    } else {
        resolve(raw)
    }
}

/// A code type that says nothing about which class methods belong to.
pub fn informative(ty: &str) -> bool {
    !split_top(ty, '|').iter().all(|part| {
        matches!(
            part.trim().to_ascii_lowercase().as_str(),
            "" | "unknown" | "mixed" | "object" | "array" | "iterable" | "null"
        )
    })
}

fn erased(ty: &str) -> Vec<String> {
    let mut parts: Vec<String> = split_top(ty, '|')
        .into_iter()
        .map(|part| {
            let part = part.trim();
            part.split('<').next().unwrap_or(part).to_ascii_lowercase()
        })
        .collect();
    parts.sort();
    parts.dedup();
    parts
}

/// Combines a type established by code with an optional PHPDoc type.
///
/// Code wins. The annotation is used (and the result marked `true`) only when code
/// gives no informative type, or when the annotation refines the same classes with
/// generic or element information (`ActiveQuery` → `ActiveQuery<X>`).
pub fn refine(code: &str, doc: Option<&str>) -> (String, bool) {
    let Some(doc) = doc.filter(|d| informative(d)) else {
        return (code.to_owned(), false);
    };
    if !informative(code) {
        return (doc.to_owned(), true);
    }
    // `: string` + `@return class-string<T>` — annotation names the constructed class.
    if code.eq_ignore_ascii_case("string")
        && split_top(doc, '|')
            .iter()
            .any(|p| p.trim().to_ascii_lowercase().starts_with("class-string"))
    {
        return (doc.to_owned(), true);
    }
    if doc != code && (doc.contains('<') || doc.contains("[]")) && erased(doc) == erased(code) {
        return (doc.to_owned(), true);
    }
    (code.to_owned(), false)
}

/// Inner types of every `class-string<T>` part in a union (skips bare `class-string`).
pub fn class_string_targets(ty: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in split_top(ty, '|') {
        let part = part.trim().strip_suffix("[]").unwrap_or(part.trim());
        let lower = part.to_ascii_lowercase();
        if lower == "class-string" {
            continue;
        }
        let Some(rest) = lower.strip_prefix("class-string<") else {
            continue;
        };
        if !rest.ends_with('>') {
            continue;
        }
        let Some(open) = part.find('<') else {
            continue;
        };
        let inner = part[open + 1..part.len() - 1].trim();
        if !inner.is_empty() && informative(inner) && !out.contains(&inner.to_owned()) {
            out.push(inner.to_owned());
        }
    }
    out
}

/// First `class-string<T>` target, if any.
pub fn class_string_target(ty: &str) -> Option<String> {
    class_string_targets(ty).into_iter().next()
}
