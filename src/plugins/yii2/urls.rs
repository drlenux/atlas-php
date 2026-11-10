//! Yii2 urlManager rules, controller/action convention, and URL string sites.
use super::syntax::{self, Expression, Names};
use crate::analyzer::{AnalysisOptions, SourceFile};
use crate::domain::{Edge, GraphSnapshot, Node};
use crate::plugins::php::tokens::{tokenize, TokenKind};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn enrich_urls(
    files: &[SourceFile],
    graph: &mut GraphSnapshot,
    options: &AnalysisOptions,
) -> Result<(), String> {
    options
        .progress
        .phase("analysis", "yii2_urls", Some(files.len()), "files");
    let module_apps = module_registrations(files, options)?;
    let mut routes: BTreeMap<String, RouteFact> = BTreeMap::new();
    // Convention: Controller::actionX → app:controller/x or app:module/controller/x
    for node in &graph.nodes {
        if node.kind != "method" {
            continue;
        }
        let Some((class, method)) = node.qualified_name.rsplit_once("::") else {
            continue;
        };
        let Some(action) = action_id(method) else {
            continue;
        };
        let Some(controller) = controller_id(class) else {
            continue;
        };
        let base = format!("{controller}/{action}");
        let module = module_id_from_path(&node.span.path);
        let app = app_id_from_path(&node.span.path);
        let mut keys = Vec::new();
        if let Some(module) = module.as_deref() {
            // Module controllers: only module-qualified ids, scoped to apps that
            // register the module (or the app folder the controller lives under).
            let apps: Vec<String> = if let Some(app) = app {
                vec![app]
            } else {
                module_apps
                    .get(module)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default()
            };
            for app in apps {
                keys.push(format!("{app}:{module}/{base}"));
            }
        } else if let Some(app) = app {
            keys.push(format!("{app}:{base}"));
        } else {
            keys.push(base.clone());
        }
        for route in keys {
            let fact = routes.entry(route.clone()).or_insert_with(|| RouteFact {
                route: route.clone(),
                pattern: route.clone(),
                actions: Vec::new(),
                path: node.span.path.clone(),
                line: node.span.start_line,
            });
            if !fact.actions.contains(&node.id) {
                fact.actions.push(node.id.clone());
            }
        }
    }
    // urlManager rules + top-level return arrays in *routes.php
    for file in files {
        options.check(graph.nodes.len() + graph.edges.len())?;
        if !file.path.ends_with(".php") {
            continue;
        }
        scan_php_rules(file, options, &mut routes)?;
    }
    let mut route_ids = BTreeMap::new();
    for (key, fact) in &routes {
        let id = format!("route:{key}");
        route_ids.insert(key.clone(), id.clone());
        if graph.nodes.iter().any(|n| n.id == id) {
            continue;
        }
        graph.nodes.push(Node {
            id: id.clone(),
            kind: "route".into(),
            name: fact.route.clone(),
            qualified_name: fact.route.clone(),
            span: crate::domain::SourceSpan {
                path: fact.path.clone(),
                start_line: fact.line,
                end_line: fact.line,
            },
            metadata: json!({
                "pattern": fact.pattern,
                "route": fact.route,
                "commented": false,
            }),
            trust: 1,
            revision: 0,
        });
        for action in &fact.actions {
            graph.edges.push(edge(
                "handles",
                &id,
                Some(action.clone()),
                &fact.path,
                fact.line,
                json!({"route": fact.route}),
            ));
        }
    }
    // PHP Url::to / createUrl + string paths in js/html/vue
    for file in files {
        options.check(graph.nodes.len() + graph.edges.len())?;
        if file.path.ends_with(".php") {
            scan_php_url_refs(file, options, &route_ids, graph)?;
        } else if is_web_asset(&file.path) {
            scan_web_url_sites(file, &route_ids, graph);
        }
    }
    Ok(())
}

struct RouteFact {
    route: String,
    pattern: String,
    actions: Vec<String>,
    path: String,
    line: usize,
}

fn is_web_asset(path: &str) -> bool {
    matches!(
        path.rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "js" | "mjs" | "html" | "vue"
    )
}

