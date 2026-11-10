use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

pub type Result<T> = std::result::Result<T, AtlasError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    NotFound,
    Conflict,
    Forbidden,
    Storage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AtlasError {
    pub kind: ErrorKind,
    pub message: String,
}
impl AtlasError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }
    pub fn storage(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Storage, message)
    }
    pub fn missing(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, message)
    }
}
impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for AtlasError {}

pub fn empty_metadata() -> Value {
    json!({})
}
fn analyzer_trust() -> u8 {
    1
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub start_line: usize,
    #[serde(default)]
    pub end_line: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub qualified_name: String,
    #[serde(default)]
    pub span: SourceSpan,
    #[serde(default = "empty_metadata", skip_serializing_if = "is_empty_metadata")]
    pub metadata: Value,
    #[serde(default = "analyzer_trust")]
    pub trust: u8,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub revision: u64,
}

fn is_empty_metadata(value: &Value) -> bool {
    value.as_object().is_none_or(serde_json::Map::is_empty)
}
fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub kind: String,
    pub from: String,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub span: SourceSpan,
    #[serde(default = "empty_metadata")]
    pub metadata: Value,
    #[serde(default = "analyzer_trust")]
    pub trust: u8,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub line: usize,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphSnapshot {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphContext {
    pub root_id: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub diagnostics: Vec<Diagnostic>,
    /// Response limits cut nodes or edges that were found.
    pub truncated: bool,
    /// Nodes at the depth boundary have further edges in the requested direction.
    #[serde(default)]
    pub depth_limited: bool,
    /// Declaration members of the root left out by `Expansion::Focused`.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub omitted_members: usize,
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexReport {
    pub nodes: usize,
    pub edges: usize,
    pub deleted_nodes: usize,
    pub deleted_edges: usize,
    pub diagnostics: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Correction {
    pub id: String,
    pub target_kind: String,
    pub target_id: String,
    pub before: Value,
    pub after: Value,
    pub before_trust: u8,
    pub revision: u64,
    pub status: String,
    pub reason: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredCorrection {
    pub correction: Correction,
    pub previous_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageElement {
    pub target_kind: String,
    pub id: String,
    pub kind: String,
    pub label: String,
    pub qualified_name: String,
    pub path: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
    pub id: String,
    pub timestamp: i64,
    pub elements: Vec<UsageElement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageBucket {
    pub start: i64,
    pub responses: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analytics {
    pub from: i64,
    pub to: i64,
    pub bucket_seconds: i64,
    pub total_responses: u64,
    pub unique_nodes: usize,
    pub unique_edges: usize,
    pub timeline: Vec<UsageBucket>,
    pub elements: Vec<UsageElement>,
}

/// Repository-level node search: text match, optional kind filter and keyset cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeSearch {
    pub text: String,
    /// Empty means every kind.
    pub kinds: Vec<String>,
    /// Return only nodes whose id is greater than this cursor.
    pub after: Option<String>,
    pub limit: usize,
    pub mode: SearchMode,
    /// Anchored regex over `span.path`; matching nodes are left out (e.g. test code).
    pub exclude_paths: Option<String>,
    /// Anchored regex over `span.path`; only matching nodes are kept (`area` filter).
    pub include_paths: Option<String>,
}

/// `Text` matches names and occurrence expressions as whole identifiers at the query
/// edges; `Substring` matches the same fields anywhere; `Symbol` matches declaration
/// names, qualified names and occurrence `member_name` exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Text,
    Substring,
    Symbol,
}

/// Compact structure of a class-like node or a method (no bodies, no metadata).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Outline {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub path: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub traits: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overridden_by: Vec<String>,
    /// Method: one line per outgoing call or queue dispatch of its call sites.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
    /// Method: one line per call site (in another method) that calls or dispatches it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub called_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// Requested sections that are empty (stated, so "none" differs from "not asked").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub none: Vec<String>,
    /// `called_by` sites under test paths that were left out.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub excluded_tests: usize,
    /// Untyped call sites with this method's name, counted (not listed) because many
    /// methods share the name.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub name_only_callers: usize,
}

/// How current the atlas is: last successful publication and pending file changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Freshness {
    /// UTC seconds of the last successful publication; `None` before the first one.
    pub indexed_at: Option<i64>,
    pub state: String,
    pub changes_pending: bool,
    /// Files changed after the last publication (at most 100): answers about them may
    /// be stale even when `indexed_at` is recent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchPage {
    pub nodes: Vec<Node>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Both,
    In,
    Out,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    #[default]
    Summary,
    Full,
}

/// Callers of a method (or usages of a class's members), one tree level per call level.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CallerTree {
    pub root_id: String,
    pub root_label: String,
    pub root_path: String,
    pub root_line: usize,
    /// Call levels requested.
    pub levels: usize,
    pub children: Vec<CallerNode>,
    /// The node budget cut the tree.
    pub truncated: bool,
    /// Caller sites under test paths that were left out.
    pub excluded_tests: usize,
    /// Class root: members without usages from outside the class.
    pub members_without_usages: usize,
    /// Untyped call sites with the root method's name, counted because the name is common.
    pub name_only_callers: usize,
}

/// One caller (all its sites merged) or, under a class root, one used member.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CallerNode {
    /// Caller symbol (`A\B::run`), member symbol under a class root, or the file path
    /// of code outside any function.
    pub label: String,
    pub id: Option<String>,
    pub path: String,
    pub lines: Vec<usize>,
    pub resolutions: Vec<String>,
    /// Branch conditions of the first site, outermost first.
    pub conditions: Vec<String>,
    pub expression: String,
    pub children: Vec<CallerNode>,
    /// `recursion` when the caller is already on the path from the root.
    pub note: Option<String>,
    /// Area prefix of the caller's code (`.atlas/setup.yaml` `areas`).
    pub area: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallersRequest {
    pub root_id: String,
    pub levels: usize,
    pub max_nodes: usize,
    /// Anchored path regex of caller sites to leave out (test code).
    pub exclude_paths: Option<String>,
    /// Anchored path regex of caller sites to keep (`area` filter).
    pub include_paths: Option<String>,
}

/// How far listing edges (membership, class hierarchy) are expanded in a context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Expansion {
    /// Every expanded node lists members and hierarchy neighbours (reviewer graph).
    #[default]
    Full,
    /// Agent view: declaration members are never listed (see `atlas_outline`) and are
    /// counted for the root; the root keeps its own occurrence sites and locals; class
    /// hierarchy edges are followed only from the root.
    Focused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextRequest {
    pub root_id: String,
    pub depth: usize,
    pub max_nodes: usize,
    pub direction: Direction,
    pub detail: Detail,
    pub include_diagnostics: bool,
    pub expansion: Expansion,
}

/// A project folder group (application) from `.atlas/setup.yaml` `areas`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    /// Label shown on entities and used as the namespace-less name prefix.
    pub prefix: String,
    /// Folders (relative to the project root) that belong to the area.
    pub paths: Vec<String>,
    /// Code used by every area (e.g. `common`): it keeps the union of application contexts.
    #[serde(default)]
    pub shared: bool,
}

/// The area whose folder is the longest prefix of `path` (folder boundaries only).
pub fn area_of<'a>(areas: &'a [Area], path: &str) -> Option<&'a Area> {
    areas
        .iter()
        .flat_map(|area| area.paths.iter().map(move |folder| (area, folder)))
        .filter(|(_, folder)| {
            let folder = folder.trim_end_matches('/');
            path == folder || path.starts_with(&format!("{folder}/"))
        })
        .max_by_key(|(_, folder)| folder.trim_end_matches('/').len())
        .map(|(area, _)| area)
}
