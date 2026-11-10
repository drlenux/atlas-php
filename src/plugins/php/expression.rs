use super::{syntax::Unit, tokens::TokenKind};

#[derive(Clone, Debug)]
pub struct Expr {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

#[derive(Clone, Debug)]
pub struct ArrayItem {
    pub key: Option<Expr>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Variable(String),
    Literal(String),
    Name(String),
    Array(Vec<ArrayItem>),
    New(String, Vec<Expr>),
    Closure(usize),
    Call {
        receiver: Option<Box<Expr>>,
        member: String,
        dynamic: bool,
        static_: bool,
        args: Vec<Expr>,
    },
    Property {
        receiver: Box<Expr>,
        member: String,
        dynamic: bool,
        static_: bool,
        nullsafe: bool,
    },
    Index {
        base: Box<Expr>,
        index: Option<Box<Expr>>,
    },
    Assign(Box<Expr>, Box<Expr>),
    CompoundAssign(String, Box<Expr>, Box<Expr>),
    Ternary {
        condition: Box<Expr>,
        then_branch: Option<Box<Expr>>,
        else_branch: Box<Expr>,
    },
    Cast {
        type_name: String,
        expr: Box<Expr>,
    },
    Binary(String, Box<Expr>, Box<Expr>),
    Unary(String, Box<Expr>),
    Unknown(Vec<Expr>),
}

const CAST_TYPES: &[&str] = &[
    "int", "integer", "bool", "boolean", "float", "double", "real", "string", "array", "object",
    "unset",
];

pub fn parse(unit: &Unit, start: usize, end: usize) -> Result<Expr, String> {
    parse_depth(unit, start, end, 0)
}

fn parse_depth(u: &Unit, start: usize, end: usize, depth: usize) -> Result<Expr, String> {
    if depth > 128 {
        return Err("expression nesting exceeds supported limit".into());
    }
    if start >= end {
        return Err("missing expression".into());
    }
    let wrap = |kind| Expr { start, end, kind };
    if u.is(start, "function") || u.is(start, "fn") {
        return Ok(wrap(Kind::Closure(start)));
    }
    if u.is(start, "(") && u.pairs[start] == Some(end - 1) {
        return parse_depth(u, start + 1, end - 1, depth + 1);
    }
    if trailing_operator(u, end - 1) {
        return Err("missing expression after operator".into());
    }
    if let Some(expr) = binary_or_ternary(u, start, end, depth)? {
        return Ok(expr);
    }
    if let Some(close) = cast_close(u, start, end) {
        let type_name = normalize_cast(&u.tokens[start + 1].text);
        return Ok(wrap(Kind::Cast {
            type_name,
            expr: Box::new(parse_depth(u, close + 1, end, depth + 1)?),
        }));
    }
    if ["!", "-", "+", "clone", "throw", "@"]
        .iter()
        .any(|s| u.is(start, s))
    {
        return Ok(wrap(Kind::Unary(
            u.tokens[start].text.clone(),
            Box::new(parse_depth(u, start + 1, end, depth + 1)?),
        )));
    }
    if u.is(start, "new") {
        let name = u
            .tokens
            .get(start + 1)
            .ok_or("missing constructed class")?
            .text
            .clone();
        if name.eq_ignore_ascii_case("class") {
            // Opaque anonymous class: skip ctor args / extends / implements / body.
            let mut at = start + 2;
            if u.is(at, "(") {
                at = u.pairs[at].ok_or("unclosed anonymous class arguments")? + 1;
            }
            while at < end && !u.is(at, "{") {
                at += 1;
            }
            let close = u
                .pairs
                .get(at)
                .copied()
                .flatten()
                .ok_or("unclosed anonymous class body")?;
            return Ok(Expr {
                start,
                end: close + 1,
                kind: Kind::New("{anonymous}".into(), Vec::new()),
            });
        }
        let args = if u.is(start + 2, "(") {
            arguments(u, start + 3, u.pairs[start + 2].unwrap(), depth + 1)?
        } else {
            Vec::new()
        };
        return Ok(wrap(Kind::New(name, args)));
    }
    let (mut expr, mut next) = primary(u, start, end, depth)?;
    while next < end {
        if u.is(next, "(") {
            let close = u.pairs[next].ok_or("unclosed call")?;
            let args = arguments(u, next + 1, close, depth + 1)?;
            // `foo()`, `$fn()`, or `($this->credit)()` — property/callable invoke
            // keeps the callee as receiver with member `__invoke`.
            expr = match expr.kind {
                Kind::Name(name) => Expr {
                    start,
                    end: close + 1,
                    kind: Kind::Call {
                        receiver: None,
                        member: name,
                        dynamic: false,
                        static_: false,
                        args,
                    },
                },
                Kind::Variable(name) => Expr {
                    start,
                    end: close + 1,
                    kind: Kind::Call {
                        receiver: None,
                        member: format!("${name}"),
                        dynamic: true,
                        static_: false,
                        args,
                    },
                },
                other => {
                    let callee = Expr {
                        start: expr.start,
                        end: expr.end,
                        kind: other,
                    };
                    Expr {
                        start,
                        end: close + 1,
                        kind: Kind::Call {
                            receiver: Some(Box::new(callee)),
                            member: "__invoke".into(),
                            dynamic: false,
                            static_: false,
                            args,
                        },
                    }
                }
            };
            next = close + 1;
            continue;
        }
        if u.is(next, "->") || u.is(next, "?->") || u.is(next, "::") {
            let static_ = u.is(next, "::");
            let nullsafe = u.is(next, "?->");
            let token = u
                .tokens
                .get(next + 1)
                .ok_or("missing member after access operator")?;
            let member = token.text.clone();
            let dynamic = token.kind == TokenKind::Variable && !static_;
            let after = next + 2;
            if u.is(after, "(") {
                let close = u.pairs[after].unwrap();
                let args = arguments(u, after + 1, close, depth + 1)?;
                expr = Expr {
                    start,
                    end: close + 1,
                    kind: Kind::Call {
                        receiver: Some(Box::new(expr)),
                        member,
                        dynamic,
                        static_,
                        args,
                    },
                };
                next = close + 1;
            } else {
                expr = Expr {
                    start,
                    end: after,
                    kind: Kind::Property {
                        receiver: Box::new(expr),
                        member,
                        dynamic,
                        static_,
                        nullsafe,
                    },
                };
                next = after;
            }
            continue;
        }
        if u.is(next, "[") {
            let close = u.pairs[next].ok_or("unclosed index")?;
            let index = if next + 1 < close {
                Some(Box::new(parse_depth(u, next + 1, close, depth + 1)?))
            } else {
                None
            };
            expr = Expr {
                start,
                end: close + 1,
                kind: Kind::Index {
                    base: Box::new(expr),
                    index,
                },
            };
            next = close + 1;
            continue;
        }
        // One Unknown for the unsupported remainder; keep the already-parsed left side.
        let mut children = vec![expr];
        if next < end {
            if let Ok(rest) = parse_depth(u, next, end, depth + 1) {
                flatten_unknown(&mut children, rest);
            }
        }
        return Ok(wrap(Kind::Unknown(children)));
    }
    Ok(expr)
}

fn primary(u: &Unit, start: usize, _end: usize, depth: usize) -> Result<(Expr, usize), String> {
    if u.is(start, "[") || (u.is(start, "array") && u.is(start + 1, "(")) {
        let open = if u.is(start, "[") { start } else { start + 1 };
        let close = u.pairs[open].ok_or("unclosed array")?;
        let items = array_items(u, open + 1, close, depth + 1)?;
        return Ok((
            Expr {
                start,
                end: close + 1,
                kind: Kind::Array(items),
            },
            close + 1,
        ));
    }
    if u.is(start, "(") {
        let close = u.pairs[start].ok_or("unclosed group")?;
        return Ok((parse_depth(u, start + 1, close, depth + 1)?, close + 1));
    }
    let t = &u.tokens[start];
    let kind = match t.kind {
        TokenKind::Variable => Kind::Variable(t.text.trim_start_matches('$').into()),
        TokenKind::String => Kind::Literal("string".into()),
        TokenKind::Number => {
            Kind::Literal(if t.text.contains('.') { "float" } else { "int" }.into())
        }
        TokenKind::Ident => match t.text.to_ascii_lowercase().as_str() {
            "null" => Kind::Literal("null".into()),
            "true" | "false" => Kind::Literal("bool".into()),
            _ => Kind::Name(t.text.clone()),
        },
        _ => Kind::Unknown(Vec::new()),
    };
    Ok((
        Expr {
            start,
            end: start + 1,
            kind,
        },
        start + 1,
    ))
}

fn binary_or_ternary(
    u: &Unit,
    start: usize,
    end: usize,
    depth: usize,
) -> Result<Option<Expr>, String> {
    let mut operator = None;
    let mut best = usize::MAX;
    let mut i = start;
    while i < end {
        if let Some(close) = u.pairs[i] {
            if close > i {
                i = close + 1;
                continue;
            }
        }
        let (prec, right_assoc) = precedence(u, i);
        if i > start
            && i + 1 < end
            && prec < usize::MAX
            && (prec < best || (prec == best && choose(operator, i, right_assoc)))
        {
            operator = Some(i);
            best = prec;
        }
        i += 1;
    }
    let Some(op) = operator else {
        return Ok(None);
    };
    if u.is(op, "?") {
        return Ok(Some(parse_ternary(u, start, end, op, depth)?));
    }
    let left = Box::new(parse_depth(u, start, op, depth + 1)?);
    let right = Box::new(parse_depth(u, op + 1, end, depth + 1)?);
    let text = u.tokens[op].text.clone();
    let kind = if text == "=" {
        Kind::Assign(left, right)
    } else if matches!(
        text.as_str(),
        "+=" | "-=" | "*=" | "/=" | ".=" | "%=" | "??="
    ) {
        Kind::CompoundAssign(text, left, right)
    } else {
        Kind::Binary(text, left, right)
    };
    Ok(Some(Expr { start, end, kind }))
}

fn choose(current: Option<usize>, candidate: usize, right_assoc: bool) -> bool {
    match current {
        None => true,
        Some(op) if right_assoc => candidate >= op,
        Some(op) => candidate <= op,
    }
}

fn parse_ternary(
    u: &Unit,
    start: usize,
    end: usize,
    question: usize,
    depth: usize,
) -> Result<Expr, String> {
    let condition = Box::new(parse_depth(u, start, question, depth + 1)?);
    if u.is(question + 1, ":") {
        let else_branch = Box::new(parse_depth(u, question + 2, end, depth + 1)?);
        return Ok(Expr {
            start,
            end,
            kind: Kind::Ternary {
                condition,
                then_branch: None,
                else_branch,
            },
        });
    }
    let colon = find_ternary_colon(u, question + 1, end).ok_or("ternary requires ':'")?;
    let then_branch = Box::new(parse_depth(u, question + 1, colon, depth + 1)?);
    let else_branch = Box::new(parse_depth(u, colon + 1, end, depth + 1)?);
    Ok(Expr {
        start,
        end,
        kind: Kind::Ternary {
            condition,
            then_branch: Some(then_branch),
            else_branch,
        },
    })
}

fn find_ternary_colon(u: &Unit, start: usize, end: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < end {
        if let Some(close) = u.pairs[i] {
            if close > i {
                i = close + 1;
                continue;
            }
        }
        if u.is(i, "?") {
            depth += 1;
        } else if u.is(i, ":") {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
        }
        i += 1;
    }
    None
}

fn precedence(u: &Unit, i: usize) -> (usize, bool) {
    match u.tokens[i].text.as_str() {
        "or" => (1, false),
        "xor" => (2, false),
        "and" => (3, false),
        "=" | "+=" | "-=" | "*=" | "/=" | ".=" | "%=" | "??=" => (4, true),
        "?" => (5, false),
        "??" => (6, true),
        "||" => (7, false),
        "&&" => (8, false),
        "==" | "!=" | "===" | "!==" | "<" | ">" | "<=" | ">=" => (9, false),
        "." | "+" | "-" => (10, false),
        "*" | "/" | "%" => (11, false),
        _ => (usize::MAX, false),
    }
}

fn trailing_operator(u: &Unit, i: usize) -> bool {
    matches!(
        u.tokens[i].text.as_str(),
        "=" | "+="
            | "-="
            | "*="
            | "/="
            | ".="
            | "%="
            | "??="
            | "??"
            | "||"
            | "or"
            | "&&"
            | "and"
            | "xor"
            | "=="
            | "!="
            | "==="
            | "!=="
            | "<"
            | ">"
            | "<="
            | ">="
            | "."
            | "+"
            | "-"
            | "*"
            | "/"
            | "%"
            | "?"
            | ":"
    )
}

fn cast_close(u: &Unit, start: usize, end: usize) -> Option<usize> {
    if !u.is(start, "(") {
        return None;
    }
    let close = u.pairs[start]?;
    if close != start + 2 || close + 1 >= end {
        return None;
    }
    let name = u.tokens[start + 1].text.to_ascii_lowercase();
    CAST_TYPES.iter().any(|ty| *ty == name).then_some(close)
}

fn normalize_cast(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "integer" | "int" => "int".into(),
        "boolean" | "bool" => "bool".into(),
        "double" | "real" | "float" => "float".into(),
        "string" => "string".into(),
        "array" => "array".into(),
        "object" => "object".into(),
        "unset" => "null".into(),
        other => other.into(),
    }
}

