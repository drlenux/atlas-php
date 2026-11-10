//! Native, bounded PHP structural analysis. Source is data, never executable input.
mod builtins;
pub mod doc;
mod expression;
mod flow;
mod syntax;
pub mod tokens;
use crate::{
    analyzer::{incremental, AnalysisOptions, LanguagePlugin, MemberSource, SourceFile},
    domain::{Diagnostic, Edge, GraphSnapshot, Node},
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use syntax::{Class, Function, Unit};

pub struct PhpLanguagePlugin;
impl LanguagePlugin for PhpLanguagePlugin {
    fn id(&self) -> &'static str {
        "php"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["php"]
    }
    fn analyze(
        &self,
        files: &[SourceFile],
        options: &AnalysisOptions,
    ) -> Result<GraphSnapshot, String> {
        analyze_files(files, options, None)
    }
    fn analyze_with_members(
        &self,
        files: &[SourceFile],
        options: &AnalysisOptions,
        members: &dyn MemberSource,
    ) -> Result<GraphSnapshot, String> {
        analyze_files(files, options, Some(members))
    }
}

fn analyze_files(
    files: &[SourceFile],
    options: &AnalysisOptions,
    members: Option<&dyn MemberSource>,
) -> Result<GraphSnapshot, String> {
    let target = syntax::target_version(
        options
            .language_versions
            .get("php")
            .map(String::as_str)
            .unwrap_or("8.2"),
    )?;
    let mut project = Project::new(options);
    options
        .progress
        .phase("analysis", "php_parse", Some(files.len()), "files");
    options
        .progress
        .set_workers(options.workers.min(files.len()));
    let mut parsed: Vec<_> = files.iter().map(|file| (file, None)).collect();
    crate::parallel::try_for_each_mut(
        &mut parsed,
        options.workers,
        || (),
        |_, _, (file, slot)| {
            options.progress.work_started(&file.path);
            let result = parse_file(file, options, target);
            options.progress.work_finished(&file.path);
            if let Err(error) = &result {
                if error.message.contains("time limit")
                    || error.message.contains("graph element limit")
                {
                    return Err(error.message.clone());
                }
            }
            *slot = Some(result);
            Ok(())
        },
    )?;
    // Merge in source order, including deterministic handling of duplicate symbols.
    for (_, result) in parsed {
        match result.expect("successful worker filled its result slot") {
            Ok((unit, mut declarations)) => {
                let unit_index = project.units.len();
                for class in declarations.classes.values_mut() {
                    class.unit = unit_index;
                }
                for function in declarations.functions.values_mut() {
                    function.unit = unit_index;
                }
                project.units.push(unit);
                project.classes.extend(declarations.classes);
                project.functions.extend(declarations.functions);
                project.closures.extend(
                    declarations
                        .closures
                        .into_iter()
                        .map(|((_, token), id)| ((unit_index, token), id)),
                );
            }
            Err(diagnostic) => project.diagnostics.push(diagnostic),
        }
    }
    options
        .progress
        .phase("analysis", "php_graph", None, "items");
    project.apply_areas();
    project.index = ClassIndex::build(&project.classes);
    project.declarations()?;
    project.relations()?;
    if let Some(source) = members {
        let declarations = GraphSnapshot {
            nodes: project.nodes.values().cloned().collect(),
            edges: project.edges.values().cloned().collect(),
            diagnostics: Vec::new(),
        };
        project.merge_members(source.members(&declarations)?)?;
    }
    // Incremental rebuild: bodies only in changed files and their dependents.
    let scope = options.incremental.as_ref().and_then(|context| {
        let declarations: Vec<Node> = project.nodes.values().cloned().collect();
        let current = incremental::declaration_surface(&declarations);
        let scope = incremental::incremental_scope(&incremental::ScopeInput {
            changed: &context.changed,
            removed: &context.removed,
            previous: &context.previous_surface,
            current: &current,
            files,
            nodes: &declarations,
        });
        context.record(scope.clone());
        scope
    });
    // Library declarations feed types and targets; their bodies are not analyzed.
    // Closures inside functions are analyzed by their enclosing function, which
    // knows the types of captured variables.
    let functions: Vec<_> = project
        .functions
        .values()
        .filter(|f| !options.declaration_only(&project.units[f.unit].file.path))
        .filter(|f| !nested_closure(f))
        .filter(|f| {
            scope
                .as_ref()
                .is_none_or(|paths| paths.contains(&project.units[f.unit].file.path))
        })
        .cloned()
        .collect();
    // Two passes over the same functions: total is 2× so the bar stays honest.
    options.progress.phase(
        "analysis",
        "php_flow",
        Some(functions.len().saturating_mul(2)),
        "functions",
    );
    let workers = options.workers.min(functions.len().max(1)).max(1);
    options
        .progress
        .set_workers(workers.min(functions.len().max(1)));
    let base_nodes = std::mem::take(&mut project.nodes);
    let base_edges = std::mem::take(&mut project.edges);
    // Pass 1: collect body returns. Pass 2: callers see inferred_return of callees.
    let pass1 = run_flow_pass(
        &functions,
        &base_nodes,
        base_edges.len(),
        &project,
        options,
        workers,
    )?;
    for (position, outcome) in pass1.into_iter().enumerate() {
        let function = &functions[position];
        if let Ok((nodes, _, _)) = outcome {
            if let Some(inferred) = inferred_return(function, nodes.get(&function.id)) {
                if let Some(f) = project.functions.get_mut(&function.id) {
                    f.inferred_return = Some(inferred);
                }
            }
        }
    }
    let pass2 = run_flow_pass(
        &functions,
        &base_nodes,
        base_edges.len(),
        &project,
        options,
        workers,
    )?;
    project.nodes = base_nodes;
    project.edges = base_edges;
    for (position, outcome) in pass2.into_iter().enumerate() {
        let function = &functions[position];
        match outcome {
            Ok((nodes, edges, diagnostics)) => {
                project.nodes.extend(nodes);
                project.edges.extend(edges);
                project.diagnostics.extend(diagnostics);
            }
            Err(error) => {
                // Keep file declarations: one bad method body must not drop the class.
                let path = project.units[function.unit].file.path.clone();
                project.diagnostics.push(Diagnostic {
                    path,
                    line: project.units[function.unit]
                        .span(function.start, function.end)
                        .start_line,
                    code: "analysis_limitation".into(),
                    message: error,
                });
            }
        }
    }
    options
        .progress
        .phase("analysis", "php_finalize", None, "items");
    project.origins();
    options.check(project.nodes.len() + project.edges.len())?;
    let mut graph = GraphSnapshot {
        nodes: project.nodes.into_values().collect(),
        edges: project.edges.into_values().collect(),
        diagnostics: project.diagnostics,
    };
    crate::analyzer::sync_occurrence_resolution(&mut graph);
    Ok(graph)
}
type FlowDelta = (
    BTreeMap<String, Node>,
    BTreeMap<String, Edge>,
    Vec<Diagnostic>,
);