fn controller_id(class: &str) -> Option<String> {
    let short = class.rsplit('\\').next()?;
    let name = short.strip_suffix("Controller")?;
    if name.is_empty() {
        return None;
    }
    Some(camel_to_id(name))
}

fn action_id(method: &str) -> Option<String> {
    let rest = method.strip_prefix("action")?;
    let mut chars = rest.chars();
    let first = chars.next()?;
    if !first.is_uppercase() {
        return None;
    }
    Some(camel_to_id(rest))
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

fn normalize_route(raw: &str) -> String {
    let mut s = raw.trim().trim_matches('/');
    if let Some((path, _)) = s.split_once('?') {
        s = path;
    }
    if let Some((path, _)) = s.split_once('#') {
        s = path;
    }
    // Drop HTTP verb prefixes used in rules: "POST v1/oauth2/phone"
    if let Some((verb, rest)) = s.split_once(' ') {
        if matches!(
            verb.to_ascii_uppercase().as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        ) {
            s = rest.trim();
        }
    }
    s.trim_matches('/').to_ascii_lowercase()
}

fn scan_php_rules(
    file: &SourceFile,
    options: &AnalysisOptions,
    routes: &mut BTreeMap<String, RouteFact>,
) -> Result<(), String> {
    let is_routes_file = file
        .path
        .rsplit('/')
        .next()
        .is_some_and(|n| n.eq_ignore_ascii_case("routes.php"));
    if !is_routes_file
        && !file.source.contains("urlManager")
        && !file.source.contains("'rules'")
        && !file.source.contains("\"rules\"")
    {
        return Ok(());
    }
    let tokens = tokenize(&file.source, options)?;
    let names = Names::default();
    // Top-level return [ ... ] in routes.php
    if is_routes_file {
        if let Some(array_tokens) = top_level_return_array(&tokens) {
            if let Ok(Expression::Array(items)) = syntax::expression(array_tokens, &names, options)
            {
                ingest_rule_items(&items, file, routes);
            }
        }
    }
    // 'rules' / "rules" => [ ... ]
    for at in 0..tokens.len().saturating_sub(2) {
        options.check(0)?;
        if syntax::string(&tokens[at].text).as_deref() != Some("rules")
            || tokens[at + 1].text != "=>"
        {
            continue;
        }
        let start = at + 2;
        let end = expression_end(&tokens, start);
        if let Ok(Expression::Array(items)) =
            syntax::expression(&tokens[start..end], &names, options)
        {
            ingest_rule_items(&items, file, routes);
        }
    }
    Ok(())
}

/// `app/config/*.php` → `'modules' => ['game' => …]` maps module id → apps that load it.
fn module_registrations(
    files: &[SourceFile],
    options: &AnalysisOptions,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in files {
        if !file.path.ends_with(".php") {
            continue;
        }
        let Some(app) = app_id_from_path(&file.path) else {
            continue;
        };
        if !file.source.contains("modules") {
            continue;
        }
        let tokens = tokenize(&file.source, options)?;
        let names = Names::default();
        for at in 0..tokens.len().saturating_sub(2) {
            options.check(0)?;
            if syntax::string(&tokens[at].text).as_deref() != Some("modules")
                || tokens[at + 1].text != "=>"
            {
                continue;
            }
            let start = at + 2;
            let end = expression_end(&tokens, start);
            let Ok(Expression::Array(items)) =
                syntax::expression(&tokens[start..end], &names, options)
            else {
                continue;
            };
            for (key, _) in items {
                if let Some(module) = key {
                    let id = module.to_ascii_lowercase();
                    if !id.is_empty() {
                        out.entry(id).or_default().insert(app.clone());
                    }
                }
            }
        }
    }
    Ok(out)
}

fn top_level_return_array(
    tokens: &[crate::plugins::php::tokens::Token],
) -> Option<&[crate::plugins::php::tokens::Token]> {
    let return_at = tokens.iter().position(|t| t.is("return"))?;
    let start = return_at + 1;
    if tokens.get(start)?.text != "[" && !(tokens.get(start)?.is("array")) {
        return None;
    }
    let open = if tokens[start].text == "[" {
        start
    } else {
        start + 1
    };
    let close = syntax::closing(tokens, open)?;
    Some(&tokens[open..=close])
}

fn ingest_rule_items(
    items: &[(Option<String>, Expression)],
    file: &SourceFile,
    routes: &mut BTreeMap<String, RouteFact>,
) {
    for (key, value) in items {
        if let Some(pattern) = key.as_deref() {
            match value {
                Expression::Text(t) => {
                    insert_route(routes, pattern, t, file);
                    continue;
                }
                // 'mock-payplace/<action>' => ['class' => …, 'route' => …] / pattern-only array
                Expression::Array(fields) => {
                    ingest_rule_array(pattern, fields, file, routes);
                    continue;
                }
                _ => continue,
            }
        }
        // ['pattern' => '…', 'route' => '…'] or ['class' => UrlRule::class, 'extraPatterns' => …]
        let Expression::Array(fields) = value else {
            continue;
        };
        ingest_rule_array("", fields, file, routes);
    }
}

fn ingest_rule_array(
    keyed_pattern: &str,
    fields: &[(Option<String>, Expression)],
    file: &SourceFile,
    routes: &mut BTreeMap<String, RouteFact>,
) {
    let mut pattern = (!keyed_pattern.is_empty()).then(|| keyed_pattern.to_owned());
    let mut route = None;
    let mut controllers: Vec<String> = Vec::new();
    let mut extras: Vec<(String, String)> = Vec::new();
    for (fk, fv) in fields {
        let Some(name) = fk.as_deref() else {
            continue;
        };
        match (name, fv) {
            ("pattern", Expression::Text(t)) => pattern = Some(t.clone()),
            ("route", Expression::Text(t)) => route = Some(t.clone()),
            ("controller", Expression::Text(t)) => controllers.push(t.clone()),
            ("controller", Expression::Array(items)) => {
                for (_ck, cv) in items {
                    if let Expression::Text(t) = cv {
                        controllers.push(t.clone());
                    }
                }
            }
            ("extraPatterns", Expression::Array(extra_items)) => {
                for (ek, ev) in extra_items {
                    if let (Some(pat), Expression::Text(tgt)) = (ek.as_deref(), ev) {
                        extras.push((pat.to_owned(), tgt.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    let pattern = pattern.unwrap_or_default();
    if let Some(tgt) = route.as_deref() {
        let pat = if pattern.is_empty() { tgt } else { &pattern };
        insert_route(routes, pat, tgt, file);
    } else if !pattern.is_empty() && keyed_pattern.is_empty() {
        // pattern without route: treat pattern path as target id
        insert_route(routes, &pattern, &pattern, file);
    } else if !keyed_pattern.is_empty() && route.is_none() {
        // 'mock-payplace/<action>' => ['class' => UrlRule::class] — pattern is the route
        insert_route(routes, keyed_pattern, keyed_pattern, file);
    }
    for prefix in &controllers {
        let prefix = normalize_route(prefix);
        if prefix.is_empty() {
            continue;
        }
        insert_route(routes, &prefix, &prefix, file);
        for (pat, action) in &extras {
            let full = format!("{prefix}/{}", normalize_route(action));
            insert_route(routes, pat, &full, file);
        }
    }
}

fn insert_route(
    routes: &mut BTreeMap<String, RouteFact>,
    pattern: &str,
    target: &str,
    file: &SourceFile,
) {
    let route = normalize_route(target);
    if route.is_empty() {
        return;
    }
    let pattern_norm = normalize_route(pattern);
    // Scope rules.php routes by app folder so partner/api do not collide.
    let key = if let Some(app) = app_id_from_path(&file.path) {
        format!("{app}:{route}")
    } else {
        route.clone()
    };
    routes.entry(key.clone()).or_insert_with(|| RouteFact {
        route: key.clone(),
        pattern: pattern_norm,
        actions: Vec::new(),
        path: file.path.clone(),
        line: 1,
    });
}

fn app_id_from_path(path: &str) -> Option<String> {
    let first = path.split('/').next()?.to_ascii_lowercase();
    matches!(
        first.as_str(),
        "partner" | "api" | "admin" | "frontend" | "console" | "grcallback" | "backend"
    )
    .then_some(first)
}

fn module_id_from_path(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    while let Some(part) = parts.next() {
        if part == "modules" {
            let id = parts.next()?;
            if !id.is_empty() && id != "controllers" {
                return Some(id.to_ascii_lowercase());
            }
        }
    }
    None
}

fn expression_end(tokens: &[crate::plugins::php::tokens::Token], start: usize) -> usize {
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
                depth = depth.saturating_sub(1);
            }
            "," | ";" if depth == 0 => return at,
            _ => {}
        }
    }
    tokens.len()
}

fn scan_php_url_refs(
    file: &SourceFile,
    options: &AnalysisOptions,
    route_ids: &BTreeMap<String, String>,
    graph: &mut GraphSnapshot,
) -> Result<(), String> {
    let tokens = tokenize(&file.source, options)?;
    let mut i = 0;
    while i + 4 < tokens.len() {
        options.check(0)?;
        // Url::to( or Url::toRoute(
        let is_url_helper = tokens[i].text == "Url"
            && tokens.get(i + 1).is_some_and(|t| t.text == "::")
            && tokens
                .get(i + 2)
                .is_some_and(|t| t.text == "to" || t.text == "toRoute")
            && tokens.get(i + 3).is_some_and(|t| t.text == "(");
        let is_create_url =
            tokens[i].text == "createUrl" && tokens.get(i + 1).is_some_and(|t| t.text == "(");
        if !is_url_helper && !is_create_url {
            i += 1;
            continue;
        }
        let open = if is_url_helper { i + 3 } else { i + 1 };
        let Some(close) = syntax::closing(&tokens, open) else {
            i += 1;
            continue;
        };
        let route = first_route_string(&tokens[open + 1..close]);
        if let Some(route) = route {
            let key = normalize_route(&route);
            if let Some(route_id) = resolve_route_id(route_ids, &key, &file.path) {
                let line = tokens[i].line;
                let site_id = format!("url_site:{}:{}", file.path, tokens[i].start);
                if !graph.nodes.iter().any(|n| n.id == site_id) {
                    graph.nodes.push(Node {
                        id: site_id.clone(),
                        kind: "url_site".into(),
                        name: route.chars().take(80).collect(),
                        qualified_name: site_id.clone(),
                        span: crate::domain::SourceSpan {
                            path: file.path.clone(),
                            start_line: line,
                            end_line: line,
                        },
                        metadata: json!({
                            "expression": format!("Url…({route})"),
                            "route": key,
                            "operation": "url",
                            "commented": false,
                        }),
                        trust: 1,
                        revision: 0,
                    });
                    graph.edges.push(edge(
                        "references",
                        &site_id,
                        Some(route_id),
                        &file.path,
                        line,
                        json!({"route": key}),
                    ));
                }
            }
        }
        i = close + 1;
    }
    Ok(())
}

fn first_route_string(tokens: &[crate::plugins::php::tokens::Token]) -> Option<String> {
    // ['game/view', ...] or 'game/view' or "/game/view"
    let mut depth = 0usize;
    for token in tokens {
        match token.text.as_str() {
            "[" | "(" => depth += 1,
            "]" | ")" => depth = depth.saturating_sub(1),
            _ => {}
        }
        if token.kind == TokenKind::String {
            let s = syntax::string(&token.text)?;
            if s.contains('/') || s.contains('-') || !s.contains('\\') {
                return Some(s);
            }
        }
    }
    let _ = depth;
    None
}

fn scan_web_url_sites(
    file: &SourceFile,
    route_ids: &BTreeMap<String, String>,
    graph: &mut GraphSnapshot,
) {
    // Quote scanner that skips // and /* */ so apostrophes in comments
    // (common in UA/RU JS like mybonus.js) do not swallow real path strings.
    let mut i = 0;
    let bytes = file.source.as_bytes();
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = i.saturating_add(2);
            continue;
        }
        if c == b'\'' || c == b'"' {
            let quote = c;
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i > start {
                if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
                    maybe_url_site(file, s, start, route_ids, graph);
                }
            }
            i += 1;
            continue;
        }
        i += 1;
    }
}

fn resolve_route_id(route_ids: &BTreeMap<String, String>, key: &str, path: &str) -> Option<String> {
    // App from file path wins (Url::to in partner → partner:game/view, not bare module route).
    if let Some(app) = app_id_from_path(path) {
        let scoped = format!("{app}:{key}");
        if let Some(id) = route_ids.get(&scoped) {
            return Some(id.clone());
        }
        // Unique hit among this app's routes with matching suffix.
        let prefix = format!("{app}:");
        let mut hits: Vec<&String> = route_ids
            .keys()
            .filter(|k| {
                k.starts_with(&prefix)
                    && (k.as_str() == scoped.as_str()
                        || k.ends_with(&format!(":{key}"))
                        || k.ends_with(&format!("/{key}"))
                        || k.strip_prefix(&prefix) == Some(key))
            })
            .collect();
        hits.sort();
        hits.dedup_by_key(|k| route_ids.get(*k));
        if hits.len() == 1 {
            return route_ids.get(hits[0]).cloned();
        }
        // No app-scoped route — do not fall back to bare module routes from other apps.
        return None;
    }
    if let Some(id) = route_ids.get(key) {
        return Some(id.clone());
    }
    let mut hits: Vec<&String> = route_ids
        .keys()
        .filter(|k| {
            k.as_str() == key
                || k.ends_with(&format!(":{key}"))
                || k.ends_with(&format!("/{key}"))
                || key.ends_with(k.as_str())
        })
        .collect();
    hits.sort();
    hits.dedup_by_key(|k| route_ids.get(*k));
    if hits.len() == 1 {
        return route_ids.get(hits[0]).cloned();
    }
    None
}

fn maybe_url_site(
    file: &SourceFile,
    raw: &str,
    offset: usize,
    route_ids: &BTreeMap<String, String>,
    graph: &mut GraphSnapshot,
) {
    let key = normalize_route(raw);
    if key.is_empty() || key.len() < 3 || !key.contains('/') {
        return;
    }
    let Some(route_id) = resolve_route_id(route_ids, &key, &file.path) else {
        return;
    };
    let line = file.source[..offset.min(file.source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    let site_id = format!("url_site:{}:{offset}", file.path);
    if graph.nodes.iter().any(|n| n.id == site_id) {
        return;
    }
    graph.nodes.push(Node {
        id: site_id.clone(),
        kind: "url_site".into(),
        name: raw.chars().take(80).collect(),
        qualified_name: site_id.clone(),
        span: crate::domain::SourceSpan {
            path: file.path.clone(),
            start_line: line,
            end_line: line,
        },
        metadata: json!({
            "expression": raw,
            "route": key,
            "operation": "url",
            "commented": false,
        }),
        trust: 1,
        revision: 0,
    });
    graph.edges.push(edge(
        "references",
        &site_id,
        Some(route_id.clone()),
        &file.path,
        line,
        json!({"route": key}),
    ));
}

fn edge(
    kind: &str,
    from: &str,
    to: Option<String>,
    path: &str,
    line: usize,
    mut metadata: serde_json::Value,
) -> Edge {
    let id = format!(
        "edge:{}",
        crate::plugins::php::tokens::fingerprint([
            kind,
            from,
            to.as_deref().unwrap_or(""),
            path,
            &line.to_string()
        ])
    );
    metadata["resolution"] = json!(if to.is_some() {
        "resolved"
    } else {
        "unresolved"
    });
    metadata["fingerprint"] = json!(crate::plugins::php::tokens::fingerprint([
        kind,
        path,
        &line.to_string()
    ]));
    Edge {
        id,
        kind: kind.into(),
        from: from.into(),
        to,
        span: crate::domain::SourceSpan {
            path: path.into(),
            start_line: line,
            end_line: line,
        },
        metadata,
        trust: 1,
        revision: 0,
    }
}
