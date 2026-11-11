//! Language-neutral contracts for extensions compiled into Atlas.
use crate::domain::GraphSnapshot;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
pub mod incremental;
mod registry;
pub use registry::PluginRegistry;

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct AnalysisOptions {
    pub language_versions: BTreeMap<String, String>,
    pub max_graph_elements: usize,
    pub deadline: Instant,
    pub workers: usize,
    pub progress: crate::progress::IndexingProgress,
    /// Path prefixes analyzed for declarations only (no function bodies).
    pub declaration_paths: Vec<String>,
    /// Set for an incremental rebuild; the language plugin records the scope it used.
    pub incremental: Option<std::sync::Arc<Incremental>>,
    /// Application folders (`.atlas/setup.yaml` `areas`).
    pub areas: Vec<crate::domain::Area>,
}

/// Context of an incremental rebuild. A language plugin that supports it analyzes
/// bodies only within the scope it records; one that does not leaves it unset, which
/// the coordinator treats as a full rebuild.
#[derive(Debug, Default)]
pub struct Incremental {
    pub changed: std::collections::BTreeSet<String>,
    pub removed: std::collections::BTreeSet<String>,
    pub previous_surface: incremental::Surface,
    scope: std::sync::Mutex<ScopeDecision>,
}
#[derive(Debug, Default)]
enum ScopeDecision {
    #[default]
    Unset,
    Full,
    Paths(std::collections::BTreeSet<String>),
}
impl Incremental {
    pub fn new(
        changed: std::collections::BTreeSet<String>,
        removed: std::collections::BTreeSet<String>,
        previous_surface: incremental::Surface,
    ) -> Self {
        Self {
            changed,
            removed,
            previous_surface,
            scope: Default::default(),
        }
    }
    /// `None` means the plugin analyzed every body. A full decision always wins.
    pub fn record(&self, scope: Option<std::collections::BTreeSet<String>>) {
        let mut decision = self.scope.lock().unwrap_or_else(|e| e.into_inner());
        *decision = match (std::mem::take(&mut *decision), scope) {
            (ScopeDecision::Full, _) | (_, None) => ScopeDecision::Full,
            (ScopeDecision::Paths(mut paths), Some(more)) => {
                paths.extend(more);
                ScopeDecision::Paths(paths)
            }
            (ScopeDecision::Unset, Some(paths)) => ScopeDecision::Paths(paths),
        };
    }
    /// The recorded scope; `None` when a full rebuild happened or nothing was recorded.
    pub fn scope(&self) -> Option<std::collections::BTreeSet<String>> {
        match &*self.scope.lock().unwrap_or_else(|e| e.into_inner()) {
            ScopeDecision::Paths(paths) => Some(paths.clone()),
            _ => None,
        }
    }
}
impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            language_versions: BTreeMap::new(),
            max_graph_elements: 2_000_000,
            deadline: Instant::now() + Duration::from_secs(900),
            workers: 1,
            progress: Default::default(),
            declaration_paths: Vec::new(),
            incremental: None,
            areas: Vec::new(),
        }
    }
}
impl AnalysisOptions {
    /// Files under `declaration_paths` contribute declarations only.
    pub fn declaration_only(&self, path: &str) -> bool {
        self.declaration_paths.iter().any(|prefix| {
            let prefix = prefix.trim_end_matches('/');
            path == prefix || path.starts_with(&format!("{prefix}/"))
        })
    }
    pub fn check(&self, elements: usize) -> Result<(), String> {
        if Instant::now() >= self.deadline {
            return Err("analysis time limit exceeded".into());
        }
        if elements > self.max_graph_elements {
            return Err("analysis graph element limit exceeded".into());
        }
        Ok(())
    }
}

/// Supplies framework members (property nodes with `type`, `type_source: framework`,
/// `framework`, `framework_evidence` and optional `implemented_by`) for declarations.
pub trait MemberSource {
    fn members(&self, declarations: &GraphSnapshot) -> Result<GraphSnapshot, String>;
}

pub trait LanguagePlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    fn analyze(
        &self,
        files: &[SourceFile],
        options: &AnalysisOptions,
    ) -> Result<GraphSnapshot, String>;
    /// `analyze` with framework members merged after declarations and before bodies.
    /// Languages without that step keep the default.
    fn analyze_with_members(
        &self,
        files: &[SourceFile],
        options: &AnalysisOptions,
        _members: &dyn MemberSource,
    ) -> Result<GraphSnapshot, String> {
        self.analyze(files, options)
    }
}

pub trait FrameworkPlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn language(&self) -> &'static str;
    /// Extra source extensions loaded for enrichment (e.g. js/html for URL sites).
    fn extra_extensions(&self) -> &'static [&'static str] {
        &[]
    }
    /// Members the framework establishes from code or configuration, before bodies
    /// are analyzed. `declarations` holds the language plugin's declaration graph.
    fn members(
        &self,
        _files: &[SourceFile],
        _declarations: &GraphSnapshot,
        _options: &AnalysisOptions,
    ) -> Result<GraphSnapshot, String> {
        Ok(GraphSnapshot::default())
    }
    fn enrich(
        &self,
        files: &[SourceFile],
        graph: &mut GraphSnapshot,
        options: &AnalysisOptions,
    ) -> Result<(), String>;
}

/// Edge kinds that state the effect of an occurrence site (dispatches are extra links).
const EFFECT_KINDS: [&str; 7] = [
    "calls",
    "reads",
    "writes",
    "property_access",
    "constant_access",
    "magic_get",
    "magic_set",
];

/// An occurrence node (`*_site`) mirrors the `resolution` of its effect edge, so that a
/// client reading only nodes (search) sees the same status as one reading edges.
/// Run after every step that may change edges (finalization, framework enrichment).
pub fn sync_occurrence_resolution(graph: &mut GraphSnapshot) {
    let resolutions: std::collections::HashMap<String, serde_json::Value> = graph
        .edges
        .iter()
        .filter(|e| EFFECT_KINDS.contains(&e.kind.as_str()))
        .map(|e| (e.from.clone(), e.metadata["resolution"].clone()))
        .collect();
    for node in graph.nodes.iter_mut().filter(|n| n.kind.ends_with("_site")) {
        if let Some(resolution) = resolutions.get(&node.id) {
            node.metadata["resolution"] = resolution.clone();
        }
    }
}
