use crate::{
    domain::*,
    grep_search::{self, GrepPage, GrepRequest},
    progress::IndexingProgress,
    project::SourceWalk,
    repository::Repository,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct AtlasService {
    repository: Box<dyn Repository>,
    progress: IndexingProgress,
}

/// Arguments for live source reads (`atlas_read`).
#[derive(Clone, Debug, Default)]
pub struct ReadSourceRequest<'a> {
    pub path: Option<&'a str>,
    pub entity_id: Option<&'a str>,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub around_line: Option<usize>,
    pub context: usize,
    pub max_lines: usize,
    /// Exclusion patterns of the readable scope besides the built-in ones.
    pub read_excludes: &'a [String],
}
/// Live source lines returned by `atlas_read` / `read_source`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SourceSlice {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: Vec<String>,
    pub truncated: bool,
}

/// Arguments for the `withGrep` supplement of a node search.
#[derive(Clone, Debug)]
pub struct SupplementRequest<'a> {
    /// The node search the supplement completes (text, mode, kinds, node cursor).
    pub search: &'a NodeSearch,
    pub grep_cursor: Option<&'a str>,
    pub include_vendor: bool,
    pub max_file_bytes: u64,
}

/// Grep hits per supplement page.
const SUPPLEMENT_LIMIT: usize = 20;
/// Atlas matches read to decide which grep lines are already covered.
const SUPPLEMENT_COVERAGE_LIMIT: usize = 10_000;

impl AtlasService {
    pub fn new(repository: Box<dyn Repository>) -> Self {
        Self {
            repository,
            progress: IndexingProgress::default(),
        }
    }

    pub fn indexing_progress(&self) -> IndexingProgress {
        self.progress.clone()
    }

    pub fn source_paths(&mut self) -> Result<Vec<String>> {
        self.repository.source_paths()
    }

    /// Live text search over the watched source tree (MCP `atlas_grep`).
    pub fn grep_source(
        &mut self,
        root: &Path,
        walk: SourceWalk<'_>,
        request: GrepRequest<'_>,
    ) -> Result<GrepPage> {
        let paths = self.grep_paths(&walk)?;
        Ok(grep_search::grep_page(root, &paths, &request, &[]))
    }

    /// Graph source paths plus the walked tree (files without graph nodes included).
    fn grep_paths(&mut self, walk: &SourceWalk<'_>) -> Result<Vec<String>> {
        // Indexed paths obey the text search scope too (index and grep scopes differ).
        let mut set: BTreeSet<String> = self
            .source_paths()?
            .into_iter()
            .filter(|path| {
                !crate::project::always_excluded(path)
                    && !crate::project::matches_exclusion(path, walk.excludes)
            })
            .collect();
        if let Ok(walked) = walk.list_paths() {
            set.extend(walked);
        }
        Ok(set.into_iter().collect())
    }

    /// `withGrep`: source lines matching the search that atlas did not return.
    pub fn grep_supplement(
        &mut self,
        root: &Path,
        walk: SourceWalk<'_>,
        request: &SupplementRequest<'_>,
    ) -> Result<Option<GrepPage>> {
        let search = request.search;
        // Node paging must not repeat the supplement; it has its own grep cursor.
        if search.after.is_some() && request.grep_cursor.is_none() {
            return Ok(None);
        }
        let symbol = search.mode == SearchMode::Symbol;
        let needle = grep_search::grep_query_for_mode(&search.text, symbol);
        // Every atlas match for the query, not only the current page, covers its lines.
        // Test nodes still cover their lines, or test code would reappear as grep hits.
        let covered = self.repository.search_nodes(&NodeSearch {
            after: None,
            limit: SUPPLEMENT_COVERAGE_LIMIT,
            exclude_paths: None,
            ..search.clone()
        })?;
        let paths = self.grep_paths(&walk)?;
        Ok(Some(grep_search::grep_page(
            root,
            &paths,
            &GrepRequest {
                query: &needle,
                limit: SUPPLEMENT_LIMIT,
                after: request.grep_cursor,
                kinds: &search.kinds,
                symbol_mode: symbol,
                // Same match rule as the node search: `text` = whole identifier.
                word: search.mode == SearchMode::Text,
                max_file_bytes: request.max_file_bytes,
                include_vendor: request.include_vendor,
                ..GrepRequest::default()
            },
            &covered,
        )))
    }

    /// Live file slice by path or by graph entity span (MCP `atlas_read`).
    pub fn read_source(
        &mut self,
        root: &Path,
        request: &ReadSourceRequest<'_>,
    ) -> Result<SourceSlice> {
        let max_lines = request.max_lines.max(1);
        let (rel, start, end) = if let Some(id) = request.entity_id {
            let node = self
                .repository
                .node(id)?
                .ok_or_else(|| AtlasError::missing("node not found"))?;
            let start = node.span.start_line.max(1);
            let end = node.span.end_line.max(start);
            (node.span.path, start, end)
        } else {
            let rel = request
                .path
                .filter(|p| !p.is_empty())
                .ok_or_else(|| AtlasError::invalid("path or entity_id required"))?
                .replace('\\', "/");
            if rel.contains("..") || Path::new(&rel).is_absolute() {
                return Err(AtlasError::invalid(
                    "path must be relative within the project",
                ));
            }
            let (start, end) = if let Some(around) = request.around_line {
                let ctx = request.context.min(50);
                let start = around.saturating_sub(ctx).max(1);
                let end = around.saturating_add(ctx);
                (start, end)
            } else {
                let start = request.start_line.unwrap_or(1).max(1);
                let end = request
                    .end_line
                    .unwrap_or(start.saturating_add(max_lines.saturating_sub(1)));
                (start, end.max(start))
            };
            (rel, start, end)
        };
        if crate::project::always_excluded(&rel)
            || crate::project::matches_exclusion(&rel, request.read_excludes)
        {
            return Err(AtlasError::invalid("path is outside the readable scope"));
        }
        let (lines, truncated) = grep_search::read_lines(root, &rel, start, end, max_lines)
            .map_err(AtlasError::invalid)?;
        Ok(SourceSlice {
            path: rel,
            start_line: start,
            end_line: start
                .saturating_add(lines.len().saturating_sub(1))
                .max(start),
            lines,
            truncated,
        })
    }

    pub fn index(
        &mut self,
        snapshot: GraphSnapshot,
        indexed_paths: Vec<String>,
        removed_paths: Vec<String>,
    ) -> Result<IndexReport> {
        let progress = self.progress.clone();
        progress.phase("publishing", "waiting_for_writer", None, "items");
        transactional(self.repository.as_mut(), |r| {
            let report = apply_snapshot(r, snapshot, indexed_paths, removed_paths, &progress)?;
            r.set_meta(INDEXED_AT, &now().to_string())?;
            progress.phase("publishing", "commit", None, "items");
            Ok(report)
        })
    }

    /// Full-data search for the reviewer interface.
    pub fn search(&mut self, query: &str, limit: usize) -> Result<Vec<Node>> {
        let search = NodeSearch {
            text: query.into(),
            limit,
            ..NodeSearch::default()
        };
        Ok(self.search_page(&search, Detail::Full)?.nodes)
    }

    pub fn search_page(&mut self, search: &NodeSearch, detail: Detail) -> Result<SearchPage> {
        if search.limit == 0 || search.limit > 10_000 {
            return Err(AtlasError::invalid("search limit must be 1..10000"));
        }
        validate_kinds(&search.kinds)?;
        // One extra row tells whether another page exists without a count query.
        let probe = NodeSearch {
            limit: search.limit + 1,
            ..search.clone()
        };
        let mut nodes = self.repository.search_nodes(&probe)?;
        let next_cursor = if nodes.len() > search.limit {
            nodes.truncate(search.limit);
            nodes.last().map(|n| {
                crate::repository::encode_search_cursor(
                    crate::repository::relevance_score(n, &search.text, search.mode),
                    &n.id,
                )
            })
        } else {
            None
        };
        Ok(SearchPage {
            nodes: nodes.into_iter().map(|n| project_node(n, detail)).collect(),
            next_cursor,
        })
    }

    /// Full-data context in both directions with diagnostics, for the reviewer interface.
    pub fn context(&mut self, root_id: &str, depth: usize, limit: usize) -> Result<GraphContext> {
        self.context_view(&ContextRequest {
            root_id: root_id.into(),
            depth,
            max_nodes: limit,
            direction: Direction::Both,
            detail: Detail::Full,
            include_diagnostics: true,
            expansion: Expansion::Full,
        })
    }

    /// Target id of a membership edge when it is a declaration (not an occurrence
    /// site, branch or local variable). Missing targets are left to the caller.
    fn declaration_target(&mut self, edge: &Edge) -> Result<Option<String>> {
        let Some(target) = edge.to.as_deref() else {
            return Ok(None);
        };
        Ok(self
            .repository
            .node(target)?
            .filter(|node| !is_occurrence(node) && node.kind != "variable")
            .map(|node| node.id))
    }

