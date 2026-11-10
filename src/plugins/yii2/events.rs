//! Yii2 `Event::on` / `$obj->on`, `off`, and `trigger` → handler calls.
use super::syntax::{self, Names};
use crate::analyzer::{AnalysisOptions, SourceFile};
use crate::domain::{Edge, GraphSnapshot, Node, SourceSpan};
use crate::plugins::php::tokens::{tokenize, Token, TokenKind};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
struct Binding {
    owner: String,
    /// Match keys: `EVENT_*` names and/or runtime string values.
    keys: BTreeSet<String>,
    handlers: Vec<String>,
    path: String,
    line: usize,
    off: bool,
}

pub(super) fn enrich_events(
    files: &[SourceFile],
    graph: &mut GraphSnapshot,
    options: &AnalysisOptions,
) -> Result<(), String> {
    options
        .progress
        .phase("analysis", "yii2_events", Some(files.len()), "files");
    let mut bindings = Vec::new();
    for file in files {
        options.check(graph.nodes.len() + graph.edges.len())?;
        if !file.path.ends_with(".php") {
            continue;
        }
        scan_file(file, options, &mut bindings)?;
    }
    let method_ids: BTreeSet<String> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "method")
        .map(|n| n.id.clone())
        .collect();
    let const_values = constant_values(graph);

    let mut by_owner_key: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for binding in bindings.iter().filter(|b| !b.off) {
        let keys = expand_keys(&binding.keys, &binding.owner, &const_values);
        let primary = primary_key(&keys);
        let event_id = ensure_event_node(graph, binding, &keys, &primary);
        for handler in &binding.handlers {
            if !method_ids.contains(handler) {
                continue;
            }
            for key in &keys {
                by_owner_key
                    .entry((binding.owner.clone(), key.clone()))
                    .or_default()
                    .insert(handler.clone());
            }
            let edge_id = format!("edge:handles:{event_id}:{handler}:{}", binding.line);
            if graph.edges.iter().any(|e| e.id == edge_id) {
                continue;
            }
            graph.edges.push(Edge {
                id: edge_id,
                kind: "handles".into(),
                from: event_id.clone(),
                to: Some(handler.clone()),
                span: SourceSpan {
                    path: binding.path.clone(),
                    start_line: binding.line,
                    end_line: binding.line,
                },
                metadata: json!({
                    "framework": "yii2",
                    "framework_evidence": "event_on",
                    "event": primary,
                }),
                trust: 1,
                revision: 0,
            });
        }
    }

    // off: record unsubscribes; do not drop handlers that also have `on` (register/unregister).
    // Always materialize the event node — off-only (e.g. yii\debug) must not dangle.
    for binding in bindings.iter().filter(|b| b.off) {
        let keys = expand_keys(&binding.keys, &binding.owner, &const_values);
        let primary = primary_key(&keys);
        let event_id = ensure_event_node(graph, binding, &keys, &primary);
        for handler in &binding.handlers {
            if !method_ids.contains(handler) {
                continue;
            }
            let edge_id = format!(
                "edge:unsubscribes:{}:{}:{}",
                binding.owner, handler, binding.line
            );
            if graph.edges.iter().any(|e| e.id == edge_id) {
                continue;
            }
            graph.edges.push(Edge {
                id: edge_id,
                kind: "unsubscribes".into(),
                from: event_id.clone(),
                to: Some(handler.clone()),
                span: SourceSpan {
                    path: binding.path.clone(),
                    start_line: binding.line,
                    end_line: binding.line,
                },
                metadata: json!({
                    "framework": "yii2",
                    "framework_evidence": "event_off",
                    "event": primary,
                    "keys": keys.iter().cloned().collect::<Vec<_>>(),
                }),
                trust: 1,
                revision: 0,
            });
        }
    }

    let trigger_sites: Vec<(String, String, BTreeSet<String>, String, usize)> = graph
        .edges
        .iter()
        .filter(|e| e.kind == "calls" && e.metadata["member_name"] == "trigger")
        .filter_map(|e| {
            let expr = e.metadata["expression"].as_str()?;
            let receiver = e.metadata["receiver_class"].as_str().unwrap_or("");
            let mut keys = event_keys_from_expr(expr);
            if keys.is_empty() {
                return None;
            }
            let owner = resolve_trigger_owner(receiver, expr, graph)
                .or_else(|| enclosing_class(graph, &e.from))?;
            keys = expand_keys(&keys, &owner, &const_values);
            Some((
                e.from.clone(),
                owner,
                keys,
                e.span.path.clone(),
                e.span.start_line,
            ))
        })
        .collect();

    for (site_id, owner, keys, path, line) in trigger_sites {
        let mut handlers = BTreeSet::new();
        for key in &keys {
            for ((bound_owner, bound_key), set) in &by_owner_key {
                if bound_key != key {
                    continue;
                }
                if owners_equivalent(bound_owner, &owner) {
                    handlers.extend(set.iter().cloned());
                }
            }
        }
        let primary = primary_key(&keys);
        for handler in handlers {
            let edge_id = format!("edge:calls:event:{site_id}:{handler}");
            if graph.edges.iter().any(|e| e.id == edge_id) {
                continue;
            }
            graph.edges.push(Edge {
                id: edge_id,
                kind: "calls".into(),
                from: site_id.clone(),
                to: Some(handler),
                span: SourceSpan {
                    path: path.clone(),
                    start_line: line,
                    end_line: line,
                },
                metadata: json!({
                    "framework": "yii2",
                    "framework_evidence": "event_trigger",
                    "member_name": primary,
                    "operation": "trigger",
                    "resolution": "resolved",
                    "type_source": "framework",
                    "receiver_class": owner,
                    "expression": format!("trigger({owner}::{primary})"),
                    "event": format!("{owner}::{primary}"),
                }),
                trust: 1,
                revision: 0,
            });
        }
    }
    Ok(())
}

