use crate::domain::*;
use std::collections::{BTreeMap, BTreeSet};

/// Persistence contract. Services own policies; implementations provide atomic storage.
/// Mutations must be enclosed in begin/commit and serialized against other writers.
pub trait Repository: Send {
    fn begin(&mut self) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
    fn rollback(&mut self) -> Result<()>;
    fn nodes(&mut self) -> Result<Vec<Node>>;
    fn edges(&mut self) -> Result<Vec<Edge>>;
    fn node(&mut self, id: &str) -> Result<Option<Node>>;
    fn edge(&mut self, id: &str) -> Result<Option<Edge>>;
    fn put_node(&mut self, node: &Node) -> Result<()>;
    fn put_edge(&mut self, edge: &Edge) -> Result<()>;
    fn delete_node(&mut self, id: &str) -> Result<()>;
    fn delete_edge(&mut self, id: &str) -> Result<()>;
    fn corrections(&mut self) -> Result<Vec<StoredCorrection>>;
    fn put_correction(&mut self, correction: &StoredCorrection) -> Result<()>;
    fn diagnostics(&mut self) -> Result<Vec<Diagnostic>>;
    fn set_diagnostics(&mut self, diagnostics: &[Diagnostic]) -> Result<()>;
    fn delivery(&mut self, id: &str) -> Result<Option<Delivery>>;
    fn deliveries(&mut self, from: i64, to: i64) -> Result<Vec<Delivery>>;
    fn put_delivery(&mut self, delivery: &Delivery) -> Result<()>;
    fn meta(&mut self, key: &str) -> Result<Option<String>>;
    fn set_meta(&mut self, key: &str, value: &str) -> Result<()>;
    /// Removes every stored atlas fact: graph, diagnostics, corrections, deliveries, meta.
    fn reset(&mut self) -> Result<()>;
    /// Which of `ids` are stored nodes (one round trip for a partial publication).
    fn existing_node_ids(&mut self, ids: &BTreeSet<String>) -> Result<BTreeSet<String>> {
        let mut found = BTreeSet::new();
        for id in ids {
            if self.node(id)?.is_some() {
                found.insert(id.clone());
            }
        }
        Ok(found)
    }

    fn put_nodes(&mut self, nodes: &[Node]) -> Result<()> {
        for node in nodes {
            self.put_node(node)?;
        }
        Ok(())
    }
    fn put_edges(&mut self, edges: &[Edge]) -> Result<()> {
        for edge in edges {
            self.put_edge(edge)?;
        }
        Ok(())
    }
    fn delete_nodes(&mut self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.delete_node(id)?;
        }
        Ok(())
    }
    fn delete_edges(&mut self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.delete_edge(id)?;
        }
        Ok(())
    }

    /// Load nodes whose span.path is in `paths`. Empty paths → empty result.
    fn nodes_for_paths(&mut self, paths: &BTreeSet<String>) -> Result<Vec<Node>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .nodes()?
            .into_iter()
            .filter(|n| paths.contains(&n.span.path))
            .collect())
    }

    /// Load edges whose span.path is in `paths`, plus edges that touch those nodes.
    fn edges_for_paths(
        &mut self,
        paths: &BTreeSet<String>,
        node_ids: &BTreeSet<String>,
    ) -> Result<Vec<Edge>> {
        if paths.is_empty() && node_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .edges()?
            .into_iter()
            .filter(|e| {
                paths.contains(&e.span.path)
                    || node_ids.contains(&e.from)
                    || e.to.as_ref().is_some_and(|id| node_ids.contains(id))
            })
            .collect())
    }

    fn source_paths(&mut self) -> Result<Vec<String>> {
        let mut paths: std::collections::BTreeSet<String> =
            self.nodes()?.into_iter().map(|n| n.span.path).collect();
        paths.extend(self.edges()?.into_iter().map(|e| e.span.path));
        paths.remove("");
        Ok(paths.into_iter().collect())
    }

    /// Matches ordered by relevance then id; `after` is `"{score}\\t{id}"` (or legacy bare id).
    fn search_nodes(&mut self, search: &NodeSearch) -> Result<Vec<Node>> {
        let query = search.text.to_lowercase();
        let symbol = query.trim_start_matches('$').to_owned();
        let after = parse_search_cursor(search.after.as_deref());
        let excluded = search
            .exclude_paths
            .as_deref()
            .map(regex::Regex::new)
            .transpose()
            .map_err(|e| AtlasError::invalid(format!("invalid path exclusion: {e}")))?;
        let included = search
            .include_paths
            .as_deref()
            .map(regex::Regex::new)
            .transpose()
            .map_err(|e| AtlasError::invalid(format!("invalid area filter: {e}")))?;
        let mut ranked: Vec<(u8, Node)> = self
            .nodes()?
            .into_iter()
            .filter(|n| search.kinds.is_empty() || search.kinds.contains(&n.kind))
            .filter(|n| {
                excluded
                    .as_ref()
                    .is_none_or(|re| !re.is_match(&n.span.path))
            })
            .filter(|n| included.as_ref().is_none_or(|re| re.is_match(&n.span.path)))
            .filter(|n| match search.mode {
                SearchMode::Text => search_match(n, &query, contains_identifier, true),
                SearchMode::Substring => search_match(n, &query, |hay, q| hay.contains(q), false),
                SearchMode::Symbol => symbol_match(n, &query, &symbol),
            })
            .map(|n| {
                let score = relevance_score(&n, &search.text, search.mode);
                (score, n)
            })
            .filter(|(score, n)| after.as_ref().is_none_or(|a| (*score, n.id.as_str()) > *a))
            .collect();
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
        Ok(ranked
            .into_iter()
            .take(search.limit)
            .map(|(_, n)| n)
            .collect())
    }
    fn adjacent_edges(&mut self, id: &str) -> Result<Vec<Edge>> {
        Ok(self
            .edges()?
            .into_iter()
            .filter(|e| e.from == id || e.to.as_deref() == Some(id))
            .collect())
    }
}

