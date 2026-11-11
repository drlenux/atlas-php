//! Native Yii 2 enrichment. Consumes PHP facts and source text; never executes project code.
mod events;
mod params;
mod syntax;
mod urls;
mod views;

use crate::analyzer::{AnalysisOptions, FrameworkPlugin, SourceFile};
use crate::domain::{Diagnostic, Edge, GraphSnapshot, Node};
use crate::plugins::php::tokens::{tokenize, Token, TokenKind};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use syntax::{Classes, Expression, Names};

pub struct Yii2Plugin;
impl FrameworkPlugin for Yii2Plugin {
    fn id(&self) -> &'static str {
        "yii2"
    }
    fn language(&self) -> &'static str {
        "php"
    }
    fn extra_extensions(&self) -> &'static [&'static str] {
        &["js", "mjs", "html", "vue"]
    }
    fn members(
        &self,
        files: &[SourceFile],
        declarations: &GraphSnapshot,
        options: &AnalysisOptions,
    ) -> Result<GraphSnapshot, String> {
        let index = Index::new(&declarations.nodes, &declarations.edges);
        options
            .progress
            .phase("analysis", "yii2_members", None, "items");
        let mut nodes = relation_members(&index, &declarations.nodes, files, options)?;
        nodes.extend(query_members(&index, &declarations.nodes, options));
        let (components, diagnostics) = component_members(&index, files, options)?;
        nodes.extend(components);
        Ok(GraphSnapshot {
            nodes,
            edges: Vec::new(),
            diagnostics,
        })
    }
    fn enrich(
        &self,
        files: &[SourceFile],
        graph: &mut GraphSnapshot,
        options: &AnalysisOptions,
    ) -> Result<(), String> {
        options.check(graph.nodes.len() + graph.edges.len())?;
        let index = Index::new(&graph.nodes, &graph.edges);
        options
            .progress
            .phase("analysis", "yii2_components", Some(files.len()), "files");
        let components = components(files, options)?;
        let behavior_jobs: Vec<(&Node, String)> = index
            .classes
            .values()
            .filter(|class| index.is_a(&class.qualified_name, "yii\\base\\Component"))
            .filter_map(|class| {
                let methods = index.methods(&class.qualified_name, "behaviors");
                match methods.as_slice() {
                    [method] => Some((*method, normalize(&class.qualified_name))),
                    _ => None,
                }
            })
            .collect();
        options.progress.phase(
            "analysis",
            "yii2_behaviors",
            Some(behavior_jobs.len()),
            "classes",
        );
        let mut behavior_slots: Vec<Option<Result<(String, Classes), String>>> =
            behavior_jobs.iter().map(|_| None).collect();
        let workers = options.workers.min(behavior_jobs.len().max(1)).max(1);
        options
            .progress
            .set_workers(workers.min(behavior_jobs.len().max(1)));
        crate::parallel::try_for_each_mut(
            &mut behavior_slots,
            workers,
            || (),
            |_, position, slot| {
                options.check(graph.nodes.len() + graph.edges.len())?;
                let (method, key) = &behavior_jobs[position];
                options.progress.work_started(&method.span.path);
                let classes = behavior_classes(method, files, options);
                options.progress.work_finished(&method.span.path);
                *slot = Some(classes.map(|classes| (key.clone(), classes)));
                Ok(())
            },
        )?;
        let mut behaviors = BTreeMap::new();
        for slot in behavior_slots {
            let (key, classes) = slot.expect("yii2 behavior worker filled its result slot")?;
            behaviors.insert(key, classes);
        }
        let mut diagnostics = Vec::new();
        // Calls that no class, ancestor, descendant or attached behavior declares.
        let mut undeclared_sites: BTreeSet<String> = BTreeSet::new();
        let mut undeclared_locations: BTreeSet<(String, usize)> = BTreeSet::new();
        for (edge_i, edge) in graph.edges.iter_mut().enumerate() {
            if edge_i % 1024 == 0 {
                options.check(
                    index.classes.len() + index.method_nodes.len() + graph.diagnostics.len(),
                )?;
            }
            if !matches!(edge.kind.as_str(), "calls" | "property_access") || edge.to.is_some() {
                continue;
            }
            let member = edge.metadata["member_name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if !identifier(&member) {
                continue;
            }
            let operation = edge.metadata["operation"]
                .as_str()
                .unwrap_or(if edge.kind == "calls" { "call" } else { "read" })
                .to_owned();
            let component = component_receiver(edge, options)?;
            if let Some(name) = component {
                let components = components.for_path(&edge.span.path, &options.areas);
                let configured = components.classes.get(&name).cloned().unwrap_or(Classes {
                    unknown: true,
                    ..Classes::default()
                });
                let mut targets = BTreeSet::new();
                let mut unknown = configured.unknown || components.unknown;
                for class in &configured.names {
                    let methods = if operation == "call" {
                        index.methods(class, &member)
                    } else if index.is_yii_object(class) {
                        index.methods(class, &accessor(&member, &operation))
                    } else {
                        vec![]
                    };
                    if methods.is_empty() {
                        unknown = true;
                    }
                    targets.extend(methods.into_iter().map(|n| n.id.clone()));
                }
                let ambiguous = configured.names.len() != 1 || unknown;
                apply(edge, targets, ambiguous, &operation, "component");
                if ambiguous {
                    let code = if configured.names.is_empty() {
                        "unknown_component"
                    } else {
                        "ambiguous_component"
                    };
                    diagnostics.push(diagnostic(
                        edge,
                        code,
                        format!("Yii component {name}: active configuration is not uniquely established"),
                    ));
                }
                continue;
            }
            let Some(receiver) = edge.metadata["receiver_class"].as_str().map(str::to_owned) else {
                continue;
            };
            // `$payment` typed `Payment|null` / `@var Payment|null` — try each class part.
            let receiver_parts: Vec<String> = receiver
                .split('|')
                .map(str::trim)
                .filter(|p| {
                    !p.is_empty()
                        && !matches!(
                            p.to_ascii_lowercase().as_str(),
                            "null" | "false" | "true" | "unknown" | "mixed"
                        )
                })
                .map(|p| p.split('<').next().unwrap_or(p).to_owned())
                .collect();
            let lookup = if operation == "call" {
                member.clone()
            } else {
                accessor(&member, &operation)
            };
            let mut targets: BTreeSet<String> = BTreeSet::new();
            let mut unknown = false;
            let mut any_yii = false;
            for part in &receiver_parts {
                if !index.is_yii_object(part) {
                    continue;
                }
                any_yii = true;
                if operation != "call" {
                    targets.extend(
                        index
                            .methods(part, &lookup)
                            .into_iter()
                            .map(|n| n.id.clone()),
                    );
                }
                if index.is_a(part, "yii\\base\\Component") {
                    if let Some(registered) = behaviors.get(&normalize(part)) {
                        unknown |= registered.unknown;
                        for behavior in &registered.names {
                            let methods = index.methods(behavior, &lookup);
                            if !index.classes.contains_key(&normalize(behavior)) {
                                unknown = true;
                            }
                            targets.extend(methods.into_iter().map(|n| n.id.clone()));
                        }
                    }
                }
            }
            if !any_yii {
                continue;
            }
            if !targets.is_empty() || unknown {
                // Property→getter: one `calls` edge (not magic_get+calls) so called_by is unique.
                if operation == "read" && targets.len() == 1 && !unknown {
                    let getter = targets.iter().next().cloned().expect("len==1");
                    let meta = relation_property_call(edge, getter.clone()).metadata;
                    edge.kind = "calls".into();
                    edge.to = Some(getter);
                    edge.metadata = meta;
                } else {
                    apply(edge, targets, unknown, &operation, "behavior_or_accessor");
                    if edge.to.is_none() {
                        diagnostics.push(diagnostic(
                            edge,
                            "ambiguous_yii_member",
                            format!(
                                "Yii member {receiver}::{member} has no unique static implementation"
                            ),
                        ));
                    }
                }
            } else if operation == "call"
                && only_component_magic(edge)
                && edge.metadata["unindexed_ancestor"].is_null()
                && receiver_parts
                    .iter()
                    .all(|part| index.is_yii_object(part) && !index.declared_below(part, &member))
            {
                // Component::__call only dispatches to behaviors, and none declares it.
                edge.metadata["undeclared"] = json!(true);
                undeclared_sites.insert(edge.from.clone());
                undeclared_locations.insert((edge.span.path.clone(), edge.span.start_line));
                diagnostics.push(diagnostic(
                    edge,
                    "undeclared_member",
                    format!(
                        "Method {receiver}::{member}() is not declared in the class, its ancestors, descendants or behaviors"
                    ),
                ));
            }
        }
        graph.diagnostics.retain(|d| {
            d.code != "unresolved_call" || !undeclared_locations.contains(&(d.path.clone(), d.line))
        });
        // A resolved framework edge supersedes only a base unresolved-member diagnostic
        // when no remaining unresolved edge exists at that source location.
        let resolved: BTreeSet<_> = graph
            .edges
            .iter()
            .filter(|e| e.metadata["framework"] == "yii2" && e.to.is_some())
            .map(|e| (e.span.path.clone(), e.span.start_line))
            .collect();
        let unresolved: BTreeSet<_> = graph
            .edges
            .iter()
            .filter(|e| e.to.is_none())
            .map(|e| (e.span.path.clone(), e.span.start_line))
            .collect();
        graph.diagnostics.retain(|d| {
            !matches!(d.code.as_str(), "unresolved_call" | "unresolved_property")
                || !resolved.contains(&(d.path.clone(), d.line))
                || unresolved.contains(&(d.path.clone(), d.line))
        });
        graph.diagnostics.extend(diagnostics);
        let dispatches = queue_dispatches(&index, &graph.edges);
        let rules_edges = rules_validator_edges(&index, files, graph, options)?;
        drop(index);
        for node in &mut graph.nodes {
            if undeclared_sites.contains(&node.id) {
                node.metadata["undeclared"] = json!(true);
            }
        }
        graph.edges.extend(dispatches);
        graph.edges.extend(rules_edges);
        urls::enrich_urls(files, graph, options)?;
        events::enrich_events(files, graph, options)?;
        views::enrich_views(files, graph, options)?;
        params::enrich_params(files, graph, options)?;
        options.check(graph.nodes.len() + graph.edges.len() + graph.diagnostics.len())
    }
}