fn ensure_event_node(
    graph: &mut GraphSnapshot,
    binding: &Binding,
    keys: &BTreeSet<String>,
    primary: &str,
) -> String {
    let event_id = format!("event:{}::{primary}", binding.owner);
    if graph.nodes.iter().any(|n| n.id == event_id) {
        return event_id;
    }
    graph.nodes.push(Node {
        id: event_id.clone(),
        kind: "event".into(),
        name: primary.to_owned(),
        qualified_name: format!("{}::{primary}", binding.owner),
        span: SourceSpan {
            path: binding.path.clone(),
            start_line: binding.line,
            end_line: binding.line,
        },
        metadata: json!({
            "owner": binding.owner,
            "event": primary,
            "keys": keys.iter().cloned().collect::<Vec<_>>(),
            "framework": "yii2",
            "commented": false,
        }),
        trust: 1,
        revision: 0,
    });
    event_id
}

fn constant_values(graph: &GraphSnapshot) -> BTreeMap<String, String> {
    graph
        .nodes
        .iter()
        .filter(|n| n.kind == "constant")
        .filter_map(|n| {
            let value = n.metadata["value"]
                .as_str()?
                .trim()
                .trim_matches('\'')
                .trim_matches('"');
            if value.is_empty() {
                return None;
            }
            Some((n.qualified_name.clone(), value.to_owned()))
        })
        .collect()
}

fn expand_keys(
    keys: &BTreeSet<String>,
    owner: &str,
    const_values: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut out = keys.clone();
    for key in keys {
        if key.starts_with("EVENT_") {
            let q = format!("{owner}::{key}");
            if let Some(v) = const_values.get(&q) {
                out.insert(v.clone());
            }
            // also any Owner::EVENT_* with same short owner suffix
            for (qn, v) in const_values {
                if qn.ends_with(&format!("::{key}")) {
                    out.insert(v.clone());
                    if let Some((o, _)) = qn.rsplit_once("::") {
                        if o.ends_with(owner) || owner.ends_with(o) {
                            out.insert(v.clone());
                        }
                    }
                }
            }
        } else {
            // string value → any EVENT_* constant with this runtime value
            // (name may be declared on User while Event::on targets RegistrationController)
            for (qn, v) in const_values {
                if v == key {
                    if let Some((_, name)) = qn.rsplit_once("::") {
                        if name.starts_with("EVENT_") {
                            out.insert(name.to_owned());
                        }
                    }
                }
            }
            let _ = owner;
        }
    }
    out
}

