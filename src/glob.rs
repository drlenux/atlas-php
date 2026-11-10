//! Path globs → anchored regular expressions shared by grep path filters, test-path
//! exclusion and PostgreSQL `~` predicates.
//!
//! `*` and `**` match any characters including `/` (so `*.sh` matches at any depth),
//! `?` one character, `[abc]` / `[!abc]` a class, `{a,b}` alternatives.

/// Anchored regex source for `glob` (Rust `regex` and PostgreSQL ARE compatible).
pub fn to_regex(glob: &str) -> String {
    format!("^{}$", body(glob))
}

fn body(glob: &str) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                while i + 1 < chars.len() && chars[i + 1] == '*' {
                    i += 1;
                }
                out.push_str(".*");
            }
            '?' => out.push('.'),
            '[' => match chars[i + 1..].iter().position(|&c| c == ']') {
                Some(len) if len > 0 => {
                    let class: String = chars[i + 1..i + 1 + len].iter().collect();
                    let (negated, members) = match class.strip_prefix('!') {
                        Some(rest) => (true, rest.to_owned()),
                        None => (false, class),
                    };
                    out.push('[');
                    if negated {
                        out.push('^');
                    }
                    out.push_str(&members.replace('\\', "\\\\"));
                    out.push(']');
                    i += len + 1;
                }
                _ => out.push_str("\\["),
            },
            '{' => match chars[i + 1..].iter().position(|&c| c == '}') {
                Some(len) => {
                    let inner: String = chars[i + 1..i + 1 + len].iter().collect();
                    let alternatives: Vec<String> = inner.split(',').map(body).collect();
                    out.push_str(&format!("(?:{})", alternatives.join("|")));
                    i += len + 1;
                }
                None => out.push_str("\\{"),
            },
            c if "\\.+()|^$]}".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// Anchored regex matching any of `globs`; `None` when the list is empty.
pub fn any_regex(globs: &[String]) -> Option<String> {
    if globs.is_empty() {
        return None;
    }
    let parts: Vec<String> = globs
        .iter()
        .map(|g| {
            format!(
                "(?:{})",
                to_regex(g).trim_start_matches('^').trim_end_matches('$')
            )
        })
        .collect();
    Some(format!("^(?:{})$", parts.join("|")))
}

/// Include/exclude filter from `path_glob` entries (`!` prefix excludes).
#[derive(Debug, Default)]
pub struct PathFilter {
    include: Option<regex::Regex>,
    exclude: Option<regex::Regex>,
}

impl PathFilter {
    pub fn new(globs: &[String]) -> Result<Self, String> {
        let (excluded, included): (Vec<String>, Vec<String>) = globs
            .iter()
            .filter(|g| !g.trim().is_empty())
            .map(|g| g.trim().to_owned())
            .partition(|g| g.starts_with('!'));
        let excluded: Vec<String> = excluded
            .into_iter()
            .map(|g| g.trim_start_matches('!').to_owned())
            .collect();
        let compile = |globs: &[String]| -> Result<Option<regex::Regex>, String> {
            any_regex(globs)
                .map(|source| {
                    regex::Regex::new(&source).map_err(|e| format!("invalid path_glob: {e}"))
                })
                .transpose()
        };
        Ok(Self {
            include: compile(&included)?,
            exclude: compile(&excluded)?,
        })
    }

    pub fn allows(&self, path: &str) -> bool {
        self.include.as_ref().is_none_or(|re| re.is_match(path))
            && !self.exclude.as_ref().is_some_and(|re| re.is_match(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    fn matches(glob: &str, path: &str) -> bool {
        Regex::new(&to_regex(glob)).unwrap().is_match(path)
    }

    #[test]
    fn star_and_double_star_cross_directories() {
        assert!(matches("*.sh", "deploy/cron/run.sh"));
        assert!(matches("**/tests/*", "common/tests/unit/A.php"));
        assert!(!matches("*.sh", "deploy/run.shx"));
        assert!(matches("tests/*", "tests/a.php"));
        assert!(!matches("tests/*", "common/tests/a.php"));
    }

    #[test]
    fn question_mark_classes_and_negated_classes() {
        assert!(matches("a?.php", "ab.php"));
        assert!(matches("[ab]*.php", "b.php"));
        assert!(!matches("[!p]*", "partner/x.php"));
        assert!(matches("[!p]*", "admin/x.php"));
    }

    #[test]
    fn alternatives_and_regex_metacharacters_are_literal() {
        assert!(matches("*.{yml,yaml}", "a/b.yaml"));
        assert!(!matches("*.{yml,yaml}", "a/b.json"));
        assert!(matches("a+b(1).php", "a+b(1).php"));
        assert!(!matches("a.php", "aXphp"));
    }

    #[test]
    fn path_filter_includes_and_excludes() {
        let filter = PathFilter::new(&["*.php".into(), "!*/tests/*".into()]).unwrap();
        assert!(filter.allows("common/models/A.php"));
        assert!(!filter.allows("common/tests/ATest.php"));
        assert!(!filter.allows("deploy/run.sh"));
        let only_exclude = PathFilter::new(&["!*.md".into()]).unwrap();
        assert!(only_exclude.allows("run.sh"));
        assert!(!only_exclude.allows("README.md"));
    }

    #[test]
    fn any_regex_combines_globs() {
        let re = Regex::new(&any_regex(&["tests/*".into(), "*Test.php".into()]).unwrap()).unwrap();
        assert!(re.is_match("tests/a.php"));
        assert!(re.is_match("common/FooTest.php"));
        assert!(!re.is_match("common/Foo.php"));
        assert!(any_regex(&[]).is_none());
    }
}
