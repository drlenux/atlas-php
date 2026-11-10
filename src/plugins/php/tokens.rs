//! Bounded PHP lexer. Comments and quoted data cannot introduce declarations.
use crate::analyzer::AnalysisOptions;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Ident,
    Variable,
    String,
    Number,
    Symbol,
}
#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub end_line: usize,
    pub kind: TokenKind,
}
impl Token {
    pub fn is(&self, value: &str) -> bool {
        self.text.eq_ignore_ascii_case(value)
    }
}

#[derive(Clone, Debug)]
pub struct Comment {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub end_line: usize,
    pub text: String,
}

pub fn tokenize(source: &str, options: &AnalysisOptions) -> Result<Vec<Token>, String> {
    Ok(lex(source, options)?.0)
}

pub fn lex(source: &str, options: &AnalysisOptions) -> Result<(Vec<Token>, Vec<Comment>), String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut comments = Vec::new();
    let (mut at, mut line) = (0, 1);
    let mut php = false;
    while at < bytes.len() {
        if at % 1024 == 0 {
            options.check(0)?;
        }
        if !php {
            if bytes
                .get(at..at + 5)
                .is_some_and(|tag| tag.eq_ignore_ascii_case(b"<?php"))
                && bytes.get(at + 5).is_none_or(u8::is_ascii_whitespace)
            {
                at += 5;
                php = true;
                continue;
            }
            if source[at..].starts_with("<?=") {
                at += 3;
                php = true;
                continue;
            }
            if bytes[at] == b'\n' {
                line += 1;
            }
            at += source[at..].chars().next().unwrap().len_utf8();
            continue;
        }
        if source[at..].starts_with("?>") {
            at += 2;
            php = false;
            continue;
        }
        if bytes[at].is_ascii_whitespace() {
            if bytes[at] == b'\n' {
                line += 1;
            }
            at += 1;
            continue;
        }
        if source[at..].starts_with("//") || (bytes[at] == b'#' && !source[at..].starts_with("#["))
        {
            let start = at;
            let first_line = line;
            at += if source[at..].starts_with("//") { 2 } else { 1 };
            let text_start = at;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            comments.push(Comment {
                start,
                end: at,
                line: first_line,
                end_line: first_line,
                text: source[text_start..at].trim().into(),
            });
            continue;
        }
        if source[at..].starts_with("/*") {
            let start = at;
            let first_line = line;
            at += 2;
            let text_start = at;
            let mut closed = false;
            while at < bytes.len() {
                if bytes[at..].starts_with(b"*/") {
                    comments.push(Comment {
                        start,
                        end: at + 2,
                        line: first_line,
                        end_line: line,
                        text: source[text_start..at].trim().into(),
                    });
                    at += 2;
                    closed = true;
                    break;
                }
                if bytes[at] == b'\n' {
                    line += 1;
                }
                at += 1;
            }
            if !closed {
                return Err("unterminated block comment".into());
            }
            continue;
        }
        let start = at;
        let first_line = line;
        let kind;
        if matches!(bytes[at], b'\'' | b'"' | b'`') {
            kind = TokenKind::String;
            let quote = bytes[at];
            at += 1;
            let mut closed = false;
            while at < bytes.len() {
                if bytes[at] == b'\n' {
                    line += 1;
                }
                if bytes[at] == b'\\' {
                    at += 1;
                    if at < bytes.len() {
                        if bytes[at] == b'\n' {
                            line += 1;
                        }
                        at += 1;
                    }
                    continue;
                }
                if bytes[at] == quote {
                    at += 1;
                    closed = true;
                    break;
                }
                at += 1;
            }
            if !closed {
                return Err("unterminated quoted string".into());
            }
        } else if source[at..].starts_with("<<<") {
            kind = TokenKind::String;
            at += 3;
            while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
                at += 1;
            }
            let quoted = if at < bytes.len() && matches!(bytes[at], b'\'' | b'"') {
                let q = bytes[at];
                at += 1;
                Some(q)
            } else {
                None
            };
            let name_start = at;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            let label = &source[name_start..at];
            if label.is_empty() {
                return Err("invalid heredoc label".into());
            }
            if let Some(q) = quoted {
                if bytes.get(at) != Some(&q) {
                    return Err("invalid heredoc quote".into());
                }
                at += 1;
            }
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            if at < bytes.len() {
                at += 1;
                line += 1;
            }
            let mut closed = false;
            while at < bytes.len() {
                let begin = at;
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                let text = source[begin..at].trim_start();
                if text.starts_with(label)
                    && text[label.len()..]
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                {
                    at = begin + (source[begin..at].len() - text.len()) + label.len();
                    closed = true;
                    break;
                }
                if at < bytes.len() {
                    at += 1;
                    line += 1;
                }
            }
            if !closed {
                return Err("unterminated heredoc".into());
            }
        } else if bytes[at] == b'$' && bytes.get(at + 1).is_some_and(|b| ident_start(*b)) {
            kind = TokenKind::Variable;
            at += 2;
            while at < bytes.len() && ident_continue(bytes[at]) {
                at += 1;
            }
        } else if ident_start(bytes[at]) || bytes[at] == b'\\' {
            kind = TokenKind::Ident;
            at += 1;
            while at < bytes.len() && (ident_continue(bytes[at]) || bytes[at] == b'\\') {
                at += 1;
            }
        } else if bytes[at].is_ascii_digit() {
            kind = TokenKind::Number;
            at += 1;
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric() || matches!(bytes[at], b'.' | b'_'))
            {
                at += 1;
            }
        } else {
            kind = TokenKind::Symbol;
            let operators = [
                "?->", "===", "!==", "<=>", "??=", "...", "**=", "<<=", ">>=", "->", "::", "=>",
                "==", "!=", "<=", ">=", "&&", "||", "??", "+=", "-=", "*=", "/=", ".=", "%=", "++",
                "--", "**", "<<", ">>", "#[",
            ];
            if let Some(op) = operators.iter().find(|op| source[at..].starts_with(**op)) {
                at += op.len();
            } else {
                at += source[at..].chars().next().unwrap().len_utf8();
            }
        }
        out.push(Token {
            text: source[start..at].into(),
            start,
            end: at,
            line: first_line,
            end_line: line,
            kind,
        });
    }
    options.check(0)?;
    Ok((out, comments))
}
fn ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 128
}
fn ident_continue(byte: u8) -> bool {
    ident_start(byte) || byte.is_ascii_digit()
}

pub fn matching(tokens: &[Token]) -> Result<Vec<Option<usize>>, String> {
    let mut pairs = vec![None; tokens.len()];
    let mut stack: Vec<(usize, &str)> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if t.kind != TokenKind::Symbol {
            continue;
        }
        match t.text.as_str() {
            "(" | "[" | "{" => stack.push((i, &t.text)),
            "#[" => stack.push((i, "[")),
            ")" | "]" | "}" => {
                let Some((open, kind)) = stack.pop() else {
                    return Err(format!("unmatched {} at line {}", t.text, t.line));
                };
                if !matches!(
                    (kind, t.text.as_str()),
                    ("(", ")") | ("[", "]") | ("{", "}")
                ) {
                    return Err(format!("mismatched delimiter at line {}", t.line));
                }
                pairs[i] = Some(open);
                pairs[open] = Some(i);
            }
            _ => {}
        }
    }
    if let Some((i, _)) = stack.last() {
        return Err(format!("unclosed delimiter at line {}", tokens[*i].line));
    }
    Ok(pairs)
}

pub fn fingerprint<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("php-struct-v1:{hash:016x}")
}