struct Index<'a> {
    classes: BTreeMap<String, &'a Node>,
    method_nodes: BTreeMap<(String, String), Vec<&'a Node>>,
    available: BTreeMap<String, Vec<&'a Node>>,
    /// Proven subclasses (normalized name → normalized descendant names).
    descendants: BTreeMap<String, Vec<String>>,
    /// Classes whose parent chain leaves the index (possible hidden placement in the tree).
    incomplete: Vec<String>,
}
impl<'a> Index<'a> {
    fn new(nodes: &'a [Node], edges: &[Edge]) -> Self {
        let classes: BTreeMap<String, &Node> = nodes
            .iter()
            .filter(|n| matches!(n.kind.as_str(), "class" | "trait" | "interface"))
            .map(|n| (normalize(&n.qualified_name), n))
            .collect();
        let mut method_nodes: BTreeMap<(String, String), Vec<&Node>> = BTreeMap::new();
        for node in nodes.iter().filter(|n| n.kind == "method") {
            if let Some((owner, name)) = node.qualified_name.rsplit_once("::") {
                method_nodes
                    .entry((normalize(owner), name.to_lowercase()))
                    .or_default()
                    .push(node);
            }
        }
        let by_id: std::collections::HashMap<&str, &Node> =
            nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut available: BTreeMap<String, Vec<&Node>> = BTreeMap::new();
        for edge in edges.iter().filter(|e| e.kind == "available_via") {
            if let (Some(owner), Some(target)) = (
                by_id.get(edge.from.as_str()).copied(),
                edge.to.as_deref().and_then(|id| by_id.get(id).copied()),
            ) {
                available
                    .entry(normalize(&owner.qualified_name))
                    .or_default()
                    .push(target);
            }
        }
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, node) in &classes {
            if let Some(parent) = node.metadata["parent"].as_str() {
                children
                    .entry(normalize(parent))
                    .or_default()
                    .push(name.clone());
            }
            if let Some(traits) = node.metadata["traits"].as_array() {
                for trait_name in traits.iter().filter_map(Value::as_str) {
                    children
                        .entry(normalize(trait_name))
                        .or_default()
                        .push(name.clone());
                }
            }
        }
        let mut descendants: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for root in classes.keys() {
            let mut stack = children.get(root).cloned().unwrap_or_default();
            let mut seen = BTreeSet::from([root.clone()]);
            let mut out = Vec::new();
            while let Some(next) = stack.pop() {
                if !seen.insert(next.clone()) {
                    continue;
                }
                out.push(next.clone());
                if let Some(more) = children.get(&next) {
                    stack.extend(more.iter().cloned());
                }
            }
            if !out.is_empty() {
                descendants.insert(root.clone(), out);
            }
        }
        let incomplete: Vec<String> = classes
            .keys()
            .filter(|name| {
                let mut current = (*name).clone();
                let mut seen = BTreeSet::new();
                while seen.insert(current.clone()) {
                    let Some(node) = classes.get(&current) else {
                        return true;
                    };
                    match node.metadata["parent"].as_str() {
                        Some(parent) => current = normalize(parent),
                        None => return false,
                    }
                }
                true
            })
            .cloned()
            .collect();
        Self {
            classes,
            method_nodes,
            available,
            descendants,
            incomplete,
        }
    }
    fn is_a(&self, class: &str, base: &str) -> bool {
        let mut current = normalize(class);
        let mut seen = BTreeSet::new();
        let base = normalize(base);
        while seen.insert(current.clone()) {
            if current == base {
                return true;
            }
            let Some(parent) = self
                .classes
                .get(&current)
                .and_then(|n| n.metadata["parent"].as_str())
            else {
                return false;
            };
            current = normalize(parent);
        }
        false
    }
    /// A class below `class` declares `name`: proven descendants, or a class whose
    /// ancestry leaves the index (it may sit under `class` via a missing vendor parent).
    /// Incomplete classes are listed once in `Index`; we do not run `is_a` over the
    /// whole class table per edge (that dominated Yii time on Slotor).
    fn declared_below(&self, class: &str, name: &str) -> bool {
        let base = normalize(class);
        let name = name.to_lowercase();
        let has = |c: &str| !self.find_methods(c, &name, &mut BTreeSet::new()).is_empty();
        self.descendants
            .get(&base)
            .into_iter()
            .flatten()
            .any(|c| has(c))
            || self.incomplete.iter().any(|c| c != &base && has(c))
    }
    /// `extends` names a class that is not in the index (outside declaration_paths).
    fn parent_unindexed(&self, class: &str) -> bool {
        self.classes
            .get(&normalize(class))
            .and_then(|n| n.metadata["parent"].as_str())
            .is_some_and(|parent| !self.classes.contains_key(&normalize(parent)))
    }
    fn is_yii_object(&self, class: &str) -> bool {
        self.is_a(class, "yii\\base\\Component") || self.is_a(class, "yii\\base\\BaseObject")
    }
    fn methods(&self, class: &str, name: &str) -> Vec<&'a Node> {
        self.find_methods(
            &normalize(class),
            &name.to_lowercase(),
            &mut BTreeSet::new(),
        )
    }
    fn find_methods(&self, class: &str, name: &str, seen: &mut BTreeSet<String>) -> Vec<&'a Node> {
        if !seen.insert(class.into()) {
            return vec![];
        }
        if let Some(methods) = self.method_nodes.get(&(class.into(), name.into())) {
            return methods.clone();
        }
        if let Some(methods) = self.available.get(class) {
            let matches: Vec<_> = methods
                .iter()
                .copied()
                .filter(|n| {
                    n.qualified_name
                        .rsplit("::")
                        .next()
                        .is_some_and(|m| m.eq_ignore_ascii_case(name))
                })
                .collect();
            if !matches.is_empty() {
                return matches;
            }
        }
        let Some(node) = self.classes.get(class) else {
            return vec![];
        };
        let mut methods = Vec::new();
        if let Some(traits) = node.metadata["traits"].as_array() {
            for trait_name in traits.iter().filter_map(Value::as_str) {
                methods.extend(self.find_methods(&normalize(trait_name), name, seen));
            }
        }
        if !methods.is_empty() {
            return methods;
        }
        node.metadata["parent"]
            .as_str()
            .map(|parent| self.find_methods(&normalize(parent), name, seen))
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct ComponentDefinitions {
    classes: BTreeMap<String, Classes>,
    unknown: bool,
    /// First configuration location (path, line) per component name.
    sources: BTreeMap<String, (String, usize)>,
    /// Where the component map was first found to be set at runtime.
    unknown_at: Option<(String, usize)>,
}

