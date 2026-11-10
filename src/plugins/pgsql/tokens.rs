//! Bounded PostgreSQL/SQL lexer. Comments and literals never become declarations.
use crate::analyzer::AnalysisOptions;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Ident,
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
    pub kind: TokenKind,
    /// True when the identifier was written in double quotes (case preserved).
    pub quoted: bool,
}

impl Token {
    pub fn is(&self, value: &str) -> bool {
        !self.quoted && self.text.eq_ignore_ascii_case(value)
    }
    pub fn as_ident(&self) -> Option<&str> {
        matches!(self.kind, TokenKind::Ident).then_some(self.text.as_str())
    }
}

pub fn tokenize(source: &str, options: &AnalysisOptions) -> Result<Vec<Token>, String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let (mut at, mut line) = (0, 1);
    while at < bytes.len() {
        if at % 1024 == 0 {
            options.check(0)?;
        }
        let b = bytes[at];
        if b.is_ascii_whitespace() {
            if b == b'\n' {
                line += 1;
            }
            at += 1;
            continue;
        }
        if b == b'-' && bytes.get(at + 1) == Some(&b'-') {
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at += 2;
            while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                if bytes[at] == b'\n' {
                    line += 1;
                }
                at += 1;
            }
            at = (at + 2).min(bytes.len());
            continue;
        }
        let start = at;
        let first_line = line;
        let (kind, quoted) = if b == b'\'' {
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\'' {
                    if bytes.get(at + 1) == Some(&b'\'') {
                        at += 2;
                        continue;
                    }
                    at += 1;
                    break;
                }
                if bytes[at] == b'\n' {
                    line += 1;
                }
                at += source[at..].chars().next().unwrap().len_utf8();
            }
            (TokenKind::String, false)
        } else if b == b'$' {
            // Dollar-quoted string: $tag$ ... $tag$
            let tag_end = bytes[at + 1..]
                .iter()
                .position(|&c| c == b'$')
                .map(|i| at + 1 + i)
                .ok_or_else(|| format!("unterminated dollar quote at line {line}"))?;
            let tag = &source[at..=tag_end];
            at = tag_end + 1;
            loop {
                if at >= bytes.len() {
                    return Err(format!("unterminated dollar quote at line {first_line}"));
                }
                if source[at..].starts_with(tag) {
                    at += tag.len();
                    break;
                }
                if bytes[at] == b'\n' {
                    line += 1;
                }
                at += source[at..].chars().next().unwrap().len_utf8();
            }
            (TokenKind::String, false)
        } else if b == b'"' {
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'"' {
                    if bytes.get(at + 1) == Some(&b'"') {
                        at += 2;
                        continue;
                    }
                    at += 1;
                    break;
                }
                if bytes[at] == b'\n' {
                    line += 1;
                }
                at += source[at..].chars().next().unwrap().len_utf8();
            }
            let raw = &source[start + 1..at.saturating_sub(1)];
            out.push(Token {
                text: raw.replace("\"\"", "\""),
                start,
                end: at,
                line: first_line,
                kind: TokenKind::Ident,
                quoted: true,
            });
            continue;
        } else if b.is_ascii_digit() {
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
                at += 1;
            }
            (TokenKind::Number, false)
        } else if ident_start(b) {
            at += 1;
            while at < bytes.len() && ident_continue(bytes[at]) {
                at += source[at..].chars().next().unwrap().len_utf8();
            }
            (TokenKind::Ident, false)
        } else {
            let two = source.get(at..at + 2).unwrap_or("");
            if matches!(two, "::" | "<=" | ">=" | "<>" | "!=") {
                at += 2;
            } else {
                at += source[at..].chars().next().unwrap().len_utf8();
            }
            (TokenKind::Symbol, false)
        };
        let text = if quoted {
            unreachable!("quoted identifiers are pushed above")
        } else if kind == TokenKind::Ident {
            source[start..at].to_ascii_lowercase()
        } else {
            source[start..at].into()
        };
        out.push(Token {
            text,
            start,
            end: at,
            line: first_line,
            kind,
            quoted: false,
        });
    }
    options.check(0)?;
    Ok(out)
}

fn ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 128
}
fn ident_continue(byte: u8) -> bool {
    ident_start(byte) || byte.is_ascii_digit()
}

pub fn fingerprint(parts: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.as_ref().bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("pgsql-struct-v1:{hash:016x}")
}
