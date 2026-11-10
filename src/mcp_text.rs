//! Presenter: grep-style text for MCP tool results (the web API keeps JSON).
//!
//! Every function is pure: domain structs in, text out. Format reference:
//! `openspec/changes/text-mcp-and-grep-supplement/design.md`.
use crate::domain::{
    CallerNode, CallerTree, Correction, Edge, Freshness, GraphContext, Node, Outline, SearchPage,
};
use crate::grep_search::{GrepHit, GrepPage};
use crate::service::SourceSlice;

use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub fn index_line(freshness: &Freshness) -> String {
    let mut line = format!("# index state={}", freshness.state);
    if let Some(at) = freshness.indexed_at {
        let _ = write!(line, " indexed_at={at}");
    }
    if freshness.changes_pending {
        line.push_str(" changes_pending");
    }
    if !freshness.pending_paths.is_empty() {
        let _ = write!(
            line,
            " pending={}",
            text(&freshness.pending_paths.join(","))
        );
    }
    line
}

/// Result text without its leading `# index …` line (batch sections share one).
pub fn without_index(text: &str) -> &str {
    match text.split_once('\n') {
        Some((first, rest)) if first.starts_with("# index ") => rest,
        _ if text.starts_with("# index ") => "",
        _ => text,
    }
}

/// `path:start[-end]: kind name [resolution] if(cond)… key=value… [qn=…] | id=… [rev=…] [trust=…]`
pub fn node_line(node: &Node) -> String {
    // Occurrence names are the first line of their expression: show the whole
    // expression once instead of a truncated name plus `expression=`.
    let short_name = collapse(&node.name);
    let name = node
        .metadata
        .get("expression")
        .and_then(Value::as_str)
        .map(collapse)
        .filter(|expression| !short_name.is_empty() && expression.starts_with(&short_name))
        .unwrap_or(short_name);
    let area = node.metadata["area"]
        .as_str()
        .map(|area| format!("[{area}] "))
        .unwrap_or_default();
    let mut line = format!("{} {area}{} {}", location(&node.span), node.kind, name);
    let receiver_class = node.metadata.get("receiver_class");
    let shown_elsewhere = |key: &str, value: &Value| match key {
        "expression" => value.as_str().map(collapse).as_deref() == Some(name.as_str()),
        // Shown as the `[area]` label after the location.
        "area" => true,
        // The root class of a receiver is only informative when it differs.
        "receiver_root_class" => Some(value) == receiver_class,
        _ => false,
    };
    push_metadata(&mut line, &node.metadata, &shown_elsewhere);
    if !node.qualified_name.is_empty() {
        let _ = write!(line, " qn={}", text(&node.qualified_name));
    }
    push_identity(&mut line, &node.id, node.revision, node.trust);
    line
}

/// `tests_excluded`: test paths were left out (header says so, include_tests restores).
pub fn search(
    freshness: &Freshness,
    page: &SearchPage,
    grep: Option<&GrepPage>,
    tests_excluded: bool,
) -> String {
    let mut lines = vec![index_line(freshness)];
    let cursor = page
        .next_cursor
        .as_deref()
        .map_or("complete".to_owned(), |c| format!("next_cursor={c}"));
    let tests = if tests_excluded {
        " tests=excluded"
    } else {
        ""
    };
    lines.push(format!(
        "# search nodes={} {cursor}{tests}",
        page.nodes.len()
    ));
    lines.extend(page.nodes.iter().map(node_line));
    if let Some(grep) = grep {
        lines.push(format!(
            "# grep supplement {}",
            grep_summary(grep, "next_grep_cursor")
        ));
        lines.extend(grep_body(grep));
    }
    lines.join("\n")
}

pub fn context(freshness: &Freshness, context: &GraphContext) -> String {
    // Root first, then the remaining nodes in their stable (id) order.
    let mut nodes: Vec<&Node> = context.nodes.iter().collect();
    nodes.sort_by_key(|n| n.id != context.root_id);
    let refs: BTreeMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i + 1))
        .collect();
    let mut header = format!(
        "# context root={} nodes={} edges={} truncated={} depth_limited={}",
        reference(&context.root_id, &refs),
        context.nodes.len(),
        context.edges.len(),
        context.truncated,
        context.depth_limited
    );
    if context.omitted_members > 0 {
        let _ = write!(header, " omitted_members={}", context.omitted_members);
    }
    let mut lines = vec![index_line(freshness), header];
    lines.extend(
        nodes
            .iter()
            .enumerate()
            .map(|(i, n)| format!("[{}] {}", i + 1, node_line(n))),
    );
    if !context.edges.is_empty() {
        lines.push("# edges".into());
        let by_id: BTreeMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), *n)).collect();
        lines.extend(
            context
                .edges
                .iter()
                .map(|e| edge_line(e, &refs, by_id.get(e.from.as_str()).copied())),
        );
    }
    if !context.diagnostics.is_empty() {
        lines.push("# diagnostics".into());
        lines.extend(
            context
                .diagnostics
                .iter()
                .map(|d| format!("{}:{}: {} {}", d.path, d.line, d.code, collapse(&d.message))),
        );
    }
    lines.join("\n")
}