fn primary_key(keys: &BTreeSet<String>) -> String {
    keys.iter()
        .find(|k| k.starts_with("EVENT_"))
        .cloned()
        .or_else(|| keys.iter().next().cloned())
        .unwrap_or_else(|| "unknown".into())
}

/// Yii fires handlers registered for the *sender object class*, not the class that
/// declares `EVENT_*`. `User::EVENT_X` on `$this->trigger(...)` inside `RegistrationController`
/// matches `Event::on(RegistrationController::class, User::EVENT_X, …)`.
fn resolve_trigger_owner(receiver: &str, expr: &str, graph: &GraphSnapshot) -> Option<String> {
    // Event::trigger(ClassName::class, …) — class-level trigger; owner is the first arg.
    if is_event_static_trigger(expr, receiver) {
        if let Some(owner) = class_arg_from_trigger_expr(expr, graph) {
            return Some(owner);
        }
    }
    let receiver = receiver
        .split('|')
        .map(str::trim)
        .find(|p| !p.is_empty() && *p != "unknown" && !is_event_framework_class(p))?;
    Some(receiver.split('<').next().unwrap_or(receiver).to_owned())
}

fn is_event_framework_class(class: &str) -> bool {
    let c = class.trim_start_matches('\\');
    c == "Event" || c.ends_with("\\Event") || c == "yii\\base\\Event" || c == "yii\\base\\Component"
}

fn owners_equivalent(a: &str, b: &str) -> bool {
    let a = a.trim_start_matches('\\');
    let b = b.trim_start_matches('\\');
    a.eq_ignore_ascii_case(b) || a.ends_with(&format!("\\{b}")) || b.ends_with(&format!("\\{a}"))
}

fn is_event_static_trigger(expr: &str, receiver: &str) -> bool {
    let e = expr.replace(' ', "");
    e.contains("Event::trigger(")
        || e.contains("\\Event::trigger(")
        || (is_event_framework_class(receiver) && e.contains("::trigger("))
}

fn class_arg_from_trigger_expr(expr: &str, graph: &GraphSnapshot) -> Option<String> {
    // Event::trigger(Foo::class, …) / Event::trigger(class: Foo::class|__CLASS__, …)
    let after = expr.split_once("trigger(")?.1;
    let arg = after.split(',').next()?.trim();
    let arg = arg
        .strip_prefix("class:")
        .map(str::trim)
        .unwrap_or(arg)
        .trim();
    if arg.starts_with("static::class")
        || arg.starts_with("self::class")
        || arg == "__CLASS__"
        || arg.eq_ignore_ascii_case("__CLASS__")
    {
        return None; // resolved via enclosing_class(call_site)
    }
    let name = arg
        .strip_suffix("::class")
        .unwrap_or(arg)
        .trim()
        .trim_start_matches('\\');
    if name.is_empty() || matches!(name, "static" | "self" | "parent" | "__CLASS__") {
        return None;
    }
    // Many apps have several `Payment` classes; prefer the one that declares the EVENT_*.
    let event_name = event_keys_from_expr(expr)
        .into_iter()
        .find(|k| k.starts_with("EVENT_"));
    let mut candidates: Vec<&Node> = graph
        .nodes
        .iter()
        .filter(|n| {
            matches!(n.kind.as_str(), "class" | "trait")
                && (n.qualified_name == name
                    || n.qualified_name.ends_with(&format!("\\{name}"))
                    || n.name == name)
        })
        .collect();
    if candidates.is_empty() {
        return Some(name.to_owned());
    }
    if let Some(ev) = &event_name {
        if let Some(c) = candidates.iter().find(|c| {
            graph.nodes.iter().any(|n| {
                n.kind == "constant" && n.qualified_name == format!("{}::{ev}", c.qualified_name)
            })
        }) {
            return Some(c.qualified_name.clone());
        }
    }
    // Prefer namespaced FQCN over a bare short-name stub (`class:Payment`).
    candidates.sort_by_key(|c| std::cmp::Reverse(c.qualified_name.matches('\\').count()));
    Some(candidates[0].qualified_name.clone())
}

