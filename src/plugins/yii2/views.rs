//! Yii2 `$this->render` / `renderPartial` / `renderAjax` / `renderFile` → view files + params.
use crate::analyzer::{AnalysisOptions, SourceFile};
use crate::domain::{Edge, GraphSnapshot, Node, SourceSpan};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const RENDER_METHODS: &[&str] = &["render", "renderpartial", "renderajax", "renderfile"];

pub(super) fn enrich_views(
    files: &[SourceFile],
    graph: &mut GraphSnapshot,
    options: &AnalysisOptions,
) -> Result<(), String> {
    options
        .progress
        .phase("analysis", "yii2_views", Some(files.len()), "files");
    let file_paths: BTreeSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let controller_paths: BTreeMap<String, String> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "class")
        .filter(|n| n.qualified_name.ends_with("Controller"))
        .map(|n| (n.qualified_name.clone(), n.span.path.clone()))
        .collect();

    let mut renders: Vec<RenderFact> = Vec::new();
    for edge in graph.edges.iter().filter(|e| e.kind == "calls") {
        options.check(graph.nodes.len() + graph.edges.len())?;
        let member = edge.metadata["member_name"].as_str().unwrap_or("");
        if !RENDER_METHODS
            .iter()
            .any(|m| member.eq_ignore_ascii_case(m))
        {
            continue;
        }
        let Some(view_arg) = edge.metadata["arguments"]
            .as_array()
            .and_then(|a| a.first())
        else {
            continue;
        };
        let Some(view_name) = view_name_from_arg(view_arg, &edge.metadata) else {
            continue;
        };
        let params = params_from_args(edge.metadata["arguments"].as_array());
        let receiver = edge.metadata["receiver_class"].as_str().unwrap_or("");
        let owner =
            enclosing_controller(graph, &edge.from).or_else(|| class_if_controller(receiver));
        let candidates = resolve_view_paths(
            &view_name,
            member,
            owner.as_deref(),
            &edge.span.path,
            &controller_paths,
            &file_paths,
        );
        if candidates.is_empty() {
            continue;
        }
        renders.push(RenderFact {
            from: edge.from.clone(),
            call_id: edge.id.clone(),
            path: edge.span.path.clone(),
            line: edge.span.start_line,
            view_name,
            params,
            candidates,
            method: member.to_owned(),
        });
    }

    for fact in &renders {
        options.check(graph.nodes.len() + graph.edges.len())?;
        for view_path in &fact.candidates {
            let view_id = format!("view:{view_path}");
            if !graph.nodes.iter().any(|n| n.id == view_id) {
                graph.nodes.push(Node {
                    id: view_id.clone(),
                    kind: "view".into(),
                    name: view_path.rsplit('/').next().unwrap_or(view_path).to_owned(),
                    qualified_name: view_path.clone(),
                    span: SourceSpan {
                        path: view_path.clone(),
                        start_line: 1,
                        end_line: 1,
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "path": view_path,
                        "commented": false,
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
            let edge_id = format!("edge:renders:{}:{view_id}:{}", fact.call_id, fact.line);
            if graph.edges.iter().any(|e| e.id == edge_id) {
                continue;
            }
            graph.edges.push(Edge {
                id: edge_id,
                kind: "renders".into(),
                from: fact.from.clone(),
                to: Some(view_id.clone()),
                span: SourceSpan {
                    path: fact.path.clone(),
                    start_line: fact.line,
                    end_line: fact.line,
                },
                metadata: json!({
                    "framework": "yii2",
                    "framework_evidence": "render",
                    "method": fact.method,
                    "view": fact.view_name,
                    "params": fact.params.iter().map(Value::from).collect::<Vec<_>>(),
                    "resolution": if fact.candidates.len() == 1 { "resolved" } else { "candidate" },
                }),
                trust: 1,
                revision: 0,
            });
            inject_view_params(graph, view_path, &view_id, &fact.params, fact);
        }
    }
    Ok(())
}

struct RenderFact {
    from: String,
    call_id: String,
    path: String,
    line: usize,
    view_name: String,
    params: Vec<Param>,
    candidates: Vec<String>,
    method: String,
}

#[derive(Clone)]
struct Param {
    name: String,
    type_name: String,
    type_source: String,
}

impl From<&Param> for Value {
    fn from(p: &Param) -> Self {
        json!({
            "name": p.name,
            "type": p.type_name,
            "type_source": p.type_source,
        })
    }
}

fn params_from_args(args: Option<&Vec<Value>>) -> Vec<Param> {
    let Some(args) = args else {
        return Vec::new();
    };
    // render($view, $params) — params is 2nd; renderFile same.
    let params_arg = args.get(1).or_else(|| args.first());
    let Some(items) = params_arg
        .and_then(|a| a.get("items"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(Param {
                name: item.get("name")?.as_str()?.to_owned(),
                type_name: item
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown")
                    .to_owned(),
                type_source: item
                    .get("type_source")
                    .and_then(|t| t.as_str())
                    .unwrap_or("code")
                    .to_owned(),
            })
        })
        .collect()
}

fn view_name_from_arg(_arg: &Value, metadata: &Value) -> Option<String> {
    // Prefer literal from expression: render('index', …) / render("index", …
    let expr = metadata.get("expression")?.as_str()?;
    let after = expr.split_once('(')?.1;
    string_literal_at_start(after.trim_start())
}

fn string_literal_at_start(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let quote = bytes[0];
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return (!out.is_empty()).then_some(out);
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    None
}

fn enclosing_controller(graph: &GraphSnapshot, site_id: &str) -> Option<String> {
    let mut current = site_id.to_owned();
    for _ in 0..4 {
        let parent = graph.edges.iter().find_map(|e| {
            (e.kind == "contains" && e.to.as_deref() == Some(current.as_str()))
                .then(|| e.from.clone())
        })?;
        let node = graph.nodes.iter().find(|n| n.id == parent)?;
        if node.kind == "method" {
            let class = node.qualified_name.rsplit_once("::")?.0;
            if class.ends_with("Controller") {
                return Some(class.to_owned());
            }
            current = parent;
            continue;
        }
        if node.kind == "class" && node.qualified_name.ends_with("Controller") {
            return Some(node.qualified_name.clone());
        }
        current = parent;
    }
    None
}

fn class_if_controller(receiver: &str) -> Option<String> {
    let part = receiver
        .split('|')
        .map(str::trim)
        .find(|p| p.ends_with("Controller") && !p.is_empty())?;
    Some(part.split('<').next().unwrap_or(part).to_owned())
}

fn resolve_view_paths(
    view: &str,
    method: &str,
    controller: Option<&str>,
    call_path: &str,
    controller_paths: &BTreeMap<String, String>,
    files: &BTreeSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let method_l = method.to_ascii_lowercase();
    if method_l == "renderfile" {
        for path in expand_alias_or_path(view, call_path) {
            push_existing(&mut out, &path, files);
        }
        return out;
    }
    // Alias / absolute application view
    if view.starts_with('@') || view.starts_with('/') {
        for path in expand_alias_or_path(view, call_path) {
            push_existing(&mut out, &path, files);
        }
        return out;
    }
    // Relative to controller viewPath (and theme overrides from BaseController pathMap).
    if let Some(class) = controller {
        let ctrl_file = controller_paths.get(class).map(String::as_str);
        let bases = controller_view_bases(class, ctrl_file);
        for base in bases {
            let path = format!("{base}/{view}.php");
            push_existing(&mut out, &path, files);
            // Theme override: @app/views → @app/themes/<domain>/… (domain unknown → all themes)
            if let Some(app) = app_root_from_views_base(&base) {
                for theme in theme_view_candidates(app, &base, view, files) {
                    push_existing(&mut out, &theme, files);
                }
            }
        }
    }
    // View::render from inside a view file — same directory / relative
    if call_path.contains("/views/") || call_path.contains("/themes/") {
        if let Some(dir) = call_path.rsplit_once('/').map(|(d, _)| d) {
            let path = if view.contains('/') {
                format!("{dir}/../{view}.php") // rare; normalize below
            } else {
                format!("{dir}/{view}.php")
            };
            push_existing(&mut out, &normalize_path(&path), files);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn controller_view_bases(class: &str, controller_file: Option<&str>) -> Vec<String> {
    let mut bases = Vec::new();
    let id = controller_id(class);
    // partner\controllers\MyBonusController + file partner/controllers/… → partner/views/my-bonus
    if let Some(file) = controller_file {
        if let Some(root) = file.split_once("/controllers/").map(|(r, _)| r) {
            if let Some(id) = &id {
                bases.push(format!("{root}/views/{id}"));
            }
        }
        // modules/.../controllers/... → modules/.../views/{id}
        if let Some(idx) = file.find("/modules/") {
            if let Some(controllers_at) = file[idx..].find("/controllers/") {
                let module_root = &file[..idx + controllers_at];
                if let Some(id) = &id {
                    // api subfolder controllers: views may live under views/api/…
                    if let Some(sub) = file.split("/controllers/").nth(1).and_then(|rest| {
                        rest.rsplit_once('/')
                            .map(|(dir, _)| dir)
                            .filter(|d| !d.is_empty())
                    }) {
                        bases.push(format!("{module_root}/views/{sub}/{id}"));
                    }
                    bases.push(format!("{module_root}/views/{id}"));
                }
            }
        }
    }
    // Namespace fallback: partner\controllers\X → partner/views/…
    if let Some(id) = id {
        if let Some(ns) = class.rsplit_once("\\controllers\\").map(|(n, _)| n) {
            let root = ns.replace('\\', "/");
            let candidate = format!("{root}/views/{id}");
            if !bases.contains(&candidate) {
                bases.push(candidate);
            }
        }
    }
    bases
}

fn controller_id(class: &str) -> Option<String> {
    let short = class.rsplit('\\').next()?;
    let name = short.strip_suffix("Controller")?;
    if name.is_empty() {
        return None;
    }
    Some(camel_to_id(name))
}

fn camel_to_id(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn expand_alias_or_path(view: &str, call_path: &str) -> Vec<String> {
    let mut raw = view.trim().trim_matches('/').to_owned();
    if let Some(alias) = raw.strip_prefix('@') {
        // @partner/views/foo → partner/views/foo; @particles/x → particles/x (approx)
        raw = alias.replace('\\', "/");
    } else if view.starts_with('/') {
        // /layouts/main → {app}/views/layouts/main from call site app
        if let Some(app) = call_path.split('/').next() {
            raw = format!("{app}/views{}", view);
        } else {
            raw = format!("views{}", view);
        }
    }
    if !raw.ends_with(".php") {
        raw.push_str(".php");
    }
    vec![normalize_path(&raw)]
}

fn app_root_from_views_base(base: &str) -> Option<&str> {
    base.split_once("/views/").map(|(app, _)| app)
}

fn theme_view_candidates(
    app: &str,
    views_base: &str,
    view: &str,
    files: &BTreeSet<String>,
) -> Vec<String> {
    // BaseController pathMap: '@partner/views' => '@partner/themes/' . $domain
    // So partner/views/my-bonus/index.php ↔ partner/themes/<domain>/my-bonus/index.php
    let suffix = views_base
        .strip_prefix(&format!("{app}/views"))
        .unwrap_or("")
        .trim_start_matches('/');
    let mut out = Vec::new();
    let prefix = format!("{app}/themes/");
    for path in files {
        if !path.starts_with(&prefix) || !path.ends_with(&format!("/{view}.php")) {
            continue;
        }
        // themes/<domain>/<suffix>/<view>.php
        let rest = &path[prefix.len()..];
        let Some((domain, rem)) = rest.split_once('/') else {
            continue;
        };
        let expected = if suffix.is_empty() {
            format!("{view}.php")
        } else {
            format!("{suffix}/{view}.php")
        };
        if rem == expected || rem.ends_with(&format!("/{expected}")) {
            let _ = domain;
            out.push(path.clone());
        }
    }
    out
}

fn push_existing(out: &mut Vec<String>, path: &str, files: &BTreeSet<String>) {
    let path = normalize_path(path);
    if files.contains(&path) {
        out.push(path);
    }
}

fn normalize_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// Params become typed view variables + annotate matching read_sites in the view file.
fn inject_view_params(
    graph: &mut GraphSnapshot,
    view_path: &str,
    view_id: &str,
    params: &[Param],
    fact: &RenderFact,
) {
    for param in params {
        let var_id = format!("variable:{view_id}:${}", param.name);
        if !graph.nodes.iter().any(|n| n.id == var_id) {
            graph.nodes.push(Node {
                id: var_id.clone(),
                kind: "variable".into(),
                name: format!("${}", param.name),
                qualified_name: format!("{view_path}::${}", param.name),
                span: SourceSpan {
                    path: view_path.to_owned(),
                    start_line: 1,
                    end_line: 1,
                },
                metadata: json!({
                    "type": param.type_name,
                    "type_source": "framework",
                    "framework": "yii2",
                    "framework_evidence": "render_param",
                    "commented": false,
                }),
                trust: 1,
                revision: 0,
            });
            graph.edges.push(Edge {
                id: format!("edge:contains:{view_id}:{var_id}"),
                kind: "contains".into(),
                from: view_id.to_owned(),
                to: Some(var_id.clone()),
                span: SourceSpan {
                    path: view_path.to_owned(),
                    start_line: 1,
                    end_line: 1,
                },
                metadata: json!({}),
                trust: 1,
                revision: 0,
            });
        }
        let provide_id = format!("edge:provides:{}:{var_id}:{}", fact.call_id, fact.line);
        if !graph.edges.iter().any(|e| e.id == provide_id) {
            graph.edges.push(Edge {
                id: provide_id,
                kind: "provides".into(),
                from: fact.from.clone(),
                to: Some(var_id.clone()),
                span: SourceSpan {
                    path: fact.path.clone(),
                    start_line: fact.line,
                    end_line: fact.line,
                },
                metadata: json!({
                    "framework": "yii2",
                    "framework_evidence": "render_param",
                    "name": param.name,
                    "type": param.type_name,
                }),
                trust: 1,
                revision: 0,
            });
        }
        // Annotate existing occurrence sites in the view that mention $name.
        for edge in graph.edges.iter_mut() {
            if edge.span.path != view_path {
                continue;
            }
            let member = edge.metadata["member_name"].as_str().unwrap_or("");
            let expr = edge.metadata["expression"].as_str().unwrap_or("");
            let is_var = member == param.name
                || expr == format!("${}", param.name)
                || expr.starts_with(&format!("${}->", param.name))
                || expr.starts_with(&format!("${}::", param.name));
            if !is_var {
                continue;
            }
            if edge.metadata["receiver_class"]
                .as_str()
                .is_none_or(|c| c.is_empty() || c == "unknown" || c == "mixed")
                && param.type_name != "unknown"
                && !param.type_name.is_empty()
            {
                edge.metadata["receiver_class"] = json!(param.type_name);
                edge.metadata["type_source"] = json!("framework");
                edge.metadata["framework"] = json!("yii2");
                edge.metadata["framework_evidence"] = json!("render_param");
            }
        }
        for node in graph.nodes.iter_mut() {
            if node.span.path != view_path {
                continue;
            }
            if node.kind == "variable" && node.name == format!("${}", param.name) {
                let ty = node.metadata["type"].as_str().unwrap_or("unknown");
                if ty == "unknown" || ty.is_empty() || ty == "mixed" {
                    node.metadata["type"] = json!(param.type_name);
                    node.metadata["type_source"] = json!("framework");
                    node.metadata["framework"] = json!("yii2");
                    node.metadata["framework_evidence"] = json!("render_param");
                }
            }
        }
    }
}