/// Component definitions of all configs and, per non-shared area, of the configs that apply to
/// it: its own folders, shared areas and files outside every area.
#[derive(Default)]
struct AreaComponents {
    all: ComponentDefinitions,
    by_area: BTreeMap<String, ComponentDefinitions>,
}

impl AreaComponents {
    /// Definitions for code at `path` (its area's, or the union for shared/no area).
    fn for_path(&self, path: &str, areas: &[crate::domain::Area]) -> &ComponentDefinitions {
        crate::domain::area_of(areas, path)
            .and_then(|area| self.by_area.get(&area.prefix))
            .unwrap_or(&self.all)
    }
}

fn components(files: &[SourceFile], options: &AnalysisOptions) -> Result<AreaComponents, String> {
    let mut slots: Vec<Option<Result<ComponentDefinitions, String>>> =
        files.iter().map(|_| None).collect();
    let workers = options.workers.min(files.len().max(1)).max(1);
    crate::parallel::try_for_each_mut(
        &mut slots,
        workers,
        || (),
        |_, position, slot| {
            let file = &files[position];
            options.progress.work_started(&file.path);
            let scanned = scan_components(file, options);
            options.progress.work_finished(&file.path);
            *slot = Some(scanned);
            Ok(())
        },
    )?;
    let mut partials = Vec::new();
    for (file, slot) in files.iter().zip(slots) {
        partials.push((
            file.path.as_str(),
            slot.expect("yii2 worker filled its result slot")?,
        ));
    }
    let mut result = AreaComponents {
        all: merge_definitions(partials.iter().map(|(_, d)| d)),
        by_area: BTreeMap::new(),
    };
    for area in options.areas.iter().filter(|area| !area.shared) {
        let applies = partials.iter().filter(|(path, _)| {
            crate::domain::area_of(&options.areas, path)
                .is_none_or(|owner| owner.shared || owner.prefix == area.prefix)
        });
        result.by_area.insert(
            area.prefix.clone(),
            merge_definitions(applies.map(|(_, d)| d)),
        );
    }
    Ok(result)
}

