use super::doc::{Access, DocBlock};
use super::tokens::{self, Comment, Token, TokenKind};
use crate::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::SourceSpan,
};
use std::collections::BTreeMap;

#[derive(Clone, Default, Debug)]
pub struct Names {
    pub namespace: String,
    pub imports: BTreeMap<String, String>,
    /// Area prefix of the file (`.atlas/setup.yaml` `areas`).
    pub area: Option<String>,
    /// Global (namespace-less) names visible from this file's area → their qualified,
    /// area-prefixed names; set after every file is parsed.
    pub globals: Option<std::sync::Arc<BTreeMap<String, String>>>,
}
impl Names {
    /// Qualified name of a declaration: namespaced, or `<area>:<name>` for namespace-less
    /// code inside an area.
    pub fn declare(&self, name: &str) -> String {
        if !self.namespace.is_empty() {
            format!("{}\\{name}", self.namespace)
        } else if let Some(area) = &self.area {
            format!("{area}:{name}")
        } else {
            name.into()
        }
    }
    /// A global name as seen from this file's area (own area, shared areas, then plain).
    pub fn global(&self, name: &str) -> String {
        let plain = name.trim_start_matches('\\');
        if plain.contains('\\') {
            return plain.into();
        }
        self.globals
            .as_ref()
            .and_then(|globals| globals.get(&plain.to_ascii_lowercase()))
            .cloned()
            .unwrap_or_else(|| plain.into())
    }
    pub fn resolve(&self, name: &str) -> String {
        let resolved = self.resolve_plain(name);
        if resolved.contains('\\')
            || ["self", "parent", "static"].contains(&resolved.to_ascii_lowercase().as_str())
        {
            resolved
        } else {
            self.global(&resolved)
        }
    }
    fn resolve_plain(&self, name: &str) -> String {
        if name.starts_with('\\') {
            return name.trim_start_matches('\\').into();
        }
        if ["self", "parent", "static"].contains(&name.to_ascii_lowercase().as_str()) {
            return name.into();
        }
        let (first, tail) = name.split_once('\\').unwrap_or((name, ""));
        if let Some((_, full)) = self
            .imports
            .iter()
            .find(|(alias, _)| alias.eq_ignore_ascii_case(first))
        {
            return if tail.is_empty() {
                full.clone()
            } else {
                format!("{full}\\{tail}")
            };
        }
        if self.namespace.is_empty() {
            name.into()
        } else {
            format!("{}\\{name}", self.namespace)
        }
    }
    pub fn type_name(&self, name: &str) -> String {
        name.split('|')
            .map(|part| {
                let nullable = part.starts_with('?');
                let part = part.trim_start_matches('?');
                let value = if [
                    "int", "float", "string", "bool", "array", "object", "mixed", "void", "never",
                    "null", "false", "true", "callable", "iterable", "self", "parent", "static",
                    "unknown",
                ]
                .contains(&part.to_ascii_lowercase().as_str())
                {
                    part.into()
                } else {
                    self.resolve(part)
                };
                if nullable {
                    format!("{value}|null")
                } else {
                    value
                }
            })
            .collect::<Vec<_>>()
            .join("|")
    }
}
#[derive(Clone)]
pub struct Unit {
    pub file: SourceFile,
    pub tokens: Vec<Token>,
    pub pairs: Vec<Option<usize>>,
    pub comments: Vec<Comment>,
}
impl Unit {
    pub fn text(&self, start: usize, end: usize) -> String {
        if start >= end || start >= self.tokens.len() {
            return String::new();
        }
        self.file.source[self.tokens[start].start..self.tokens[end - 1].end].into()
    }
    pub fn span(&self, start: usize, end: usize) -> SourceSpan {
        SourceSpan {
            path: self.file.path.clone(),
            start_line: self.tokens.get(start).map_or(1, |t| t.line),
            end_line: self
                .tokens
                .get(end.saturating_sub(1))
                .map_or(1, |t| t.end_line),
        }
    }
    pub fn snippet(&self, start: usize, end: usize) -> String {
        let span = self.span(start, end);
        self.file
            .source
            .lines()
            .enumerate()
            .filter(|(i, _)| *i >= span.start_line.saturating_sub(4) && *i < span.end_line + 3)
            .map(|(_, s)| s)
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn fp(&self, start: usize, end: usize, skip: Option<usize>) -> String {
        tokens::fingerprint(
            (start..end)
                .filter(|i| Some(*i) != skip)
                .map(|i| self.tokens[i].text.as_str()),
        )
    }
    /// The PHPDoc (`/** … */`) directly before the declaration whose first token
    /// (after modifiers) is `token`, with no other tokens in between.
    pub fn docblock_before(&self, token: usize) -> Option<&Comment> {
        const MODIFIERS: [&str; 8] = [
            "public",
            "protected",
            "private",
            "static",
            "abstract",
            "final",
            "readonly",
            "var",
        ];
        let mut first = token;
        while first > 0 && MODIFIERS.iter().any(|m| self.is(first - 1, m)) {
            first -= 1;
        }
        let begin = self.tokens.get(first)?.start;
        let floor = first.checked_sub(1).map_or(0, |p| self.tokens[p].end);
        self.comments
            .iter()
            .rev()
            .find(|c| c.end <= begin && c.start >= floor && c.text.starts_with('*'))
    }
    pub fn is(&self, i: usize, text: &str) -> bool {
        self.tokens.get(i).is_some_and(|t| t.is(text))
    }
    pub fn end_statement(&self, start: usize, end: usize) -> usize {
        let mut i = start;
        while i < end {
            if self.is(i, ";") {
                return i;
            }
            if let Some(close) = self.pairs[i] {
                if close > i {
                    i = close + 1;
                    continue;
                }
            }
            i += 1;
        }
        end
    }
    pub fn split(&self, start: usize, end: usize, separator: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let (mut begin, mut i) = (start, start);
        while i < end {
            if self.is(i, separator) {
                out.push((begin, i));
                begin = i + 1;
            } else if let Some(close) = self.pairs[i] {
                if close > i {
                    i = close + 1;
                    continue;
                }
            }
            i += 1;
        }
        if begin < end {
            out.push((begin, end));
        }
        out
    }
}
#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    /// Effective type: code type refined by `@param` (see `doc::refine`).
    pub type_name: String,
    pub token: usize,
    /// `type_name` depends on a PHPDoc annotation.
    pub annotated: bool,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub id: String,
    pub name: String,
    pub qualified: String,
    pub class: Option<String>,
    pub names: Names,
    pub unit: usize,
    pub start: usize,
    pub end: usize,
    pub name_token: Option<usize>,
    pub body: Option<(usize, usize)>,
    pub arrow: bool,
    pub closure: bool,
    pub parameters: Vec<Parameter>,
    pub return_type: String,
    /// Canonical `@return` type, kept apart from the code type.
    pub doc_return: Option<String>,
    /// Body-inferred return from a prior flow pass, when declared/doc are not informative.
    pub inferred_return: Option<String>,
    pub uses_union_type: bool,
    pub owner: Option<String>,
    /// Declared only by a class `@method` tag: no PHP body or signature exists.
    pub magic: bool,
    /// Variables a closure captures with `use (...)`.
    pub captures: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Class {
    pub id: String,
    pub name: String,
    pub qualified: String,
    pub kind: String,
    pub names: Names,
    pub unit: usize,
    pub start: usize,
    pub end: usize,
    pub name_token: usize,
    pub parent: Option<String>,
    pub interfaces: Vec<String>,
    pub traits: Vec<String>,
    pub aliases: BTreeMap<String, (Option<String>, String)>,
    pub preferred: BTreeMap<String, String>,
    pub methods: BTreeMap<String, String>,
    pub properties: Vec<Property>,
    /// `@template` parameter names declared on the class.
    pub templates: Vec<String>,
    /// `const` declarations and enum `case`s.
    pub constants: Vec<Constant>,
    /// Framework return types of calls on this class: lowercase method → (type, exact).
    pub framework_returns: BTreeMap<String, (String, bool)>,
}

#[derive(Clone, Debug)]
pub struct Constant {
    pub name: String,
    pub token: usize,
    /// Source text of the value (empty for a pure enum case).
    pub value: String,
    /// Token range of the RHS (`=` … end), for scanning `Class::CONST` refs.
    pub value_range: Option<(usize, usize)>,
}
#[derive(Clone, Debug)]
pub struct Property {
    pub name: String,
    pub token: usize,
    /// Effective type: code type refined by `@var` (see `doc::refine`).
    pub type_name: String,
    pub uses_union_type: bool,
    /// `type_name` (or, for magic properties, the property itself) comes from PHPDoc.
    pub annotated: bool,
    /// Set for `@property*` tags of the class docblock: no PHP declaration exists.
    pub magic: Option<Access>,
    /// Docblock line for magic properties.
    pub doc_line: Option<usize>,
}
#[derive(Default)]
pub struct Declarations {
    pub classes: BTreeMap<String, Class>,
    pub functions: BTreeMap<String, Function>,
    pub closures: BTreeMap<(usize, usize), String>,
}

pub fn target_version(value: &str) -> Result<(u32, u32), String> {
    let Some((major, minor)) = value.split_once('.') else {
        return Err("php_version must use major.minor".into());
    };
    let version = (
        major.parse::<u32>().map_err(|_| "invalid php_version")?,
        minor.parse::<u32>().map_err(|_| "invalid php_version")?,
    );
    if !matches!(version, (7, 0..=4) | (8, 0..=5)) {
        return Err("php_version must be between 7.0 and 8.5".into());
    }
    Ok(version)
}

pub fn check_version(
    unit: &Unit,
    declarations: &Declarations,
    target: (u32, u32),
) -> Result<(), String> {
    for (i, token) in unit.tokens.iter().enumerate() {
        if token.kind == TokenKind::String {
            continue;
        }
        let feature = if token.is("fn") && unit.is(i + 1, "(") {
            Some(((7, 4), "arrow functions"))
        } else if token.is("?->") {
            Some(((8, 0), "nullsafe access"))
        } else if token.is("match") && unit.is(i + 1, "(") {
            Some(((8, 0), "match expressions"))
        } else if token.is("#[") {
            Some(((8, 0), "attributes"))
        } else if token.is("enum") {
            Some(((8, 1), "enums"))
        } else if token.is("readonly") && unit.is(i + 1, "class") {
            Some(((8, 2), "readonly classes"))
        } else if token.is("readonly") {
            Some(((8, 1), "readonly properties"))
        } else {
            None
        };
        if let Some((minimum, name)) = feature {
            if target < minimum {
                return Err(format!(
                    "unsupported {name} for PHP {}.{}; requires {}.{}",
                    target.0, target.1, minimum.0, minimum.1
                ));
            }
        }
    }
    if target < (8, 0)
        && (declarations.functions.values().any(|f| f.uses_union_type)
            || declarations
                .classes
                .values()
                .any(|c| c.properties.iter().any(|p| p.uses_union_type)))
    {
        return Err("unsupported union types for PHP target before 8.0".into());
    }
    Ok(())
}

pub fn parse(
    unit_index: usize,
    unit: &Unit,
    options: &AnalysisOptions,
) -> Result<Declarations, String> {
    let mut parser = Parser {
        unit_index,
        unit,
        options,
        out: Declarations::default(),
        closure_counts: BTreeMap::new(),
    };
    let names = Names {
        area: crate::domain::area_of(&options.areas, &unit.file.path).map(|a| a.prefix.clone()),
        ..Names::default()
    };
    parser.items(0, unit.tokens.len(), names, None, None, false, 0)?;
    Ok(parser.out)
}
struct Parser<'a> {
    unit_index: usize,
    unit: &'a Unit,
    options: &'a AnalysisOptions,
    out: Declarations,
    closure_counts: BTreeMap<String, usize>,
}
impl Parser<'_> {
    // Recursive parser context stays explicit at each declaration boundary.
    #[allow(clippy::too_many_arguments)]
    fn items(
        &mut self,
        start: usize,
        end: usize,
        mut names: Names,
        class: Option<String>,
        owner: Option<String>,
        inside_function: bool,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 128 {
            return Err("declaration nesting exceeds supported limit".into());
        }
        let mut i = start;
        while i < end {
            self.options.check(0)?;
            // `$obj->namespace` / `Foo::namespace` are members, not declarations.
            if self.unit.is(i, "namespace")
                && !inside_function
                && !(i > 0
                    && (self.unit.is(i - 1, "->")
                        || self.unit.is(i - 1, "?->")
                        || self.unit.is(i - 1, "::")))
            {
                let value = self
                    .unit
                    .tokens
                    .get(i + 1)
                    .filter(|t| t.kind == TokenKind::Ident)
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                let marker = if value.is_empty() { i + 1 } else { i + 2 };
                let nested = Names {
                    namespace: value,
                    imports: BTreeMap::new(),
                    area: names.area.clone(),
                    globals: None,
                };
                if self.unit.is(marker, "{") {
                    let close = self.unit.pairs[marker].unwrap();
                    self.items(marker + 1, close, nested, None, None, false, depth + 1)?;
                    i = close + 1;
                    continue;
                }
                if !self.unit.is(marker, ";") {
                    return Err("invalid namespace declaration".into());
                }
                names = nested;
                i = marker + 1;
                continue;
            }
            if self.unit.is(i, "use") && !inside_function {
                if let Some(class) = &class {
                    i = self.trait_use(i, end, class, &names)?;
                } else {
                    i = self.imports(i, end, &mut names)?;
                }
                continue;
            }
            // Named args (`class: Foo::class`) must not start a class declaration.
            // `new class …` is an anonymous class (optional ctor args, extends/implements).
            {
                let kind_kw = ["class", "interface", "trait", "enum"]
                    .iter()
                    .copied()
                    .find(|v| self.unit.is(i, v));
                let anonymous = kind_kw == Some("class") && i > 0 && self.unit.is(i - 1, "new");
                let named = kind_kw.is_some()
                    && !(i > 0 && self.unit.is(i - 1, "::"))
                    && self.unit.tokens.get(i + 1).is_some_and(|t| {
                        t.kind == TokenKind::Ident && !t.is("extends") && !t.is("implements")
                    });
                if anonymous || named {
                    let kind = self.unit.tokens[i].text.to_ascii_lowercase();
                    let (name, qualified, name_token, mut cursor) = if anonymous {
                        let line = self.unit.tokens[i].line;
                        let name = format!("{{anonymous@{}:{i}}}", line);
                        let qualified = names.declare(&name);
                        let mut cursor = i + 1;
                        if self.unit.is(cursor, "(") {
                            cursor = self.unit.pairs[cursor]
                                .ok_or("unclosed anonymous class constructor arguments")?
                                + 1;
                        }
                        (name, qualified, i, cursor)
                    } else {
                        let token = self
                            .unit
                            .tokens
                            .get(i + 1)
                            .filter(|t| t.kind == TokenKind::Ident)
                            .ok_or("missing class-like name")?;
                        let name = token.text.clone();
                        let qualified = names.declare(&name);
                        (name, qualified, i + 1, i + 2)
                    };
                    let id = format!("{kind}:{qualified}");
                    let (mut parent, mut interfaces) = (None, Vec::new());
                    while cursor < end && !self.unit.is(cursor, "{") {
                        if self.unit.is(cursor, "extends") || self.unit.is(cursor, "implements") {
                            let mode = self.unit.tokens[cursor].text.to_ascii_lowercase();
                            cursor += 1;
                            while cursor < end
                                && !self.unit.is(cursor, "{")
                                && !self.unit.is(cursor, "implements")
                            {
                                if self.unit.tokens[cursor].kind == TokenKind::Ident {
                                    let target = names.resolve(&self.unit.tokens[cursor].text);
                                    if mode == "extends" && kind == "class" {
                                        parent = Some(target);
                                    } else {
                                        interfaces.push(target);
                                    }
                                }
                                cursor += 1;
                            }
                        } else {
                            cursor += 1;
                        }
                    }
                    if cursor >= end {
                        return Err("missing class body".into());
                    }
                    let close = self.unit.pairs[cursor].ok_or("unclosed class body")?;
                    let doc = self.unit.docblock_before(i).map(|comment| {
                        (
                            DocBlock::parse(&comment.text, &|raw| names.resolve(raw)),
                            comment.line,
                        )
                    });
                    if self.out.classes.contains_key(&qualified) {
                        return Err(format!("duplicate class-like declaration {qualified}"));
                    }
                    self.out.classes.insert(
                        qualified.clone(),
                        Class {
                            id: id.clone(),
                            name,
                            qualified: qualified.clone(),
                            kind,
                            names: names.clone(),
                            unit: self.unit_index,
                            start: i,
                            end: close + 1,
                            name_token,
                            parent,
                            interfaces,
                            traits: Vec::new(),
                            aliases: BTreeMap::new(),
                            preferred: BTreeMap::new(),
                            methods: BTreeMap::new(),
                            constants: Vec::new(),
                            framework_returns: BTreeMap::new(),
                            properties: doc
                                .as_ref()
                                .map(|(doc, line)| {
                                    doc.properties
                                        .iter()
                                        .map(|p| Property {
                                            name: p.name.clone(),
                                            token: name_token,
                                            type_name: p.type_name.clone(),
                                            uses_union_type: false,
                                            annotated: true,
                                            magic: Some(p.access),
                                            doc_line: Some(*line),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                            templates: doc
                                .as_ref()
                                .map(|(doc, _)| doc.templates.clone())
                                .unwrap_or_default(),
                        },
                    );
                    for method in doc.map(|(doc, _)| doc.methods).unwrap_or_default() {
                        self.magic_method(&qualified, &id, &names, i, method);
                    }
                    self.items(
                        cursor + 1,
                        close,
                        names.clone(),
                        Some(qualified),
                        Some(id),
                        false,
                        depth + 1,
                    )?;
                    i = close + 1;
                    continue;
                }
            }
            if self.unit.is(i, "function") || self.unit.is(i, "fn") {
                i = self.function(
                    i,
                    end,
                    &names,
                    class.clone(),
                    owner.clone(),
                    inside_function,
                    depth,
                )?;
                continue;
            }
            if let Some(class_name) = class.as_ref().filter(|_| {
                !inside_function && (self.unit.is(i, "const") || self.unit.is(i, "case"))
            }) {
                let stop = self.unit.end_statement(i, end);
                let constants = self.constants(i, stop.min(end));
                self.out
                    .classes
                    .get_mut(class_name)
                    .unwrap()
                    .constants
                    .extend(constants);
                i = stop.saturating_add(1);
                continue;
            }
            if let Some(class_name) = class
                .as_ref()
                .filter(|_| !inside_function && self.unit.tokens[i].kind == TokenKind::Variable)
            {
                let mut type_start = i;
                while type_start > start {
                    let previous = &self.unit.tokens[type_start - 1];
                    if [
                        "public",
                        "protected",
                        "private",
                        "static",
                        "readonly",
                        "var",
                    ]
                    .iter()
                    .any(|modifier| previous.is(modifier))
                        || !(previous.kind == TokenKind::Ident
                            || ["?", "|", "&"].contains(&previous.text.as_str()))
                    {
                        break;
                    }
                    type_start -= 1;
                }
                let raw_type = (type_start..i)
                    .map(|index| self.unit.tokens[index].text.as_str())
                    .collect::<String>();
                let code_type = if raw_type.is_empty() {
                    "unknown".into()
                } else {
                    names.type_name(&raw_type)
                };
                let templates = self.out.classes[class_name].templates.clone();
                let doc_var = self.unit.docblock_before(type_start).and_then(|comment| {
                    let doc =
                        DocBlock::parse(&comment.text, &|raw| template_or(raw, &templates, &names));
                    doc.vars.into_iter().next().map(|v| v.type_name)
                });
                let (type_name, annotated) = super::doc::refine(&code_type, doc_var.as_deref());
                let property = Property {
                    name: self.unit.tokens[i].text.trim_start_matches('$').into(),
                    token: i,
                    type_name,
                    uses_union_type: raw_type.contains('|'),
                    annotated,
                    magic: None,
                    doc_line: None,
                };
                let properties = &mut self.out.classes.get_mut(class_name).unwrap().properties;
                // A real declaration replaces a same-named @property tag.
                properties.retain(|p| p.magic.is_none() || p.name != property.name);
                properties.push(property);
                let stop = self.unit.end_statement(i, end);
                i = stop.saturating_add(1);
                continue;
            }
            i += 1;
        }
        Ok(())
    }
    fn imports(&self, start: usize, end: usize, names: &mut Names) -> Result<usize, String> {
        let stop = self.unit.end_statement(start, end);
        if stop == end {
            return Err("unterminated import".into());
        }
        let begin = if self.unit.is(start + 1, "function") || self.unit.is(start + 1, "const") {
            start + 2
        } else {
            start + 1
        };
        for (a, b) in self.unit.split(begin, stop, ",") {
            if a >= b {
                continue;
            }
            if (a..b).any(|i| self.unit.is(i, "{")) {
                return Err("grouped imports are not supported by this structural parser".into());
            }
            let full = self.unit.tokens[a]
                .text
                .trim_start_matches('\\')
                .to_string();
            let alias = if a + 2 < b && self.unit.is(a + 1, "as") {
                self.unit.tokens[a + 2].text.clone()
            } else {
                full.rsplit('\\').next().unwrap_or(&full).into()
            };
            names.imports.insert(alias, full);
        }
        Ok(stop + 1)
    }
    fn trait_use(
        &mut self,
        start: usize,
        end: usize,
        class: &str,
        names: &Names,
    ) -> Result<usize, String> {
        let mut i = start + 1;
        let mut traits = Vec::new();
        while i < end && !self.unit.is(i, ";") && !self.unit.is(i, "{") {
            if self.unit.tokens[i].kind == TokenKind::Ident {
                traits.push(names.resolve(&self.unit.tokens[i].text));
            }
            i += 1;
        }
        let info = self.out.classes.get_mut(class).unwrap();
        info.traits.extend(traits);
        if self.unit.is(i, ";") {
            return Ok(i + 1);
        }
        if !self.unit.is(i, "{") {
            return Err("unterminated trait use".into());
        }
        let close = self.unit.pairs[i].unwrap();
        for (a, b) in self.unit.split(i + 1, close, ";") {
            if a >= b {
                continue;
            }
            let (t, m, next) = if a + 2 < b && self.unit.is(a + 1, "::") {
                (
                    Some(names.resolve(&self.unit.tokens[a].text)),
                    self.unit.tokens[a + 2].text.clone(),
                    a + 3,
                )
            } else {
                (None, self.unit.tokens[a].text.clone(), a + 1)
            };
            if self.unit.is(next, "insteadof") {
                if let Some(t) = t {
                    info.preferred.insert(m.to_ascii_lowercase(), t);
                }
            } else if self.unit.is(next, "as") {
                if let Some(alias) = (next + 1..b).rev().find(|p| {
                    self.unit.tokens[*p].kind == TokenKind::Ident
                        && !["public", "protected", "private", "final"]
                            .contains(&self.unit.tokens[*p].text.to_ascii_lowercase().as_str())
                }) {
                    info.aliases
                        .insert(self.unit.tokens[alias].text.to_ascii_lowercase(), (t, m));
                }
            }
        }
        Ok(close + 1)
    }
    // Recursive parser context stays explicit at each declaration boundary.
    #[allow(clippy::too_many_arguments)]
    fn function(
        &mut self,
        start: usize,
        end: usize,
        names: &Names,
        class: Option<String>,
        owner: Option<String>,
        inside_function: bool,
        depth: usize,
    ) -> Result<usize, String> {
        let arrow = self.unit.is(start, "fn");
        let class_templates = class
            .as_ref()
            .and_then(|c| self.out.classes.get(c))
            .map(|c| c.templates.clone())
            .unwrap_or_default();
        let doc = self.unit.docblock_before(start).map(|comment| {
            let method_templates = DocBlock::parse(&comment.text, &|raw| raw.to_owned()).templates;
            let templates: Vec<String> = class_templates
                .iter()
                .chain(&method_templates)
                .cloned()
                .collect();
            DocBlock::parse(&comment.text, &|raw| template_or(raw, &templates, names))
        });
        let mut at = start + 1;
        if self.unit.is(at, "&") {
            at += 1;
        }
        let name_token = if self
            .unit
            .tokens
            .get(at)
            .is_some_and(|t| t.kind == TokenKind::Ident)
        {
            let n = Some(at);
            at += 1;
            n
        } else {
            None
        };
        if !self.unit.is(at, "(") {
            return Err("function declaration is missing parameter list".into());
        }
        let param_end = self.unit.pairs[at].ok_or("unclosed parameter list")?;
        let mut parameters = Vec::new();
        let mut uses_union_type = false;
        for (a, b) in self.unit.split(at + 1, param_end, ",") {
            if let Some(var) = (a..b).find(|i| self.unit.tokens[*i].kind == TokenKind::Variable) {
                let raw = (a..var)
                    .filter(|i| {
                        !self.unit.is(*i, "&")
                            && !self.unit.is(*i, "...")
                            && !["public", "protected", "private", "readonly"]
                                .iter()
                                .any(|v| self.unit.is(*i, v))
                    })
                    .map(|i| self.unit.tokens[i].text.as_str())
                    .collect::<String>();
                uses_union_type |= raw.contains('|');
                let name: String = self.unit.tokens[var].text.trim_start_matches('$').into();
                let code_type = if raw.is_empty() {
                    "unknown".into()
                } else {
                    names.type_name(&raw)
                };
                let doc_param = doc.as_ref().and_then(|d| d.params.get(&name));
                let (type_name, annotated) =
                    super::doc::refine(&code_type, doc_param.map(String::as_str));
                parameters.push(Parameter {
                    name,
                    type_name,
                    token: var,
                    annotated,
                });
            }
        }
        at = param_end + 1;
        let mut captures = Vec::new();
        if self.unit.is(at, "use") && self.unit.is(at + 1, "(") {
            let close = self.unit.pairs[at + 1].unwrap();
            captures = (at + 2..close)
                .filter(|t| self.unit.tokens[*t].kind == TokenKind::Variable)
                .map(|t| self.unit.tokens[t].text.trim_start_matches('$').to_owned())
                .collect();
            at = close + 1;
        }
        let mut return_type = "unknown".to_string();
        if self.unit.is(at, ":") {
            at += 1;
            let begin = at;
            while at < end
                && !self.unit.is(at, "{")
                && !self.unit.is(at, ";")
                && !self.unit.is(at, "=>")
            {
                at += 1;
            }
            uses_union_type |= (begin..at).any(|i| self.unit.is(i, "|"));
            return_type = names.type_name(
                &(begin..at)
                    .map(|i| self.unit.tokens[i].text.as_str())
                    .collect::<String>(),
            );
        }
        let (body, next) = if arrow {
            if !self.unit.is(at, "=>") {
                return Err("arrow function is missing =>".into());
            }
            let mut stop = at + 1;
            while stop < end {
                if [";", ",", ")", "]"].iter().any(|s| self.unit.is(stop, s)) {
                    break;
                }
                if let Some(close) = self.unit.pairs[stop] {
                    if close > stop {
                        stop = close + 1;
                        continue;
                    }
                }
                stop += 1;
            }
            (Some((at + 1, stop)), stop)
        } else if self.unit.is(at, "{") {
            let close = self.unit.pairs[at].unwrap();
            (Some((at + 1, close)), close + 1)
        } else if self.unit.is(at, ";") {
            (None, at + 1)
        } else {
            return Err("function declaration is missing a body or semicolon".into());
        };
        let closure = name_token.is_none();
        let (name, qualified, id) = if let Some(token) = name_token {
            let name = self.unit.tokens[token].text.clone();
            if let Some(class_name) = class.as_ref().filter(|_| !inside_function) {
                let q = format!("{class_name}::{name}");
                (name, q.clone(), format!("method:{q}"))
            } else {
                let q = names.declare(&name);
                (name, q.clone(), format!("function:{q}"))
            }
        } else {
            let fp = self.unit.fp(start, next, None);
            let key = format!("{}:{fp}", owner.as_deref().unwrap_or(&self.unit.file.path));
            let counter = self.closure_counts.entry(key.clone()).or_default();
            *counter += 1;
            let q = format!("{key}:{}", *counter);
            (
                if arrow { "{arrow}" } else { "{closure}" }.into(),
                q.clone(),
                format!("function:{q}"),
            )
        };
        // A real declaration replaces a same-named `@method` tag.
        if self.out.functions.get(&id).is_some_and(|f| !f.magic) {
            return Err(format!("duplicate function declaration {qualified}"));
        }
        if id.starts_with("method:") {
            self.out
                .classes
                .get_mut(class.as_ref().unwrap())
                .unwrap()
                .methods
                .insert(name.to_ascii_lowercase(), id.clone());
        }
        self.out.functions.insert(
            id.clone(),
            Function {
                id: id.clone(),
                name,
                qualified,
                class: class.clone(),
                names: names.clone(),
                unit: self.unit_index,
                start,
                end: next,
                name_token,
                body,
                arrow,
                closure,
                parameters,
                return_type,
                doc_return: doc.and_then(|d| d.return_type),
                inferred_return: None,
                uses_union_type,
                owner: owner.clone(),
                magic: false,
                captures,
            },
        );
        if closure {
            self.out
                .closures
                .insert((self.unit_index, start), id.clone());
        }
        if let Some((a, b)) = body {
            self.items(a, b, names.clone(), class, Some(id), true, depth + 1)?;
        }
        Ok(next)
    }
}

impl Parser<'_> {
    /// `const [type] A = 1, B = 2` or enum `case A [= 'a']` between `keyword` and `stop`.
    fn constants(&self, keyword: usize, stop: usize) -> Vec<Constant> {
        self.unit
            .split(keyword + 1, stop, ",")
            .into_iter()
            .filter_map(|(a, b)| {
                let equals = (a..b).find(|t| self.unit.is(*t, "="));
                // The name is the identifier right before `=` (or the last token).
                let name_token = equals.unwrap_or(b).checked_sub(1)?.max(a);
                let token = self.unit.tokens.get(name_token)?;
                (token.kind == TokenKind::Ident).then(|| Constant {
                    name: token.text.clone(),
                    token: name_token,
                    value: equals
                        .map(|e| self.unit.text(e + 1, b).chars().take(80).collect())
                        .unwrap_or_default(),
                    value_range: equals.map(|e| (e + 1, b)),
                })
            })
            .collect()
    }
    /// Registers a class `@method` tag as a body-less method of the class.
    fn magic_method(
        &mut self,
        class: &str,
        class_id: &str,
        names: &Names,
        token: usize,
        method: super::doc::DocMethod,
    ) {
        let qualified = format!("{class}::{}", method.name);
        let id = format!("method:{qualified}");
        let lower = method.name.to_ascii_lowercase();
        let Some(owner) = self.out.classes.get_mut(class) else {
            return;
        };
        owner.methods.insert(lower, id.clone());
        self.out.functions.insert(
            id.clone(),
            Function {
                id,
                name: method.name,
                qualified,
                class: Some(class.to_owned()),
                names: names.clone(),
                unit: self.unit_index,
                start: token,
                end: token + 2,
                name_token: Some(token + 1),
                body: None,
                arrow: false,
                closure: false,
                parameters: Vec::new(),
                return_type: "unknown".into(),
                doc_return: Some(method.return_type),
                inferred_return: None,
                uses_union_type: false,
                owner: Some(class_id.to_owned()),
                magic: true,
                captures: Vec::new(),
            },
        );
    }
}

/// Resolves a PHPDoc class name unless it names a `@template` parameter.
fn template_or(raw: &str, templates: &[String], names: &Names) -> String {
    if templates.iter().any(|t| t == raw) {
        raw.to_owned()
    } else {
        names.resolve(raw)
    }
}