/// Callers tree: header, root line, then `├`/`└` levels (one per call level).
pub fn callers(freshness: &Freshness, tree: &CallerTree) -> String {
    let mut header = format!(
        "# callers {} levels={} {}",
        text(&tree.root_id),
        tree.levels,
        if tree.truncated {
            "truncated"
        } else {
            "complete"
        }
    );
    if tree.excluded_tests > 0 {
        let _ = write!(header, " excluded.tests={}", tree.excluded_tests);
    }
    if tree.name_only_callers > 0 {
        let _ = write!(header, " possible.name_only={}", tree.name_only_callers);
    }
    if tree.members_without_usages > 0 {
        let _ = write!(
            header,
            " members_without_usages={}",
            tree.members_without_usages
        );
    }
    let mut lines = vec![
        index_line(freshness),
        header,
        format!("{} {}:{}", tree.root_label, tree.root_path, tree.root_line),
    ];
    push_caller_levels(&mut lines, &tree.children, "");
    lines.join("\n")
}

fn push_caller_levels(lines: &mut Vec<String>, nodes: &[CallerNode], prefix: &str) {
    for (index, node) in nodes.iter().enumerate() {
        let last = index + 1 == nodes.len();
        let (branch, indent) = if last {
            ("└ ", "  ")
        } else {
            ("├ ", "│ ")
        };
        lines.push(format!("{prefix}{branch}{}", caller_line(node)));
        push_caller_levels(lines, &node.children, &format!("{prefix}{indent}"));
    }
}

/// `label L1 L2 [resolution] if(cond): expression [(note)]`; a class member (no call
/// expression) prints its declaration as `label path:line`.
fn caller_line(node: &CallerNode) -> String {
    if node.expression.is_empty() && node.resolutions.is_empty() {
        let line = node.lines.first().copied().unwrap_or_default();
        return format!("{} {}:{line}", node.label, node.path);
    }
    let mut line = match &node.area {
        Some(area) => format!("[{area}] {}", node.label),
        None => node.label.clone(),
    };
    for number in &node.lines {
        let _ = write!(line, " L{number}");
    }
    if !node.resolutions.is_empty() {
        let _ = write!(line, " [{}]", node.resolutions.join(", "));
    }
    // The innermost conditions are the ones that guard the call; outer ones are elided.
    let shown = node.conditions.len().saturating_sub(TREE_CONDITIONS);
    if shown > 0 {
        line.push_str(" …");
    }
    for condition in &node.conditions[shown..] {
        let _ = write!(line, " if({})", short(&collapse(condition)));
    }
    let _ = write!(line, ": {}", short(&collapse(&node.expression)));
    if let Some(note) = &node.note {
        let _ = write!(line, " ({note})");
    }
    line
}

/// Conditions per callers-tree line (innermost first kept).
const TREE_CONDITIONS: usize = 2;

/// Long call expressions keep their first 100 characters.
fn short(text: &str) -> String {
    if text.chars().count() > 100 {
        format!("{}…", text.chars().take(99).collect::<String>())
    } else {
        text.to_owned()
    }
}

pub fn grep(freshness: &Freshness, page: &GrepPage) -> String {
    let mut lines = vec![
        index_line(freshness),
        format!("# grep {}", grep_summary(page, "next_cursor")),
    ];
    lines.extend(grep_body(page));
    lines.join("\n")
}