fn searchable_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "method"
            | "function"
            | "variable"
            | "property"
            | "trait"
            | "interface"
            | "enum"
            | "route"
            | "event"
            | "view"
            | "config_key"
    )
}

/// Lower is better: exact name/id → declarations → other → occurrence sites.
pub(crate) fn relevance_score(node: &Node, query: &str, _mode: SearchMode) -> u8 {
    let q = query.to_lowercase();
    let symbol = q.trim_start_matches('$');
    if node.id.eq_ignore_ascii_case(query)
        || node.name.eq_ignore_ascii_case(symbol)
        || (!node.qualified_name.is_empty() && node.qualified_name.eq_ignore_ascii_case(query))
    {
        return 0;
    }
    if searchable_declaration(&node.kind) {
        return 1;
    }
    if matches!(
        node.kind.as_str(),
        "call_site" | "write_site" | "read_site" | "return_site" | "url_site" | "branch"
    ) {
        return 3;
    }
    2
}

pub(crate) fn encode_search_cursor(score: u8, id: &str) -> String {
    // `|` is URL-safe and does not appear in atlas node ids.
    format!("{score}|{id}")
}

fn parse_search_cursor(after: Option<&str>) -> Option<(u8, &str)> {
    let after = after.filter(|c| !c.is_empty())?;
    if let Some((score, id)) = after.split_once('|') {
        let score = score.parse::<u8>().ok()?;
        return Some((score, id));
    }
    // Legacy bare-id / tab cursors.
    if let Some((score, id)) = after.split_once('\t') {
        let score = score.parse::<u8>().ok()?;
        return Some((score, id));
    }
    Some((0, after))
}

/// Exact, case-insensitive: declaration name or qualified name, or the member an
/// occurrence site names. Expression text (and string literals in it) is ignored.
/// `Owner::member` / `Owner::$member` also matches occurrence sites by member + receiver.
/// Write sites also match `value_constant` (`STATUS_REFUSE` / `Owner::STATUS_REFUSE`).
fn symbol_match(node: &Node, query: &str, symbol: &str) -> bool {
    let bare = |value: &str| value.trim_start_matches('$').to_lowercase();
    node.id.eq_ignore_ascii_case(query)
        || (searchable_declaration(&node.kind)
            && (bare(&node.name) == symbol || node.qualified_name.to_lowercase() == query))
        || node.metadata["member_name"]
            .as_str()
            .is_some_and(|member| bare(member) == symbol)
        || value_constant_match(node, query, symbol)
        || qualified_occurrence_match(node, query)
}

