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
    // Flow file-scope now analyzes top-level `$this->render`; token-scan is fallback
    // only for view paths that still lack a render call_site.
    let flow_render_paths: BTreeSet<String> = graph
        .edges
        .iter()
        .filter(|e| e.kind == "calls")
        .filter(|e| {
            let member = e.metadata["member_name"].as_str().unwrap_or("");
            RENDER_METHODS
                .iter()
                .any(|m| member.eq_ignore_ascii_case(m))
        })
        .map(|e| e.span.path.clone())
        .collect();
    renders.extend(
        scan_view_source_renders(files, graph, &controller_paths, &file_paths, options)?
            .into_iter()
            .filter(|fact| !flow_render_paths.contains(&fact.path)),
    );
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
        let owner = enclosing_controller(graph, &edge.from)
            .or_else(|| class_if_controller(receiver))
            .or_else(|| controller_owning_view_file(graph, &edge.span.path));
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
            controller: owner,
        });
    }

    for fact in &renders {
        options.check(graph.nodes.len() + graph.edges.len())?;
        // Token-scan facts use `from: view:{path}`; ensure that source node exists
        // even when no controller `render` created it yet (nested-only views).
        if let Some(source_path) = fact.from.strip_prefix("view:") {
            ensure_view_node(graph, source_path);
            if let Some(controller) = &fact.controller {
                ensure_view_owned(graph, fact, source_path, controller);
            }
        }
        for view_path in &fact.candidates {
            let view_id = format!("view:{view_path}");
            ensure_view_node(graph, view_path);
            if let Some(controller) = &fact.controller {
                ensure_view_owned(graph, fact, view_path, controller);
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
    // `$this->layout = 'main'` / writes to layout → layouts/<name>.php owned by controller.
    link_layout_views(graph, &file_paths, &controller_paths);
    // Nested `$this->render` inside a view inherits the controller that owns the parent view.
    propagate_view_ownership(graph);
    Ok(())
}

fn propagate_view_ownership(graph: &mut GraphSnapshot) {
    let mut pending: Vec<(String, String, String, usize, String)> = Vec::new();
    for edge in graph.edges.iter().filter(|e| e.kind == "renders") {
        let Some(view_id) = edge.to.clone() else {
            continue;
        };
        let owned = graph.edges.iter().any(|e| {
            e.kind == "contains"
                && e.to.as_deref() == Some(view_id.as_str())
                && e.from.starts_with("class:")
        });
        if owned {
            continue;
        }
        let Some(controller) = controller_owning_view_file(graph, &edge.span.path) else {
            continue;
        };
        pending.push((
            format!("class:{controller}"),
            view_id,
            edge.span.path.clone(),
            edge.span.start_line,
            controller,
        ));
    }
    for (class_id, view_id, path, line, controller) in pending {
        if !graph.nodes.iter().any(|n| n.id == class_id) {
            continue;
        }
        let owns_id = format!("edge:contains:{class_id}:{view_id}");
        if graph.edges.iter().any(|e| e.id == owns_id) {
            continue;
        }
        graph.edges.push(Edge {
            id: owns_id,
            kind: "contains".into(),
            from: class_id,
            to: Some(view_id),
            span: SourceSpan {
                path,
                start_line: line,
                end_line: line,
            },
            metadata: json!({
                "framework": "yii2",
                "framework_evidence": "nested_render",
                "controller": controller,
                "resolution": "resolved",
            }),
            trust: 1,
            revision: 0,
        });
    }
}

fn ensure_view_node(graph: &mut GraphSnapshot, view_path: &str) {
    let view_id = format!("view:{view_path}");
    if graph.nodes.iter().any(|n| n.id == view_id) {
        return;
    }
    graph.nodes.push(Node {
        id: view_id,
        kind: "view".into(),
        name: view_path.rsplit('/').next().unwrap_or(view_path).to_owned(),
        qualified_name: view_path.to_owned(),
        span: SourceSpan {
            path: view_path.to_owned(),
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

fn ensure_view_owned(
    graph: &mut GraphSnapshot,
    fact: &RenderFact,
    view_path: &str,
    controller: &str,
) {
    let class_id = format!("class:{controller}");
    if !graph.nodes.iter().any(|n| n.id == class_id) {
        return;
    }
    let view_id = format!("view:{view_path}");
    let owns_id = format!("edge:contains:{class_id}:{view_id}");
    if graph.edges.iter().any(|e| e.id == owns_id) {
        return;
    }
    graph.edges.push(Edge {
        id: owns_id,
        kind: "contains".into(),
        from: class_id,
        to: Some(view_id),
        span: SourceSpan {
            path: fact.path.clone(),
            start_line: fact.line,
            end_line: fact.line,
        },
        metadata: json!({
            "framework": "yii2",
            "framework_evidence": "render",
            "view": fact.view_name,
            "resolution": if fact.candidates.len() == 1 {
                "resolved"
            } else {
                "candidate"
            },
        }),
        trust: 1,
        revision: 0,
    });
}

fn scan_view_source_renders(
    files: &[SourceFile],
    graph: &GraphSnapshot,
    controller_paths: &BTreeMap<String, String>,
    file_paths: &BTreeSet<String>,
    options: &AnalysisOptions,
) -> Result<Vec<RenderFact>, String> {
    use crate::plugins::php::tokens::{tokenize, TokenKind};
    let mut out = Vec::new();
    for file in files
        .iter()
        .filter(|f| f.path.contains("/views/") || f.path.contains("/themes/"))
    {
        options.check(out.len())?;
        let Ok(tokens) = tokenize(&file.source, options) else {
            continue;
        };
        let mut i = 0;
        while i + 4 < tokens.len() {
            // $this->render('x' / $this->renderPartial(
            let is_this = tokens[i].kind == TokenKind::Variable
                && tokens[i].text.eq_ignore_ascii_case("$this");
            if !(is_this
                && tokens[i + 1].text == "->"
                && tokens[i + 2].kind == TokenKind::Ident
                && tokens[i + 3].text == "(")
            {
                i += 1;
                continue;
            }
            let method = tokens[i + 2].text.to_ascii_lowercase();
            if !RENDER_METHODS.contains(&method.as_str()) {
                i += 1;
                continue;
            }
            let arg = &tokens[i + 4];
            let Some(view_name) = matches!(arg.kind, TokenKind::String)
                .then(|| string_literal_at_start(&arg.text))
                .flatten()
            else {
                i += 1;
                continue;
            };
            let controller = controller_owning_view_file(graph, &file.path)
                .or_else(|| controller_from_view_path(&file.path, controller_paths));
            let candidates = resolve_view_paths(
                &view_name,
                &method,
                controller.as_deref(),
                &file.path,
                controller_paths,
                file_paths,
            );
            if candidates.is_empty() {
                i += 1;
                continue;
            }
            out.push(RenderFact {
                from: format!("view:{}", file.path),
                call_id: format!("view-src:{}:{}:{method}", file.path, arg.line),
                path: file.path.clone(),
                line: arg.line.max(1),
                view_name,
                params: Vec::new(),
                candidates,
                method,
                controller,
            });
            i += 5;
        }
    }
    Ok(out)
}

fn controller_from_view_path(
    path: &str,
    controller_paths: &BTreeMap<String, String>,
) -> Option<String> {
    // partner/views/site/index.php → partner\controllers\SiteController
    // partner/themes/slot/site/index.php → partner\controllers\SiteController
    let path = path.replace('\\', "/");
    let (app, rest) = if let Some(r) = path.split_once("/views/") {
        r
    } else if let Some((app, after)) = path.split_once("/themes/") {
        let rest = after.split_once('/')?.1; // drop theme domain
        (app, rest)
    } else {
        return None;
    };
    let folder = rest.split('/').next()?.to_owned();
    if folder.is_empty() || folder == "layouts" {
        return None;
    }
    // my-bonus → MyBonusController
    let class_short = folder
        .split('-')
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<String>()
        + "Controller";
    let qn = format!("{app}\\controllers\\{class_short}").replace('/', "\\");
    if controller_paths.contains_key(&qn) {
        return Some(qn);
    }
    // Case-insensitive match on short name.
    controller_paths
        .keys()
        .find(|k| {
            k.rsplit('\\').next().is_some_and(|short| {
                short.eq_ignore_ascii_case(&class_short)
                    && k.to_ascii_lowercase().contains(&format!(
                        "\\controllers\\{}",
                        class_short.to_ascii_lowercase()
                    ))
            })
        })
        .cloned()
}

fn controller_owning_view_file(graph: &GraphSnapshot, path: &str) -> Option<String> {
    let view_id = format!("view:{path}");
    graph.edges.iter().find_map(|e| {
        (e.kind == "contains"
            && e.to.as_deref() == Some(view_id.as_str())
            && e.from.starts_with("class:")
            && e.from.contains("Controller"))
        .then(|| e.from.strip_prefix("class:").unwrap_or(&e.from).to_owned())
    })
}

fn link_layout_views(
    graph: &mut GraphSnapshot,
    files: &BTreeSet<String>,
    controller_paths: &BTreeMap<String, String>,
) {
    #[derive(Clone)]
    struct LayoutFact {
        from: String,
        edge_id: String,
        path: String,
        line: usize,
        name: String,
        controller: String,
    }
    let facts: Vec<LayoutFact> = graph
        .edges
        .iter()
        .filter(|e| {
            matches!(e.kind.as_str(), "writes" | "property_access")
                && e.metadata["member_name"]
                    .as_str()
                    .is_some_and(|m| m.eq_ignore_ascii_case("layout"))
        })
        .filter_map(|edge| {
            let raw = edge.metadata["value_expression"]
                .as_str()
                .or_else(|| edge.metadata["value_constant"].as_str())
                .unwrap_or("");
            let name = literal_layout_name(raw)?;
            let controller = enclosing_controller(graph, &edge.from).or_else(|| {
                edge.metadata["receiver_class"]
                    .as_str()
                    .and_then(class_if_controller)
            })?;
            Some(LayoutFact {
                from: edge.from.clone(),
                edge_id: edge.id.clone(),
                path: edge.span.path.clone(),
                line: edge.span.start_line,
                name,
                controller,
            })
        })
        .collect();
    for fact in facts {
        let ctrl_file = controller_paths.get(&fact.controller).map(String::as_str);
        let bases = controller_view_bases(&fact.controller, ctrl_file);
        let layout = name_from_layout(&fact.name);
        let mut candidates = Vec::new();
        for base in bases {
            if let Some((app_views, _)) = base.rsplit_once('/') {
                push_existing(
                    &mut candidates,
                    &format!("{app_views}/layouts/{layout}.php"),
                    files,
                );
            }
            if let Some(app) = app_root_from_views_base(&base) {
                push_existing(
                    &mut candidates,
                    &format!("{app}/views/layouts/{layout}.php"),
                    files,
                );
            }
        }
        for view_path in candidates {
            let view_id = format!("view:{view_path}");
            if !graph.nodes.iter().any(|n| n.id == view_id) {
                graph.nodes.push(Node {
                    id: view_id.clone(),
                    kind: "view".into(),
                    name: view_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&view_path)
                        .to_owned(),
                    qualified_name: view_path.clone(),
                    span: SourceSpan {
                        path: view_path.clone(),
                        start_line: 1,
                        end_line: 1,
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "path": view_path,
                        "layout": true,
                        "commented": false,
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
            let class_id = format!("class:{}", fact.controller);
            let owns_id = format!("edge:contains:{class_id}:{view_id}");
            if graph.nodes.iter().any(|n| n.id == class_id)
                && !graph.edges.iter().any(|e| e.id == owns_id)
            {
                graph.edges.push(Edge {
                    id: owns_id,
                    kind: "contains".into(),
                    from: class_id,
                    to: Some(view_id.clone()),
                    span: SourceSpan {
                        path: fact.path.clone(),
                        start_line: fact.line,
                        end_line: fact.line,
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "framework_evidence": "layout",
                        "view": fact.name,
                        "resolution": "resolved",
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
            let edge_id = format!("edge:renders:{}:{view_id}:{}", fact.edge_id, fact.line);
            if !graph.edges.iter().any(|e| e.id == edge_id) {
                graph.edges.push(Edge {
                    id: edge_id,
                    kind: "renders".into(),
                    from: fact.from.clone(),
                    to: Some(view_id),
                    span: SourceSpan {
                        path: fact.path.clone(),
                        start_line: fact.line,
                        end_line: fact.line,
                    },
                    metadata: json!({
                        "framework": "yii2",
                        "framework_evidence": "layout",
                        "view": fact.name,
                        "resolution": "resolved",
                    }),
                    trust: 1,
                    revision: 0,
                });
            }
        }
    }
}

fn literal_layout_name(raw: &str) -> Option<String> {
    let t = raw.trim().trim_end_matches(';').trim();
    let bytes = t.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'\'' || bytes[0] == b'"')
        && bytes[0] == bytes[bytes.len() - 1]
    {
        let inner = &t[1..t.len() - 1];
        return (!inner.is_empty()).then(|| inner.to_owned());
    }
    None
}

fn name_from_layout(name: &str) -> &str {
    name.trim_start_matches('/')
        .trim_start_matches("layouts/")
        .trim_end_matches(".php")
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
    controller: Option<String>,
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