    pub fn context_view(&mut self, request: &ContextRequest) -> Result<GraphContext> {
        let (depth, limit) = (request.depth, request.max_nodes);
        if limit == 0 || limit > 100_000 || depth > 32 {
            return Err(AtlasError::invalid("invalid context bounds"));
        }
        let root_id = request.root_id.as_str();
        let root = self
            .repository
            .node(root_id)?
            .ok_or_else(|| AtlasError::missing("node not found"))?;
        let mut budget = NodeBudget::new(limit);
        budget.admit(&root);
        let mut nodes = BTreeMap::from([(root_id.to_owned(), root)]);
        let mut edges = BTreeMap::new();
        let mut queue = VecDeque::from([(root_id.to_owned(), 0usize)]);
        let mut truncated = false;
        let mut depth_limited = false;
        let mut omitted_members = BTreeSet::new();
        while let Some((id, distance)) = queue.pop_front() {
            // Occurrence sites always expose one effect hop to their targets;
            // other kinds stop at the requested depth.
            let expand = distance < depth || nodes.get(&id).is_some_and(is_occurrence);
            let mut adjacent = self.repository.adjacent_edges(&id)?;
            // Incoming edges (callers, owners) first: a method's own occurrence
            // sites must not exhaust the occurrence pool before its callers.
            adjacent.sort_by_key(|edge| edge.to.as_deref() != Some(id.as_str()));
            for edge in adjacent {
                if !request.direction.follows(&edge, &id) {
                    continue;
                }
                if request.expansion == Expansion::Focused && id != root_id && is_hierarchy(&edge) {
                    // A parent reached from the root must not list all its other children.
                    continue;
                }
                if request.expansion == Expansion::Focused && is_outgoing_membership(&edge, &id) {
                    // Expansion belong to atlas_outline; only the root's own sites/locals stay.
                    if id != root_id {
                        continue;
                    }
                    if let Some(member) = self.declaration_target(&edge)? {
                        omitted_members.insert(member);
                        continue;
                    }
                }
                let mut fits = true;
                for endpoint in std::iter::once(&edge.from).chain(edge.to.iter()) {
                    if nodes.contains_key(endpoint) {
                        continue;
                    }
                    if !expand {
                        depth_limited = true;
                        fits = false;
                        break;
                    }
                    let Some(node) = self.repository.node(endpoint)? else {
                        fits = false;
                        continue;
                    };
                    if !budget.admit(&node) {
                        truncated = true;
                        fits = false;
                        break;
                    }
                    nodes.insert(endpoint.clone(), node);
                    queue.push_back((endpoint.clone(), distance + 1));
                }
                if fits {
                    if edges.len() < limit.saturating_mul(8) {
                        edges.insert(edge.id.clone(), edge);
                    } else {
                        truncated = true;
                    }
                }
            }
        }
        let diagnostics = if request.include_diagnostics {
            let paths: BTreeSet<_> = nodes.values().map(|n| n.span.path.as_str()).collect();
            self.repository
                .diagnostics()?
                .into_iter()
                .filter(|d| paths.contains(d.path.as_str()))
                .take(1000)
                .collect()
        } else {
            Vec::new()
        };
        Ok(GraphContext {
            root_id: root_id.into(),
            nodes: nodes
                .into_values()
                .map(|n| project_node(n, request.detail))
                .collect(),
            edges: edges
                .into_values()
                .map(|e| project_edge(e, request.detail))
                .collect(),
            diagnostics,
            truncated,
            depth_limited,
            omitted_members: omitted_members.len(),
        })
    }

    /// Compact structure of a class-like node or a method: members as one line each
    /// and the hierarchy around it. No bodies, returns or analyzer metadata.
    /// Whether `id` names a class, interface, trait or enum (false when missing).
    pub fn is_class_like(&mut self, id: &str) -> Result<bool> {
        Ok(self
            .repository
            .node(id)?
            .is_some_and(|node| is_class_like(&node.kind)))
    }

    /// Callers tree (MCP `atlas_context direction:in`): one level per call level, sites
    /// of one caller merged; a class-like root lists its members used from outside.
    pub fn callers_tree(&mut self, request: &CallersRequest) -> Result<CallerTree> {
        let root = self
            .repository
            .node(&request.root_id)?
            .ok_or_else(|| AtlasError::missing("node not found"))?;
        let mut walk = CallerWalk::new(request)?;
        let mut tree = CallerTree {
            root_id: root.id.clone(),
            root_label: symbol_name(&root.id),
            root_path: root.span.path.clone(),
            root_line: root.span.start_line,
            levels: request.levels,
            ..CallerTree::default()
        };
        if is_class_like(&root.kind) {
            let (members, unused) = self.member_usages(&root, request.levels, &mut walk)?;
            tree.children = members;
            tree.members_without_usages = unused;
        } else if request.levels > 0 {
            let edges = self.repository.adjacent_edges(&root.id)?;
            let mut sites = self.caller_sites(&edges, &root.id, &CALL_KINDS)?;
            let (guesses, counted) = self.name_only_sites(&root)?;
            sites.extend(guesses);
            sites.sort_by(|a, b| (&a.owner_label, a.line).cmp(&(&b.owner_label, b.line)));
            tree.name_only_callers = counted;
            let mut path = vec![root.id.clone()];
            tree.children = self.caller_nodes(sites, request.levels, &mut path, &mut walk)?;
        }
        tree.truncated = walk.truncated;
        tree.excluded_tests = walk.excluded_tests;
        Ok(tree)
    }

    /// Members of a class-like root with their callers outside the class.
    fn member_usages(
        &mut self,
        root: &Node,
        levels: usize,
        walk: &mut CallerWalk,
    ) -> Result<(Vec<CallerNode>, usize)> {
        let qualified = symbol_name(&root.id);
        let internal = format!("method:{qualified}::");
        let mut members = Vec::new();
        for edge in self.repository.adjacent_edges(&root.id)? {
            if edge.kind != "contains" || edge.from != root.id {
                continue;
            }
            let Some(member) = edge.to.as_deref().map(|to| self.repository.node(to)) else {
                continue;
            };
            if let Some(member) = member?.filter(|m| {
                matches!(
                    m.kind.as_str(),
                    "method" | "function" | "property" | "constant"
                )
            }) {
                members.push(member);
            }
        }
        members.sort_by_key(|m| (m.span.start_line, m.id.clone()));
        let mut used = Vec::new();
        let mut unused = 0;
        for member in members {
            let edges = self.repository.adjacent_edges(&member.id)?;
            let sites: Vec<CallerSite> = self
                .caller_sites(&edges, &member.id, &USAGE_KINDS)?
                .into_iter()
                .filter(|site| {
                    site.owner_id
                        .as_deref()
                        .is_none_or(|owner| !owner.starts_with(&internal))
                })
                .collect();
            let mut path = vec![root.id.clone(), member.id.clone()];
            let callers = self.caller_nodes(sites, levels, &mut path, walk)?;
            if callers.is_empty() {
                unused += 1;
                continue;
            }
            used.push(CallerNode {
                label: symbol_name(&member.id),
                id: Some(member.id.clone()),
                path: member.span.path.clone(),
                lines: vec![member.span.start_line],
                children: callers,
                ..CallerNode::default()
            });
        }
        Ok((used, unused))
    }

    /// Callers of `target` down to `levels` call levels.
    fn callers(
        &mut self,
        target: &str,
        levels: usize,
        path: &mut Vec<String>,
        walk: &mut CallerWalk,
    ) -> Result<Vec<CallerNode>> {
        if levels == 0 {
            return Ok(Vec::new());
        }
        let edges = self.repository.adjacent_edges(target)?;
        let sites = self.caller_sites(&edges, target, &CALL_KINDS)?;
        self.caller_nodes(sites, levels, path, walk)
    }

    /// Groups sites by caller (sites arrive sorted by caller, line) and expands callers
    /// that are functions/methods one level further.
    fn caller_nodes(
        &mut self,
        sites: Vec<CallerSite>,
        levels: usize,
        path: &mut Vec<String>,
        walk: &mut CallerWalk,
    ) -> Result<Vec<CallerNode>> {
        let mut groups: Vec<(CallerSite, Vec<CallerSite>)> = Vec::new();
        for site in sites {
            if walk.is_outside_area(&site.path) {
                continue;
            }
            if walk.is_excluded(&site.path) {
                walk.excluded_tests += 1;
                continue;
            }
            match groups.last_mut() {
                Some((first, rest))
                    if first.owner_id == site.owner_id && first.owner_label == site.owner_label =>
                {
                    rest.push(site)
                }
                _ => groups.push((site, Vec::new())),
            }
        }
        let mut nodes = Vec::new();
        for (first, rest) in groups {
            if !walk.admit() {
                break;
            }
            let mut lines: Vec<usize> = std::iter::once(first.line)
                .chain(rest.iter().map(|s| s.line))
                .collect();
            lines.sort_unstable();
            lines.dedup();
            let mut resolutions: Vec<String> = Vec::new();
            for flag in std::iter::once(&first)
                .chain(rest.iter())
                .flat_map(|s| s.flags.iter())
            {
                if !resolutions.contains(flag) {
                    resolutions.push(flag.clone());
                }
            }
            let mut node = CallerNode {
                label: first.owner_label.clone(),
                id: first.owner_id.clone(),
                path: first.path.clone(),
                lines,
                resolutions,
                conditions: first.conditions.clone(),
                expression: first.expression.clone(),
                area: first.area.clone(),
                ..CallerNode::default()
            };
            if let Some(owner) = first.owner_id.as_deref() {
                if path.iter().any(|on_path| on_path == owner) {
                    node.note = Some("recursion".into());
                } else if levels > 1 && is_callable_id(owner) {
                    path.push(owner.to_owned());
                    node.children = self.callers(owner, levels - 1, path, walk)?;
                    path.pop();
                }
            }
            nodes.push(node);
        }
        Ok(nodes)
    }

    pub fn outline(&mut self, id: &str) -> Result<Outline> {
        Ok(self.outline_with_usage(id, &[])?.0)
    }

    /// Outline plus the node/edge ids it actually read (for usage analytics).
    /// `sections` empty → full outline; otherwise only named fields
    /// (`called_by`, `calls`, `members`, `parents`, …).
    pub fn outline_with_usage(
        &mut self,
        id: &str,
        sections: &[String],
    ) -> Result<(Outline, Vec<String>, Vec<String>)> {
        self.outline_view(id, sections, None, None)
    }