fn merge_definitions<'a>(
    partials: impl Iterator<Item = &'a ComponentDefinitions>,
) -> ComponentDefinitions {
    let mut result = ComponentDefinitions::default();
    for partial in partials {
        result.unknown |= partial.unknown;
        for (name, classes) in &partial.classes {
            result
                .classes
                .entry(name.clone())
                .or_default()
                .merge(classes.clone());
        }
        for (name, source) in &partial.sources {
            result.sources.entry(name.clone()).or_insert(source.clone());
        }
        if result.unknown_at.is_none() {
            result.unknown_at = partial.unknown_at.clone();
        }
    }
    result
}

fn scan_components(
    file: &SourceFile,
    options: &AnalysisOptions,
) -> Result<ComponentDefinitions, String> {
    let mut result = ComponentDefinitions::default();
    // The key must appear literally; skip tokenizing files that cannot contain it.
    if !file.source.contains("components") {
        return Ok(result);
    }
    let tokens = tokenize(&file.source, options)?;
    for at in 0..tokens.len().saturating_sub(2) {
        options.check(0)?;
        if syntax::string(&tokens[at].text).as_deref() != Some("components")
            || tokens[at + 1].text != "=>"
        {
            continue;
        }
        let start = at + 2;
        let end = expression_end(&tokens, start);
        match syntax::expression(&tokens[start..end], &syntax::names_at(&tokens, at), options)? {
            Expression::Array(items) => {
                for (name, value) in items {
                    if let Some(name) = name {
                        result
                            .sources
                            .entry(name.clone())
                            .or_insert((file.path.clone(), tokens[at].line));
                        result
                            .classes
                            .entry(name)
                            .or_default()
                            .merge(syntax::component_class(&value));
                    } else {
                        result.unknown = true;
                    }
                }
            }
            _ => result.unknown = true,
        }
        if result.unknown && result.unknown_at.is_none() {
            result.unknown_at = Some((file.path.clone(), tokens[at].line));
        }
    }
    Ok(result)
}

fn expression_end(tokens: &[Token], start: usize) -> usize {
    let mut depth = 0usize;
    for (at, token) in tokens.iter().enumerate().skip(start) {
        if token.kind != TokenKind::Symbol {
            continue;
        }
        match token.text.as_str() {
            "[" | "(" | "{" => depth += 1,
            "]" | ")" | "}" => {
                if depth == 0 {
                    return at;
                }
                depth -= 1;
            }
            "," | ";" if depth == 0 => return at,
            _ => {}
        }
    }
    tokens.len()
}