pub fn outline(freshness: &Freshness, outline: &Outline) -> String {
    let mut lines = vec![
        index_line(freshness),
        format!(
            "{} {} {}:{} | id={}",
            outline.kind, outline.name, outline.path, outline.line, outline.id
        ),
    ];
    let stated_none = |name: &str| outline.none.iter().any(|n| n == name);
    match &outline.owner {
        Some(owner) => lines.push(format!("owner: {owner}")),
        None if stated_none("owner") => lines.push("owner: none".into()),
        None => {}
    }
    for (name, items) in [
        ("parents", &outline.parents),
        ("interfaces", &outline.interfaces),
        ("traits", &outline.traits),
        ("children", &outline.children),
        ("implementations", &outline.implementations),
        ("overrides", &outline.overrides),
        ("overridden_by", &outline.overridden_by),
        ("calls", &outline.calls),
        ("called_by", &outline.called_by),
        ("members", &outline.members),
    ] {
        // Left-out test callers are stated on the section line.
        let excluded = if name == "called_by" {
            let mut notes = Vec::new();
            if outline.excluded_tests > 0 {
                notes.push(format!("excluded.tests={}", outline.excluded_tests));
            }
            if outline.name_only_callers > 0 {
                notes.push(format!("possible.name_only={}", outline.name_only_callers));
            }
            if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join(", "))
            }
        } else {
            String::new()
        };
        if !items.is_empty() {
            lines.push(format!("{name}{excluded}:"));
            lines.extend(items.iter().map(|item| format!("  {}", collapse(item))));
        } else if stated_none(name) {
            lines.push(format!("{name}: none{excluded}"));
        }
    }
    lines.join("\n")
}

pub fn read(freshness: &Freshness, slice: &SourceSlice) -> String {
    let truncated = if slice.truncated { " truncated" } else { "" };
    let mut lines = vec![
        index_line(freshness),
        format!(
            "# {}:{}-{}{truncated}",
            slice.path, slice.start_line, slice.end_line
        ),
    ];
    lines.extend(
        slice
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}: {line}", slice.start_line + i)),
    );
    lines.join("\n")
}

pub fn correction(correction: &Correction) -> String {
    [
        "# correction".to_owned(),
        format!("id={}", text(&correction.id)),
        format!(
            "target={} {}",
            correction.target_kind,
            text(&correction.target_id)
        ),
        format!("revision={}", correction.revision),
        format!("status={}", text(&correction.status)),
        format!("reason={}", text(&correction.reason)),
        format!("before_trust={}", correction.before_trust),
        format!("before={}", compact(&correction.before)),
        format!("after={}", compact(&correction.after)),
        format!("created_at={}", correction.created_at),
    ]
    .join("\n")
}

/// `[from] -kind-> [to] path:line [resolution] key=value… | id=… rev=…`; metadata
/// equal to the source node's (already on its line) is not repeated.
fn edge_line(edge: &Edge, refs: &BTreeMap<&str, usize>, source: Option<&Node>) -> String {
    let to = edge
        .to
        .as_deref()
        .map_or("?".to_owned(), |to| reference(to, refs));
    let mut line = format!(
        "{} -{}-> {to} {}",
        reference(&edge.from, refs),
        edge.kind,
        span_location(&edge.span)
    );
    let source_name = source.map(|node| collapse(&node.name));
    let repeated = |key: &str, value: &Value| {
        source.is_some_and(|node| node.metadata.get(key) == Some(value))
            || (key == "expression"
                && value.as_str().map(collapse).as_deref() == source_name.as_deref())
    };
    push_metadata(&mut line, &edge.metadata, &repeated);
    push_identity(&mut line, &edge.id, edge.revision, edge.trust);
    line
}

fn reference(id: &str, refs: &BTreeMap<&str, usize>) -> String {
    refs.get(id)
        .map_or_else(|| text(id), |index| format!("[{index}]"))
}

/// Summary tokens shared by `atlas_grep` and the search supplement.
fn grep_summary(page: &GrepPage, cursor_key: &str) -> String {
    let mut tokens = Vec::new();
    if !page.files.is_empty() {
        tokens.push(format!("files={}", page.files.len()));
    } else if !page.counts.is_empty() {
        tokens.push(format!("files={}", page.counts.len()));
    } else {
        tokens.push(format!("hits={}", page.hits.len()));
    }
    tokens.push(format!("scanned={}", page.scope.file_count));
    match &page.next_cursor {
        Some(cursor) => tokens.push(format!("{cursor_key}={cursor}")),
        // `complete` only when nothing is left out: no next page and a full scan.
        None if !page.incomplete => tokens.push("complete".into()),
        None => {}
    }
    if page.incomplete {
        tokens.push("incomplete".into());
    }
    if let Some(prefix) = &page.scope.path_prefix {
        tokens.push(format!("path_prefix={}", text(prefix)));
    }
    tokens.extend(
        page.scope
            .path_globs
            .iter()
            .map(|glob| format!("path_glob={}", text(glob))),
    );
    if page.scope.include_vendor {
        tokens.push("include_vendor".into());
    }
    tokens.extend(
        page.excluded
            .iter()
            .map(|(reason, count)| format!("excluded.{reason}={count}")),
    );
    tokens.join(" ")
}