    /// `outline_with_usage` whose `called_by` leaves out sites under `exclude_paths`
    /// (anchored path regex, e.g. test code) and counts them.
    pub fn outline_view(
        &mut self,
        id: &str,
        sections: &[String],
        exclude_paths: Option<&str>,
        include_paths: Option<&str>,
    ) -> Result<(Outline, Vec<String>, Vec<String>)> {
        let include = include_paths
            .map(regex::Regex::new)
            .transpose()
            .map_err(|e| AtlasError::invalid(format!("invalid area filter: {e}")))?;
        let exclude = exclude_paths
            .map(regex::Regex::new)
            .transpose()
            .map_err(|e| AtlasError::invalid(format!("invalid path exclusion: {e}")))?;
        let mut want = OutlineSections::parse(sections)?;
        let node = self
            .repository
            .node(id)?
            .ok_or_else(|| AtlasError::missing("node not found"))?;
        // Default method outline skips the redundant single-line `members` copy of itself.
        if sections.is_empty() && matches!(node.kind.as_str(), "method" | "function") {
            want.members = false;
            want.parents = false;
            want.interfaces = false;
            want.traits = false;
            want.children = false;
            want.implementations = false;
        }
        let edges = self.repository.adjacent_edges(id)?;
        let mut used_nodes = vec![id.to_owned()];
        let mut used_edges: Vec<String> = edges.iter().map(|e| e.id.clone()).collect();
        let outgoing = |kind: &str| -> Vec<String> {
            let mut ids: Vec<String> = edges
                .iter()
                .filter(|e| e.kind == kind && e.from == id)
                .map(|e| e.to.clone().unwrap_or_else(|| unindexed(e)))
                .collect();
            ids.sort();
            ids
        };
        let incoming = |kind: &str| -> Vec<String> {
            let mut ids: Vec<String> = edges
                .iter()
                .filter(|e| e.kind == kind && e.to.as_deref() == Some(id))
                .map(|e| e.from.clone())
                .collect();
            ids.sort();
            ids
        };
        let mut outline = Outline {
            id: node.id.clone(),
            kind: node.kind.clone(),
            name: if node.qualified_name.is_empty() {
                node.name.clone()
            } else {
                node.qualified_name.clone()
            },
            path: node.span.path.clone(),
            line: node.span.start_line,
            ..Outline::default()
        };
        if matches!(node.kind.as_str(), "method" | "function") {
            if want.owner {
                outline.owner = incoming("contains").into_iter().next();
                if let Some(owner) = &outline.owner {
                    used_nodes.push(owner.clone());
                }
            }
            if want.overrides {
                outline.overrides = outgoing("overrides");
                used_nodes.extend(outline.overrides.iter().cloned());
            }
            if want.overridden_by {
                outline.overridden_by = incoming("overrides");
                used_nodes.extend(outline.overridden_by.iter().cloned());
            }
            if want.members {
                outline.members = vec![member_line(&node)];
            }
            if want.calls {
                let (calls, call_edges, call_nodes) = self.outgoing_calls_usage(&edges, id)?;
                outline.calls = calls;
                used_edges.extend(call_edges);
                used_nodes.extend(call_nodes);
            }
            if want.called_by {
                let mut sites = self.caller_sites(&edges, id, &CALL_KINDS)?;
                let (guesses, counted) = self.name_only_sites(&node)?;
                sites.extend(guesses);
                // `area` filter: callers outside the requested areas are not listed.
                sites.retain(|site| include.as_ref().is_none_or(|re| re.is_match(&site.path)));
                let (callers, excluded) = format_callers(sites, exclude.as_ref());
                outline.called_by = callers;
                outline.excluded_tests = excluded;
                outline.name_only_callers = counted;
            }
            // Default method outline states only the key questions (calls, callers).
            outline.none = empty_sections(
                &outline,
                &stated_sections(
                    sections,
                    &["calls", "called_by"],
                    &[
                        "owner",
                        "overrides",
                        "overridden_by",
                        "members",
                        "calls",
                        "called_by",
                    ],
                ),
                &want,
            );
            let (nodes, edges) = finalize_usage(used_nodes, used_edges);
            return Ok((outline, nodes, edges));
        }
        if want.parents {
            outline.parents = self.ancestors(id)?;
            used_nodes.extend(outline.parents.iter().cloned());
        }
        if want.interfaces {
            outline.interfaces = outgoing("implements");
            used_nodes.extend(outline.interfaces.iter().cloned());
        }
        if want.traits {
            outline.traits = outgoing("uses_trait");
            used_nodes.extend(outline.traits.iter().cloned());
        }
        if want.children {
            outline.children = incoming("inherits");
            used_nodes.extend(outline.children.iter().cloned());
        }
        if want.implementations {
            outline.implementations = incoming("implements");
            used_nodes.extend(outline.implementations.iter().cloned());
        }
        if want.members {
            let mut members = Vec::new();
            for edge in edges
                .iter()
                .filter(|e| e.kind == "contains" && e.from == id)
            {
                if let Some(member) = edge.to.as_deref().map(|to| self.repository.node(to)) {
                    if let Some(member) = member?.filter(|m| {
                        matches!(
                            m.kind.as_str(),
                            "method" | "function" | "property" | "constant"
                        )
                    }) {
                        used_nodes.push(member.id.clone());
                        members.push(member);
                    }
                }
            }
            members.sort_by(|a, b| {
                (a.span.start_line, &a.kind, &a.name).cmp(&(b.span.start_line, &b.kind, &b.name))
            });
            outline.members = members.iter().map(member_line).collect();
        }
        outline.none = empty_sections(
            &outline,
            &stated_sections(
                sections,
                &["members"],
                &[
                    "parents",
                    "interfaces",
                    "traits",
                    "children",
                    "implementations",
                    "members",
                ],
            ),
            &want,
        );
        let (nodes, edges) = finalize_usage(used_nodes, used_edges);
        Ok((outline, nodes, edges))
    }