fn value_constant_match(node: &Node, query: &str, symbol: &str) -> bool {
    let Some(constant) = node.metadata["value_constant"].as_str() else {
        return false;
    };
    let c = constant.to_lowercase();
    let q = query.trim_start_matches('\\').to_lowercase();
    let c_short = c.rsplit("::").next().unwrap_or(c.as_str());
    let q_short = q.rsplit("::").next().unwrap_or(q.as_str());
    c == q || c_short == q_short || (!symbol.contains("::") && c_short == symbol)
}

/// `Owner::member` matches a site whose member_name is `member` and whose
/// receiver_class / receiver_root_class names Owner (FQCN, short name, or a
/// `|`-union part such as `Withdraw|null`).
fn qualified_occurrence_match(node: &Node, query: &str) -> bool {
    let Some((owner, member)) = query.rsplit_once("::") else {
        return false;
    };
    if owner.is_empty() || member.is_empty() {
        return false;
    }
    let member = member.trim_start_matches('$').to_lowercase();
    let owner = owner.to_lowercase();
    let Some(site_member) = node.metadata["member_name"].as_str() else {
        return false;
    };
    if site_member.trim_start_matches('$').to_lowercase() != member {
        return false;
    }
    ["receiver_class", "receiver_root_class"]
        .iter()
        .filter_map(|key| node.metadata[*key].as_str())
        .any(|class| class_names_owner(class, &owner))
}

/// One union/intersection fragment names `owner` (exact, `\Owner` suffix, or short).
fn class_names_owner(class: &str, owner: &str) -> bool {
    class.split(['|', '&']).map(str::trim).any(|part| {
        if part.is_empty() {
            return false;
        }
        let part = part
            .split('<')
            .next()
            .unwrap_or(part)
            .trim()
            .trim_start_matches('?')
            .to_lowercase();
        part == owner
            || part.rsplit('\\').next().is_some_and(|short| short == owner)
            || part.ends_with(&format!("\\{owner}"))
    })
}

/// `query` is lowercase; `found(haystack, query)` decides how a field matches.
/// `skip_literals`: code fields (name, expression, value_expression) are matched with
/// quoted string contents removed, so log messages that mention a name do not match.
fn search_match(
    node: &Node,
    query: &str,
    found: fn(&str, &str) -> bool,
    skip_literals: bool,
) -> bool {
    let code = |text: &str| {
        let text = text.to_lowercase();
        if skip_literals {
            found(&without_string_literals(&text), query)
        } else {
            found(&text, query)
        }
    };
    if node.id.eq_ignore_ascii_case(query) {
        return true;
    }
    if code(&node.name) {
        return true;
    }
    // A variable's qualified name is its scope (`method:A::b:$x`), not its identity.
    if searchable_declaration(&node.kind)
        && node.kind != "variable"
        && found(&node.qualified_name.to_lowercase(), query)
    {
        return true;
    }
    // Occurrence sites: LHS expression + assigned RHS (STATUS_REFUSE on write_site).
    for key in ["expression", "value_expression"] {
        if node.metadata[key].as_str().is_some_and(code) {
            return true;
        }
    }
    for key in ["value_constant", "member_name"] {
        if node.metadata[key]
            .as_str()
            .is_some_and(|s| found(&s.to_lowercase(), query))
        {
            return true;
        }
    }
    false
}

/// Replaces each quoted PHP string literal (with backslash escapes) by one space; the
/// PostgreSQL search applies the same rule with `regexp_replace`.
pub(crate) fn without_string_literals(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\'' && c != '"' {
            out.push(c);
            continue;
        }
        let quote = c;
        let mut closed = false;
        let mut literal = String::from(c);
        while let Some(inner) = chars.next() {
            literal.push(inner);
            if inner == '\\' {
                if let Some(escaped) = chars.next() {
                    literal.push(escaped);
                }
            } else if inner == quote {
                closed = true;
                break;
            }
        }
        // An unterminated quote is not a literal: keep the text as it was.
        if closed {
            out.push(' ');
        } else {
            out.push_str(&literal);
        }
    }
    out
}