fn grep_body(page: &GrepPage) -> Vec<String> {
    let mut lines: Vec<String> = page.hits.iter().flat_map(hit_lines).collect();
    lines.extend(page.files.iter().cloned());
    lines.extend(page.counts.iter().map(|(path, n)| format!("{path}:{n}")));
    lines.extend(
        page.skipped
            .iter()
            .map(|s| format!("# skipped {} {}", s.path, s.reason)),
    );
    lines.extend(
        page.errors
            .iter()
            .map(|e| format!("# error {}: {}", e.path, collapse(&e.message))),
    );
    lines
}

/// ripgrep layout: `path-N- text` context, `path:N:col: text` match.
fn hit_lines(hit: &GrepHit) -> Vec<String> {
    let first_before = hit.line.saturating_sub(hit.before.len());
    let mut lines: Vec<String> = hit
        .before
        .iter()
        .enumerate()
        .map(|(i, text)| format!("{}-{}- {text}", hit.path, first_before + i))
        .collect();
    let column = hit.column.map(|c| format!("{c}:")).unwrap_or_default();
    let commented = if hit.commented { " [commented]" } else { "" };
    lines.push(format!(
        "{}:{}:{column} {}{commented}",
        hit.path, hit.line, hit.text
    ));
    lines.extend(
        hit.after
            .iter()
            .enumerate()
            .map(|(i, text)| format!("{}-{}- {text}", hit.path, hit.line + 1 + i)),
    );
    lines
}

/// ` [resolution] if(cond)… key=value…` with keys sorted; `skip(key, value)` drops
/// fields already shown elsewhere on the line.
fn push_metadata(line: &mut String, metadata: &Value, skip: &dyn Fn(&str, &Value) -> bool) {
    let Some(fields) = metadata.as_object() else {
        return;
    };
    if let Some(resolution) = fields
        .get("resolution")
        .filter(|v| !skip("resolution", v))
        .and_then(Value::as_str)
    {
        let _ = write!(line, " [{resolution}]");
    }
    if let Some(conditions) = fields
        .get("conditions")
        .filter(|v| !skip("conditions", v))
        .and_then(Value::as_array)
    {
        for condition in conditions {
            match condition.as_str() {
                Some(c) => {
                    let _ = write!(line, " if({})", collapse(c));
                }
                None => {
                    let _ = write!(line, " if({})", compact(condition));
                }
            }
        }
    }
    let mut keys: Vec<&String> = fields.keys().collect();
    keys.sort();
    for key in keys {
        let value = &fields[key];
        if key == "resolution" || key == "conditions" || skip(key, value) {
            continue;
        }
        match value {
            Value::Bool(true) => {
                let _ = write!(line, " {key}");
            }
            Value::Null | Value::Bool(false) => {}
            Value::String(s) if s.is_empty() => {}
            Value::Array(items) if items.is_empty() => {}
            Value::Object(map) if map.is_empty() => {}
            Value::String(s) => {
                let _ = write!(line, " {key}={}", text(s));
            }
            other => {
                let _ = write!(line, " {key}={}", compact(other));
            }
        }
    }
}

fn push_identity(line: &mut String, id: &str, revision: u64, trust: u8) {
    let _ = write!(line, " | id={}", text(id));
    if revision > 0 {
        let _ = write!(line, " rev={revision}");
    }
    if trust != 1 {
        let _ = write!(line, " trust={trust}");
    }
}

/// `path:start[-end]:` (grep prefix of a node line).
fn location(span: &crate::domain::SourceSpan) -> String {
    format!("{}:", span_location(span))
}

fn span_location(span: &crate::domain::SourceSpan) -> String {
    let path = if span.path.is_empty() {
        "-"
    } else {
        &span.path
    };
    if span.end_line > span.start_line {
        format!("{path}:{}-{}", span.start_line, span.end_line)
    } else {
        format!("{path}:{}", span.start_line)
    }
}