    fn outgoing_calls_usage(
        &mut self,
        edges: &[Edge],
        method: &str,
    ) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
        let mut lines = Vec::new();
        let mut call_edges = Vec::new();
        let mut call_nodes = Vec::new();
        let mut owners = vec![method.to_owned()];
        // Nested closures are separate function nodes; include their call_sites
        // so outline(method) shows deposit/etc. inside `$credit = function () { … }`.
        for edge in edges
            .iter()
            .filter(|e| e.kind == "contains" && e.from == method)
        {
            if let Some(child_id) = edge.to.as_deref() {
                if let Some(child) = self.repository.node(child_id)? {
                    if child.kind == "function" && child.metadata["closure"] == true {
                        owners.push(child.id.clone());
                        call_nodes.push(child.id.clone());
                    }
                }
            }
        }
        for owner in &owners {
            let owner_edges = if owner == method {
                edges.to_vec()
            } else {
                self.repository.adjacent_edges(owner)?
            };
            for site_id in owner_edges
                .iter()
                .filter(|e| e.kind == "contains" && e.from == *owner)
                .filter_map(|e| e.to.as_deref())
            {
                let Some(site) = self
                    .repository
                    .node(site_id)?
                    .filter(|n| n.kind == "call_site")
                else {
                    continue;
                };
                call_nodes.push(site.id.clone());
                for call in self.repository.adjacent_edges(site_id)? {
                    if call.from == site_id && matches!(call.kind.as_str(), "calls" | "dispatches")
                    {
                        call_edges.push(call.id.clone());
                        if let Some(to) = &call.to {
                            call_nodes.push(to.clone());
                        }
                        lines.push((site.span.start_line, call_line(&site, &call)));
                    }
                }
            }
        }
        lines.sort();
        Ok((
            lines.into_iter().map(|(_, line)| line).collect(),
            call_edges,
            call_nodes,
        ))
    }

    /// Function or method containing `site`; a method/function itself for direct
    /// method→method edges (e.g. rules() validators).
    fn site_owner(&mut self, site: &Node) -> Result<Option<String>> {
        Ok(self
            .repository
            .adjacent_edges(&site.id)?
            .into_iter()
            .find(|e| e.kind == "contains" && e.to.as_deref() == Some(site.id.as_str()))
            .map(|e| e.from)
            .or_else(|| {
                (site.kind == "method" || site.kind == "function").then(|| site.id.clone())
            }))
    }

    /// Unresolved call sites with an unknown receiver that name `method`: listed (flag
    /// `unresolved: name only`) when few methods share the name, otherwise only counted.
    fn name_only_sites(&mut self, method: &Node) -> Result<(Vec<CallerSite>, usize)> {
        if !matches!(method.kind.as_str(), "method" | "function") || method.name.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let search = |kinds: &[&str], limit: usize| NodeSearch {
            text: method.name.clone(),
            kinds: kinds.iter().map(|k| (*k).to_owned()).collect(),
            limit,
            mode: SearchMode::Symbol,
            ..NodeSearch::default()
        };
        let untyped: Vec<Node> = self
            .repository
            .search_nodes(&search(&["call_site"], NAME_ONLY_SITE_LIMIT))?
            .into_iter()
            .filter(|site| {
                site.metadata["resolution"] == "unresolved"
                    && site.metadata["undeclared"] != true
                    && receiver_unknown(&site.metadata)
                    && site.metadata["member_name"]
                        .as_str()
                        .is_some_and(|m| m.eq_ignore_ascii_case(&method.name))
            })
            .collect();
        let declarations = self.repository.search_nodes(&search(
            &["method", "function"],
            NAME_ONLY_MAX_DECLARATIONS + 1,
        ))?;
        if declarations.len() > NAME_ONLY_MAX_DECLARATIONS {
            return Ok((Vec::new(), untyped.len()));
        }
        let mut sites = Vec::new();
        for site in untyped {
            let owner_id = self.site_owner(&site)?;
            sites.push(CallerSite {
                area: site.metadata["area"].as_str().map(str::to_owned),
                owner_label: owner_id
                    .as_deref()
                    .map(symbol_name)
                    .unwrap_or_else(|| site.span.path.clone()),
                owner_id,
                path: site.span.path.clone(),
                line: site.span.start_line,
                expression: site.metadata["expression"]
                    .as_str()
                    .unwrap_or(site.name.as_str())
                    .to_owned(),
                flags: vec![NAME_ONLY_FLAG.into()],
                conditions: string_list(&site.metadata["conditions"]),
            });
        }
        Ok((sites, 0))
    }

    /// Usage sites of `target` through incoming `kinds` edges, with their owning
    /// function or method, sorted by owner then line (shared by outline and tree).
    fn caller_sites(
        &mut self,
        edges: &[Edge],
        target: &str,
        kinds: &[&str],
    ) -> Result<Vec<CallerSite>> {
        let mut sites = Vec::new();
        for call in edges
            .iter()
            .filter(|e| kinds.contains(&e.kind.as_str()) && e.to.as_deref() == Some(target))
        {
            let Some(site) = self.repository.node(&call.from)? else {
                continue;
            };
            let owner_id = self.site_owner(&site)?;
            let owner_label = owner_id
                .as_deref()
                .map(symbol_name)
                .unwrap_or_else(|| site.span.path.clone());
            // Occurrence sites carry the real line; edge.span may be a placeholder (1).
            let line = if is_occurrence(&site) {
                site.span.start_line
            } else if call.span.start_line > 0 {
                call.span.start_line
            } else {
                site.span.start_line
            };
            let expression = [&call.metadata, &site.metadata]
                .into_iter()
                .find_map(|metadata| metadata["expression"].as_str().filter(|s| !s.is_empty()))
                .unwrap_or(site.name.as_str())
                .to_owned();
            let conditions = site.metadata["conditions"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|c| c.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            sites.push(CallerSite {
                area: site.metadata["area"].as_str().map(str::to_owned),
                owner_id,
                owner_label,
                path: site.span.path.clone(),
                line,
                expression,
                flags: call_flags(call),
                conditions,
            });
        }
        sites.sort_by(|a, b| (&a.owner_label, a.line).cmp(&(&b.owner_label, b.line)));
        Ok(sites)
    }

    fn ancestors(&mut self, id: &str) -> Result<Vec<String>> {
        let mut chain = Vec::new();
        let mut current = id.to_owned();
        while chain.len() < 64 {
            let Some(edge) = self
                .repository
                .adjacent_edges(&current)?
                .into_iter()
                .find(|e| e.kind == "inherits" && e.from == current)
            else {
                break;
            };
            match edge.to.clone() {
                Some(parent) if !chain.contains(&parent) => {
                    chain.push(parent.clone());
                    current = parent;
                }
                Some(_) => break,
                None => {
                    chain.push(unindexed(&edge));
                    break;
                }
            }
        }
        Ok(chain)
    }

    /// Irreversibly removes the whole atlas (graph, diagnostics, corrections and their
    /// review decisions, usage analytics, index time) in one transaction.
    pub fn reset(&mut self) -> Result<()> {
        transactional(self.repository.as_mut(), |r| r.reset())
    }

    /// Last successful publication (persisted) and the live watcher state.
    pub fn freshness(&mut self) -> Result<Freshness> {
        let snapshot = self.progress.snapshot();
        Ok(Freshness {
            indexed_at: self
                .repository
                .meta(INDEXED_AT)?
                .and_then(|value| value.parse().ok()),
            state: snapshot.state.into(),
            changes_pending: snapshot.changes_pending,
            pending_paths: snapshot.pending_paths,
        })
    }

    pub fn correct(
        &mut self,
        target_kind: &str,
        target_id: &str,
        expected_revision: u64,
        patch: Value,
        reason: &str,
    ) -> Result<Correction> {
        let fields = patch
            .as_object()
            .ok_or_else(|| AtlasError::invalid("correction must be an object"))?;
        if fields.is_empty()
            || fields
                .keys()
                .any(|k| k != "metadata" && !(target_kind == "edge" && k == "to"))
        {
            return Err(AtlasError::invalid(
                "only metadata and an edge's to field may be corrected",
            ));
        }
        if let Some(metadata) = fields.get("metadata") {
            if !metadata.is_object() || metadata.get("fingerprint").is_some() {
                return Err(AtlasError::invalid(
                    "metadata must be an object; fingerprint is analyzer-owned",
                ));
            }
        }
        transactional(self.repository.as_mut(), |r| {
            let mut target = Target::load(r, target_kind, target_id)?;
            target.check_revision(expected_revision)?;
            if target.trust() == 3 {
                return Err(AtlasError::new(
                    ErrorKind::Forbidden,
                    "developer-approved data cannot be corrected by an agent",
                ));
            }
            let before = target.semantic();
            let before_trust = target.trust();
            if let Some(to) = fields.get("to") {
                if !to.is_null() && !to.is_string() {
                    return Err(AtlasError::invalid("to must be a node ID or null"));
                }
                if let Some(id) = to.as_str() {
                    if r.node(id)?.is_none() {
                        return Err(AtlasError::missing("edge target node not found"));
                    }
                }
            }
            let mut after = before.clone();
            if let Some(metadata) = fields.get("metadata") {
                merge_patch(&mut after["metadata"], metadata);
            }
            if let Some(to) = fields.get("to") {
                after["to"] = to.clone();
            }
            let mut previous_id = None;
            for mut c in r.corrections()? {
                if c.correction.target_kind == target_kind
                    && c.correction.target_id == target_id
                    && c.correction.status == "pending"
                {
                    previous_id = Some(c.correction.id.clone());
                    c.correction.status = "superseded".into();
                    r.put_correction(&c)?;
                }
            }
            target.replace_semantic(&after, 2)?;
            target.save(r)?;
            let correction = Correction {
                id: unique_id(),
                target_kind: target_kind.into(),
                target_id: target_id.into(),
                before,
                after,
                before_trust,
                revision: target.revision(),
                status: "pending".into(),
                reason: reason.into(),
                created_at: now(),
            };
            r.put_correction(&StoredCorrection {
                correction: correction.clone(),
                previous_id,
            })?;
            Ok(correction)
        })
    }

    pub fn review(
        &mut self,
        correction_id: &str,
        expected_revision: u64,
        approve: bool,
    ) -> Result<Correction> {
        transactional(self.repository.as_mut(), |r| {
            let mut stored = r
                .corrections()?
                .into_iter()
                .find(|c| c.correction.id == correction_id)
                .ok_or_else(|| AtlasError::missing("correction not found"))?;
            if stored.correction.status != "pending" {
                return Err(AtlasError::conflict(
                    "only the current pending correction can be reviewed",
                ));
            }
            let mut target = Target::load(
                r,
                &stored.correction.target_kind,
                &stored.correction.target_id,
            )?;
            target.check_revision(expected_revision)?;
            if target.trust() != 2 {
                return Err(AtlasError::conflict("target is no longer awaiting review"));
            }
            if approve {
                target.replace_semantic(&target.semantic(), 3)?;
                stored.correction.status = "approved".into();
            } else {
                if let Some(id) = stored.correction.before.get("to").and_then(Value::as_str) {
                    if r.node(id)?.is_none() {
                        return Err(AtlasError::conflict(
                            "previous edge target was deleted; cannot restore dangling edge",
                        ));
                    }
                }
                target
                    .replace_semantic(&stored.correction.before, stored.correction.before_trust)?;
                stored.correction.status = "declined".into();
                if let Some(previous_id) = &stored.previous_id {
                    if let Some(mut previous) = r
                        .corrections()?
                        .into_iter()
                        .find(|c| &c.correction.id == previous_id)
                    {
                        previous.correction.status = "pending".into();
                        previous.correction.revision = target.revision();
                        r.put_correction(&previous)?;
                    }
                }
            }
            target.save(r)?;
            stored.correction.revision = target.revision();
            r.put_correction(&stored)?;
            Ok(stored.correction)
        })
    }

    pub fn pending_reviews(&mut self, limit: usize) -> Result<Vec<Correction>> {
        if limit == 0 || limit > 10_000 {
            return Err(AtlasError::invalid("review limit must be 1..10000"));
        }
        let mut result = vec![];
        for stored in self.repository.corrections()? {
            let mut c = stored.correction;
            if c.status == "pending" {
                if let Ok(target) =
                    Target::load(self.repository.as_mut(), &c.target_kind, &c.target_id)
                {
                    if target.trust() == 2 {
                        c.revision = target.revision();
                        result.push(c);
                    }
                }
            }
        }
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        result.truncate(limit);
        Ok(result)
    }

    pub fn record_delivery(
        &mut self,
        response_id: &str,
        node_ids: &[String],
        edge_ids: &[String],
        timestamp: i64,
    ) -> Result<()> {
        if response_id.is_empty() || response_id.len() > 256 {
            return Err(AtlasError::invalid("invalid response ID"));
        }
        transactional(self.repository.as_mut(), |r| {
            if r.delivery(response_id)?.is_some() {
                return Ok(());
            }
            let mut elements = vec![];
            for id in node_ids.iter().collect::<BTreeSet<_>>() {
                let n = r
                    .node(id)?
                    .ok_or_else(|| AtlasError::missing("delivered node not found"))?;
                elements.push(UsageElement {
                    target_kind: "node".into(),
                    id: n.id,
                    kind: n.kind,
                    label: n.name,
                    qualified_name: n.qualified_name,
                    path: n.span.path,
                    from: None,
                    to: None,
                    count: 1,
                });
            }
            for id in edge_ids.iter().collect::<BTreeSet<_>>() {
                let e = r
                    .edge(id)?
                    .ok_or_else(|| AtlasError::missing("delivered edge not found"))?;
                elements.push(UsageElement {
                    target_kind: "edge".into(),
                    id: e.id,
                    kind: e.kind.clone(),
                    label: e.kind,
                    qualified_name: String::new(),
                    path: e.span.path,
                    from: Some(e.from),
                    to: e.to,
                    count: 1,
                });
            }
            r.put_delivery(&Delivery {
                id: response_id.into(),
                timestamp,
                elements,
            })
        })
    }

    pub fn analytics(&mut self, from: i64, to: i64, bucket_seconds: i64) -> Result<Analytics> {
        if from >= to || bucket_seconds <= 0 || to.checked_sub(from).is_none() {
            return Err(AtlasError::invalid(
                "analytics requires from < to and positive bucket_seconds",
            ));
        }
        let deliveries = self.repository.deliveries(from, to)?;
        let mut timeline: BTreeMap<i64, u64> = BTreeMap::new();
        let mut elements: BTreeMap<(String, String), UsageElement> = BTreeMap::new();
        for delivery in &deliveries {
            let bucket = delivery
                .timestamp
                .div_euclid(bucket_seconds)
                .checked_mul(bucket_seconds)
                .ok_or_else(|| AtlasError::invalid("bucket overflow"))?;
            *timeline.entry(bucket).or_default() += 1;
            for item in &delivery.elements {
                elements
                    .entry((item.target_kind.clone(), item.id.clone()))
                    .and_modify(|e| e.count += 1)
                    .or_insert_with(|| item.clone());
            }
        }
        let unique_nodes = elements
            .values()
            .filter(|e| e.target_kind == "node")
            .count();
        let unique_edges = elements
            .values()
            .filter(|e| e.target_kind == "edge")
            .count();
        let mut elements: Vec<_> = elements.into_values().collect();
        elements.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
        Ok(Analytics {
            from,
            to,
            bucket_seconds,
            total_responses: deliveries.len() as u64,
            unique_nodes,
            unique_edges,
            timeline: timeline
                .into_iter()
                .map(|(start, responses)| UsageBucket { start, responses })
                .collect(),
            elements,
        })
    }
}

const INDEXED_AT: &str = "indexed_at";

/// `L<line> <expression> → <target> [resolution]`; `⇒` marks a queue dispatch.
fn call_line(site: &Node, call: &Edge) -> String {
    let arrow = if call.kind == "dispatches" {
        "⇒"
    } else {
        "→"
    };
    let target = call.to.as_deref().map_or_else(|| "?".into(), symbol_name);
    format!(
        "L{} {} {arrow} {target} [{}]",
        site.span.start_line,
        short_expression(&site.name),
        call_flags(call).join(", ")
    )
}

