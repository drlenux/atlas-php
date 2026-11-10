//! Incremental analysis scope. Pure, language-neutral functions over declaration nodes,
//! shared by language plugins (to limit body analysis) and the index coordinator (to
//! filter and publish), so that both arrive at the same scope.
use super::SourceFile;
use crate::domain::Node;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Declaration entries per source path.
pub type Surface = BTreeMap<String, BTreeSet<String>>;

const DECLARATIONS: [&str; 8] = [
    "class",
    "interface",
    "trait",
    "enum",
    "method",
    "function",
    "property",
    "constant",
];
const CLASS_LIKE: [&str; 4] = ["class", "interface", "trait", "enum"];
/// Metadata that other files' analysis depends on (types, members, hierarchy).
const SURFACE_FIELDS: [&str; 12] = [
    "type",
    "type_source",
    "declared_return_type",
    "annotated_return_type",
    "parameters",
    "parent",
    "interfaces",
    "traits",
    "magic",
    "access",
    "relation",
    "implemented_by",
];

/// What other files can observe of each file's declarations: ids and their types,
/// without positions, fingerprints or bodies (a body edit leaves it unchanged).
pub fn declaration_surface(nodes: &[Node]) -> Surface {
    let mut surface = Surface::new();
    for node in nodes {
        if !DECLARATIONS.contains(&node.kind.as_str()) || node.metadata["closure"] == true {
            continue;
        }
        let fields: Map<String, Value> = SURFACE_FIELDS
            .iter()
            .filter_map(|key| {
                node.metadata
                    .get(*key)
                    .filter(|v| !v.is_null())
                    .map(|v| ((*key).to_owned(), v.clone()))
            })
            .collect();
        surface
            .entry(node.span.path.clone())
            .or_default()
            .insert(format!("{}|{}", node.id, Value::Object(fields)));
    }
    surface
}

pub struct ScopeInput<'a> {
    /// Existing files whose content changed since the last publication.
    pub changed: &'a BTreeSet<String>,
    /// Files that no longer exist.
    pub removed: &'a BTreeSet<String>,
    pub previous: &'a Surface,
    pub current: &'a Surface,
    /// Every analyzed source (for name mentions).
    pub files: &'a [SourceFile],
    /// Current declaration nodes (for the class hierarchy).
    pub nodes: &'a [Node],
}

/// Files whose bodies must be analyzed again: the changed files, files that mention a
/// changed declaration by name, and descendants of classes whose members changed.
/// `None` means the scope is too wide and a full rebuild is the better choice.
pub fn incremental_scope(input: &ScopeInput) -> Option<BTreeSet<String>> {
    let empty = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut member_owners = BTreeSet::new();
    for path in input.changed.iter().chain(input.removed) {
        let previous = input.previous.get(path).unwrap_or(&empty);
        let current = input.current.get(path).unwrap_or(&empty);
        for entry in previous.symmetric_difference(current) {
            let id = entry.split('|').next().unwrap_or(entry);
            changed_names(id, &mut names, &mut member_owners);
        }
    }
    let mut scope: BTreeSet<String> = input.changed.clone();
    for file in input.files {
        if !scope.contains(&file.path) && names.iter().any(|n| mentions(&file.source, n)) {
            scope.insert(file.path.clone());
        }
    }
    scope.extend(descendant_paths(input.nodes, &member_owners));
    let limit = (input.files.len() / 4).max(50);
    (scope.len() <= limit).then_some(scope)
}

/// `class:A\B` → `B` (class-level change); `method:A\B::m` → `m` and owner `A\B`.
fn changed_names(id: &str, names: &mut BTreeSet<String>, owners: &mut BTreeSet<String>) {
    let Some((kind, qualified)) = id.split_once(':') else {
        return;
    };
    if CLASS_LIKE.contains(&kind) {
        names.insert(short_name(qualified).to_owned());
        return;
    }
    match qualified.rsplit_once("::") {
        Some((owner, member)) => {
            names.insert(member.trim_start_matches('$').to_owned());
            owners.insert(owner.to_ascii_lowercase());
        }
        None => {
            names.insert(short_name(qualified).to_owned());
        }
    }
}

fn short_name(qualified: &str) -> &str {
    qualified.rsplit('\\').next().unwrap_or(qualified)
}

/// Paths of class-like declarations that extend, implement or use any of `owners`.
fn descendant_paths(nodes: &[Node], owners: &BTreeSet<String>) -> BTreeSet<String> {
    let mut children: HashMap<String, Vec<&Node>> = HashMap::new();
    for node in nodes
        .iter()
        .filter(|n| CLASS_LIKE.contains(&n.kind.as_str()))
    {
        let bases = node.metadata["parent"].as_str().into_iter().chain(
            ["interfaces", "traits"]
                .iter()
                .filter_map(|key| node.metadata[*key].as_array())
                .flatten()
                .filter_map(Value::as_str),
        );
        for base in bases {
            children
                .entry(base.trim_start_matches('\\').to_ascii_lowercase())
                .or_default()
                .push(node);
        }
    }
    let mut paths = BTreeSet::new();
    let mut pending: Vec<String> = owners.iter().cloned().collect();
    let mut seen = BTreeSet::new();
    while let Some(class) = pending.pop() {
        if !seen.insert(class.clone()) {
            continue;
        }
        for child in children.get(&class).into_iter().flatten() {
            paths.insert(child.span.path.clone());
            pending.push(child.qualified_name.to_ascii_lowercase());
        }
    }
    paths
}

/// `name` occurs in `source` as a whole identifier.
fn mentions(source: &str, name: &str) -> bool {
    let identifier = |c: char| c.is_alphanumeric() || c == '_';
    !name.is_empty()
        && source.match_indices(name).any(|(at, _)| {
            let before = source[..at].chars().next_back();
            let after = source[at + name.len()..].chars().next();
            !before.is_some_and(identifier) && !after.is_some_and(identifier)
        })
}