fn run_flow_pass(
    functions: &[Function],
    base_nodes: &BTreeMap<String, Node>,
    relation_edge_count: usize,
    project: &Project<'_>,
    options: &AnalysisOptions,
    workers: usize,
) -> Result<Vec<Result<FlowDelta, String>>, String> {
    let mut flow_slots: Vec<Option<Result<FlowDelta, String>>> =
        functions.iter().map(|_| None).collect();
    crate::parallel::try_for_each_mut(
        &mut flow_slots,
        workers,
        || (),
        |_, position, slot| {
            let function = &functions[position];
            let path = project.units[function.unit].file.path.clone();
            options.progress.work_started(&path);
            let mut host = FlowHost {
                options,
                units: &project.units,
                classes: &project.classes,
                index: &project.index,
                functions: &project.functions,
                closures: &project.closures,
                nodes: NodesMap {
                    base: base_nodes,
                    local: BTreeMap::new(),
                },
                edges: BTreeMap::new(),
                diagnostics: Vec::new(),
                relation_edge_count,
            };
            let outcome = match flow::analyze(&mut host, function) {
                Ok(()) => Ok((host.nodes.local, host.edges, host.diagnostics)),
                Err(error)
                    if error.contains("time limit") || error.contains("graph element limit") =>
                {
                    options.progress.work_finished(&path);
                    return Err(error);
                }
                Err(error) => Err(error),
            };
            options.progress.work_finished(&path);
            *slot = Some(outcome);
            Ok(())
        },
    )?;
    Ok(flow_slots
        .into_iter()
        .map(|slot| slot.expect("flow worker filled its result slot"))
        .collect())
}

/// When declared/`@return` say nothing useful, take informative types from body returns.
fn inferred_return(function: &Function, node: Option<&Node>) -> Option<String> {
    let (declared, _) = doc::refine(&function.return_type, function.doc_return.as_deref());
    if doc::informative(&declared) {
        return None;
    }
    let types: Vec<&str> = node
        .and_then(|n| n.metadata["returns"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|r| r["type"].as_str())
        .filter(|ty| doc::informative(ty))
        .collect();
    if types.is_empty() {
        None
    } else {
        let mut parts = Vec::new();
        for ty in types {
            for part in ty.split('|') {
                if !part.is_empty() && !parts.contains(&part) {
                    parts.push(part);
                }
            }
        }
        (!parts.is_empty()).then(|| parts.join("|"))
    }
}

fn parse_file(
    file: &SourceFile,
    options: &AnalysisOptions,
    target: (u32, u32),
) -> Result<(Unit, syntax::Declarations), Diagnostic> {
    if file.path.contains('\0')
        || std::path::Path::new(&file.path).is_absolute()
        || std::path::Path::new(&file.path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Diagnostic {
            path: file.path.clone(),
            line: 1,
            code: "path_outside_root".into(),
            message: "source path must be relative and contained".into(),
        });
    }
    (|| {
        options.check(0)?;
        let (tokens, comments) = tokens::lex(&file.source, options)?;
        let pairs = tokens::matching(&tokens)?;
        let unit = Unit {
            file: file.clone(),
            tokens,
            pairs,
            comments,
        };
        let declarations = syntax::parse(0, &unit, options)?;
        syntax::check_version(&unit, &declarations, target)?;
        Ok::<_, String>((unit, declarations))
    })()
    .map_err(|error| Diagnostic {
        path: file.path.clone(),
        line: 1,
        code: if error.contains("unsupported") || error.contains("not supported") {
            "unsupported_syntax".into()
        } else {
            "parse_error".into()
        },
        message: error,
    })
}