/// `needle` occurs in `haystack` and is not glued to other identifier characters at
/// any edge where the needle itself starts or ends with an identifier character.
fn contains_identifier(haystack: &str, needle: &str) -> bool {
    let (hay, q) = (haystack.as_bytes(), needle.as_bytes());
    let (Some(&first), Some(&last)) = (q.first(), q.last()) else {
        return false;
    };
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    haystack.match_indices(needle).any(|(start, _)| {
        let end = start + q.len();
        (!ident(first) || start == 0 || !ident(hay[start - 1]))
            && (!ident(last) || end == hay.len() || !ident(hay[end]))
    })
}

#[derive(Clone, Default)]
struct MemoryState {
    nodes: BTreeMap<String, Node>,
    edges: BTreeMap<String, Edge>,
    corrections: BTreeMap<String, StoredCorrection>,
    diagnostics: Vec<Diagnostic>,
    deliveries: BTreeMap<String, Delivery>,
    meta: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct MemoryRepository {
    state: MemoryState,
    checkpoint: Option<MemoryState>,
}
impl MemoryRepository {
    pub fn new() -> Self {
        Self::default()
    }
}
impl Repository for MemoryRepository {
    fn begin(&mut self) -> Result<()> {
        if self.checkpoint.is_some() {
            return Err(AtlasError::storage("nested transaction"));
        }
        self.checkpoint = Some(self.state.clone());
        Ok(())
    }
    fn commit(&mut self) -> Result<()> {
        self.checkpoint
            .take()
            .ok_or_else(|| AtlasError::storage("no transaction"))?;
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        if let Some(state) = self.checkpoint.take() {
            self.state = state;
        }
        Ok(())
    }
    fn nodes(&mut self) -> Result<Vec<Node>> {
        Ok(self.state.nodes.values().cloned().collect())
    }
    fn edges(&mut self) -> Result<Vec<Edge>> {
        Ok(self.state.edges.values().cloned().collect())
    }
    fn node(&mut self, id: &str) -> Result<Option<Node>> {
        Ok(self.state.nodes.get(id).cloned())
    }
    fn edge(&mut self, id: &str) -> Result<Option<Edge>> {
        Ok(self.state.edges.get(id).cloned())
    }
    fn put_node(&mut self, n: &Node) -> Result<()> {
        self.state.nodes.insert(n.id.clone(), n.clone());
        Ok(())
    }
    fn put_edge(&mut self, e: &Edge) -> Result<()> {
        self.state.edges.insert(e.id.clone(), e.clone());
        Ok(())
    }
    fn delete_node(&mut self, id: &str) -> Result<()> {
        self.state.nodes.remove(id);
        self.state
            .edges
            .retain(|_, e| e.from != id && e.to.as_deref() != Some(id));
        Ok(())
    }
    fn delete_edge(&mut self, id: &str) -> Result<()> {
        self.state.edges.remove(id);
        Ok(())
    }
    fn corrections(&mut self) -> Result<Vec<StoredCorrection>> {
        Ok(self.state.corrections.values().cloned().collect())
    }
    fn put_correction(&mut self, c: &StoredCorrection) -> Result<()> {
        self.state
            .corrections
            .insert(c.correction.id.clone(), c.clone());
        Ok(())
    }
    fn diagnostics(&mut self) -> Result<Vec<Diagnostic>> {
        Ok(self.state.diagnostics.clone())
    }
    fn set_diagnostics(&mut self, d: &[Diagnostic]) -> Result<()> {
        self.state.diagnostics = d.to_vec();
        Ok(())
    }
    fn delivery(&mut self, id: &str) -> Result<Option<Delivery>> {
        Ok(self.state.deliveries.get(id).cloned())
    }
    fn deliveries(&mut self, from: i64, to: i64) -> Result<Vec<Delivery>> {
        Ok(self
            .state
            .deliveries
            .values()
            .filter(|d| d.timestamp >= from && d.timestamp < to)
            .cloned()
            .collect())
    }
    fn meta(&mut self, key: &str) -> Result<Option<String>> {
        Ok(self.state.meta.get(key).cloned())
    }
    fn set_meta(&mut self, key: &str, value: &str) -> Result<()> {
        self.state.meta.insert(key.into(), value.into());
        Ok(())
    }
    fn reset(&mut self) -> Result<()> {
        self.state = MemoryState::default();
        Ok(())
    }
    fn put_delivery(&mut self, d: &Delivery) -> Result<()> {
        self.state
            .deliveries
            .entry(d.id.clone())
            .or_insert_with(|| d.clone());
        Ok(())
    }
}