fn flatten_unknown(children: &mut Vec<Expr>, expr: Expr) {
    if let Kind::Unknown(nested) = expr.kind {
        children.extend(nested);
    } else {
        children.push(expr);
    }
}

fn arguments(u: &Unit, start: usize, end: usize, depth: usize) -> Result<Vec<Expr>, String> {
    let mut result = Vec::new();
    for (a, b) in u.split(start, end, ",") {
        if a >= b {
            continue;
        }
        result.push(parse_depth(u, a, b, depth + 1)?);
    }
    Ok(result)
}

fn array_items(u: &Unit, start: usize, end: usize, depth: usize) -> Result<Vec<ArrayItem>, String> {
    let mut result = Vec::new();
    for (a, b) in u.split(start, end, ",") {
        if a >= b {
            continue;
        }
        let mut key = None;
        let mut value_start = a;
        let mut i = a;
        while i < b {
            if u.is(i, "=>") {
                key = Some(parse_depth(u, a, i, depth + 1)?);
                value_start = i + 1;
                break;
            }
            if let Some(close) = u.pairs[i] {
                if close > i {
                    i = close + 1;
                    continue;
                }
            }
            i += 1;
        }
        if value_start >= b {
            return Err("missing array value".into());
        }
        result.push(ArrayItem {
            key,
            value: parse_depth(u, value_start, b, depth + 1)?,
        });
    }
    Ok(result)
}