fn behavior_classes(
    method: &Node,
    files: &[SourceFile],
    options: &AnalysisOptions,
) -> Result<Classes, String> {
    let body = method_source(method, files);
    let tokens = syntax::fragment(&body, options)?;
    let names = Names::metadata(&method.metadata);
    let mut result = Classes::default();
    let mut returns = 0usize;
    let conditional = tokens.iter().any(|t| {
        t.kind == TokenKind::Ident
            && matches!(
                t.text.to_ascii_lowercase().as_str(),
                "if" | "switch" | "match" | "try" | "for" | "foreach" | "while"
            )
    });
    let mut block_depth = 0usize;
    for (at, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "{" => block_depth += 1,
                "}" => block_depth = block_depth.saturating_sub(1),
                _ => {}
            }
        }
        if !token.is("return") || block_depth != 0 {
            continue;
        }
        returns += 1;
        match syntax::expression(
            &tokens[at + 1..expression_end(&tokens, at + 1)],
            &names,
            options,
        )? {
            Expression::Array(items) => {
                for (_, item) in items {
                    result.merge(syntax::configured_class(&item));
                }
            }
            _ => result.unknown = true,
        }
    }
    result.unknown |= conditional || returns != 1;
    Ok(result)
}

fn component_receiver(edge: &Edge, options: &AnalysisOptions) -> Result<Option<String>, String> {
    let root = edge.metadata["receiver_root_class"]
        .as_str()
        .unwrap_or_default();
    if normalize(root) != "yii" {
        return Ok(None);
    }
    let expression = edge.metadata["receiver_expression"]
        .as_str()
        .unwrap_or_default();
    let tokens = syntax::fragment(expression, options)?;
    if tokens.len() == 5
        && tokens[1].text == "::"
        && tokens[2].text == "$app"
        && tokens[3].text == "->"
        && identifier(&tokens[4].text)
    {
        return Ok(Some(tokens[4].text.clone()));
    }
    Ok(None)
}

fn apply(
    edge: &mut Edge,
    targets: BTreeSet<String>,
    unknown: bool,
    operation: &str,
    evidence: &str,
) {
    edge.metadata["framework"] = json!("yii2");
    edge.metadata["framework_evidence"] = json!(evidence);
    edge.metadata["candidates"] = json!(targets.iter().collect::<Vec<_>>());
    if targets.len() == 1 && !unknown {
        edge.to = targets.into_iter().next();
        edge.metadata["resolution"] = json!("resolved");
        if operation != "call" {
            edge.kind = if operation == "write" {
                "magic_set"
            } else {
                "magic_get"
            }
            .into();
        }
    } else {
        edge.to = None;
        edge.metadata["resolution"] = json!(if targets.is_empty() {
            "unresolved"
        } else {
            "candidate"
        });
    }
}

/// Property read `$x->rel` resolved to getRel() → calls edge so outline called_by fills.
fn relation_property_call(edge: &Edge, getter: String) -> Edge {
    Edge {
        id: format!("edge:calls:yii-rel:{}:{getter}", edge.id),
        kind: "calls".into(),
        from: edge.from.clone(),
        to: Some(getter),
        span: edge.span.clone(),
        metadata: json!({
            "expression": edge.metadata["expression"],
            "conditions": edge.metadata["conditions"],
            "resolution": "resolved",
            "type_source": "framework",
            "framework": "yii2",
            "framework_evidence": "relation_property",
            "member_name": edge.metadata["member_name"],
            "operation": "call",
            "receiver_class": edge.metadata["receiver_class"],
            "receiver_expression": edge.metadata["receiver_expression"],
            "receiver_root_class": edge.metadata["receiver_root_class"],
        }),
        trust: edge.trust,
        revision: 0,
    }
}
fn method_source(method: &Node, files: &[SourceFile]) -> String {
    method_body(method, files).0
}

/// Body text (inside `{…}`) and the 1-based file line of its first character.
fn method_body(method: &Node, files: &[SourceFile]) -> (String, usize) {
    if let Some(body) = method.metadata["body"].as_str() {
        if !body.is_empty() {
            return (body.into(), method.span.start_line);
        }
    }
    let Some(file) = files.iter().find(|f| f.path == method.span.path) else {
        return (String::new(), method.span.start_line);
    };
    let mut line_starts = vec![0usize];
    for (i, b) in file.source.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let start_line = method.span.start_line.max(1);
    let end_line = method.span.end_line.max(start_line);
    let start_idx = line_starts.get(start_line - 1).copied().unwrap_or(0);
    let end_idx = line_starts
        .get(end_line)
        .copied()
        .unwrap_or(file.source.len());
    let full = &file.source[start_idx..end_idx];
    match (full.find('{'), full.rfind('}')) {
        (Some(open), Some(close)) if open < close => {
            let body_offset = start_idx + open + 1;
            let body_line = file.source[..body_offset]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1;
            (full[open + 1..close].into(), body_line)
        }
        _ => (full.to_owned(), start_line),
    }
}

fn method_body_start_line(method: &Node, files: &[SourceFile]) -> usize {
    method_body(method, files).1
}
fn diagnostic(edge: &Edge, code: &str, message: String) -> Diagnostic {
    Diagnostic {
        path: edge.span.path.clone(),
        line: edge.span.start_line,
        code: code.into(),
        message,
    }
}
fn accessor(member: &str, operation: &str) -> String {
    let mut chars = member.chars();
    format!(
        "{}{}{}",
        if operation == "write" { "set" } else { "get" },
        chars.next().unwrap().to_uppercase(),
        chars.as_str()
    )
}