/// Name-only callers are listed when at most this many methods share the name.
const NAME_ONLY_MAX_DECLARATIONS: usize = 3;
/// Untyped call sites read per name-only lookup.
const NAME_ONLY_SITE_LIMIT: usize = 500;
const NAME_ONLY_FLAG: &str = "unresolved: name only";

/// `called_by` lines (one per caller site/expression) and the number of sites under
/// `exclude` that were left out.
fn format_callers(sites: Vec<CallerSite>, exclude: Option<&regex::Regex>) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut seen = BTreeSet::new();
    let mut excluded = 0;
    for site in sites {
        if exclude.is_some_and(|re| re.is_match(&site.path)) {
            excluded += 1;
            continue;
        }
        // calls + magic_get edges of one site must not double the row.
        if !seen.insert(format!(
            "{}\0{}\0{}",
            site.owner_label, site.line, site.expression
        )) {
            continue;
        }
        lines.push(format!(
            "{} L{}: {} [{}]",
            site.owner_label,
            site.line,
            short_expression(&site.expression),
            site.flags.join(", ")
        ));
    }
    lines.sort();
    (lines, excluded)
}

/// Receiver type unknown (no class part), so the call may target any same-named method.
fn receiver_unknown(metadata: &Value) -> bool {
    metadata["receiver_class"].as_str().is_none_or(|receiver| {
        receiver.split('|').map(str::trim).all(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                "" | "unknown" | "mixed" | "null"
            )
        })
    })
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Edges through which a method is called (callers tree levels, outline called_by).
const CALL_KINDS: [&str; 3] = ["calls", "dispatches", "magic_get"];
/// Edges through which a class member is used (class-root usages).
const USAGE_KINDS: [&str; 5] = ["calls", "dispatches", "magic_get", "reads", "writes"];

/// One usage site and the function or method that contains it.
struct CallerSite {
    /// Area prefix of the site (`metadata.area`).
    area: Option<String>,
    owner_id: Option<String>,
    owner_label: String,
    path: String,
    line: usize,
    expression: String,
    flags: Vec<String>,
    conditions: Vec<String>,
}

/// Budget and test filter shared by one callers-tree walk.
struct CallerWalk {
    remaining: usize,
    truncated: bool,
    excluded_tests: usize,
    exclude: Option<regex::Regex>,
    include: Option<regex::Regex>,
}

impl CallerWalk {
    fn new(request: &CallersRequest) -> Result<Self> {
        let exclude = request
            .exclude_paths
            .as_deref()
            .map(regex::Regex::new)
            .transpose()
            .map_err(|e| AtlasError::invalid(format!("invalid path exclusion: {e}")))?;
        let include = request
            .include_paths
            .as_deref()
            .map(regex::Regex::new)
            .transpose()
            .map_err(|e| AtlasError::invalid(format!("invalid area filter: {e}")))?;
        Ok(Self {
            remaining: request.max_nodes,
            truncated: false,
            excluded_tests: 0,
            exclude,
            include,
        })
    }

    fn is_excluded(&self, path: &str) -> bool {
        self.exclude.as_ref().is_some_and(|re| re.is_match(path))
    }

    /// Outside the requested areas (`area` filter): skipped without counting.
    fn is_outside_area(&self, path: &str) -> bool {
        self.include.as_ref().is_some_and(|re| !re.is_match(path))
    }

    /// Takes one node from the budget; marks the tree truncated when it is spent.
    fn admit(&mut self) -> bool {
        if self.remaining == 0 {
            self.truncated = true;
            return false;
        }
        self.remaining -= 1;
        true
    }
}

fn is_class_like(kind: &str) -> bool {
    matches!(kind, "class" | "interface" | "trait" | "enum")
}

fn is_callable_id(id: &str) -> bool {
    id.starts_with("method:") || id.starts_with("function:")
}

fn call_flags(call: &Edge) -> Vec<String> {
    let mut flags = vec![call.metadata["resolution"]
        .as_str()
        .unwrap_or("unresolved")
        .to_owned()];
    if let Some(queue) = call.metadata["queue"].as_str() {
        flags.push(format!("queue {queue}"));
    }
    flags
}

/// `method:A\B::run` → `A\B::run`.
fn symbol_name(id: &str) -> String {
    id.split_once(':')
        .filter(|(kind, _)| {
            matches!(
                *kind,
                "method"
                    | "function"
                    | "class"
                    | "interface"
                    | "trait"
                    | "enum"
                    | "property"
                    | "constant"
            )
        })
        .map_or(id, |(_, name)| name)
        .to_owned()
}

fn short_expression(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 80 {
        format!("{}…", flat.chars().take(79).collect::<String>())
    } else {
        flat
    }
}

/// A hierarchy edge whose target is not indexed names the symbol instead of an id.
fn unindexed(edge: &Edge) -> String {
    format!(
        "unindexed:{}",
        edge.metadata["symbol"].as_str().unwrap_or("unknown")
    )
}

/// Sections whose emptiness is stated: the defaults, or the requested ones (request
/// order) that apply to this kind of entity.
fn stated_sections<'a>(
    requested: &'a [String],
    defaults: &[&'a str],
    applicable: &[&str],
) -> Vec<&'a str> {
    if requested.is_empty() {
        return defaults.to_vec();
    }
    requested
        .iter()
        .map(String::as_str)
        .filter(|name| applicable.contains(name))
        .collect()
}

/// Names from `stated` that were wanted and came out empty, in `stated` order.
fn empty_sections(outline: &Outline, stated: &[&str], want: &OutlineSections) -> Vec<String> {
    stated
        .iter()
        .filter(|name| want.wants(name) && section_is_empty(outline, name))
        .map(|name| (*name).to_owned())
        .collect()
}

fn section_is_empty(outline: &Outline, name: &str) -> bool {
    match name {
        "owner" => outline.owner.is_none(),
        "parents" => outline.parents.is_empty(),
        "interfaces" => outline.interfaces.is_empty(),
        "traits" => outline.traits.is_empty(),
        "children" => outline.children.is_empty(),
        "implementations" => outline.implementations.is_empty(),
        "overrides" => outline.overrides.is_empty(),
        "overridden_by" => outline.overridden_by.is_empty(),
        "calls" => outline.calls.is_empty(),
        "called_by" => outline.called_by.is_empty(),
        "members" => outline.members.is_empty(),
        _ => false,
    }
}

struct OutlineSections {
    owner: bool,
    parents: bool,
    interfaces: bool,
    traits: bool,
    children: bool,
    implementations: bool,
    overrides: bool,
    overridden_by: bool,
    calls: bool,
    called_by: bool,
    members: bool,
}

impl OutlineSections {
    fn wants(&self, name: &str) -> bool {
        match name {
            "owner" => self.owner,
            "parents" => self.parents,
            "interfaces" => self.interfaces,
            "traits" => self.traits,
            "children" => self.children,
            "implementations" => self.implementations,
            "overrides" => self.overrides,
            "overridden_by" => self.overridden_by,
            "calls" => self.calls,
            "called_by" => self.called_by,
            "members" => self.members,
            _ => false,
        }
    }

    fn parse(sections: &[String]) -> Result<Self> {
        const KNOWN: &[&str] = &[
            "owner",
            "parents",
            "interfaces",
            "traits",
            "children",
            "implementations",
            "overrides",
            "overridden_by",
            "calls",
            "called_by",
            "members",
        ];
        if sections.is_empty() {
            // Method default: callers/callees/owner/overrides — not the redundant members line.
            // Class default: full hierarchy + members (set by caller via kind later).
            return Ok(Self {
                owner: true,
                parents: true,
                interfaces: true,
                traits: true,
                children: true,
                implementations: true,
                overrides: true,
                overridden_by: true,
                calls: true,
                called_by: true,
                members: true,
            });
        }
        let mut unknown = Vec::new();
        let mut set = BTreeSet::new();
        for section in sections {
            if KNOWN.contains(&section.as_str()) {
                set.insert(section.as_str());
            } else {
                unknown.push(section.clone());
            }
        }
        if !unknown.is_empty() {
            return Err(AtlasError::invalid(format!(
                "unknown outline sections: {}",
                unknown.join(", ")
            )));
        }
        Ok(Self {
            owner: set.contains("owner"),
            parents: set.contains("parents"),
            interfaces: set.contains("interfaces"),
            traits: set.contains("traits"),
            children: set.contains("children"),
            implementations: set.contains("implementations"),
            overrides: set.contains("overrides"),
            overridden_by: set.contains("overridden_by"),
            calls: set.contains("calls"),
            called_by: set.contains("called_by"),
            members: set.contains("members"),
        })
    }
}

fn finalize_usage(mut nodes: Vec<String>, mut edges: Vec<String>) -> (Vec<String>, Vec<String>) {
    nodes.retain(|id| !id.starts_with("unindexed:"));
    nodes.sort();
    nodes.dedup();
    edges.sort();
    edges.dedup();
    (nodes, edges)
}