/// Raw when unambiguous on a space-separated line, JSON-quoted otherwise. A bare `|`
/// (union types) is safe: only ` | ` with spaces separates the identity part.
fn text(value: &str) -> String {
    let ambiguous = value.is_empty()
        || value.chars().any(|c| c.is_whitespace() || c == '"')
        || value.starts_with('[')
        || value.starts_with('{');
    if ambiguous {
        serde_json::to_string(value).unwrap_or_default()
    } else {
        value.to_owned()
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Whitespace runs (including newlines of multi-line expressions) become one space.
fn collapse(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SourceSpan;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn fresh() -> Freshness {
        Freshness {
            indexed_at: Some(1_794_291_574),
            state: "completed".into(),
            changes_pending: false,
            pending_paths: vec![],
        }
    }

    fn call_site() -> Node {
        Node {
            id: "call_site:m:1".into(),
            kind: "call_site".into(),
            name: "$this->checkLifetime()".into(),
            qualified_name: String::new(),
            span: SourceSpan {
                path: "common/models/BonusCod.php".into(),
                start_line: 399,
                end_line: 399,
            },
            metadata: json!({
                "resolution": "resolved",
                "conditions": ["!($this->used_type != $typeToUse)"],
                "expression": "$this->checkLifetime()",
                "member_name": "checkLifetime",
                "receiver_class": "common\\models\\BonusCod",
                "commented": false,
                "magic": true,
                "parameters": [{"name": "userId", "type": "int"}],
                "note": "two words",
                "type": "common\\models\\BonusCod|null",
            }),
            trust: 1,
            revision: 2,
        }
    }

    fn method() -> Node {
        Node {
            id: "method:common\\models\\BonusCod::checkLifetime".into(),
            kind: "method".into(),
            name: "checkLifetime".into(),
            qualified_name: String::new(),
            span: SourceSpan {
                path: "common/models/BonusCod.php".into(),
                start_line: 674,
                end_line: 680,
            },
            metadata: json!({}),
            trust: 2,
            revision: 0,
        }
    }

    #[test]
    fn index_line_reports_state_time_and_pending_paths() {
        assert_eq!(
            index_line(&fresh()),
            "# index state=completed indexed_at=1794291574"
        );
        let pending = Freshness {
            changes_pending: true,
            pending_paths: vec!["a.php".into(), "b.php".into()],
            state: "running".into(),
            ..fresh()
        };
        assert_eq!(
            index_line(&pending),
            "# index state=running indexed_at=1794291574 changes_pending pending=a.php,b.php"
        );
    }

    #[test]
    fn node_line_is_grep_like_and_keeps_every_field() {
        assert_eq!(
            node_line(&call_site()),
            "common/models/BonusCod.php:399: call_site $this->checkLifetime() [resolved] \
             if(!($this->used_type != $typeToUse)) magic member_name=checkLifetime \
             note=\"two words\" parameters=[{\"name\":\"userId\",\"type\":\"int\"}] \
             receiver_class=common\\models\\BonusCod type=common\\models\\BonusCod|null \
             | id=call_site:m:1 rev=2"
        );
        assert_eq!(
            node_line(&method()),
            "common/models/BonusCod.php:674-680: method checkLifetime \
             | id=method:common\\models\\BonusCod::checkLifetime trust=2"
        );
    }

    #[test]
    fn truncated_name_gives_way_to_the_full_expression_once() {
        let mut node = call_site();
        node.name = "Freespin::createFreeSpin( $user->uid,".into();
        node.metadata = json!({
            "expression": "Freespin::createFreeSpin(\n    $user->uid,\n    $currency\n)",
            "receiver_class": "common\\models\\Freespin",
            "receiver_root_class": "common\\models\\Freespin",
        });
        assert_eq!(
            node_line(&node),
            "common/models/BonusCod.php:399: call_site Freespin::createFreeSpin( $user->uid, $currency ) \
             receiver_class=common\\models\\Freespin | id=call_site:m:1 rev=2"
        );
    }

    #[test]
    fn callers_tree_keeps_the_two_innermost_conditions() {
        let tree = CallerTree {
            root_id: "method:A::a".into(),
            root_label: "A::a".into(),
            root_path: "a.php".into(),
            root_line: 1,
            levels: 1,
            children: vec![CallerNode {
                label: "B::b".into(),
                lines: vec![9],
                resolutions: vec!["resolved".into()],
                conditions: vec!["$outer".into(), "$middle".into(), "$inner".into()],
                expression: "$a->a()".into(),
                ..CallerNode::default()
            }],
            ..CallerTree::default()
        };
        assert_eq!(
            callers(&fresh(), &tree).lines().last().unwrap(),
            "└ B::b L9 [resolved] … if($middle) if($inner): $a->a()"
        );
    }

    #[test]
    fn area_label_follows_the_location() {
        let mut node = method();
        node.metadata = json!({"area": "partner", "namespace": "partner\\models"});
        let line = node_line(&node);
        assert!(
            line.starts_with("common/models/BonusCod.php:674-680: [partner] method checkLifetime "),
            "{line}"
        );
        assert!(!line.contains("area="), "{line}");
        let tree = CallerTree {
            root_id: "method:A::a".into(),
            root_label: "A::a".into(),
            root_path: "a.php".into(),
            root_line: 1,
            levels: 1,
            children: vec![CallerNode {
                label: "B::b".into(),
                area: Some("admin".into()),
                lines: vec![9],
                resolutions: vec!["resolved".into()],
                expression: "$a->a()".into(),
                ..CallerNode::default()
            }],
            ..CallerTree::default()
        };
        assert_eq!(
            callers(&fresh(), &tree).lines().last().unwrap(),
            "└ [admin] B::b L9 [resolved]: $a->a()"
        );
    }

    #[test]
    fn multiline_names_collapse_and_qualified_name_is_kept_when_present() {
        let mut node = method();
        node.name = "foo(\n    $a,\n    $b\n)".into();
        node.qualified_name = "Other::foo".into();
        let line = node_line(&node);
        assert!(
            line.contains(": method foo( $a, $b ) qn=Other::foo |"),
            "{line}"
        );
    }

    #[test]
    fn search_lists_nodes_then_cursor_and_grep_supplement() {
        let page = SearchPage {
            nodes: vec![method(), call_site()],
            next_cursor: Some("1|x".into()),
        };
        let grep = GrepPage {
            hits: vec![GrepHit {
                path: "a.php".into(),
                line: 9,
                column: Some(4),
                text: "// checkLifetime note".into(),
                commented: true,
                before: vec![],
                after: vec![],
            }],
            excluded: BTreeMap::from([("vendor_excluded".into(), 3)]),
            scope: crate::grep_search::GrepScope {
                file_count: 10,
                ..Default::default()
            },
            ..GrepPage::default()
        };
        let text = search(&fresh(), &page, Some(&grep), false);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "# index state=completed indexed_at=1794291574");
        assert_eq!(lines[1], "# search nodes=2 next_cursor=1|x");
        assert!(lines[2].starts_with("common/models/BonusCod.php:674-680: method"));
        assert!(lines[3].starts_with("common/models/BonusCod.php:399: call_site"));
        assert_eq!(
            lines[4],
            "# grep supplement hits=1 scanned=10 complete excluded.vendor_excluded=3"
        );
        assert_eq!(lines[5], "a.php:9:4: // checkLifetime note [commented]");
    }

    #[test]
    fn context_numbers_nodes_root_first_and_edges_reference_them() {
        let edge = Edge {
            id: "edge:1".into(),
            kind: "calls".into(),
            from: "call_site:m:1".into(),
            to: Some("method:common\\models\\BonusCod::checkLifetime".into()),
            span: SourceSpan {
                path: "common/models/BonusCod.php".into(),
                start_line: 399,
                end_line: 399,
            },
            metadata: json!({"resolution": "resolved"}),
            trust: 1,
            revision: 1,
        };
        let external = Edge {
            id: "edge:2".into(),
            to: None,
            from: "call_site:m:1".into(),
            ..edge.clone()
        };
        let graph = GraphContext {
            root_id: "method:common\\models\\BonusCod::checkLifetime".into(),
            nodes: vec![call_site(), method()],
            edges: vec![edge, external],
            diagnostics: vec![crate::domain::Diagnostic {
                path: "a.php".into(),
                line: 3,
                code: "unresolved_call".into(),
                message: "cannot resolve".into(),
            }],
            truncated: false,
            depth_limited: true,
            omitted_members: 4,
        };
        let text = context(&fresh(), &graph);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[1],
            "# context root=[1] nodes=2 edges=2 truncated=false depth_limited=true omitted_members=4"
        );
        assert!(lines[2].starts_with("[1] common/models/BonusCod.php:674-680: method"));
        assert!(lines[3].starts_with("[2] common/models/BonusCod.php:399: call_site"));
        assert_eq!(lines[4], "# edges");
        // The edge repeats nothing its source call site already shows.
        assert_eq!(
            lines[5],
            "[2] -calls-> [1] common/models/BonusCod.php:399 | id=edge:1 rev=1"
        );
        assert!(lines[6].starts_with("[2] -calls-> ? "), "{}", lines[6]);
        assert_eq!(lines[7], "# diagnostics");
        assert_eq!(lines[8], "a.php:3: unresolved_call cannot resolve");
    }

    #[test]
    fn edges_keep_only_metadata_that_differs_from_their_source() {
        let mut edge_metadata = call_site().metadata;
        edge_metadata["arguments"] = json!([{"type": "int"}]);
        edge_metadata["resolution"] = json!("candidate");
        let graph = GraphContext {
            root_id: method().id,
            nodes: vec![method(), call_site()],
            edges: vec![Edge {
                id: "edge:1".into(),
                kind: "calls".into(),
                from: "call_site:m:1".into(),
                to: Some(method().id),
                span: call_site().span,
                metadata: edge_metadata,
                trust: 1,
                revision: 1,
            }],
            ..GraphContext::default()
        };
        let text = context(&fresh(), &graph);
        let edge = text.lines().find(|l| l.contains("-calls->")).unwrap();
        assert_eq!(
            edge,
            "[2] -calls-> [1] common/models/BonusCod.php:399 [candidate] \
             arguments=[{\"type\":\"int\"}] | id=edge:1 rev=1"
        );
    }

    #[test]
    fn grep_follows_ripgrep_conventions() {
        let page = GrepPage {
            hits: vec![GrepHit {
                path: "a.php".into(),
                line: 5,
                column: Some(3),
                text: "$this->pay();".into(),
                commented: false,
                before: vec!["// before".into()],
                after: vec!["}".into()],
            }],
            truncated: true,
            next_cursor: Some("a.php:5".into()),
            skipped: vec![crate::grep_search::GrepSkip {
                path: "big.php".into(),
                reason: "file_too_large:9>1".into(),
            }],
            incomplete: true,
            scope: crate::grep_search::GrepScope {
                file_count: 2,
                ..Default::default()
            },
            ..GrepPage::default()
        };
        let text = grep(&fresh(), &page);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[1],
            "# grep hits=1 scanned=2 next_cursor=a.php:5 incomplete"
        );
        let unfinished = GrepPage {
            incomplete: true,
            ..GrepPage::default()
        };
        assert_eq!(
            grep(&fresh(), &unfinished).lines().nth(1).unwrap(),
            "# grep hits=0 scanned=0 incomplete",
            "a page is not `complete` when the scan was incomplete"
        );
        assert_eq!(lines[2], "a.php-4- // before");
        assert_eq!(lines[3], "a.php:5:3: $this->pay();");
        assert_eq!(lines[4], "a.php-6- }");
        assert_eq!(lines[5], "# skipped big.php file_too_large:9>1");
        let files = GrepPage {
            files: vec!["a.php".into()],
            ..GrepPage::default()
        };
        assert!(grep(&fresh(), &files).lines().any(|l| l == "a.php"));
        let counts = GrepPage {
            counts: BTreeMap::from([("a.php".into(), 3)]),
            ..GrepPage::default()
        };
        assert!(grep(&fresh(), &counts).lines().any(|l| l == "a.php:3"));
    }

    #[test]
    fn outline_has_header_scalars_and_indented_sections() {
        let outline = Outline {
            id: "method:A::pay".into(),
            kind: "method".into(),
            name: "A::pay".into(),
            path: "a.php".into(),
            line: 4,
            owner: Some("class:A".into()),
            called_by: vec!["B::run L9: $a->pay() [resolved]".into()],
            ..Outline::default()
        };
        assert_eq!(
            outline_text_lines(&outline),
            [
                "method A::pay a.php:4 | id=method:A::pay",
                "owner: class:A",
                "called_by:",
                "  B::run L9: $a->pay() [resolved]",
            ]
        );
    }

    #[test]
    fn callers_tree_draws_levels_with_merged_lines() {
        let caller = |label: &str, lines: Vec<usize>, children: Vec<CallerNode>| CallerNode {
            label: label.into(),
            id: Some(format!("method:{label}")),
            path: "a.php".into(),
            lines,
            resolutions: vec!["resolved".into()],
            conditions: vec!["$ok".into()],
            expression: "$x->run(\n  1\n)".into(),
            children,
            note: None,
            area: None,
        };
        let mut recursive = caller("R::r", vec![4], vec![]);
        recursive.note = Some("recursion".into());
        let tree = CallerTree {
            root_id: "method:A::a".into(),
            root_label: "A::a".into(),
            root_path: "src/A.php".into(),
            root_line: 3,
            levels: 3,
            children: vec![
                caller("B::b", vec![20], vec![caller("C::c", vec![10, 12], vec![])]),
                recursive,
            ],
            truncated: false,
            excluded_tests: 2,
            members_without_usages: 0,
            name_only_callers: 0,
        };
        assert_eq!(
            callers(&fresh(), &tree).lines().skip(1).collect::<Vec<_>>(),
            [
                "# callers method:A::a levels=3 complete excluded.tests=2",
                "A::a src/A.php:3",
                "├ B::b L20 [resolved] if($ok): $x->run( 1 )",
                "│ └ C::c L10 L12 [resolved] if($ok): $x->run( 1 )",
                "└ R::r L4 [resolved] if($ok): $x->run( 1 ) (recursion)",
            ]
        );
    }

    #[test]
    fn class_usages_show_members_then_callers() {
        let tree = CallerTree {
            root_id: "class:K".into(),
            root_label: "K".into(),
            root_path: "src/K.php".into(),
            root_line: 1,
            levels: 1,
            children: vec![CallerNode {
                label: "K::m".into(),
                id: Some("method:K::m".into()),
                path: "src/K.php".into(),
                lines: vec![3],
                children: vec![CallerNode {
                    label: "X::x".into(),
                    lines: vec![6],
                    resolutions: vec!["resolved".into()],
                    expression: "$k->m()".into(),
                    ..CallerNode::default()
                }],
                ..CallerNode::default()
            }],
            truncated: true,
            excluded_tests: 0,
            members_without_usages: 2,
            name_only_callers: 0,
        };
        assert_eq!(
            callers(&fresh(), &tree).lines().skip(1).collect::<Vec<_>>(),
            [
                "# callers class:K levels=1 truncated members_without_usages=2",
                "K src/K.php:1",
                "└ K::m src/K.php:3",
                "  └ X::x L6 [resolved]: $k->m()",
            ]
        );
    }

    #[test]
    fn outline_states_empty_requested_sections() {
        let outline = Outline {
            id: "method:I::run".into(),
            kind: "method".into(),
            name: "I::run".into(),
            path: "a.php".into(),
            line: 3,
            overridden_by: vec!["method:A::run".into()],
            none: vec!["owner".into(), "called_by".into()],
            ..Outline::default()
        };
        assert_eq!(
            outline_text_lines(&outline),
            [
                "method I::run a.php:3 | id=method:I::run",
                "owner: none",
                "overridden_by:",
                "  method:A::run",
                "called_by: none",
            ]
        );
    }

    #[test]
    fn counted_name_only_callers_are_stated() {
        let outline = Outline {
            id: "method:P::save".into(),
            kind: "method".into(),
            name: "P::save".into(),
            path: "a.php".into(),
            line: 2,
            none: vec!["called_by".into()],
            excluded_tests: 1,
            name_only_callers: 12,
            ..Outline::default()
        };
        assert_eq!(
            outline_text_lines(&outline)[1],
            "called_by: none (excluded.tests=1, possible.name_only=12)"
        );
        let tree = CallerTree {
            root_id: "method:P::save".into(),
            root_label: "P::save".into(),
            root_path: "a.php".into(),
            root_line: 2,
            levels: 1,
            name_only_callers: 12,
            ..CallerTree::default()
        };
        assert_eq!(
            callers(&fresh(), &tree).lines().nth(1).unwrap(),
            "# callers method:P::save levels=1 complete possible.name_only=12"
        );
    }

    fn outline_text_lines(outline: &Outline) -> Vec<String> {
        super::outline(&fresh(), outline)
            .lines()
            .skip(1)
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn read_prints_numbered_lines_under_a_location_header() {
        let slice = SourceSlice {
            path: "a.php".into(),
            start_line: 4,
            end_line: 5,
            lines: vec!["function pay() {".into(), "}".into()],
            truncated: false,
        };
        let text = read(&fresh(), &slice);
        assert_eq!(
            text.lines().skip(1).collect::<Vec<_>>(),
            ["# a.php:4-5", "4: function pay() {", "5: }"]
        );
    }

    #[test]
    fn correction_lists_every_field() {
        let text = correction(&Correction {
            id: "c1".into(),
            target_kind: "node".into(),
            target_id: "method:A::pay".into(),
            before: json!({"note": "old"}),
            after: json!({"note": "new"}),
            before_trust: 1,
            revision: 3,
            status: "applied".into(),
            reason: "checked by hand".into(),
            created_at: 5,
        });
        for expected in [
            "# correction",
            "id=c1",
            "target=node method:A::pay",
            "revision=3",
            "status=applied",
            "reason=\"checked by hand\"",
            "before={\"note\":\"old\"}",
            "after={\"note\":\"new\"}",
        ] {
            assert!(
                text.lines().any(|l| l == expected),
                "missing {expected}: {text}"
            );
        }
    }
}