/// `rules()` entries like `['attr', 'validateX']` → calls from rules() to validateX.
fn rules_validator_edges(
    index: &Index,
    files: &[SourceFile],
    graph: &GraphSnapshot,
    options: &AnalysisOptions,
) -> Result<Vec<Edge>, String> {
    options
        .progress
        .phase("analysis", "yii2_rules", Some(files.len()), "files");
    let mut out: Vec<Edge> = Vec::new();
    let rules_methods: Vec<&Node> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "method" && n.name.eq_ignore_ascii_case("rules"))
        .collect();
    for rules in rules_methods {
        options.check(graph.nodes.len() + graph.edges.len() + out.len())?;
        let Some((owner, _)) = rules.qualified_name.rsplit_once("::") else {
            continue;
        };
        let body = method_source(rules, files);
        if body.is_empty() || (!body.contains('\'') && !body.contains('"')) {
            continue;
        }
        let tokens = match syntax::fragment(&body, options) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut i = 0;
        while i < tokens.len() {
            // string validator name as array value: …, 'validateLadder', …
            if tokens[i].kind == TokenKind::String {
                let Some(name) = string_token(&tokens[i].text) else {
                    i += 1;
                    continue;
                };
                if !identifier(&name) || name.eq_ignore_ascii_case("rules") {
                    i += 1;
                    continue;
                }
                // Built-in validators / types — skip obvious non-methods.
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "required"
                        | "string"
                        | "integer"
                        | "number"
                        | "boolean"
                        | "email"
                        | "url"
                        | "date"
                        | "datetime"
                        | "time"
                        | "safe"
                        | "unique"
                        | "exist"
                        | "compare"
                        | "in"
                        | "match"
                        | "filter"
                        | "trim"
                        | "default"
                        | "captcha"
                        | "file"
                        | "image"
                        | "each"
                        | "ip"
                ) {
                    i += 1;
                    continue;
                }
                let targets = index.methods(owner, &name);
                if targets.is_empty() {
                    i += 1;
                    continue;
                }
                let line = tokens[i].line;
                // Token lines are 1-based within method body text (after `{`).
                let abs_line =
                    method_body_start_line(rules, files).saturating_add(line.saturating_sub(1));
                for target in targets {
                    let edge_id = format!("edge:calls:rules:{}:{}:{abs_line}", rules.id, target.id);
                    if out.iter().any(|e: &Edge| e.id == edge_id)
                        || graph.edges.iter().any(|e| e.id == edge_id)
                    {
                        continue;
                    }
                    out.push(Edge {
                        id: edge_id,
                        kind: "calls".into(),
                        from: rules.id.clone(),
                        to: Some(target.id.clone()),
                        span: crate::domain::SourceSpan {
                            path: rules.span.path.clone(),
                            start_line: abs_line,
                            end_line: abs_line,
                        },
                        metadata: json!({
                            "framework": "yii2",
                            "framework_evidence": "rules_validator",
                            "member_name": name,
                            "operation": "call",
                            "resolution": "resolved",
                            "expression": format!("rules → {name}"),
                            "receiver_class": owner,
                        }),
                        trust: 1,
                        revision: 0,
                    });
                }
            }
            i += 1;
        }
    }
    Ok(out)
}

fn string_token(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.len() >= 2 {
        let b = t.as_bytes();
        if (b[0] == b'\'' || b[0] == b'"') && b[0] == b[t.len() - 1] {
            return Some(t[1..t.len() - 1].to_owned());
        }
    }
    None
}
fn normalize(name: &str) -> String {
    name.trim_start_matches('\\').to_lowercase()
}
fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

const RELATION_BASES: [&str; 2] = ["yii\\db\\BaseActiveRecord", "yii\\db\\ActiveRecord"];

/// `getX()` whose only top-level statement is `return $this->hasOne|hasMany(Cls, …)…;`
/// becomes property `x` typed `Cls|null` or `Cls[]`.
fn relation_members(
    index: &Index,
    nodes: &[Node],
    files: &[SourceFile],
    options: &AnalysisOptions,
) -> Result<Vec<Node>, String> {
    let mut members = Vec::new();
    for method in nodes.iter().filter(|n| n.kind == "method") {
        options.check(0)?;
        let Some((owner, name)) = method.qualified_name.rsplit_once("::") else {
            continue;
        };
        let Some(property) = name
            .strip_prefix("get")
            .filter(|rest| rest.chars().next().is_some_and(char::is_uppercase))
            .map(lower_first)
        else {
            continue;
        };
        if !RELATION_BASES.iter().any(|base| index.is_a(owner, base)) {
            continue;
        }
        let Some((relation, class)) = relation_target(method, files, options)? else {
            continue;
        };
        let type_name = if relation == "hasOne" {
            format!("{class}|null")
        } else {
            format!("{class}[]")
        };
        members.push(member(
            owner,
            &property,
            method.span.clone(),
            json!({
                "type": type_name,
                "framework_evidence": "relation",
                "relation": relation,
                "implemented_by": method.id,
            }),
        ));
        // Instance call $model->getX() yields ActiveQuery<Related> so ->one()/all() type as the model.
        let mut getter = member(
            owner,
            name,
            method.span.clone(),
            json!({
                "framework_return_type": format!("{ACTIVE_QUERY}<{class}>"),
                "exact_return": true,
                "framework_evidence": "relation",
                "implemented_by": method.id,
            }),
        );
        getter.kind = "method".into();
        getter.id = method.id.clone();
        getter.name = name.to_owned();
        getter.qualified_name = format!("{owner}::{name}");
        members.push(getter);
    }
    Ok(members)
}