pub(super) struct NodesMap<'a> {
    base: &'a BTreeMap<String, Node>,
    local: BTreeMap<String, Node>,
}
impl NodesMap<'_> {
    pub(super) fn contains_key(&self, id: &str) -> bool {
        self.local.contains_key(id) || self.base.contains_key(id)
    }
    pub(super) fn get_mut(&mut self, id: &str) -> Option<&mut Node> {
        if !self.local.contains_key(id) {
            let node = self.base.get(id)?.clone();
            self.local.insert(id.to_owned(), node);
        }
        self.local.get_mut(id)
    }
    pub(super) fn insert(&mut self, id: String, node: Node) -> Option<Node> {
        self.local.insert(id, node)
    }
    pub(super) fn len(&self) -> usize {
        self.base.len()
            + self
                .local
                .keys()
                .filter(|id| !self.base.contains_key(*id))
                .count()
    }
}
impl std::ops::Index<&str> for NodesMap<'_> {
    type Output = Node;
    fn index(&self, id: &str) -> &Node {
        self.local
            .get(id)
            .or_else(|| self.base.get(id))
            .unwrap_or_else(|| panic!("missing node {id}"))
    }
}
impl std::ops::Index<&String> for NodesMap<'_> {
    type Output = Node;
    fn index(&self, id: &String) -> &Node {
        &self[id.as_str()]
    }
}

/// Per-function flow workspace: shared declaration graph is read-only; writes stay local.
pub(super) struct FlowHost<'a> {
    pub(super) options: &'a AnalysisOptions,
    pub(super) units: &'a [Unit],
    pub(super) classes: &'a BTreeMap<String, Class>,
    pub(super) index: &'a ClassIndex,
    pub(super) functions: &'a BTreeMap<String, Function>,
    pub(super) closures: &'a BTreeMap<(usize, usize), String>,
    pub(super) nodes: NodesMap<'a>,
    pub(super) edges: BTreeMap<String, Edge>,
    pub(super) diagnostics: Vec<Diagnostic>,
    relation_edge_count: usize,
}
impl FlowHost<'_> {
    pub(super) fn elements(&self) -> usize {
        self.nodes.len() + self.relation_edge_count + self.edges.len()
    }
    pub(super) fn lookup_method(
        &self,
        class: &str,
        name: &str,
        seen: &mut BTreeSet<String>,
    ) -> Option<String> {
        lookup_method(self.classes, self.index, class, name, seen)
    }
    pub(super) fn lookup_constant(&self, class: &str, name: &str) -> Option<String> {
        lookup_constant(self.classes, self.index, class, name, &mut BTreeSet::new())
    }
    pub(super) fn lookup_property(
        &self,
        class: &str,
        name: &str,
        seen: &mut BTreeSet<String>,
    ) -> Option<String> {
        lookup_property(self.classes, self.index, class, name, seen)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn edge(
        &mut self,
        kind: &str,
        from: &str,
        to: Option<String>,
        unit: usize,
        start: usize,
        end: usize,
        mut metadata: Value,
        occurrence: &str,
    ) -> Result<(), String> {
        self.options.check(self.elements() + 1)?;
        let id = format!(
            "edge:{}",
            tokens::fingerprint([kind, from, to.as_deref().unwrap_or(""), occurrence])
        );
        let u = &self.units[unit];
        metadata["fingerprint"] = json!(tokens::fingerprint([
            kind,
            &u.fp(start, end, None),
            occurrence
        ]));
        if metadata.get("resolution").is_none() {
            metadata["resolution"] = json!(if to.is_some() {
                "resolved"
            } else {
                "unresolved"
            });
        }
        self.edges.insert(
            id.clone(),
            Edge {
                id,
                kind: kind.into(),
                from: from.into(),
                to,
                span: u.span(start, end),
                metadata,
                trust: 1,
                revision: 0,
            },
        );
        Ok(())
    }
    pub(super) fn diagnostic(&mut self, unit: usize, token: usize, code: &str, message: String) {
        let u = &self.units[unit];
        self.diagnostics.push(Diagnostic {
            path: u.file.path.clone(),
            line: u.tokens.get(token).map_or(1, |t| t.line),
            code: code.into(),
            message,
        });
    }
}

