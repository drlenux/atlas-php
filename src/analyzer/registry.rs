use super::{AnalysisOptions, FrameworkPlugin, LanguagePlugin, SourceFile};
use crate::domain::{Edge, GraphSnapshot, Node};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub struct PluginRegistry {
    languages: Vec<Box<dyn LanguagePlugin>>,
    frameworks: Vec<Box<dyn FrameworkPlugin>>,
}

impl PluginRegistry {
    pub fn catalog(&self) -> serde_json::Value {
        serde_json::json!({
            "loading": "compiled_into_atlas",
            "languages": self.languages.iter().map(|p| serde_json::json!({"id":p.id(),"extensions":p.extensions()})).collect::<Vec<_>>(),
            "frameworks": self.frameworks.iter().map(|p| serde_json::json!({"id":p.id(),"requires_language":p.language()})).collect::<Vec<_>>()
        })
    }
    pub fn new(
        languages: Vec<Box<dyn LanguagePlugin>>,
        frameworks: Vec<Box<dyn FrameworkPlugin>>,
    ) -> Result<Self, String> {
        unique(
            &languages
                .iter()
                .map(|p| p.id().to_owned())
                .collect::<Vec<_>>(),
            "registered language",
        )?;
        unique(
            &frameworks
                .iter()
                .map(|p| p.id().to_owned())
                .collect::<Vec<_>>(),
            "registered framework",
        )?;
        Ok(Self {
            languages,
            frameworks,
        })
    }
    pub fn validate(&self, languages: &[String], frameworks: &[String]) -> Result<(), String> {
        if languages.is_empty() {
            return Err("at least one language plugin is required".into());
        }
        unique(languages, "enabled language")?;
        unique(frameworks, "enabled framework")?;
        let mut extensions = BTreeSet::new();
        for id in languages {
            let language = self.language(id)?;
            for extension in language.extensions() {
                if !extensions.insert(extension.to_ascii_lowercase()) {
                    return Err(format!("conflicting file extension: {extension}"));
                }
            }
        }
        for id in frameworks {
            let framework = self.framework(id)?;
            if !languages.iter().any(|id| id == framework.language()) {
                return Err(format!(
                    "framework {} requires {} language plugin",
                    framework.id(),
                    framework.language()
                ));
            }
        }
        Ok(())
    }
    pub fn extensions(
        &self,
        languages: &[String],
        frameworks: &[String],
    ) -> Result<BTreeSet<String>, String> {
        self.validate(languages, frameworks)?;
        let mut set: BTreeSet<String> = languages
            .iter()
            .map(|id| self.language(id).expect("validated language"))
            .flat_map(|p| p.extensions().iter().map(|s| s.to_ascii_lowercase()))
            .collect();
        for id in frameworks {
            let framework = self.framework(id)?;
            for extension in framework.extra_extensions() {
                set.insert(extension.to_ascii_lowercase());
            }
        }
        Ok(set)
    }
    pub fn analyze(
        &self,
        files: &[SourceFile],
        languages: &[String],
        frameworks: &[String],
        options: &AnalysisOptions,
    ) -> Result<GraphSnapshot, String> {
        self.validate(languages, frameworks)?;
        options.check(0)?;
        let mut result = GraphSnapshot::default();
        for id in languages {
            let language = self.language(id)?;
            options
                .progress
                .phase("analysis", language.id(), None, "items");
            let selected: Vec<_> = files
                .iter()
                .filter(|file| extension_matches(&file.path, language.extensions()))
                .cloned()
                .collect();
            let frameworks_of_language: Vec<&dyn FrameworkPlugin> = frameworks
                .iter()
                .map(|id| self.framework(id))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|framework| framework.language() == id)
                .collect();
            let members = FrameworkMembers {
                frameworks: frameworks_of_language.clone(),
                files: &selected,
                options,
            };
            let mut graph = language.analyze_with_members(&selected, options, &members)?;
            let valid_php: Vec<_> = selected
                .into_iter()
                .filter(|f| {
                    !graph.diagnostics.iter().any(|d| {
                        d.path == f.path
                            && matches!(
                                d.code.as_str(),
                                "parse_error"
                                    | "file_unavailable"
                                    | "read_error"
                                    | "file_limit"
                                    | "unsupported_syntax"
                                    | "path_outside_root"
                            )
                    })
                })
                .collect();
            for framework in &frameworks_of_language {
                let extras = framework.extra_extensions();
                let mut enrich_files = valid_php.clone();
                for file in files {
                    if extension_matches(&file.path, extras)
                        && !enrich_files.iter().any(|f| f.path == file.path)
                    {
                        enrich_files.push(file.clone());
                    }
                }
                options
                    .progress
                    .phase("analysis", framework.id(), None, "items");
                framework.enrich(&enrich_files, &mut graph, options)?;
            }
            super::sync_occurrence_resolution(&mut graph);
            // Languages share stable IDs (table:/schema: from SQL-in-PHP and .sql).
            // Vec::extend would publish duplicates and fail apply_snapshot.
            merge_language_graph(&mut result, graph);
            options.check(result.nodes.len() + result.edges.len() + result.diagnostics.len())?;
        }
        label_areas(&mut result, &options.areas);
        Ok(result)
    }
    fn language(&self, id: &str) -> Result<&dyn LanguagePlugin, String> {
        self.languages
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
            .ok_or_else(|| format!("unknown language plugin: {id}"))
    }
    fn framework(&self, id: &str) -> Result<&dyn FrameworkPlugin, String> {
        self.frameworks
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
            .ok_or_else(|| format!("unknown framework plugin: {id}"))
    }
}
/// Fold another language graph into `into`, keeping one node/edge per ID.
/// Prefer a non-`external` declaration; otherwise the later language wins so a
/// `.sql` CREATE TABLE replaces an SQL-in-PHP stub with the same `table:` id.
fn merge_language_graph(into: &mut GraphSnapshot, from: GraphSnapshot) {
    let mut nodes: BTreeMap<String, Node> =
        into.nodes.drain(..).map(|n| (n.id.clone(), n)).collect();
    for node in from.nodes {
        let replace = match nodes.get(&node.id) {
            None => true,
            Some(existing) => {
                let existing_external =
                    existing.metadata.get("external") == Some(&serde_json::json!(true));
                let incoming_external =
                    node.metadata.get("external") == Some(&serde_json::json!(true));
                match (existing_external, incoming_external) {
                    (true, false) => true,
                    (false, true) => false,
                    _ => true, // same authority: later language wins
                }
            }
        };
        if replace {
            nodes.insert(node.id.clone(), node);
        }
    }
    into.nodes = nodes.into_values().collect();

    let mut edges: BTreeMap<String, Edge> =
        into.edges.drain(..).map(|e| (e.id.clone(), e)).collect();
    for edge in from.edges {
        edges.insert(edge.id.clone(), edge);
    }
    into.edges = edges.into_values().collect();
    into.diagnostics.extend(from.diagnostics);
}