fn relation_target(
    method: &Node,
    files: &[SourceFile],
    options: &AnalysisOptions,
) -> Result<Option<(&'static str, String)>, String> {
    let body = method_source(method, files);
    if !body.contains("hasOne") && !body.contains("hasMany") {
        return Ok(None);
    }
    let tokens = syntax::fragment(&body, options)?;
    // Allow simple guards: `if (…) return $this->hasOne(…); return null;`
    // Reject nested functions / loops / switch / match / try.
    if tokens.iter().any(|t| {
        t.kind == TokenKind::Ident
            && matches!(
                t.text.to_ascii_lowercase().as_str(),
                "switch" | "match" | "try" | "for" | "foreach" | "while" | "function" | "fn"
            )
    }) {
        return Ok(None);
    }
    let returns: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.is("return"))
        .map(|(at, _)| at)
        .collect();
    // Accept one consistent `return $this->hasOne|hasMany(Cls, …)`; other returns may be null/false.
    let mut chosen: Option<(&'static str, String)> = None;
    for at in returns {
        let is = |offset: usize, text: &str| tokens.get(at + offset).is_some_and(|t| t.is(text));
        // `return null;` / `return false;` / bare `return;` — ignore
        if is(1, "null") || is(1, "false") || tokens.get(at + 1).is_some_and(|t| t.text == ";") {
            continue;
        }
        if !is(1, "$this") || !is(2, "->") || !is(4, "(") {
            // Any other return shape makes the relation non-static.
            return Ok(None);
        }
        let relation = match tokens[at + 3].text.to_ascii_lowercase().as_str() {
            "hasone" => "hasOne",
            "hasmany" => "hasMany",
            _ => return Ok(None),
        };
        let start = at + 5;
        let end = expression_end(&tokens, start);
        let names = Names::metadata(&method.metadata);
        let class = match syntax::expression(&tokens[start..end], &names, options)? {
            Expression::Class(name) | Expression::Text(name) => {
                syntax::configured_class(&Expression::Class(name))
                    .names
                    .into_iter()
                    .next()
            }
            _ => None,
        };
        let Some(class) = class else {
            return Ok(None);
        };
        match &chosen {
            None => chosen = Some((relation, class)),
            Some((rel, cls)) if *rel == relation && cls == &class => {}
            Some(_) => return Ok(None), // conflicting targets (e.g. MaxWin vs UserBonus)
        }
    }
    Ok(chosen)
}

/// Configured application components become properties of `yii\base\Application`
/// when every configuration of the component is statically established.
fn component_members(
    index: &Index,
    files: &[SourceFile],
    options: &AnalysisOptions,
) -> Result<(Vec<Node>, Vec<Diagnostic>), String> {
    let Some(application) = index.classes.get(&normalize("yii\\base\\Application")) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let area_definitions = components(files, options)?;
    let definitions = &area_definitions.all;
    if definitions.unknown {
        let (path, line) = definitions.unknown_at.clone().unwrap_or_default();
        return Ok((
            Vec::new(),
            vec![Diagnostic {
                path,
                line,
                code: "dynamic_component_map".into(),
                message: "Yii components are set at runtime here, so application components are not typed anywhere; exclude this file with ATLAS_EXCLUDE if it is not an application config".into(),
            }],
        ));
    }
    let mut members = Vec::new();
    for (name, classes) in &definitions.classes {
        if classes.unknown || classes.names.is_empty() || !identifier(name) {
            continue;
        }
        let (path, line) = definitions.sources.get(name).cloned().unwrap_or_default();
        let union = classes.names.iter().cloned().collect::<Vec<_>>().join("|");
        // Per application: the class its own configs (plus shared ones) give the component.
        let by_area: serde_json::Map<String, Value> = area_definitions
            .by_area
            .iter()
            .filter_map(|(area, own)| {
                let own = own.classes.get(name)?;
                (!own.unknown && !own.names.is_empty()).then(|| {
                    (
                        area.clone(),
                        json!(own.names.iter().cloned().collect::<Vec<_>>().join("|")),
                    )
                })
            })
            .filter(|(_, ty)| ty.as_str() != Some(union.as_str()))
            .collect();
        members.push(member(
            &application.qualified_name,
            name,
            crate::domain::SourceSpan {
                path,
                start_line: line,
                end_line: line,
            },
            json!({
                "type": union,
                "types_by_area": by_area,
                "framework_evidence": "component",
            }),
        ));
    }
    Ok((members, Vec::new()))
}