pub(super) struct Project<'a> {
    options: &'a AnalysisOptions,
    units: Vec<Unit>,
    classes: BTreeMap<String, Class>,
    index: ClassIndex,
    functions: BTreeMap<String, Function>,
    closures: BTreeMap<(usize, usize), String>,
    nodes: BTreeMap<String, Node>,
    edges: BTreeMap<String, Edge>,
    diagnostics: Vec<Diagnostic>,
}
impl<'a> Project<'a> {
    fn new(options: &'a AnalysisOptions) -> Self {
        Self {
            options,
            units: Vec::new(),
            classes: BTreeMap::new(),
            index: ClassIndex::default(),
            functions: BTreeMap::new(),
            closures: BTreeMap::new(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }
    fn declarations(&mut self) -> Result<(), String> {
        for class in self.classes.values() {
            self.options.check(self.nodes.len() + self.edges.len())?;
            let u = &self.units[class.unit];
            let metadata = json!({"fingerprint":u.fp(class.start,class.end,Some(class.name_token)),"parent":class.parent,"interfaces":class.interfaces,"traits":class.traits,"namespace":class.names.namespace,"imports":class.names.imports,"commented":false});
            self.nodes.insert(
                class.id.clone(),
                Node {
                    id: class.id.clone(),
                    kind: class.kind.clone(),
                    name: class.name.clone(),
                    qualified_name: class.qualified.clone(),
                    span: u.span(class.start, class.end),
                    metadata,
                    trust: 1,
                    revision: 0,
                },
            );
            for property in &class.properties {
                let id = format!("property:{}::${}", class.qualified, property.name);
                self.nodes
                    .insert(id.clone(), property_node(&id, class, property, u));
            }
            for constant in &class.constants {
                let id = format!("constant:{}::{}", class.qualified, constant.name);
                self.nodes.insert(
                    id.clone(),
                    Node {
                        id,
                        kind: "constant".into(),
                        name: constant.name.clone(),
                        qualified_name: format!("{}::{}", class.qualified, constant.name),
                        span: u.span(constant.token, constant.token + 1),
                        metadata: json!({
                            "fingerprint": u.fp(constant.token, constant.token + 1, Some(constant.token)),
                            "value": constant.value,
                            "commented": false
                        }),
                        trust: 1,
                        revision: 0,
                    },
                );
            }
        }
        for f in self.functions.values() {
            self.options.check(self.nodes.len() + self.edges.len())?;
            let u = &self.units[f.unit];
            self.nodes.insert(f.id.clone(),Node{id:f.id.clone(),kind:if f.id.starts_with("method:"){"method"}else{"function"}.into(),name:f.name.clone(),qualified_name:f.qualified.clone(),span:u.span(f.start,f.end),metadata:json!({"fingerprint":u.fp(f.start,f.end,f.name_token),"returns":[],"implicit_return":false,"closure":f.closure,"declared_return_type":f.return_type,"parameters":f.parameters.iter().map(|p|json!({"name":p.name,"type":p.type_name,"type_source":type_source(p.annotated)})).collect::<Vec<_>>(),"annotated_return_type":f.doc_return,"namespace":f.names.namespace,"imports":f.names.imports,"commented":false}),trust:1,revision:0});
            if f.magic {
                let node = self.nodes.get_mut(&f.id).expect("inserted above");
                node.metadata["magic"] = json!(true);
                node.metadata["type_source"] = json!("annotation");
                node.metadata["fingerprint"] = json!(tokens::fingerprint([
                    "magic-method",
                    f.qualified.as_str(),
                    f.doc_return.as_deref().unwrap_or(""),
                ]));
            }
        }
        Ok(())
    }
    /// Namespace-less declarations of an area are named `<area>:<Name>`. Rewrites the global
    /// names stored while parsing (parents, traits, types) to those qualified names (globals
    /// outside every area first, then own area, then shared areas) and gives every
    /// declaration's `Names` the table of its area.
    fn apply_areas(&mut self) {
        let areas = &self.options.areas;
        if areas.is_empty() {
            return;
        }
        let mut declared: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for qualified in self.classes.keys().chain(
            self.functions
                .values()
                .filter(|f| f.class.is_none())
                .map(|f| &f.qualified),
        ) {
            if let Some((area, name)) = qualified.split_once(':') {
                if !name.contains('\\') && !name.contains("::") {
                    declared
                        .entry(area.to_owned())
                        .or_default()
                        .insert(name.to_ascii_lowercase(), qualified.clone());
                }
            }
        }
        // Globals outside every area (libraries) exist at runtime for every application;
        // a same-named class of a shared area (e.g. an IDE stub) must not shadow them.
        let plain: BTreeSet<String> = self
            .classes
            .keys()
            .chain(
                self.functions
                    .values()
                    .filter(|f| f.class.is_none())
                    .map(|f| &f.qualified),
            )
            .filter(|name| !name.contains(':') && !name.contains('\\'))
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let shared: BTreeMap<String, String> = areas
            .iter()
            .filter(|area| area.shared)
            .filter_map(|area| declared.get(&area.prefix))
            .flat_map(|names| names.clone())
            .filter(|(name, _)| !plain.contains(name))
            .collect();
        let table = |area: Option<&str>| -> std::sync::Arc<BTreeMap<String, String>> {
            let mut visible = shared.clone();
            if let Some(own) = area.and_then(|area| declared.get(area)) {
                // Two globals of one name cannot coexist at runtime: an area copy of a library
                // class is a stub or dead duplicate, so the library class keeps its name.
                visible.extend(
                    own.iter()
                        .filter(|(name, _)| !plain.contains(*name))
                        .map(|(name, qualified)| (name.clone(), qualified.clone())),
                );
            }
            std::sync::Arc::new(visible)
        };
        let mut tables: BTreeMap<Option<String>, std::sync::Arc<BTreeMap<String, String>>> =
            BTreeMap::new();
        let mut table_of = |path: &str| {
            let area = crate::domain::area_of(areas, path).map(|a| a.prefix.clone());
            tables
                .entry(area.clone())
                .or_insert_with(|| table(area.as_deref()))
                .clone()
        };
        let units = &self.units;
        for class in self.classes.values_mut() {
            let globals = table_of(&units[class.unit].file.path);
            let map = |ty: &str| remap_globals(ty, &globals);
            class.parent = class.parent.as_deref().map(map);
            class.interfaces = class.interfaces.iter().map(|t| map(t)).collect();
            class.traits = class.traits.iter().map(|t| map(t)).collect();
            for (source, _) in class.aliases.values_mut() {
                *source = source.as_deref().map(map);
            }
            for property in &mut class.properties {
                property.type_name = map(&property.type_name);
            }
            for (ty, _) in class.framework_returns.values_mut() {
                *ty = map(ty);
            }
            class.names.globals = Some(globals);
        }
        for function in self.functions.values_mut() {
            let globals = table_of(&units[function.unit].file.path);
            let map = |ty: &str| remap_globals(ty, &globals);
            for parameter in &mut function.parameters {
                parameter.type_name = map(&parameter.type_name);
            }
            function.return_type = map(&function.return_type);
            function.doc_return = function.doc_return.as_deref().map(map);
            function.names.globals = Some(globals);
        }
    }
    fn relations(&mut self) -> Result<(), String> {
        for f in self.functions.values().cloned().collect::<Vec<_>>() {
            if let Some(owner) = &f.owner {
                self.edge(
                    "contains",
                    owner,
                    Some(f.id.clone()),
                    f.unit,
                    f.start,
                    f.end,
                    json!({}),
                    "",
                )?;
            }
        }
        for c in self.classes.values().cloned().collect::<Vec<_>>() {
            for (kind, targets) in [
                ("inherits", c.parent.iter().cloned().collect::<Vec<_>>()),
                ("implements", c.interfaces.clone()),
                ("uses_trait", c.traits.clone()),
            ] {
                for target in targets {
                    let id = self.classes.get(&target).map(|c| c.id.clone());
                    self.edge(
                        kind,
                        &c.id,
                        id.clone(),
                        c.unit,
                        c.start,
                        c.end,
                        json!({"symbol":target}),
                        &target,
                    )?;
                    if id.is_none() && !self.options.declaration_only(&self.units[c.unit].file.path)
                    {
                        self.diagnostic(
                            c.unit,
                            c.start,
                            "unresolved_symbol",
                            format!("Definition not indexed: {target}"),
                        );
                    }
                }
            }
            for constant in &c.constants {
                let const_id = format!("constant:{}::{}", c.qualified, constant.name);
                self.edge(
                    "contains",
                    &c.id,
                    Some(const_id.clone()),
                    c.unit,
                    constant.token,
                    constant.token + 1,
                    json!({}),
                    "",
                )?;
                self.constant_value_reads(&c, &const_id, constant)?;
            }
            for prop in &c.properties {
                self.edge(
                    "contains",
                    &c.id,
                    Some(format!("property:{}::${}", c.qualified, prop.name)),
                    c.unit,
                    prop.token,
                    prop.token + 1,
                    json!({}),
                    "",
                )?;
            }
            for (name, target) in self.all_methods(&c.qualified, &mut BTreeSet::new()) {
                // Inherited library methods stay resolvable through lookup; materializing
                // them would add an edge per model per framework method.
                let library = self
                    .functions
                    .get(&target)
                    .is_some_and(|f| self.options.declaration_only(&self.units[f.unit].file.path));
                if !c.methods.contains_key(&name) && !library {
                    self.edge(
                        "available_via",
                        &c.id,
                        Some(target),
                        c.unit,
                        c.start,
                        c.end,
                        json!({"accessible_as":name}),
                        &name,
                    )?;
                }
            }
            for (name, method) in &c.methods {
                // Interfaces too: an implementation overrides the interface method, so
                // `overridden_by` of an interface method lists its implementations.
                for base in c.parent.iter().chain(&c.traits).chain(&c.interfaces) {
                    if let Some(target) = self.lookup_method(base, name, &mut BTreeSet::new()) {
                        let f = self.functions[method].clone();
                        self.edge(
                            "overrides",
                            method,
                            Some(target),
                            f.unit,
                            f.start,
                            f.end,
                            json!({}),
                            "",
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
    /// `const MAP = [Keys::X => Foo::class, …]`: read_site for Keys::X; class_string_values for Foo.
    fn constant_value_reads(
        &mut self,
        owner: &Class,
        const_id: &str,
        constant: &syntax::Constant,
    ) -> Result<(), String> {
        let Some((start, end)) = constant.value_range else {
            return Ok(());
        };
        let mut refs = Vec::new();
        let mut class_strings = Vec::new();
        {
            let unit = &self.units[owner.unit];
            // Walk every token; do not jump `pairs` (that would skip array bodies).
            let mut i = start;
            while i + 2 < end {
                if unit.tokens[i].kind == tokens::TokenKind::Ident
                    && unit.is(i + 1, "::")
                    && unit.tokens[i + 2].kind == tokens::TokenKind::Ident
                {
                    let member = unit.tokens[i + 2].text.clone();
                    let class = owner.names.resolve(&unit.tokens[i].text);
                    let expression = unit.text(i, i + 3);
                    if member.eq_ignore_ascii_case("class") {
                        if !class_strings.contains(&class) {
                            class_strings.push(class);
                        }
                    } else if let Some(target) = lookup_constant(
                        &self.classes,
                        &self.index,
                        &class,
                        &member,
                        &mut BTreeSet::new(),
                    ) {
                        refs.push((target, class, member, expression, i));
                    }
                    i += 3;
                    continue;
                }
                i += 1;
            }
        }
        if !class_strings.is_empty() {
            if let Some(node) = self.nodes.get_mut(const_id) {
                node.metadata["class_string_values"] = json!(class_strings);
                let element = class_strings
                    .iter()
                    .map(|c| format!("class-string<{c}>[]"))
                    .collect::<Vec<_>>()
                    .join("|");
                node.metadata["type"] = json!(element);
            }
        }
        for (target, class, member, expression, i) in refs {
            let fp = self.units[owner.unit].fp(i, i + 3, None);
            let site_id = format!("read_site:{const_id}:{fp}");
            if !self.nodes.contains_key(&site_id) {
                self.options
                    .check(self.nodes.len() + self.edges.len() + 2)?;
                let span = self.units[owner.unit].span(i, i + 3);
                self.nodes.insert(
                    site_id.clone(),
                    Node {
                        id: site_id.clone(),
                        kind: "read_site".into(),
                        name: expression.chars().take(80).collect(),
                        qualified_name: site_id.clone(),
                        span,
                        metadata: json!({
                            "fingerprint": fp,
                            "expression": expression,
                            "conditions": [],
                            "commented": false,
                            "member_name": member,
                            "operation": "constant",
                            "receiver_class": class,
                            "resolution": "resolved",
                        }),
                        trust: 1,
                        revision: 0,
                    },
                );
                self.edge(
                    "contains",
                    const_id,
                    Some(site_id.clone()),
                    owner.unit,
                    i,
                    i + 3,
                    json!({}),
                    &fp,
                )?;
            }
            self.edge(
                "reads",
                &site_id,
                Some(target),
                owner.unit,
                i,
                i + 3,
                json!({
                    "member_name": member,
                    "receiver_class": class,
                    "operation": "constant",
                    "expression": expression,
                }),
                &member,
            )?;
        }
        Ok(())
    }
    fn lookup_method(
        &self,
        class: &str,
        name: &str,
        seen: &mut BTreeSet<String>,
    ) -> Option<String> {
        lookup_method(&self.classes, &self.index, class, name, seen)
    }
    /// A framework member `kind: method` refines the return type of calls on its class
    /// (`framework_return_type`, `exact_return`); it creates no node and no target.
    fn merge_return_refinement(&mut self, node: &Node) {
        let (Some((class_name, method)), Some(returns)) = (
            node.qualified_name.rsplit_once("::"),
            node.metadata["framework_return_type"].as_str(),
        ) else {
            return;
        };
        let key = if self.classes.contains_key(class_name) {
            class_name.to_owned()
        } else if let Some(key) = self.index.canonical(class_name) {
            key.clone()
        } else {
            return;
        };
        let exact = node.metadata["exact_return"] == true;
        self.classes
            .get_mut(&key)
            .expect("class key resolved")
            .framework_returns
            .insert(method.to_ascii_lowercase(), (returns.to_owned(), exact));
    }
    /// Adds framework members as magic properties. Precedence: a property declared in
    /// PHP beats the framework, and the framework (code or configuration) beats PHPDoc.
    fn merge_members(&mut self, members: GraphSnapshot) -> Result<(), String> {
        self.diagnostics.extend(members.diagnostics);
        for mut node in members.nodes {
            if node.kind == "method" {
                self.merge_return_refinement(&node);
                continue;
            }
            let Some((class_name, name)) = node.qualified_name.rsplit_once("::$") else {
                continue;
            };
            let key = if self.classes.contains_key(class_name) {
                class_name.to_owned()
            } else if let Some(key) = self.index.canonical(class_name) {
                key.clone()
            } else {
                continue;
            };
            let name = name.to_owned();
            let class = self.classes.get_mut(&key).expect("class key resolved");
            if node.kind != "property"
                || class
                    .properties
                    .iter()
                    .any(|p| p.name == name && p.magic.is_none())
            {
                continue;
            }
            class.properties.retain(|p| p.name != name);
            let type_name = node.metadata["type"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned();
            class.properties.push(syntax::Property {
                name: name.clone(),
                token: class.name_token,
                type_name: type_name.clone(),
                uses_union_type: false,
                annotated: false,
                magic: Some(doc::Access::Read),
                doc_line: Some(node.span.start_line),
            });
            let (class_id, unit, start, end) =
                (class.id.clone(), class.unit, class.start, class.end);
            let id = format!("property:{key}::${name}");
            node.id = id.clone();
            node.qualified_name = format!("{key}::${name}");
            node.metadata["magic"] = json!(true);
            node.metadata["type_source"] = json!("framework");
            node.metadata["commented"] = json!(false);
            node.metadata["fingerprint"] = json!(tokens::fingerprint([
                "framework-member",
                name.as_str(),
                type_name.as_str(),
            ]));
            let getter = node.metadata["implemented_by"]
                .as_str()
                .filter(|getter| self.nodes.contains_key(*getter))
                .map(str::to_owned);
            self.nodes.insert(id.clone(), node);
            self.edge(
                "contains",
                &class_id,
                Some(id.clone()),
                unit,
                start,
                end,
                json!({}),
                "",
            )?;
            if let Some(getter) = getter {
                self.edge(
                    "implemented_by",
                    &id,
                    Some(getter),
                    unit,
                    start,
                    end,
                    json!({}),
                    "",
                )?;
            }
        }
        Ok(())
    }
    fn all_methods(&self, class: &str, seen: &mut BTreeSet<String>) -> BTreeMap<String, String> {
        if !seen.insert(class.into()) {
            return BTreeMap::new();
        }
        let Some(c) = self.classes.get(class) else {
            return BTreeMap::new();
        };
        let mut names: BTreeSet<_> = c.methods.keys().chain(c.aliases.keys()).cloned().collect();
        for base in c.parent.iter().chain(&c.traits) {
            names.extend(self.all_methods(base, seen).into_keys());
        }
        names
            .into_iter()
            .filter_map(|n| {
                self.lookup_method(class, &n, &mut BTreeSet::new())
                    .map(|id| (n, id))
            })
            .collect()
    }
    // Keep graph evidence and its source context explicit at the insertion boundary.
    #[allow(clippy::too_many_arguments)]
    fn edge(
        &mut self,
        kind: &str,
        from: &str,
        to: Option<String>,
        unit: usize,
        start: usize,
        end: usize,
        mut metadata: Value,
        occurrence: &str,
    ) -> Result<(), String> {
        self.options
            .check(self.nodes.len() + self.edges.len() + 1)?;
        let id = format!(
            "edge:{}",
            tokens::fingerprint([kind, from, to.as_deref().unwrap_or(""), occurrence])
        );
        let u = &self.units[unit];
        metadata["fingerprint"] = json!(tokens::fingerprint([
            kind,
            &u.fp(start, end, None),
            occurrence
        ]));
        if metadata.get("resolution").is_none() {
            metadata["resolution"] = json!(if to.is_some() {
                "resolved"
            } else {
                "unresolved"
            });
        }
        self.edges.insert(
            id.clone(),
            Edge {
                id,
                kind: kind.into(),
                from: from.into(),
                to,
                span: u.span(start, end),
                metadata,
                trust: 1,
                revision: 0,
            },
        );
        Ok(())
    }
    fn diagnostic(&mut self, unit: usize, token: usize, code: &str, message: String) {
        let u = &self.units[unit];
        self.diagnostics.push(Diagnostic {
            path: u.file.path.clone(),
            line: u.tokens.get(token).map_or(1, |t| t.line),
            code: code.into(),
            message,
        });
    }
    fn origins(&mut self) {
        let nodes = self.nodes.clone();
        for n in self.nodes.values_mut() {
            if let Some(returns) = n.metadata["returns"].as_array_mut() {
                for r in returns {
                    if let Some(origins) = r["origins"].as_array_mut() {
                        for origin in origins {
                            if let Some(target) =
                                origin["symbol_id"].as_str().and_then(|id| nodes.get(id))
                            {
                                origin["definition"] = json!({"id":target.id,"span":target.span,"returns":target.metadata["returns"].as_array().map(|rs|rs.iter().map(|r|json!({"line":r["line"],"expression":r["expression"],"type":r["type"],"snippet":r["snippet"]})).collect::<Vec<_>>()).unwrap_or_default()});
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A closure owned by a function or method (not by a class or file).
pub(super) fn nested_closure(function: &Function) -> bool {
    function.closure
        && function
            .owner
            .as_deref()
            .is_some_and(|owner| owner.starts_with("method:") || owner.starts_with("function:"))
}

/// `type_source` value for graph metadata: PHPDoc-derived facts may be wrong.
pub(super) fn type_source(annotated: bool) -> &'static str {
    if annotated {
        "annotation"
    } else {
        "code"
    }
}

fn property_node(id: &str, class: &Class, property: &syntax::Property, u: &Unit) -> Node {
    let mut metadata = json!({
        "type": property.type_name,
        "type_source": type_source(property.annotated),
        "commented": false
    });
    let span = match (property.magic, property.doc_line) {
        (Some(access), line) => {
            let line = line.unwrap_or_else(|| u.span(class.start, class.start + 1).start_line);
            metadata["magic"] = json!(true);
            metadata["access"] = json!(access.as_str());
            metadata["fingerprint"] = json!(tokens::fingerprint([
                "magic-property",
                property.name.as_str(),
                property.type_name.as_str(),
            ]));
            crate::domain::SourceSpan {
                path: u.file.path.clone(),
                start_line: line,
                end_line: line,
            }
        }
        (None, _) => {
            metadata["fingerprint"] =
                json!(u.fp(property.token, property.token + 1, Some(property.token)));
            u.span(property.token, property.token + 1)
        }
    };
    Node {
        id: id.to_owned(),
        kind: "property".into(),
        name: format!("${}", property.name),
        qualified_name: format!("{}::${}", class.qualified, property.name),
        span,
        metadata,
        trust: 1,
        revision: 0,
    }
}

/// Case-insensitive class names and concrete descendants, built once per analysis
/// so that lookups do not scan every class (vendor declarations make that costly).
#[derive(Default)]
pub(super) struct ClassIndex {
    lower: HashMap<String, String>,
    descendants: HashMap<String, Vec<String>>,
}
impl ClassIndex {
    fn build(classes: &BTreeMap<String, Class>) -> Self {
        let lower: HashMap<String, String> = classes
            .keys()
            .map(|name| (name.to_ascii_lowercase(), name.clone()))
            .collect();
        let mut descendants: HashMap<String, Vec<String>> = HashMap::new();
        for (name, class) in classes {
            if class.kind != "class" {
                continue;
            }
            let mut pending = vec![name.clone()];
            let mut seen = BTreeSet::new();
            while let Some(current) = pending.pop() {
                let key = current.to_ascii_lowercase();
                if !seen.insert(key.clone()) {
                    continue;
                }
                descendants.entry(key).or_default().push(name.clone());
                if let Some(c) = classes.get(&current) {
                    pending.extend(c.parent.iter().chain(&c.interfaces).cloned());
                }
            }
        }
        Self { lower, descendants }
    }
    fn canonical(&self, name: &str) -> Option<&String> {
        self.lower.get(&name.to_ascii_lowercase())
    }
    /// Concrete classes that are `class` or extend/implement it.
    pub(super) fn descendants(&self, class: &str) -> &[String] {
        self.descendants
            .get(&class.to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }
}

fn lookup_method(
    classes: &BTreeMap<String, Class>,
    index: &ClassIndex,
    class: &str,
    name: &str,
    seen: &mut BTreeSet<String>,
) -> Option<String> {
    let class = if classes.contains_key(class) {
        class.to_owned()
    } else {
        index.canonical(class)?.clone()
    };
    if !seen.insert(class.clone()) {
        return None;
    }
    let c = &classes[&class];
    let name = name.to_ascii_lowercase();
    if let Some(id) = c.methods.get(&name) {
        return Some(id.clone());
    }
    if let Some((selected, original)) = c.aliases.get(&name) {
        for target in selected.iter().chain(if selected.is_none() {
            c.traits.iter()
        } else {
            [].iter()
        }) {
            if let Some(method) = lookup_method(classes, index, target, original, &mut seen.clone())
            {
                return Some(method);
            }
        }
    }
    let traits = c
        .preferred
        .get(&name)
        .map(|s| vec![s])
        .unwrap_or_else(|| c.traits.iter().collect());
    let matches: BTreeSet<_> = traits
        .into_iter()
        .filter_map(|t| lookup_method(classes, index, t, &name, &mut seen.clone()))
        .collect();
    if matches.len() == 1 {
        return matches.into_iter().next();
    }
    if matches.len() > 1 {
        return None;
    }
    if let Some(parent) = &c.parent {
        if let Some(method) = lookup_method(classes, index, parent, &name, seen) {
            return Some(method);
        }
    }
    if c.kind == "interface" {
        for parent in &c.interfaces {
            if let Some(method) = lookup_method(classes, index, parent, &name, &mut seen.clone()) {
                return Some(method);
            }
        }
    }
    None
}
/// A class constant or enum case: the class itself, then parents and interfaces.
fn lookup_constant(
    classes: &BTreeMap<String, Class>,
    index: &ClassIndex,
    class: &str,
    name: &str,
    seen: &mut BTreeSet<String>,
) -> Option<String> {
    let key = if classes.contains_key(class) {
        class.to_owned()
    } else {
        index.canonical(class)?.clone()
    };
    if !seen.insert(key.clone()) {
        return None;
    }
    let c = &classes[&key];
    if c.constants.iter().any(|constant| constant.name == name) {
        return Some(format!("constant:{key}::{name}"));
    }
    c.parent
        .iter()
        .chain(&c.interfaces)
        .find_map(|base| lookup_constant(classes, index, base, name, seen))
}

/// First class in the hierarchy of `class` (itself included) missing from the index.
pub(super) fn unindexed_ancestor(
    classes: &BTreeMap<String, Class>,
    index: &ClassIndex,
    class: &str,
) -> Option<String> {
    let mut current = class.to_owned();
    for _ in 0..64 {
        let Some(name) = classes
            .get(&current)
            .map(|_| current.clone())
            .or_else(|| index.canonical(&current).cloned())
        else {
            return Some(current);
        };
        current = classes[&name].parent.clone()?;
    }
    None
}

fn lookup_property(
    classes: &BTreeMap<String, Class>,
    index: &ClassIndex,
    class: &str,
    name: &str,
    seen: &mut BTreeSet<String>,
) -> Option<String> {
    let key = if classes.contains_key(class) {
        class.to_owned()
    } else {
        index.canonical(class)?.clone()
    };
    if !seen.insert(key.clone()) {
        return None;
    }
    let c = classes.get(&key)?;
    if c.properties.iter().any(|p| p.name == name) {
        return Some(format!("property:{key}::${name}"));
    }
    c.parent
        .as_ref()
        .and_then(|p| lookup_property(classes, index, p, name, seen))
}

/// Replaces namespace-less class names in a type expression (`Foo|null`, `Foo[]`,
/// `Gen<Foo>`) by their area-qualified names from `globals`.
fn remap_globals(ty: &str, globals: &BTreeMap<String, String>) -> String {
    if globals.is_empty() {
        return ty.to_owned();
    }
    let mut out = String::with_capacity(ty.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        let plain = word.trim_start_matches('\\');
        match globals.get(&plain.to_ascii_lowercase()) {
            Some(qualified) if !plain.contains('\\') && !plain.is_empty() => {
                out.push_str(qualified)
            }
            _ => out.push_str(word),
        }
        word.clear();
    };
    for c in ty.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '\\' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}