/// `L<line> <kind> <name>(<type $param>, …): <type> [flags]` — one member, no ids:
/// member ids follow `method:<Class>::<name>` and `property:<Class>::$<name>`.
fn member_line(node: &Node) -> String {
    let meta = &node.metadata;
    let known = |t: &&str| !t.is_empty() && *t != "unknown";
    let mut flags: Vec<&str> = Vec::new();
    if meta["magic"] == true {
        flags.push("magic");
    }
    match meta["type_source"].as_str() {
        Some("annotation") => flags.push("annotation"),
        Some("framework") => flags.push("framework"),
        _ => {}
    }
    let mut line = format!("L{} {} {}", node.span.start_line, node.kind, node.name);
    let type_name = if node.kind == "property" {
        meta["type"].as_str().filter(known).map(str::to_owned)
    } else {
        let parameters: Vec<String> = meta["parameters"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|p| {
                        let name = p["name"].as_str().unwrap_or("");
                        match p["type"].as_str().filter(known) {
                            Some(t) => format!("{t} ${name}"),
                            None => format!("${name}"),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        line.push_str(&format!("({})", parameters.join(", ")));
        match meta["declared_return_type"].as_str().filter(known) {
            Some(declared) => Some(declared.to_owned()),
            None => {
                let annotated = meta["annotated_return_type"].as_str().filter(known);
                if annotated.is_some() {
                    flags.push("annotation");
                }
                annotated.map(str::to_owned)
            }
        }
    };
    if let Some(type_name) = type_name {
        line.push_str(&format!(": {type_name}"));
    }
    flags.dedup();
    if !flags.is_empty() {
        line.push_str(&format!(" [{}]", flags.join(", ")));
    }
    line
}

/// Occurrence and branch nodes are numerous and light. They get their own pool of the
/// same size so that a method's read sites cannot displace its callers.
fn is_occurrence(node: &Node) -> bool {
    matches!(
        node.kind.as_str(),
        "branch" | "call_site" | "read_site" | "write_site" | "return_site" | "url_site"
    )
}

struct NodeBudget {
    declarations: usize,
    occurrences: usize,
}
impl NodeBudget {
    fn new(limit: usize) -> Self {
        Self {
            declarations: limit,
            occurrences: limit,
        }
    }
    fn admit(&mut self, node: &Node) -> bool {
        let pool = if is_occurrence(node) {
            &mut self.occurrences
        } else {
            &mut self.declarations
        };
        if *pool == 0 {
            return false;
        }
        *pool -= 1;
        true
    }
}

/// Class hierarchy edge; a shared parent or trait fans out to every user class.
fn is_hierarchy(edge: &Edge) -> bool {
    matches!(edge.kind.as_str(), "inherits" | "implements" | "uses_trait")
}

/// `contains` / `available_via` edge leaving `current` (a member listing).
fn is_outgoing_membership(edge: &Edge, current: &str) -> bool {
    edge.from == current && matches!(edge.kind.as_str(), "contains" | "available_via")
}

impl Direction {
    fn follows(self, edge: &Edge, current: &str) -> bool {
        match self {
            Direction::Both => true,
            Direction::Out => edge.from == current,
            Direction::In => edge.to.as_deref() == Some(current),
        }
    }
}

/// Bulky analyzer metadata omitted from summary projections. Everything else,
/// including agent and developer corrections, stays visible.
const HEAVY_METADATA: [&str; 7] = [
    "fingerprint",
    "returns",
    "imports",
    "assignments",
    "usages",
    "body",
    "source",
];

fn summarize(metadata: &mut Value, detail: Detail) {
    if detail == Detail::Summary {
        if let Some(fields) = metadata.as_object_mut() {
            for key in HEAVY_METADATA {
                fields.remove(key);
            }
            // Drop empty noise that only inflates MCP token counts.
            fields.retain(|_, value| match value {
                Value::Null => false,
                Value::Array(items) if items.is_empty() => false,
                Value::Object(map) if map.is_empty() => false,
                Value::String(text) if text.is_empty() => false,
                _ => true,
            });
        }
    }
}
fn project_node(mut node: Node, detail: Detail) -> Node {
    summarize(&mut node.metadata, detail);
    if detail == Detail::Summary {
        // Drop redundant QN copies that only inflate MCP/search token counts.
        // Keep revision: agents need it for atlas_correct after reading a summary.
        let unprefixed_id = node
            .id
            .strip_prefix(node.kind.as_str())
            .and_then(|rest| rest.strip_prefix(':'));
        if node.qualified_name == node.id
            || node.qualified_name == node.name
            || node.qualified_name.is_empty()
            || unprefixed_id == Some(node.qualified_name.as_str())
        {
            node.qualified_name.clear();
        }
    }
    node
}
fn project_edge(mut edge: Edge, detail: Detail) -> Edge {
    summarize(&mut edge.metadata, detail);
    edge
}

const MAX_SEARCH_KINDS: usize = 20;

fn validate_kinds(kinds: &[String]) -> Result<()> {
    if kinds.len() > MAX_SEARCH_KINDS {
        return Err(AtlasError::invalid("at most 20 kinds"));
    }
    let valid = |kind: &String| {
        !kind.is_empty()
            && kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    match kinds.iter().find(|k| !valid(k)) {
        Some(kind) => Err(AtlasError::invalid(format!(
            "kind {kind:?} must match [a-z0-9_]+"
        ))),
        None => Ok(()),
    }
}

fn transactional<T>(
    r: &mut dyn Repository,
    action: impl FnOnce(&mut dyn Repository) -> Result<T>,
) -> Result<T> {
    r.begin()?;
    match action(r) {
        Ok(value) => match r.commit() {
            Ok(()) => Ok(value),
            Err(error) => {
                let _ = r.rollback();
                Err(error)
            }
        },
        Err(error) => {
            let _ = r.rollback();
            Err(error)
        }
    }
}

enum Target {
    Node(Node),
    Edge(Edge),
}
impl Target {
    fn load(r: &mut dyn Repository, kind: &str, id: &str) -> Result<Self> {
        match kind {
            "node" => r
                .node(id)?
                .map(Self::Node)
                .ok_or_else(|| AtlasError::missing("node not found")),
            "edge" => r
                .edge(id)?
                .map(Self::Edge)
                .ok_or_else(|| AtlasError::missing("edge not found")),
            _ => Err(AtlasError::invalid("target_kind must be node or edge")),
        }
    }
    fn revision(&self) -> u64 {
        match self {
            Self::Node(n) => n.revision,
            Self::Edge(e) => e.revision,
        }
    }
    fn trust(&self) -> u8 {
        match self {
            Self::Node(n) => n.trust,
            Self::Edge(e) => e.trust,
        }
    }
    fn check_revision(&self, expected: u64) -> Result<()> {
        if self.revision() == expected {
            Ok(())
        } else {
            Err(AtlasError::conflict("object revision has changed"))
        }
    }
    fn semantic(&self) -> Value {
        match self {
            Self::Node(n) => json!({"metadata":n.metadata}),
            Self::Edge(e) => json!({"metadata":e.metadata,"to":e.to}),
        }
    }
    fn replace_semantic(&mut self, value: &Value, trust: u8) -> Result<()> {
        let revision = self
            .revision()
            .checked_add(1)
            .ok_or_else(|| AtlasError::conflict("revision overflow"))?;
        match self {
            Self::Node(n) => {
                let fingerprint = n.metadata.get("fingerprint").cloned();
                n.metadata = value["metadata"].clone();
                if let Some(fp) = fingerprint {
                    n.metadata["fingerprint"] = fp;
                }
                n.trust = trust;
                n.revision = revision;
            }
            Self::Edge(e) => {
                let fingerprint = e.metadata.get("fingerprint").cloned();
                e.metadata = value["metadata"].clone();
                if let Some(fp) = fingerprint {
                    e.metadata["fingerprint"] = fp;
                }
                e.to = value.get("to").and_then(Value::as_str).map(str::to_owned);
                e.trust = trust;
                e.revision = revision;
            }
        }
        Ok(())
    }
    fn save(&self, r: &mut dyn Repository) -> Result<()> {
        match self {
            Self::Node(n) => r.put_node(n),
            Self::Edge(e) => r.put_edge(e),
        }
    }
}

fn merge_patch(target: &mut Value, patch: &Value) {
    if let Some(fields) = patch.as_object() {
        if !target.is_object() {
            *target = json!({});
        }
        for (key, value) in fields {
            if value.is_null() {
                target.as_object_mut().unwrap().remove(key);
            } else {
                merge_patch(&mut target[key], value);
            }
        }
    } else {
        *target = patch.clone();
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn unique_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!(
        "c-{:x}-{:x}-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn fingerprint(metadata: &Value) -> Option<&str> {
    metadata
        .get("fingerprint")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// A level-1 object belongs to the analyzer, so "unchanged" means every analyzer fact
/// is equal, not only the source fingerprint: a newer analyzer may infer more about
/// unchanged code (resolution, types, provenance).
fn node_unchanged(old: &Node, next: &Node) -> bool {
    old.kind == next.kind
        && old.name == next.name
        && old.qualified_name == next.qualified_name
        && old.span == next.span
        && old.metadata == next.metadata
}

fn edge_unchanged(old: &Edge, next: &Edge) -> bool {
    old.kind == next.kind
        && old.from == next.from
        && old.to == next.to
        && old.span == next.span
        && old.metadata == next.metadata
}

fn edge_fingerprint_key(edge: &Edge) -> Option<(String, String, String)> {
    fingerprint(&edge.metadata).map(|fp| (edge.kind.clone(), edge.from.clone(), fp.to_owned()))
}

fn collision_free_id(base: &str, reserved: &BTreeSet<String>) -> String {
    let mut suffix = 1u64;
    loop {
        let candidate = format!("{base}~{suffix}");
        if !reserved.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

type SemanticKey = (String, String, Option<String>);

fn semantic_key(node: &Node, remap: &BTreeMap<String, String>) -> Option<SemanticKey> {
    if node.qualified_name.is_empty() {
        return None;
    }
    let scope = node
        .metadata
        .get("scope")
        .and_then(Value::as_str)
        .map(|id| remap.get(id).cloned().unwrap_or_else(|| id.to_owned()));
    Some((node.kind.clone(), node.qualified_name.clone(), scope))
}

// Only fields defined by the analyzer contract contain graph IDs. Source expressions,
// FQNs, snippets and arbitrary annotation text must never be rewritten as strings.
fn remap_metadata(metadata: &mut Value, remap: &BTreeMap<String, String>) {
    fn identifier(value: &mut Value, remap: &BTreeMap<String, String>) {
        if let Some(replacement) = value.as_str().and_then(|id| remap.get(id)) {
            *value = Value::String(replacement.clone());
        }
    }
    fn identifiers(value: &mut Value, remap: &BTreeMap<String, String>) {
        if let Some(items) = value.as_array_mut() {
            for item in items {
                identifier(item, remap);
            }
        }
    }
    if let Some(scope) = metadata.get_mut("scope") {
        identifier(scope, remap);
    }
    if let Some(candidates) = metadata.get_mut("candidates") {
        identifiers(candidates, remap);
    }
    for collection in ["returns", "assignments"] {
        if let Some(items) = metadata.get_mut(collection).and_then(Value::as_array_mut) {
            for item in items {
                if let Some(origins) = item.get_mut("origins").and_then(Value::as_array_mut) {
                    for origin in origins {
                        if let Some(id) = origin.get_mut("symbol_id") {
                            identifier(id, remap);
                        }
                        if let Some(candidates) = origin.get_mut("candidates") {
                            identifiers(candidates, remap);
                        }
                        if let Some(id) = origin.get_mut("definition").and_then(|d| d.get_mut("id"))
                        {
                            identifier(id, remap);
                        }
                    }
                }
            }
        }
    }
}
fn refresh_metadata(previous: &Value, next: &Value, trust: u8) -> Value {
    if trust == 1 {
        return next.clone();
    }
    let mut protected = previous.clone();
    if let Some(fp) = next.get("fingerprint") {
        protected["fingerprint"] = fp.clone();
    }
    protected
}

fn apply_snapshot(
    r: &mut dyn Repository,
    mut snapshot: GraphSnapshot,
    indexed_paths: Vec<String>,
    removed_paths: Vec<String>,
    progress: &IndexingProgress,
) -> Result<IndexReport> {
    let scopes: BTreeSet<String> = indexed_paths.into_iter().chain(removed_paths).collect();
    progress.set_files_total(scopes.len());
    progress.phase("publishing", "load_previous", Some(scopes.len()), "files");
    let old_nodes: BTreeMap<String, Node> = r
        .nodes_for_paths(&scopes)?
        .into_iter()
        .map(|n| (n.id.clone(), n))
        .collect();
    let scoped_node_ids: BTreeSet<_> = old_nodes.keys().cloned().collect();
    let old_edges: BTreeMap<String, Edge> = r
        .edges_for_paths(&scopes, &scoped_node_ids)?
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    progress.advance(scopes.len(), None);
    let mut incoming_ids = BTreeSet::new();
    for node in &snapshot.nodes {
        if !incoming_ids.insert(node.id.clone()) {
            return Err(AtlasError::invalid(format!(
                "duplicate node ID in snapshot: {}",
                node.id
            )));
        }
    }
    let mut incoming_edge_ids = BTreeSet::new();
    for edge in &snapshot.edges {
        if !incoming_edge_ids.insert(edge.id.clone()) {
            return Err(AtlasError::invalid(format!(
                "duplicate edge ID in snapshot: {}",
                edge.id
            )));
        }
    }
    for node in &snapshot.nodes {
        if node.id.is_empty() {
            return Err(AtlasError::invalid("empty node ID in snapshot"));
        }
        if !node.metadata.is_object() {
            return Err(AtlasError::invalid(format!(
                "node {} metadata is not an object",
                node.id
            )));
        }
        if !scopes.contains(&node.span.path) {
            return Err(AtlasError::invalid(format!(
                "node {} path {} outside successful index scope",
                node.id, node.span.path
            )));
        }
    }
    let mut remap = BTreeMap::new();
    let mut retained = BTreeSet::new();
    let mut skipped = BTreeSet::new();
    let match_total = snapshot.nodes.len();
    progress.phase("publishing", "match_nodes", Some(match_total), "items");
    for (position, incoming) in snapshot.nodes.iter().enumerate() {
        let compatible = old_nodes.get(&incoming.id).is_some_and(|old| {
            old.kind == incoming.kind
                && old.qualified_name == incoming.qualified_name
                && old.metadata.get("scope").and_then(Value::as_str)
                    == incoming.metadata.get("scope").and_then(Value::as_str)
        });
        if compatible {
            remap.insert(incoming.id.clone(), incoming.id.clone());
            retained.insert(incoming.id.clone());
        }
        if position % 1024 == 0 || position + 1 == match_total {
            progress.advance(position + 1, Some(&incoming.span.path));
        }
    }
    // Index remaining old nodes once. Matching must stay O(n log n); scanning every
    // old node for each incoming site is not viable after finer-graph emission.
    let empty_remap = BTreeMap::new();
    let mut old_by_semantic: BTreeMap<SemanticKey, Vec<String>> = BTreeMap::new();
    let mut old_by_fingerprint: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for old in old_nodes.values() {
        if retained.contains(&old.id) || !scopes.contains(&old.span.path) {
            continue;
        }
        if let Some(key) = semantic_key(old, &empty_remap) {
            old_by_semantic.entry(key).or_default().push(old.id.clone());
        }
        if let Some(fp) = fingerprint(&old.metadata) {
            old_by_fingerprint
                .entry((old.kind.clone(), fp.to_owned()))
                .or_default()
                .push(old.id.clone());
        }
    }
    // Enabling `areas` re-keys namespace-less code (`class:Foo` → `class:console:Foo`). That
    // is a naming change, not a code rename: the new spelling wins unless the old object
    // holds reviewed or agent knowledge (trust > 1 or an endpoint of such an edge).
    let protected_ids: BTreeSet<String> = old_nodes
        .values()
        .filter(|n| n.trust > 1)
        .map(|n| n.id.clone())
        .chain(
            old_edges
                .values()
                .filter(|e| e.trust > 1)
                .flat_map(|e| std::iter::once(e.from.clone()).chain(e.to.clone())),
        )
        .collect();
    let area_rekey = |old_id: &str, incoming: &Node| {
        !protected_ids.contains(old_id)
            && incoming.metadata["area"].as_str().is_some_and(|area| {
                incoming.id.replacen(&format!("{area}:"), "", 1) == old_id && incoming.id != old_id
            })
    };
    // Persistent IDs deliberately survive renames. On subsequent scans their current
    // semantic identity, rather than the obsolete ID spelling, recognizes body edits.
    // Repeat to resolve owners before scoped variables even for unordered snapshots.
    loop {
        let unmatched: Vec<&Node> = snapshot
            .nodes
            .iter()
            .filter(|n| !remap.contains_key(&n.id))
            .collect();
        let mut incoming_counts: BTreeMap<SemanticKey, usize> = BTreeMap::new();
        for node in &unmatched {
            if let Some(key) = semantic_key(node, &remap) {
                *incoming_counts.entry(key).or_default() += 1;
            }
        }
        let mut matched = false;
        for incoming in unmatched {
            let Some(key) = semantic_key(incoming, &remap) else {
                continue;
            };
            if incoming_counts.get(&key) != Some(&1) {
                continue;
            }
            let Some(candidates) = old_by_semantic.get(&key) else {
                continue;
            };
            let available: Vec<_> = candidates
                .iter()
                .filter(|id| !retained.contains(*id))
                .collect();
            if available.len() != 1 || area_rekey(available[0], incoming) {
                continue;
            }
            let old_id = available[0].clone();
            remap.insert(incoming.id.clone(), old_id.clone());
            retained.insert(old_id);
            matched = true;
            if remap.len() % 1024 == 0 {
                progress.advance(remap.len().min(match_total), Some(&incoming.span.path));
            }
        }
        if !matched {
            break;
        }
    }
    let unmatched: Vec<&Node> = snapshot
        .nodes
        .iter()
        .filter(|n| !remap.contains_key(&n.id))
        .collect();
    let mut incoming_fp_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for node in &unmatched {
        if let Some(fp) = fingerprint(&node.metadata) {
            *incoming_fp_counts
                .entry((node.kind.clone(), fp.to_owned()))
                .or_default() += 1;
        }
    }
    for incoming in unmatched {
        let Some(fp) = fingerprint(&incoming.metadata) else {
            continue;
        };
        let key = (incoming.kind.clone(), fp.to_owned());
        if incoming_fp_counts.get(&key) != Some(&1) {
            continue;
        }
        let Some(candidates) = old_by_fingerprint.get(&key) else {
            continue;
        };
        let available: Vec<_> = candidates
            .iter()
            .filter(|id| !retained.contains(*id))
            .collect();
        if available.len() != 1 || area_rekey(available[0], incoming) {
            continue;
        }
        let old_id = available[0].clone();
        remap.insert(incoming.id.clone(), old_id.clone());
        retained.insert(old_id);
    }
    progress.advance(
        match_total,
        snapshot.nodes.last().map(|n| n.span.path.as_str()),
    );
    // A changed name and changed body cannot prove deletion of protected knowledge.
    // Preserve the prior object and withhold ambiguous replacements until a unique match exists.
    // Endpoint identity is also material to protected edges; retaining it does not
    // promote the endpoint's own trust level.
    let protected_endpoints: BTreeSet<_> = old_edges
        .values()
        .filter(|e| e.trust > 1)
        .flat_map(|e| std::iter::once(e.from.clone()).chain(e.to.clone()))
        .collect();
    let unmatched_protected: Vec<_> = old_nodes
        .values()
        .filter(|n| {
            (n.trust > 1 || protected_endpoints.contains(&n.id))
                && scopes.contains(&n.span.path)
                && !retained.contains(&n.id)
        })
        .collect();
    for old in unmatched_protected {
        let candidates: Vec<_> = snapshot
            .nodes
            .iter()
            .filter(|n| n.kind == old.kind && !remap.contains_key(&n.id))
            .collect();
        if !candidates.is_empty() {
            retained.insert(old.id.clone());
            skipped.extend(candidates.into_iter().map(|n| n.id.clone()));
            snapshot.diagnostics.push(Diagnostic{path:old.span.path.clone(),line:old.span.start_line,code:"identity_ambiguous".into(),message:format!("Protected object {} retained: rename/deletion could not be established uniquely",old.id)});
        }
    }
    // An analyzer key can be reused after its declaration was renamed, while that
    // original spelling remains the persistent ID of the renamed object. Reserve all
    // old IDs and incoming keys; never let a fresh declaration overwrite that object.
    let mut reserved: BTreeSet<_> = old_nodes
        .keys()
        .cloned()
        .chain(incoming_ids.iter().cloned())
        .collect();
    reserved.extend(remap.values().cloned());
    for incoming in &snapshot.nodes {
        if skipped.contains(&incoming.id) || remap.contains_key(&incoming.id) {
            continue;
        }
        let persistent = if old_nodes.contains_key(&incoming.id) {
            collision_free_id(&incoming.id, &reserved)
        } else {
            incoming.id.clone()
        };
        reserved.insert(persistent.clone());
        remap.insert(incoming.id.clone(), persistent);
    }
    let node_total = snapshot.nodes.len();
    progress.phase("publishing", "write_nodes", Some(node_total), "items");
    let mut prepared_nodes = Vec::with_capacity(node_total);
    for (position, mut n) in snapshot.nodes.into_iter().enumerate() {
        if skipped.contains(&n.id) {
            continue;
        }
        remap_metadata(&mut n.metadata, &remap);
        let incoming_id = n.id.clone();
        n.id = remap
            .get(&incoming_id)
            .cloned()
            .unwrap_or(incoming_id.clone());
        if let Some(old) = old_nodes.get(&n.id) {
            // Skip only stable-id no-ops; fingerprint rematches still bump revision.
            if incoming_id == old.id && old.trust == 1 && node_unchanged(old, &n) {
                retained.insert(n.id.clone());
                if position % 1024 == 0 || position + 1 == node_total {
                    progress.advance(position + 1, Some(&n.span.path));
                }
                continue;
            }
            n.metadata = refresh_metadata(&old.metadata, &n.metadata, old.trust);
            n.trust = old.trust;
            n.revision = old.revision + 1;
        } else {
            n.trust = 1;
            n.revision = 1;
        }
        retained.insert(n.id.clone());
        let path = n.span.path.clone();
        prepared_nodes.push(n);
        if prepared_nodes.len() % 1024 == 0 || position + 1 == node_total {
            progress.advance(position + 1, Some(&path));
        }
    }
    r.put_nodes(&prepared_nodes)?;
    // These are exactly the nodes visible in this transaction: the previous graph
    // plus successfully written nodes. Deletions happen after edge reconciliation.
    let mut available_nodes: BTreeSet<_> =
        old_nodes.keys().chain(retained.iter()).cloned().collect();
    let edge_total = snapshot.edges.len();
    let mut incoming_edges = Vec::with_capacity(edge_total);
    progress.phase("publishing", "prepare_edges", Some(edge_total), "items");
    let mut remapped = Vec::with_capacity(edge_total);
    for mut edge in snapshot.edges {
        if !edge.metadata.is_object() {
            return Err(AtlasError::invalid("edge metadata must be an object"));
        }
        remap_metadata(&mut edge.metadata, &remap);
        if skipped.contains(&edge.from) || edge.to.as_ref().is_some_and(|id| skipped.contains(id)) {
            continue;
        }
        edge.from = remap.get(&edge.from).cloned().unwrap_or(edge.from);
        edge.to = edge.to.map(|id| remap.get(&id).cloned().unwrap_or(id));
        remapped.push(edge);
    }
    // A partial (incremental) publication links into unchanged paths: their nodes are
    // stored but not loaded, so look the remaining endpoints up in one query.
    let outside: BTreeSet<String> = remapped
        .iter()
        .flat_map(|edge| std::iter::once(&edge.from).chain(edge.to.iter()))
        .filter(|id| !available_nodes.contains(*id))
        .cloned()
        .collect();
    available_nodes.extend(r.existing_node_ids(&outside)?);
    for (position, edge) in remapped.into_iter().enumerate() {
        if let Some(unknown) = std::iter::once(&edge.from)
            .chain(edge.to.iter())
            .find(|id| !available_nodes.contains(*id))
        {
            return Err(AtlasError::invalid(format!(
                "snapshot edge {} refers to an unknown node {unknown}",
                edge.id
            )));
        }
        let path = edge.span.path.clone();
        incoming_edges.push(edge);
        if incoming_edges.len() % 1024 == 0 || position + 1 == edge_total {
            progress.advance(position + 1, Some(&path));
        }
    }
    let mut kept_edges = BTreeSet::new();
    let mut edge_remap = BTreeMap::new();
    let mut skipped_edges = BTreeSet::new();
    progress.phase(
        "publishing",
        "match_edges",
        Some(incoming_edges.len()),
        "items",
    );
    for edge in &incoming_edges {
        if old_edges.contains_key(&edge.id) {
            edge_remap.insert(edge.id.clone(), edge.id.clone());
            kept_edges.insert(edge.id.clone());
        }
    }
    // A call site has one immutable matching key. Index unmatched candidates once;
    // exact ID matches must be excluded before testing fingerprint uniqueness.
    let mut old_candidates = BTreeMap::new();
    for old in old_edges.values() {
        if kept_edges.contains(&old.id) || !scopes.contains(&old.span.path) {
            continue;
        }
        if let Some(key) = edge_fingerprint_key(old) {
            old_candidates
                .entry(key)
                .and_modify(|candidate| *candidate = None)
                .or_insert(Some(old));
        }
    }
    let mut incoming_counts = BTreeMap::new();
    for edge in &incoming_edges {
        if !edge_remap.contains_key(&edge.id) {
            if let Some(key) = edge_fingerprint_key(edge) {
                *incoming_counts.entry(key).or_insert(0usize) += 1;
            }
        }
    }
    for (position, edge) in incoming_edges.iter().enumerate() {
        if edge_remap.contains_key(&edge.id) {
            if position % 1024 == 0 || position + 1 == incoming_edges.len() {
                progress.advance(position + 1, Some(&edge.span.path));
            }
            continue;
        }
        if let Some(key) = edge_fingerprint_key(edge) {
            if let Some(Some(old)) = old_candidates.get(&key) {
                if incoming_counts.get(&key) == Some(&1) {
                    edge_remap.insert(edge.id.clone(), old.id.clone());
                    kept_edges.insert(old.id.clone());
                }
            }
        }
        if position % 1024 == 0 || position + 1 == incoming_edges.len() {
            progress.advance(position + 1, Some(&edge.span.path));
        }
    }
    drop(old_candidates);
    drop(incoming_counts);
    let unmatched_protected: Vec<_> = old_edges
        .values()
        .filter(|old| {
            old.trust > 1 && scopes.contains(&old.span.path) && !kept_edges.contains(&old.id)
        })
        .collect();
    let mut unmatched_groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for edge in &incoming_edges {
        if !edge_remap.contains_key(&edge.id) {
            unmatched_groups
                .entry((edge.kind.clone(), edge.from.clone()))
                .or_default()
                .push(edge.id.clone());
        }
    }
    let mut skipped_groups = BTreeSet::new();
    progress.phase(
        "publishing",
        "retain_protected_edges",
        Some(unmatched_protected.len()),
        "items",
    );
    for (position, old) in unmatched_protected.into_iter().enumerate() {
        progress.advance(position + 1, Some(&old.span.path));
        let key = (old.kind.clone(), old.from.clone());
        if let Some(candidates) = unmatched_groups.get(&key) {
            kept_edges.insert(old.id.clone());
            if skipped_groups.insert(key) {
                skipped_edges.extend(candidates.iter().cloned());
            }
            snapshot.diagnostics.push(Diagnostic {
                path: old.span.path.clone(),
                line: old.span.start_line,
                code: "identity_ambiguous".into(),
                message: format!(
                    "Protected edge {} retained: changed call site could not be matched uniquely",
                    old.id
                ),
            });
        }
    }
    drop(unmatched_groups);
    let edge_total = incoming_edges.len();
    progress.phase("publishing", "write_edges", Some(edge_total), "items");
    let mut prepared_edges = Vec::with_capacity(edge_total);
    for (position, mut edge) in incoming_edges.into_iter().enumerate() {
        let path = edge.span.path.clone();
        if skipped_edges.contains(&edge.id) {
            if position % 1024 == 0 || position + 1 == edge_total {
                progress.advance(position + 1, Some(&path));
            }
            continue;
        }
        if let Some(old) = edge_remap.get(&edge.id).and_then(|id| old_edges.get(id)) {
            let same_id = edge.id == old.id;
            edge.id = old.id.clone();
            if same_id && old.trust == 1 && edge_unchanged(old, &edge) {
                kept_edges.insert(edge.id.clone());
                if position % 1024 == 0 || position + 1 == edge_total {
                    progress.advance(position + 1, Some(&path));
                }
                continue;
            }
            edge.metadata = refresh_metadata(&old.metadata, &edge.metadata, old.trust);
            if old.trust > 1 {
                edge.to = old.to.clone();
            }
            edge.trust = old.trust;
            edge.revision = old.revision + 1;
        } else {
            edge.trust = 1;
            edge.revision = 1;
        }
        kept_edges.insert(edge.id.clone());
        prepared_edges.push(edge);
        if prepared_edges.len() % 1024 == 0 || position + 1 == edge_total {
            progress.advance(position + 1, Some(&path));
        }
    }
    r.put_edges(&prepared_edges)?;
    let deleted_nodes: Vec<_> = old_nodes
        .values()
        .filter(|n| scopes.contains(&n.span.path) && !retained.contains(&n.id))
        .map(|n| n.id.clone())
        .collect();
    let removed: BTreeSet<_> = deleted_nodes.iter().cloned().collect();
    let ambiguous_paths: BTreeSet<_> = snapshot
        .diagnostics
        .iter()
        .filter(|d| d.code == "identity_ambiguous")
        .map(|d| d.path.clone())
        .collect();
    // Use the pre-write edge set; skip ids rewritten in this transaction so an
    // updated edge is not deleted because its former target left the graph.
    let deleted_edges: Vec<_> = old_edges
        .values()
        .filter(|edge| {
            if kept_edges.contains(&edge.id) {
                return false;
            }
            removed.contains(&edge.from)
                || edge.to.as_ref().is_some_and(|id| removed.contains(id))
                || (scopes.contains(&edge.span.path) && !ambiguous_paths.contains(&edge.span.path))
        })
        .map(|edge| edge.id.clone())
        .collect();
    let cleanup_total = deleted_nodes.len() + deleted_edges.len();
    progress.phase("publishing", "cleanup", Some(cleanup_total.max(1)), "items");
    progress.advance(0, None);
    r.delete_edges(&deleted_edges)?;
    progress.advance(deleted_edges.len().max(1).min(cleanup_total.max(1)), None);
    r.delete_nodes(&deleted_nodes)?;
    progress.advance(cleanup_total.max(1), None);
    for mut correction in r.corrections()? {
        let c = &mut correction.correction;
        if c.status == "pending"
            && (c.target_kind == "node" && removed.contains(&c.target_id)
                || c.target_kind == "edge" && deleted_edges.iter().any(|id| id == &c.target_id))
        {
            c.status = "deleted".into();
            r.put_correction(&correction)?;
        }
    }
    // A path the new analysis reported on (e.g. a file that failed to parse and so is
    // outside the published scope) gets the new diagnostics instead of accumulating.
    let reported: BTreeSet<&str> = snapshot
        .diagnostics
        .iter()
        .map(|d| d.path.as_str())
        .collect();
    let mut diagnostics: Vec<_> = r
        .diagnostics()?
        .into_iter()
        .filter(|d| !scopes.contains(&d.path) && !reported.contains(d.path.as_str()))
        .collect();
    diagnostics.extend(snapshot.diagnostics);
    r.set_diagnostics(&diagnostics)?;
    Ok(IndexReport {
        nodes: retained.len(),
        edges: kept_edges.len(),
        deleted_nodes: deleted_nodes.len(),
        deleted_edges: deleted_edges.len(),
        diagnostics: diagnostics.len(),
    })
}