fn member(owner: &str, name: &str, span: crate::domain::SourceSpan, extra: Value) -> Node {
    let mut metadata = json!({
        "type_source": "framework",
        "framework": "yii2",
        "magic": true,
        "access": "read",
    });
    if let (Some(target), Some(extra)) = (metadata.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
    Node {
        id: format!("property:{owner}::${name}"),
        kind: "property".into(),
        name: format!("${name}"),
        qualified_name: format!("{owner}::${name}"),
        span,
        metadata,
        trust: 1,
        revision: 0,
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

const QUEUE: &str = "yii\\queue\\Queue";

/// `…->push($job)` resolved to a `yii\queue\Queue` method, whose first argument has
/// a class type with `execute()`, dispatches to that `execute()` from the call site.
fn queue_dispatches(index: &Index, edges: &[Edge]) -> Vec<Edge> {
    let mut found = Vec::new();
    for call in edges.iter().filter(|e| e.kind == "calls") {
        // Several queue drivers (candidates) all end up executing the same job.
        let targets: Vec<&str> = match call.to.as_deref() {
            Some(target) => vec![target],
            None => call.metadata["candidates"]
                .as_array()
                .map(|c| c.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default(),
        };
        let queue_push = |target: &&str| {
            target
                .strip_prefix("method:")
                .and_then(|q| q.rsplit_once("::"))
                .is_some_and(|(owner, method)| {
                    method.eq_ignore_ascii_case("push") && index.is_a(owner, QUEUE)
                })
        };
        if targets.is_empty() || !targets.iter().all(queue_push) {
            continue;
        }
        let argument = &call.metadata["arguments"][0];
        let job_type = argument["type"].as_str().unwrap_or("unknown");
        // Provenance comes from type_source: a candidate call can be annotation-based too.
        let annotated =
            call.metadata["type_source"] == "annotation" || argument["type_source"] == "annotation";
        for job in job_classes(job_type) {
            for execute in index.methods(&job, "execute") {
                found.push(dispatch(call, &execute.id, &job, annotated));
            }
        }
    }
    found
}

fn job_classes(ty: &str) -> Vec<String> {
    ty.split('|')
        .map(|part| part.split('<').next().unwrap_or(part).trim())
        .filter(|part| part.contains('\\') || part.chars().next().is_some_and(char::is_uppercase))
        .filter(|part| !part.ends_with("[]"))
        .map(str::to_owned)
        .collect()
}

fn dispatch(call: &Edge, execute: &str, job: &str, annotated: bool) -> Edge {
    use crate::plugins::php::tokens::fingerprint;
    let queue = call.metadata["receiver_expression"]
        .as_str()
        .and_then(queue_component);
    Edge {
        id: format!(
            "edge:{}",
            fingerprint(["dispatches", call.from.as_str(), execute, call.id.as_str()])
        ),
        kind: "dispatches".into(),
        from: call.from.clone(),
        to: Some(execute.to_owned()),
        span: call.span.clone(),
        metadata: json!({
            "fingerprint": fingerprint([
                "dispatches",
                call.metadata["fingerprint"].as_str().unwrap_or(""),
                execute,
            ]),
            "framework": "yii2",
            "framework_evidence": "queue",
            "queue": queue,
            "job_class": job,
            "resolution": if annotated { "annotated" } else { "resolved" },
            "type_source": if annotated { "annotation" } else { "code" },
            "conditions": call.metadata["conditions"],
        }),
        trust: 1,
        revision: 0,
    }
}

/// `Yii::$app->queueGame->delay(5)` → `queueGame`.
fn queue_component(receiver: &str) -> Option<String> {
    let after = &receiver[receiver.find("$app->")? + "$app->".len()..];
    let name: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

const ACTIVE_QUERY: &str = "yii\\db\\ActiveQuery";

/// Yii guarantees that `X::find()` returns the query class declared by `find()` of the
/// model (or its ancestors), bound to `X`: a return refinement `Q<X>` that is exact, so
/// calls on the query do not fan out to other models' query classes.
fn query_members(index: &Index, nodes: &[Node], options: &AnalysisOptions) -> Vec<Node> {
    nodes
        .iter()
        .filter(|n| n.kind == "class")
        .filter(|n| !options.declaration_only(&n.span.path))
        .filter_map(|model| {
            let known_ar = RELATION_BASES
                .iter()
                .any(|base| index.is_a(&model.qualified_name, base));
            let unindexed_parent = index.parent_unindexed(&model.qualified_name);
            // Proven ActiveRecord, or extends a vendor parent outside declaration_paths
            // (e.g. common\models\User → Da\User\Model\User while only yiisoft is indexed).
            if !known_ar && !unindexed_parent {
                return None;
            }
            let find = index
                .methods(&model.qualified_name, "find")
                .into_iter()
                .next()
                .or_else(|| {
                    index
                        .methods("yii\\db\\ActiveRecord", "find")
                        .into_iter()
                        .next()
                })?;
            let evidence = if known_ar {
                "active_query"
            } else {
                "active_query_unindexed_parent"
            };
            let query = query_class(index, find).unwrap_or_else(|| ACTIVE_QUERY.to_owned());
            let mut member = member(
                &model.qualified_name,
                "find",
                model.span.clone(),
                json!({
                    "framework_return_type": format!("{query}<{}>", model.qualified_name),
                    "exact_return": true,
                    "framework_evidence": evidence,
                    "implemented_by": find.id,
                }),
            );
            member.kind = "method".into();
            member.id = format!("method:{}::find", model.qualified_name);
            member.name = "find".into();
            member.qualified_name = format!("{}::find", model.qualified_name);
            Some(member)
        })
        .collect()
}

/// The query class named by `find()`'s declared or annotated return type.
fn query_class(index: &Index, find: &Node) -> Option<String> {
    ["declared_return_type", "annotated_return_type"]
        .iter()
        .filter_map(|key| find.metadata[*key].as_str())
        .flat_map(|ty| ty.split('|'))
        .map(|part| part.split('<').next().unwrap_or(part).trim())
        .find(|part| index.is_a(part, ACTIVE_QUERY))
        .map(str::to_owned)
}

/// The call's only magic dispatcher is `yii\base\Component::__call` (behaviors).
fn only_component_magic(edge: &Edge) -> bool {
    edge.metadata["dispatcher"]
        .as_str()
        .is_some_and(|d| d.eq_ignore_ascii_case("method:yii\\base\\Component::__call"))
}