/// Every node under an area carries its prefix (`metadata.area`), whatever the plugin.
fn label_areas(graph: &mut GraphSnapshot, areas: &[crate::domain::Area]) {
    if areas.is_empty() {
        return;
    }
    for node in &mut graph.nodes {
        if let Some(area) = crate::domain::area_of(areas, &node.span.path) {
            node.metadata["area"] = serde_json::json!(area.prefix);
        }
    }
}

fn unique(ids: &[String], kind: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.is_empty() || !seen.insert(id) {
            return Err(format!("invalid or duplicate {kind} plugin: {id}"));
        }
    }
    Ok(())
}
fn extension_matches(path: &str, extensions: &[&str]) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            extensions
                .iter()
                .any(|allowed| e.eq_ignore_ascii_case(allowed))
        })
}

/// Collects members of every selected framework of one language (Mediator: the
/// language plugin does not know which frameworks exist).
struct FrameworkMembers<'a> {
    frameworks: Vec<&'a dyn FrameworkPlugin>,
    files: &'a [SourceFile],
    options: &'a AnalysisOptions,
}
impl super::MemberSource for FrameworkMembers<'_> {
    fn members(&self, declarations: &GraphSnapshot) -> Result<GraphSnapshot, String> {
        let mut result = GraphSnapshot::default();
        for framework in &self.frameworks {
            self.options
                .progress
                .phase("analysis", framework.id(), None, "items");
            let members = framework.members(self.files, declarations, self.options)?;
            result.nodes.extend(members.nodes);
            result.diagnostics.extend(members.diagnostics);
        }
        Ok(result)
    }
}