/// `call_site` → containing method/function → class (for `Event::trigger(static::class, …)`).
fn enclosing_class(graph: &GraphSnapshot, site_id: &str) -> Option<String> {
    let parent = graph.edges.iter().find_map(|e| {
        (e.kind == "contains" && e.to.as_deref() == Some(site_id)).then(|| e.from.clone())
    })?;
    let parent_node = graph.nodes.iter().find(|n| n.id == parent)?;
    if parent_node.kind == "method" {
        let q = &parent_node.qualified_name;
        return q.rsplit_once("::").map(|(c, _)| c.to_owned());
    }
    // Nested closure: climb one more contains edge.
    let grand = graph.edges.iter().find_map(|e| {
        (e.kind == "contains" && e.to.as_deref() == Some(parent.as_str())).then(|| e.from.clone())
    })?;
    let grand_node = graph.nodes.iter().find(|n| n.id == grand)?;
    if grand_node.kind == "method" {
        return grand_node
            .qualified_name
            .rsplit_once("::")
            .map(|(c, _)| c.to_owned());
    }
    None
}

fn event_keys_from_expr(expr: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let Some(idx) = expr.find("::EVENT_") {
        let rest = &expr[idx + 2..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.starts_with("EVENT_") {
            keys.insert(name);
        }
    }
    // trigger('literal') / trigger("literal")
    for (quote, other) in [('\'', '"'), ('"', '\'')] {
        let _ = other;
        let mut rest = expr;
        while let Some(start) = rest.find(quote) {
            let after = &rest[start + 1..];
            if let Some(end) = after.find(quote) {
                let lit = &after[..end];
                if !lit.is_empty() && !lit.contains('$') && lit.len() < 120 {
                    keys.insert(lit.to_owned());
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    keys
}

fn scan_file(
    file: &SourceFile,
    options: &AnalysisOptions,
    out: &mut Vec<Binding>,
) -> Result<(), String> {
    let src = &file.source;
    if !(src.contains("::on")
        || src.contains("->on")
        || src.contains("::off")
        || src.contains("->off"))
    {
        return Ok(());
    }
    let tokens = tokenize(src, options)?;
    let names = file_names(&tokens);
    let mut i = 0;
    while i < tokens.len() {
        options.check(0)?;
        if let Some((open, close, off, instance)) = detect_call(&tokens, i) {
            let args = split_args(&tokens[open + 1..close]);
            if let Some(binding) = parse_binding(&args, &names, instance, off, file, tokens[i].line)
            {
                out.push(binding);
            }
            i = close + 1;
            continue;
        }
        i += 1;
    }
    Ok(())
}

/// Returns (open_paren_index, close, is_off, is_instance).
fn detect_call(tokens: &[Token], i: usize) -> Option<(usize, usize, bool, bool)> {
    // Event::on( / Component::on( / Foo::off(
    if i + 3 < tokens.len()
        && tokens[i + 1].text == "::"
        && (tokens[i + 2].text == "on" || tokens[i + 2].text == "off")
        && tokens[i + 3].text == "("
        && (tokens[i].text == "Event"
            || tokens[i].text.ends_with("Event")
            || tokens[i].text == "Component")
    {
        let open = i + 3;
        let close = syntax::closing(tokens, open)?;
        return Some((open, close, tokens[i + 2].text == "off", false));
    }
    // \yii\base\Event::on(
    if (tokens[i].text == "on" || tokens[i].text == "off")
        && i >= 2
        && tokens[i - 1].text == "::"
        && tokens.get(i + 1).is_some_and(|t| t.text == "(")
        && tokens[..i]
            .iter()
            .rev()
            .take(8)
            .any(|t| t.text.contains("Event"))
    {
        let open = i + 1;
        let close = syntax::closing(tokens, open)?;
        return Some((open, close, tokens[i].text == "off", false));
    }
    // $this->on( / $x->off(
    if tokens[i].text == "->"
        && tokens
            .get(i + 1)
            .is_some_and(|t| t.text == "on" || t.text == "off")
        && tokens.get(i + 2).is_some_and(|t| t.text == "(")
    {
        let open = i + 2;
        let close = syntax::closing(tokens, open)?;
        return Some((open, close, tokens[i + 1].text == "off", true));
    }
    None
}

fn parse_binding(
    args: &[Vec<Token>],
    names: &Names,
    instance: bool,
    off: bool,
    file: &SourceFile,
    line: usize,
) -> Option<Binding> {
    let mut owner = None;
    let mut keys = BTreeSet::new();
    let mut handler_toks = None;

    if args.iter().any(|a| named_arg(a).is_some()) {
        for arg in args {
            let Some((key, value)) = named_arg(arg) else {
                continue;
            };
            match key.as_str() {
                "class" => owner = class_from_tokens(value, names),
                "name" => keys.extend(event_keys_from_tokens(value)),
                "handler" => handler_toks = Some(value.to_vec()),
                _ => {}
            }
        }
    } else if instance {
        // on($name, $handler) / off($name, $handler?)
        if args.is_empty() {
            return None;
        }
        keys.extend(event_keys_from_tokens(&args[0]));
        owner = owner_from_event_keys(&keys, names).or_else(|| current_class(names));
        if args.len() >= 2 {
            handler_toks = Some(args[1].clone());
        }
    } else if args.len() >= 2 {
        owner = class_from_tokens(&args[0], names);
        keys.extend(event_keys_from_tokens(&args[1]));
        if args.len() >= 3 {
            handler_toks = Some(args[2].clone());
        }
    }

    let owner = owner?;
    if keys.is_empty() {
        return None;
    }
    let handlers = handler_toks
        .as_deref()
        .map(|t| handlers_from_tokens(t, names))
        .unwrap_or_default();
    if handlers.is_empty() && !off {
        return None;
    }
    // off without handler still records class/event-level unsubscribe
    Some(Binding {
        owner,
        keys,
        handlers,
        path: file.path.clone(),
        line,
        off,
    })
}

fn owner_from_event_keys(keys: &BTreeSet<String>, names: &Names) -> Option<String> {
    // When keys only have EVENT_* we need the class from tokens — handled elsewhere.
    // For self::EVENT already expanded to EVENT_* only; owner is current_class.
    let _ = keys;
    current_class(names)
}

fn named_arg(tokens: &[Token]) -> Option<(String, &[Token])> {
    let colon = tokens.iter().position(|t| t.text == ":")?;
    if colon == 0 {
        return None;
    }
    // `class: X` — key is ident before colon; skip `::`
    if colon >= 1 && tokens[colon - 1].text == ":" {
        return None;
    }
    let key = tokens[colon - 1].text.clone();
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((key, &tokens[colon + 1..]))
}

fn class_from_tokens(tokens: &[Token], names: &Names) -> Option<String> {
    let class_kw = tokens.iter().position(|t| t.text == "class")?;
    if class_kw < 2 || tokens[class_kw - 1].text != "::" {
        return None;
    }
    let mut parts = Vec::new();
    for t in &tokens[..class_kw - 1] {
        if t.kind == TokenKind::Ident || t.text == "\\" {
            parts.push(t.text.as_str());
        }
    }
    let raw = parts.concat();
    if raw.eq_ignore_ascii_case("self") || raw.eq_ignore_ascii_case("static") {
        return current_class(names);
    }
    names.resolve(&raw)
}

fn current_class(names: &Names) -> Option<String> {
    names.imports.get("__current_class__").cloned()
}

fn event_keys_from_tokens(tokens: &[Token]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for w in tokens.windows(3) {
        if w[1].text == "::" && w[2].text.starts_with("EVENT_") {
            keys.insert(w[2].text.clone());
        }
    }
    for t in tokens {
        if t.kind == TokenKind::String {
            if let Some(s) = syntax::string(&t.text) {
                if !s.is_empty() {
                    keys.insert(s);
                }
            }
        }
    }
    keys
}

fn handlers_from_tokens(tokens: &[Token], names: &Names) -> Vec<String> {
    let mut out = Vec::new();
    // [Class::class, 'method'] / [$this, 'method']
    if let Some(open) = tokens.iter().position(|t| t.text == "[") {
        if let Some(method) = tokens[open + 1..]
            .iter()
            .find(|t| t.kind == TokenKind::String)
            .and_then(|t| syntax::string(&t.text))
        {
            let class = class_from_tokens(&tokens[open + 1..], names)
                .or_else(|| {
                    tokens[open + 1..].iter().find_map(|t| {
                        if t.kind == TokenKind::Variable && t.text.trim_start_matches('$') == "this"
                        {
                            current_class(names)
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| current_class(names));
            if let Some(class) = class {
                out.push(format!("method:{class}::{method}"));
            }
        }
    }
    // Class::method( inside static fn
    let mut i = 0;
    while i + 3 < tokens.len() {
        if tokens[i + 1].text == "::"
            && tokens[i].kind == TokenKind::Ident
            && tokens[i + 2].kind == TokenKind::Ident
            && tokens.get(i + 3).is_some_and(|t| t.text == "(")
            && !tokens[i + 2].text.starts_with("EVENT_")
            && tokens[i + 2].text != "class"
            && tokens[i + 2].text != "on"
            && tokens[i + 2].text != "off"
        {
            if let Some(class) = names.resolve(&tokens[i].text).or_else(|| {
                if tokens[i].text == "self" || tokens[i].text == "static" {
                    current_class(names)
                } else {
                    None
                }
            }) {
                out.push(format!("method:{class}::{}", tokens[i + 2].text));
            }
        }
        i += 1;
    }
    out.sort();
    out.dedup();
    out
}

fn split_args(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut args = Vec::new();
    let mut cur = Vec::new();
    let mut depth = 0usize;
    for token in tokens {
        if token.kind == TokenKind::Symbol {
            match token.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth = depth.saturating_sub(1),
                "," if depth == 0 => {
                    if !cur.is_empty() {
                        args.push(std::mem::take(&mut cur));
                    }
                    continue;
                }
                _ => {}
            }
        }
        cur.push(token.clone());
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    args
}

fn file_names(tokens: &[Token]) -> Names {
    let mut names = Names::default();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].text == "namespace"
            && tokens
                .get(i + 1)
                .is_some_and(|t| t.kind == TokenKind::Ident)
        {
            let mut parts = Vec::new();
            let mut j = i + 1;
            while j < tokens.len() && tokens[j].text != ";" {
                if tokens[j].kind == TokenKind::Ident || tokens[j].text == "\\" {
                    parts.push(tokens[j].text.as_str());
                }
                j += 1;
            }
            names.namespace = parts.concat().trim_matches('\\').to_owned();
            i = j;
            continue;
        }
        if tokens[i].text == "use" {
            let mut parts = Vec::new();
            let mut alias = None;
            let mut j = i + 1;
            while j < tokens.len() && tokens[j].text != ";" {
                if tokens[j].text == "as"
                    && tokens
                        .get(j + 1)
                        .is_some_and(|t| t.kind == TokenKind::Ident)
                {
                    alias = Some(tokens[j + 1].text.clone());
                    j += 2;
                    continue;
                }
                if tokens[j].kind == TokenKind::Ident || tokens[j].text == "\\" {
                    parts.push(tokens[j].text.clone());
                }
                j += 1;
            }
            let fqcn = parts.concat().trim_matches('\\').to_owned();
            let short = alias
                .unwrap_or_else(|| fqcn.rsplit('\\').next().unwrap_or(fqcn.as_str()).to_owned());
            names.imports.insert(short.to_ascii_lowercase(), fqcn);
            i = j;
            continue;
        }
        if tokens[i].text == "class"
            && tokens
                .get(i + 1)
                .is_some_and(|t| t.kind == TokenKind::Ident)
            && i > 0
            && tokens[i - 1].text != "::"
            && tokens[i - 1].text != "new"
        {
            let short = tokens[i + 1].text.clone();
            let fqcn = if names.namespace.is_empty() {
                short.clone()
            } else {
                format!("{}\\{short}", names.namespace)
            };
            names.imports.insert("__current_class__".into(), fqcn);
        }
        i += 1;
    }
    names
}
