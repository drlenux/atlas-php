use super::{
    doc::{class_string_targets, informative, refine, split_top, DocBlock},
    expression::{self, Expr, Kind},
    syntax::{Class, Function, Unit},
    tokens::{self, TokenKind},
    FlowHost,
};
use crate::domain::Node;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Binding {
    type_name: String,
    exact: bool,
    origins: Vec<Value>,
    /// The type depends on a PHPDoc annotation (may be wrong).
    annotated: bool,
    /// Last assigned string literal, for SQL-in-PHP (`$sql = "SELECT…"; createCommand($sql)`).
    string_value: Option<String>,
}

/// A type together with its provenance. `annotated` is true when any part of the
/// reasoning used PHPDoc; results derived from it are reported as `annotated`.
#[derive(Clone, Debug)]
struct Typed {
    name: String,
    annotated: bool,
    /// The runtime class is exactly this type (not a subclass): no override candidates.
    exact: bool,
}
impl Typed {
    fn code(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            annotated: false,
            exact: false,
        }
    }
    fn unknown() -> Self {
        Self::code("unknown")
    }
    fn tainted(mut self, annotated: bool) -> Self {
        self.annotated |= annotated;
        self
    }
}
type Env = BTreeMap<String, Binding>;
pub(super) fn analyze(project: &mut FlowHost<'_>, function: &Function) -> Result<(), String> {
    analyze_with(project, function, Env::new(), BTreeMap::new())
}

/// Analyzes `function` with variables inherited from an enclosing scope (closure
/// captures) and the enclosing inline `@var` annotations.
fn analyze_with(
    project: &mut FlowHost<'_>,
    function: &Function,
    inherited: Env,
    annotated_vars: BTreeMap<String, String>,
) -> Result<(), String> {
    let unit = project.units[function.unit].clone();
    // Start after the token that opens the body (`{`), so a leading `@var` counts.
    // File-scope bodies start at token 0 (no `{`): begin at the first comment so
    // leading view `@var` annotations are not skipped.
    let comment_cursor = match function.body {
        Some((0, _)) => 0,
        Some((start, _)) => unit
            .tokens
            .get(start.saturating_sub(1))
            .map_or(0, |opening| {
                unit.comments.partition_point(|c| c.start < opening.end)
            }),
        None => usize::MAX,
    };
    let mut scope = Scope {
        project,
        function,
        unit,
        occurrences: BTreeMap::new(),
        annotated_vars,
        comment_cursor,
    };
    let mut env = inherited;
    for param in &function.parameters {
        let origin = json!({"kind":"parameter","span":scope.unit.span(param.token,param.token+1),"expression":scope.unit.text(param.token,param.token+1)});
        env.insert(
            param.name.clone(),
            Binding {
                type_name: scope.class_type(&param.type_name),
                exact: false,
                origins: vec![origin],
                annotated: param.annotated,
                string_value: None,
            },
        );
        scope.variable(
            &param.name,
            param.token,
            param.token + 1,
            &env,
            true,
            None,
            &[],
        )?;
    }
    if let Some(class) = &function.class {
        // Trait methods run on a using class: type `$this` as the union of users.
        let type_name = trait_receiver_type(scope.project.classes, class);
        env.insert(
            "this".into(),
            Binding {
                type_name,
                exact: false,
                origins: vec![
                    json!({"kind":"receiver","symbol_id":scope.project.classes[class].id}),
                ],
                annotated: false,
                string_value: None,
            },
        );
    } else if let Some((start, _)) = function.body {
        // File-scope / scripts: seed `@var` (incl. `$this`) before the first statement.
        scope.read_annotations(start);
        for (name, type_name) in scope.annotated_vars.clone() {
            env.entry(name).or_insert_with(|| Binding {
                type_name: type_name.clone(),
                exact: false,
                origins: vec![json!({"kind":"annotation"})],
                annotated: true,
                string_value: None,
            });
        }
    }
    if let Some((start, end)) = function.body {
        if function.arrow {
            let expr = expression::parse(&scope.unit, start, end)?;
            scope.walk(&expr, &mut env, &[])?;
            scope.returned(start, end, Some(&expr), &env, &[], "arrow_expression");
        } else {
            let mut conditions = Vec::new();
            let terminated = scope.block(start, end, &mut env, &mut conditions, 0)?;
            scope.project.nodes.get_mut(&function.id).unwrap().metadata["implicit_return"] =
                json!(!terminated);
        }
    }
    scope.index_comments()?;
    Ok(())
}
struct Scope<'a, 'b> {
    project: &'a mut FlowHost<'b>,
    function: &'a Function,
    unit: Unit,
    occurrences: BTreeMap<String, usize>,
    /// `/** @var Type $name */` seen so far in the body (fallback types).
    annotated_vars: BTreeMap<String, String>,
    /// Next unread comment of the unit (comments are sorted by position).
    comment_cursor: usize,
}
#[derive(Clone)]
struct Call {
    target: Option<String>,
    candidates: Vec<String>,
    resolution: String,
    receiver_class: String,
    receiver_expression: String,
    receiver_root_class: String,
    dispatcher: Option<String>,
    dynamic: bool,
    /// Resolution relied on a PHPDoc-derived receiver type.
    annotated: bool,
    /// The receiver type (with generic argument) that matched `target`.
    matched: Typed,
    /// When unresolved: a class of the receiver hierarchy that is not indexed.
    unindexed_ancestor: Option<String>,
    /// Every receiver class is indexed and nothing (ancestor, trait, descendant, `__call`)
    /// declares the member: the call cannot succeed at runtime.
    undeclared: bool,
    /// A PHP built-in method `(Class::method, return type)` outside the index.
    builtin: Option<(String, String)>,
}
impl Scope<'_, '_> {
    fn block(
        &mut self,
        start: usize,
        end: usize,
        env: &mut Env,
        conditions: &mut Vec<String>,
        depth: usize,
    ) -> Result<bool, String> {
        if depth > 128 {
            return Err("statement nesting exceeds supported limit".into());
        }
        let (mut i, mut terminated) = (start, false);
        while i < end {
            self.project.options.check(self.project.elements())?;
            let mut current = conditions.clone();
            if terminated {
                current.push("unreachable_after_exit".into());
            }
            if self.unit.is(i, ";") {
                i += 1;
                continue;
            }
            self.read_annotations(i);
            // Nested decls only — do not skip the function currently being analyzed
            // (file-scope bodies start at token 0 and would otherwise no-op).
            if let Some(f) =
                self.project.functions.values().find(|f| {
                    f.id != self.function.id && f.unit == self.function.unit && f.start == i
                })
            {
                i = f.end;
                continue;
            }
            // File-scope analysis: skip class-like declarations already owned by Project.
            if let Some(c) = self
                .project
                .classes
                .values()
                .find(|c| c.unit == self.function.unit && c.start == i)
            {
                i = c.end;
                continue;
            }
            if self.unit.is(i, "namespace") || self.unit.is(i, "use") || self.unit.is(i, "declare")
            {
                if self.unit.is(i, "namespace") {
                    // `namespace Foo;` or `namespace Foo { … }` (body already has classes).
                    let mut j = i + 1;
                    while j < end && !self.unit.is(j, ";") && !self.unit.is(j, "{") {
                        j += 1;
                    }
                    if self.unit.is(j, "{") {
                        i = self.unit.pairs[j].map(|c| c + 1).unwrap_or(j + 1);
                    } else {
                        i = j.saturating_add(1);
                    }
                    continue;
                }
                if self.unit.is(i, "declare") && self.unit.is(i + 1, "(") {
                    let close = self.unit.pairs[i + 1].unwrap_or(i + 1);
                    i = close + 1;
                    if self.unit.is(i, ";") {
                        i += 1;
                    }
                    continue;
                }
                // `use …;`
                i = self.unit.end_statement(i, end).saturating_add(1);
                continue;
            }
            if self.unit.is(i, "return") {
                let stop = self.unit.end_statement(i + 1, end);
                if stop == end {
                    return Err("return statement requires semicolon".into());
                }
                let expr = if i + 1 < stop {
                    Some(expression::parse(&self.unit, i + 1, stop)?)
                } else {
                    None
                };
                if let Some(expr) = &expr {
                    self.walk(expr, env, &current)?;
                }
                self.returned(i, stop + 1, expr.as_ref(), env, &current, "return");
                terminated = true;
                i = stop + 1;
                continue;
            }
            if self.unit.is(i, "if") {
                let mut next = i;
                let mut negative = Vec::new();
                let mut remaining = Vec::new();
                loop {
                    if !self.unit.is(next + 1, "(") {
                        return Err("if condition requires parentheses".into());
                    }
                    let close = self.unit.pairs[next + 1].unwrap();
                    let condition = self.unit.text(next + 2, close);
                    let keyword = if self.unit.is(next, "elseif")
                        || (self.unit.is(next, "else") && self.unit.is(next + 1, "if"))
                    {
                        "elseif"
                    } else {
                        "if"
                    };
                    let expr = expression::parse(&self.unit, next + 2, close)?;
                    let mut branch_env = env.clone();
                    self.walk(&expr, &mut branch_env, &current)?;
                    let (a, b, after) = self.body(close + 1, end)?;
                    let mut branch_conditions =
                        current.iter().chain(&negative).cloned().collect::<Vec<_>>();
                    branch_conditions.push(condition.clone());
                    self.ensure_branch(keyword, next, after, &condition, &branch_conditions)?;
                    if self.block(a, b, &mut branch_env, &mut branch_conditions, depth + 1)? {
                        self.record_guard(&condition, (next, a, b, after), end);
                    } else {
                        remaining.push((branch_env, branch_conditions));
                    }
                    negative.push(format!("!({condition})"));
                    next = after;
                    if self.unit.is(next, "elseif") {
                        continue;
                    }
                    if self.unit.is(next, "else") && self.unit.is(next + 1, "if") {
                        next += 1;
                        continue;
                    }
                    break;
                }
                let mut else_env = env.clone();
                let mut else_conditions =
                    current.iter().chain(&negative).cloned().collect::<Vec<_>>();
                if self.unit.is(next, "else") {
                    let (a, b, after) = self.body(next + 1, end)?;
                    self.ensure_branch("else", next, after, "", &else_conditions)?;
                    if !self.block(a, b, &mut else_env, &mut else_conditions, depth + 1)? {
                        remaining.push((else_env, else_conditions));
                    }
                    next = after;
                } else {
                    remaining.push((else_env, else_conditions));
                }
                if remaining.is_empty() {
                    terminated = true;
                } else {
                    *env = merge(remaining.iter().map(|(e, _)| e));
                    if remaining.len() == 1 {
                        *conditions = remaining.remove(0).1;
                    }
                }
                i = next;
                continue;
            }
            if self.unit.is(i, "unset") {
                if !self.unit.is(i + 1, "(") {
                    return Err("unset requires parentheses".into());
                }
                let close = self.unit.pairs[i + 1].unwrap();
                for (a, b) in self.unit.split(i + 2, close, ",") {
                    if a < b && self.unit.tokens[a].kind == TokenKind::Variable {
                        let name = self.unit.tokens[a].text.trim_start_matches('$').to_string();
                        self.variable(&name, a, b, env, true, None, &current)?;
                        env.remove(&name);
                    }
                }
                i = close + 1;
                if self.unit.is(i, ";") {
                    i += 1;
                }
                continue;
            }
            if ["foreach", "while", "for", "switch"]
                .iter()
                .any(|v| self.unit.is(i, v))
            {
                let keyword = self.unit.tokens[i].text.to_ascii_lowercase();
                if !self.unit.is(i + 1, "(") {
                    return Err(format!("{keyword} requires parentheses"));
                }
                let close = self.unit.pairs[i + 1].unwrap();
                let (a, b, next) = self.body(close + 1, end)?;
                let mut loop_env = env.clone();
                let mut loop_conditions = current.clone();
                loop_conditions.push(format!("{keyword} ({})", self.unit.text(i + 2, close)));
                if keyword == "foreach" {
                    let as_token = (i + 2..close)
                        .find(|p| self.unit.is(*p, "as"))
                        .ok_or("foreach requires as")?;
                    let expr = expression::parse(&self.unit, i + 2, as_token)?;
                    self.walk(&expr, &mut loop_env, &current)?;
                    let iterated = self.typed(&expr, &loop_env);
                    let element = Typed {
                        name: element_type(&iterated.name),
                        annotated: iterated.annotated,
                        exact: false,
                    };
                    let arrow = (as_token + 1..close).find(|p| self.unit.is(*p, "=>"));
                    let value_from = arrow.map_or(as_token + 1, |a| a + 1);
                    for p in as_token + 1..close {
                        if self.unit.tokens[p].kind == TokenKind::Variable {
                            let name = self.unit.tokens[p].text.trim_start_matches('$').to_string();
                            let value = p >= value_from && element.name != "unknown";
                            loop_env.insert(name.clone(),Binding{type_name:if value {element.name.clone()} else {"unknown".into()},exact:false,origins:vec![json!({"kind":"iteration","span":self.unit.span(i+2,as_token),"expression":self.unit.text(i+2,as_token)})],annotated:value && element.annotated,string_value:None});
                            self.variable(&name, p, p + 1, &loop_env, true, None, &current)?;
                        }
                    }
                } else {
                    for (x, y) in self.unit.split(i + 2, close, ";") {
                        if x < y {
                            let expr = expression::parse(&self.unit, x, y)?;
                            self.walk(&expr, &mut loop_env, &current)?;
                        }
                    }
                }
                self.block(a, b, &mut loop_env, &mut loop_conditions, depth + 1)?;
                *env = merge([&*env, &loop_env]);
                i = next;
                continue;
            }
            if self.unit.is(i, "case") || self.unit.is(i, "default") {
                // PHP allows `case EXPR:` ; some project code uses `case EXPR;` instead.
                let mut stop = i + 1;
                while stop < end && !self.unit.is(stop, ":") && !self.unit.is(stop, ";") {
                    stop += 1;
                }
                if stop == end {
                    return Err("case requires colon or semicolon".into());
                }
                // Each case/default is a fresh entry of the switch jump table: a prior
                // `return`/`break` must not mark this arm unreachable_after_exit.
                terminated = false;
                conditions.push(self.unit.text(i, stop));
                i = stop + 1;
                continue;
            }
            if self.unit.is(i, "try") || self.unit.is(i, "finally") || self.unit.is(i, "do") {
                let kind = self.unit.tokens[i].text.clone();
                let (a, b, next) = self.body(i + 1, end)?;
                let mut nested_env = env.clone();
                let mut nested_conditions = current.clone();
                nested_conditions.push(kind.clone());
                let exits = self.block(a, b, &mut nested_env, &mut nested_conditions, depth + 1)?;
                *env = merge([&*env, &nested_env]);
                if kind.eq_ignore_ascii_case("finally") && exits {
                    terminated = true;
                }
                i = next;
                continue;
            }
            if self.unit.is(i, "catch") {
                if !self.unit.is(i + 1, "(") {
                    return Err("catch requires parentheses".into());
                }
                let close = self.unit.pairs[i + 1].unwrap();
                let (a, b, next) = self.body(close + 1, end)?;
                let mut catch_env = env.clone();
                for token in i + 2..close {
                    if self.unit.tokens[token].kind == TokenKind::Variable {
                        let name = self.unit.tokens[token]
                            .text
                            .trim_start_matches('$')
                            .to_string();
                        let type_name = self
                            .function
                            .names
                            .type_name(&self.unit.text(i + 2, token).replace(' ', ""));
                        catch_env.insert(
                            name.clone(),
                            Binding {
                                type_name,
                                exact: false,
                                origins: Vec::new(),
                                annotated: false,
                                string_value: None,
                            },
                        );
                        self.variable(&name, token, token + 1, &catch_env, true, None, &current)?;
                    }
                }
                let mut nested = current.clone();
                nested.push(format!("catch {}", self.unit.text(i + 2, close)));
                self.block(a, b, &mut catch_env, &mut nested, depth + 1)?;
                *env = merge([&*env, &catch_env]);
                i = next;
                continue;
            }
            if self.unit.is(i, "{") {
                let close = self.unit.pairs[i].unwrap();
                if self.block(i + 1, close, env, &mut current, depth + 1)? {
                    terminated = true;
                }
                i = close + 1;
                continue;
            }
            if ["break", "continue"].iter().any(|v| self.unit.is(i, v)) {
                // Leaves the current switch arm / loop body; following statements in
                // this sequential scan are dead until the next `case`/`default`.
                terminated = true;
                i = self.unit.end_statement(i, end).saturating_add(1);
                continue;
            }
            let begin = if self.unit.is(i, "echo") || self.unit.is(i, "print") {
                i + 1
            } else {
                i
            };
            let stop = self.unit.end_statement(i, end);
            // Views / short-echo `<?= … ?>` often omit `;` before `?>` (no token).
            // Accept a trailing expression at the end of the analyzed span.
            let (expr_end, advance) = if stop < end {
                (stop, stop + 1)
            } else if begin < end && expression::parse(&self.unit, begin, end).is_ok() {
                (end, end)
            } else {
                return Err(format!(
                    "statement is missing semicolon at line {}",
                    self.unit.tokens[i].line
                ));
            };
            if begin < expr_end {
                let expr = expression::parse(&self.unit, begin, expr_end)?;
                self.walk(&expr, env, &current)?;
                if self.unit.is(i, "throw") || self.unit.is(i, "exit") {
                    terminated = true;
                }
            }
            i = advance;
        }
        Ok(terminated)
    }
    /// Stores an early-exit `if` branch as `metadata.guards` of the analyzed function, so
    /// callers can show the checks that precede a call (`isLockTakeBonuses` before
    /// `createFreeSpin`). `span` is `(keyword, body start, body end, after statement)`;
    /// `scope_end` is the end of the enclosing block: the guard applies to later sites
    /// inside that block only.
    fn record_guard(
        &mut self,
        condition: &str,
        span: (usize, usize, usize, usize),
        scope_end: usize,
    ) {
        let (keyword, body_start, body_end, after) = span;
        let tokens = &self.unit.tokens;
        let (Some(first), Some(last)) = (tokens.get(keyword), tokens.get(after.saturating_sub(1)))
        else {
            return;
        };
        let scope_line = tokens
            .get(scope_end)
            .or(tokens.last())
            .map_or(last.line, |t| t.line);
        let guard = json!({
            "line": first.line,
            "end_line": last.line,
            "scope_end_line": scope_line,
            "condition": clip_text(condition.split_whitespace().collect::<Vec<_>>().join(" "), GUARD_CONDITION_MAX),
            "exit": self.exit_kind(body_start, body_end),
        });
        let Some(node) = self.project.nodes.get_mut(&self.function.id) else {
            return;
        };
        if !node.metadata["guards"].is_array() {
            node.metadata["guards"] = json!([]);
        }
        if let Some(guards) = node.metadata["guards"].as_array_mut() {
            if guards.len() < GUARDS_PER_FUNCTION {
                guards.push(guard);
            }
        }
    }

    /// Statement-level exit keyword of a terminating branch (nested blocks, calls and
    /// closures are skipped); `exit` when the branch ends through nested control flow.
    fn exit_kind(&self, start: usize, end: usize) -> &'static str {
        let mut p = start;
        while p < end {
            if matches!(self.unit.tokens[p].text.as_str(), "{" | "(" | "[") {
                if let Some(close) = self.unit.pairs[p] {
                    p = close + 1;
                    continue;
                }
            }
            match self.unit.tokens[p].text.to_ascii_lowercase().as_str() {
                "return" => return "return",
                "throw" => return "throw",
                "continue" => return "continue",
                "break" => return "break",
                "exit" | "die" => return "exit",
                _ => {}
            }
            p += 1;
        }
        "exit"
    }
    fn body(&self, start: usize, end: usize) -> Result<(usize, usize, usize), String> {
        if self.unit.is(start, "{") {
            let close = self.unit.pairs[start].ok_or("unclosed statement block")?;
            Ok((start + 1, close, close + 1))
        } else if self.unit.is(start, ":") {
            Err("alternative PHP control-flow syntax is unsupported".into())
        } else {
            let stop = self.unit.end_statement(start, end);
            if stop == end {
                return Err("missing statement body".into());
            }
            Ok((start, stop + 1, stop + 1))
        }
    }
    fn walk(&mut self, expr: &Expr, env: &mut Env, conditions: &[String]) -> Result<(), String> {
        self.walk_expr(expr, env, conditions, true)
    }
    fn walk_expr(
        &mut self,
        expr: &Expr,
        env: &mut Env,
        conditions: &[String],
        diagnose_unknown: bool,
    ) -> Result<(), String> {
        self.project.options.check(self.project.elements())?;
        match &expr.kind {
            Kind::Variable(name) => {
                self.variable(name, expr.start, expr.end, env, false, None, conditions)?
            }
            Kind::Assign(lhs, rhs) => {
                self.walk_expr(rhs, env, conditions, true)?;
                self.assign_target(lhs, rhs, expr, env, conditions)?;
            }
            Kind::CompoundAssign(op, lhs, rhs) => {
                self.walk_expr(rhs, env, conditions, true)?;
                if let Kind::Variable(name) = &lhs.kind {
                    let (left, right) = (self.typed(lhs, env), self.typed(rhs, env));
                    let annotated = left.annotated || right.annotated;
                    let type_name = union([left.name.as_str(), right.name.as_str()]);
                    let mut origins = vec![
                        json!({"kind":"assignment","operator":op,"span":self.unit.span(expr.start,expr.end),"expression":self.unit.text(expr.start,expr.end)}),
                    ];
                    origins.extend(self.origins(rhs, env));
                    env.insert(
                        name.clone(),
                        Binding {
                            type_name,
                            exact: false,
                            origins,
                            annotated,
                            string_value: None,
                        },
                    );
                    self.variable(name, lhs.start, lhs.end, env, true, Some(rhs), conditions)?;
                    let id = format!("variable:{}:${name}", self.function.id);
                    if let Some(last) = self
                        .project
                        .nodes
                        .get_mut(&id)
                        .and_then(|node| node.metadata["assignments"].as_array_mut())
                        .and_then(|items| items.last_mut())
                    {
                        last["expression"] = json!(self.unit.text(expr.start, expr.end));
                        last["span"] = json!(self.unit.span(expr.start, expr.end));
                        last["operator"] = json!(op);
                    }
                } else {
                    self.assign_target(lhs, rhs, expr, env, conditions)?;
                }
            }
            Kind::Call {
                receiver,
                args,
                member,
                ..
            } => {
                if let Some(receiver) = receiver {
                    self.walk_expr(receiver, env, conditions, true)?;
                }
                for arg in args {
                    self.walk_expr(arg, env, conditions, true)?;
                }
                let call = self.resolve(expr, env);
                // Argument types let framework plugins follow values passed to calls
                // (e.g. a job pushed to a queue) without re-analyzing expressions.
                let labeled: Vec<Option<String>> = args
                    .iter()
                    .map(|arg| argument_label(&self.unit, arg))
                    .collect();
                // Named args / array keys are what callers-output must keep when the
                // expression is clipped, so such calls also store clipped values.
                let structured = labeled.iter().any(Option::is_some)
                    || args.iter().any(|arg| matches!(&arg.kind, Kind::Array(items) if items.iter().any(|i| i.key.is_some())));
                let arguments: Vec<Value> = args
                    .iter()
                    .zip(labeled)
                    .map(|(arg, label)| {
                        let typed = self.typed(arg, env);
                        let mut value = json!({
                            "type": typed.name,
                            "type_source": super::type_source(typed.annotated),
                        });
                        // `...$args` expands to an unknown number of arguments.
                        if matches!(&arg.kind, Kind::Unary(op, _) if op == "...") {
                            value["spread"] = json!(true);
                        }
                        if structured {
                            value["value"] = json!(clip_text(self.unit.text(arg.start, arg.end), ARG_VALUE_MAX));
                            if let Some(label) = label {
                                value["name"] = json!(label);
                            }
                        }
                        // Array literal keys+value types (render params, config maps).
                        if let Kind::Array(items) = &arg.kind {
                            let items: Vec<Value> = items
                                .iter()
                                .filter_map(|item| {
                                    let name =
                                        item.key.as_ref().and_then(|k| self.expr_string(k))?;
                                    let typed = self.typed(&item.value, env);
                                    Some(json!({
                                        "name": name,
                                        "value": clip_text(self.unit.text(item.value.start, item.value.end), ARG_VALUE_MAX),
                                        "type": typed.name,
                                        "type_source": super::type_source(typed.annotated),
                                    }))
                                })
                                .collect();
                            if !items.is_empty() {
                                value["items"] = json!(items);
                            }
                        }
                        value
                    })
                    .collect();
                let mut metadata = json!({"arguments":arguments,"expression":clip_expr(self.unit.text(expr.start,expr.end)),"conditions":conditions,"resolution":call.resolution,"type_source":super::type_source(call.annotated),"candidates":call.candidates,"receiver_class":call.receiver_class,"receiver_expression":clip_expr(call.receiver_expression.clone()),"receiver_root_class":call.receiver_root_class,"member_name":member,"operation":"call","dispatcher":call.dispatcher,"unindexed_ancestor":call.unindexed_ancestor,"builtin":call.builtin.as_ref().map(|(name, _)| name)});
                if structured {
                    if let Some(open) = self.unit.pairs[expr.end - 1].filter(|o| *o < expr.end) {
                        metadata["callee"] = json!(clip_expr(self.unit.text(expr.start, open)));
                    }
                }
                if call.undeclared {
                    metadata["undeclared"] = json!(true);
                }
                // Candidate unions (if/else receivers) must still appear in called_by:
                // emit one calls edge per target, reusing the same call_site.
                let mut targets: Vec<Option<String>> = Vec::new();
                if let Some(target) = &call.target {
                    targets.push(Some(target.clone()));
                }
                for candidate in &call.candidates {
                    if !targets
                        .iter()
                        .any(|t| t.as_deref() == Some(candidate.as_str()))
                    {
                        targets.push(Some(candidate.clone()));
                    }
                }
                if targets.is_empty() {
                    targets.push(None);
                }
                for target in targets {
                    self.emit_effect(
                        "calls",
                        "call_site",
                        target,
                        expr.start,
                        expr.end,
                        metadata.clone(),
                        conditions,
                    )?;
                }
                // Mass-assignment helpers: updateAttributes(['field'=>…]) → write_site per key.
                self.mass_assignment_writes(
                    member,
                    args,
                    expr,
                    &call.receiver_class,
                    &call.receiver_expression,
                    env,
                    conditions,
                )?;
                // Query Builder / AR: where/select/batchInsert → attribute/column access.
                self.query_builder_fields(
                    member,
                    args,
                    expr,
                    &call.receiver_class,
                    &call.receiver_expression,
                    env,
                    conditions,
                )?;
                // Raw SQL in Yii/PDO-style APIs → sql_site + table reads/writes.
                self.emit_embedded_sql(member, args, expr, env)?;
                if call.dynamic {
                    self.project.diagnostic(
                        self.function.unit,
                        expr.start,
                        "dynamic_call",
                        format!(
                            "Cannot resolve dynamic call {}",
                            self.unit.text(expr.start, expr.end)
                        ),
                    );
                } else if call.resolution == "candidate" {
                    self.project.diagnostic(
                        self.function.unit,
                        expr.start,
                        "ambiguous_dispatch",
                        "Runtime implementation is not established".into(),
                    );
                } else if call.undeclared {
                    self.project.diagnostic(
                        self.function.unit,
                        expr.start,
                        "undeclared_member",
                        format!(
                            "Method {}::{member}() is not declared in the class, its ancestors, traits or descendants: {}",
                            erase(class_parts(&call.receiver_class).first().map(String::as_str).unwrap_or("?")),
                            self.unit.text(expr.start, expr.end)
                        ),
                    );
                } else if call.resolution == "unresolved" {
                    self.project.diagnostic(
                        self.function.unit,
                        expr.start,
                        "unresolved_call",
                        match &call.unindexed_ancestor {
                            Some(class) => format!(
                                "Cannot resolve call {}: class {class} of the receiver hierarchy is not in the index",
                                self.unit.text(expr.start, expr.end)
                            ),
                            None => format!(
                                "Cannot resolve call {}",
                                self.unit.text(expr.start, expr.end)
                            ),
                        },
                    );
                }
            }
            Kind::Property {
                receiver,
                member,
                dynamic,
                static_,
                nullsafe,
            } => self.property(
                expr, receiver, member, *dynamic, *static_, *nullsafe, false, env, conditions, None,
            )?,
            Kind::Index { base, index } => {
                self.walk_expr(base, env, conditions, true)?;
                if let Some(index) = index {
                    self.walk_expr(index, env, conditions, true)?;
                    self.attributes_bag_access(base, index, false, env, conditions, None)?;
                    self.model_array_access(base, index, false, env, conditions, None)?;
                }
            }
            Kind::Cast { expr: inner, .. } => self.walk_expr(inner, env, conditions, true)?,
            Kind::Ternary {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(condition, env, conditions, true)?;
                let cond_text = self.unit.text(condition.start, condition.end);
                if let Some(then_branch) = then_branch {
                    let mut then_conditions = conditions.to_vec();
                    then_conditions.push(cond_text.clone());
                    self.walk_expr(then_branch, env, &then_conditions, true)?;
                    let mut else_conditions = conditions.to_vec();
                    else_conditions.push(format!("!({cond_text})"));
                    self.walk_expr(else_branch, env, &else_conditions, true)?;
                } else {
                    let mut else_conditions = conditions.to_vec();
                    else_conditions.push(format!("!({cond_text})"));
                    self.walk_expr(else_branch, env, &else_conditions, true)?;
                }
            }
            Kind::New(class, args) => {
                for arg in args {
                    self.walk_expr(arg, env, conditions, true)?;
                }
                let constructed = self.constructed_type(class, env);
                for part in class_parts(&constructed.name) {
                    let class = erase(&part).to_owned();
                    if let Some(target) =
                        self.project
                            .lookup_method(&class, "__construct", &mut BTreeSet::new())
                    {
                        self.emit_effect(
                            "calls",
                            "call_site",
                            Some(target),
                            expr.start,
                            expr.end,
                            json!({
                                "construction": true,
                                "expression": self.unit.text(expr.start, expr.end),
                                "receiver_class": class,
                                "member_name": "__construct",
                                "operation": "call",
                                "type_source": super::type_source(constructed.annotated)
                            }),
                            conditions,
                        )?;
                    }
                }
            }
            Kind::Array(items) => {
                // PHP callable arrays: [$this,'m'], [self::class,'m'], [Foo::class,'m']
                if let Some((target, meta)) = self.callable_array_call(items, expr, env, conditions)
                {
                    self.emit_effect(
                        "calls",
                        "call_site",
                        Some(target),
                        expr.start,
                        expr.end,
                        meta,
                        conditions,
                    )?;
                }
                for item in items {
                    if let Some(key) = &item.key {
                        self.walk_expr(key, env, conditions, true)?;
                    }
                    self.walk_expr(&item.value, env, conditions, true)?;
                }
            }
            Kind::Match { subject, arms } => {
                self.walk_expr(subject, env, conditions, true)?;
                for arm in arms {
                    self.walk_expr(arm, env, conditions, true)?;
                }
            }
            Kind::Unknown(children) => {
                for child in children {
                    self.walk_expr(child, env, conditions, false)?;
                }
                if diagnose_unknown {
                    self.project.diagnostic(
                        self.function.unit,
                        expr.start,
                        "analysis_limitation",
                        format!(
                            "Expression semantics are unknown: {}",
                            self.unit.text(expr.start, expr.end)
                        ),
                    );
                }
            }
            Kind::Binary(_, a, b) => {
                self.walk_expr(a, env, conditions, true)?;
                self.walk_expr(b, env, conditions, true)?;
            }
            Kind::Unary(_, e) => self.walk_expr(e, env, conditions, true)?,
            Kind::Closure(start) => self.closure(*start, env)?,
            Kind::Literal(_) | Kind::Name(_) => {}
        }
        Ok(())
    }
    fn assign_target(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        assign: &Expr,
        env: &mut Env,
        conditions: &[String],
    ) -> Result<(), String> {
        if let Kind::Variable(name) = &lhs.kind {
            let Typed {
                name: type_name,
                annotated,
                exact: typed_exact,
            } = self.typed(rhs, env);
            let exact = typed_exact
                || matches!(&rhs.kind, Kind::New(..))
                || matches!(&rhs.kind,Kind::Variable(n)if env.get(n).is_some_and(|b|b.exact));
            let mut origins = vec![
                json!({"kind":"assignment","span":self.unit.span(assign.start,assign.end),"expression":self.unit.text(assign.start,assign.end)}),
            ];
            origins.extend(self.origins(rhs, env));
            env.insert(
                name.clone(),
                Binding {
                    type_name,
                    exact,
                    origins,
                    annotated,
                    string_value: self.expr_string(rhs),
                },
            );
            self.variable(name, lhs.start, lhs.end, env, true, Some(rhs), conditions)?;
        } else if let Kind::Property {
            receiver,
            member,
            dynamic,
            static_,
            nullsafe,
        } = &lhs.kind
        {
            self.property(
                lhs,
                receiver,
                member,
                *dynamic,
                *static_,
                *nullsafe,
                true,
                env,
                conditions,
                Some(rhs),
            )?;
        } else if let Kind::Index { base, index } = &lhs.kind {
            self.walk_expr(base, env, conditions, true)?;
            if let Some(index) = index {
                self.walk_expr(index, env, conditions, true)?;
                self.attributes_bag_access(base, index, true, env, conditions, Some(rhs))?;
                self.model_array_access(base, index, true, env, conditions, Some(rhs))?;
            }
        } else {
            self.walk_expr(lhs, env, conditions, true)?;
        }
        Ok(())
    }
    // Keep graph evidence and its source context explicit at the insertion boundary.
    #[allow(clippy::too_many_arguments)]
    fn property(
        &mut self,
        expr: &Expr,
        receiver: &Expr,
        member: &str,
        dynamic: bool,
        static_: bool,
        nullsafe: bool,
        write: bool,
        env: &mut Env,
        conditions: &[String],
        assigned: Option<&Expr>,
    ) -> Result<(), String> {
        self.walk_expr(receiver, env, conditions, true)?;
        if static_ && !member.starts_with('$') {
            return self.constant(expr, receiver, member, env, conditions);
        }
        let receiver_type = self.receiver_type(receiver, static_, env);
        let class = receiver_type.name.clone();
        let name = member.trim_start_matches('$');
        let target = if dynamic {
            None
        } else {
            class_parts(&class).iter().find_map(|part| {
                self.project
                    .lookup_property(erase(part), name, &mut BTreeSet::new())
            })
        };
        let target_metadata = target
            .as_ref()
            .map(|id| self.project.nodes[id].metadata.clone());
        let annotated = receiver_type.annotated
            || target_metadata
                .as_ref()
                .is_some_and(|m| m["type_source"] == "annotation");
        let framework = target_metadata
            .as_ref()
            .filter(|m| m["type_source"] == "framework");
        let dispatcher = if target.is_none() {
            class_parts(&class).iter().find_map(|part| {
                self.project.lookup_method(
                    erase(part),
                    if write { "__set" } else { "__get" },
                    &mut BTreeSet::new(),
                )
            })
        } else {
            None
        };
        let kind = if target.is_some() {
            if write {
                "writes"
            } else {
                "reads"
            }
        } else {
            "property_access"
        };
        let site_kind = if write { "write_site" } else { "read_site" };
        let mut metadata = json!({"receiver_class":class,"receiver_expression":clip_expr(self.unit.text(receiver.start,receiver.end)),"receiver_root_class":self.root_class(receiver),"member_name":name,"operation":if write{"write"}else{"read"},"nullsafe":nullsafe,"expression":clip_expr(self.unit.text(expr.start,expr.end)),"conditions":conditions,"dispatcher":dispatcher,"type_source":super::type_source(annotated)});
        if write {
            if let Some(rhs) = assigned {
                self.attach_assigned_value(&mut metadata, rhs, env);
            }
        }
        if target.is_some() {
            metadata["resolution"] = json!(if annotated { "annotated" } else { "resolved" });
        }
        if let (Some(member), false) = (framework, annotated) {
            metadata["type_source"] = json!("framework");
            metadata["framework"] = member["framework"].clone();
            metadata["framework_evidence"] = member["framework_evidence"].clone();
        }
        self.emit_effect(
            kind,
            site_kind,
            target.clone(),
            expr.start,
            expr.end,
            metadata,
            conditions,
        )?;
        // Yii AR relation property `$model->bonus` is implemented by getBonus(); surface as calls
        // so outline called_by on the getter is non-empty.
        if !write {
            if let Some(getter) = target_metadata
                .as_ref()
                .and_then(|m| m["implemented_by"].as_str())
                .filter(|id| id.starts_with("method:"))
            {
                let meta = json!({
                    "expression": self.unit.text(expr.start, expr.end),
                    "conditions": conditions,
                    "resolution": "resolved",
                    "type_source": "framework",
                    "framework": "yii2",
                    "framework_evidence": "relation_property",
                    "member_name": name,
                    "operation": "call",
                    "receiver_class": class,
                    "receiver_expression": self.unit.text(receiver.start, receiver.end),
                    "receiver_root_class": self.root_class(receiver),
                });
                self.emit_effect(
                    "calls",
                    "call_site",
                    Some(getter.to_owned()),
                    expr.start,
                    expr.end,
                    meta,
                    conditions,
                )?;
            }
        }
        if dynamic {
            self.project.diagnostic(
                self.function.unit,
                expr.start,
                "dynamic_property",
                "Dynamic property name cannot be resolved".into(),
            );
        }
        Ok(())
    }

    /// `[$this,'method']` / `[self::class,'method']` / `[Foo::class,'method']` → calls edge.
    fn callable_array_call(
        &self,
        items: &[expression::ArrayItem],
        expr: &Expr,
        env: &Env,
        conditions: &[String],
    ) -> Option<(String, Value)> {
        if items.len() != 2 {
            return None;
        }
        // String literals are stored as Kind::Literal("string"); read source text.
        if !matches!(&items[1].value.kind, Kind::Literal(_)) {
            return None;
        }
        let method = literal_string(&self.unit.text(items[1].value.start, items[1].value.end))?;
        if method.is_empty() || method.starts_with('$') {
            return None;
        }
        let class = self.callable_array_class(&items[0].value, env)?;
        let target = self
            .project
            .lookup_method(&class, &method, &mut BTreeSet::new())?;
        let meta = json!({
            "expression": self.unit.text(expr.start, expr.end),
            "conditions": conditions,
            "resolution": "resolved",
            "type_source": "code",
            "member_name": method,
            "operation": "call",
            "receiver_class": class,
            "receiver_expression": self.unit.text(items[0].value.start, items[0].value.end),
            "framework_evidence": "callable_array",
        });
        Some((target, meta))
    }

    fn callable_array_class(&self, expr: &Expr, env: &Env) -> Option<String> {
        match &expr.kind {
            Kind::Variable(name) if name == "this" => {
                let class = env.get("this")?.type_name.clone();
                let part = class_parts(&class).into_iter().next()?;
                Some(erase(&part).to_owned())
            }
            Kind::Property {
                receiver,
                member,
                static_: true,
                ..
            } if member.eq_ignore_ascii_case("class") => {
                let Kind::Name(name) = &receiver.kind else {
                    return None;
                };
                let resolved = self.resolve_class(name);
                informative(&resolved).then_some(resolved)
            }
            Kind::Name(name) => {
                let resolved = self.resolve_class(name);
                informative(&resolved).then_some(resolved)
            }
            _ => None,
        }
    }

    /// `updateAttributes(['a'=>1])` / `setAttributes([...])` / `load(['Model'=>['a'=>1]])`
    /// / `setAttribute('a', …)` → write_site edges for attribute names.
    #[allow(clippy::too_many_arguments)]
    fn mass_assignment_writes(
        &mut self,
        member: &str,
        args: &[Expr],
        call: &Expr,
        receiver_class: &str,
        receiver_expression: &str,
        env: &Env,
        conditions: &[String],
    ) -> Result<(), String> {
        let member_l = member.to_ascii_lowercase();
        let fields: Vec<(String, usize, usize, Option<Expr>)> = match member_l.as_str() {
            "setattribute" => {
                let Some(name_expr) = args.first() else {
                    return Ok(());
                };
                let Some(name) = self.expr_string(name_expr) else {
                    return Ok(());
                };
                let value = args.get(1).cloned();
                vec![(name, name_expr.start, name_expr.end, value)]
            }
            "updateattributes" | "setattributes" | "updateall" => {
                let Some(Kind::Array(items)) = args.first().map(|a| &a.kind) else {
                    return Ok(());
                };
                self.array_field_writes(items)
            }
            "load" | "loadmultiple" => {
                // load($data) / load($data, 'FormName') — literal keys; else all known attrs.
                let Some(first) = args.first() else {
                    return Ok(());
                };
                if let Kind::Array(items) = &first.kind {
                    let mut fields = self.array_field_writes(items);
                    for item in items {
                        if let Kind::Array(nested) = &item.value.kind {
                            fields.extend(self.array_field_writes(nested));
                        }
                    }
                    if !fields.is_empty() {
                        fields
                    } else {
                        return self.emit_known_attribute_writes(
                            member,
                            call,
                            &if informative(receiver_class) {
                                receiver_class.to_owned()
                            } else {
                                "unknown".into()
                            },
                            receiver_expression,
                            conditions,
                            "load",
                        );
                    }
                } else {
                    return self.emit_known_attribute_writes(
                        member,
                        call,
                        &if informative(receiver_class) {
                            receiver_class.to_owned()
                        } else {
                            "unknown".into()
                        },
                        receiver_expression,
                        conditions,
                        "load",
                    );
                }
            }
            "save" | "insert" | "upsert" => {
                return self.emit_known_attribute_writes(
                    member,
                    call,
                    &if informative(receiver_class) {
                        receiver_class.to_owned()
                    } else {
                        "unknown".into()
                    },
                    receiver_expression,
                    conditions,
                    member_l.as_str(),
                );
            }
            "delete" => {
                // Known attrs as reads (row identity used by delete).
                return self.emit_known_attribute_access(
                    member,
                    call,
                    &if informative(receiver_class) {
                        receiver_class.to_owned()
                    } else {
                        "unknown".into()
                    },
                    receiver_expression,
                    conditions,
                    "delete",
                    false,
                );
            }
            _ => return Ok(()),
        };
        let receiver_class = if informative(receiver_class) {
            receiver_class.to_owned()
        } else {
            "unknown".into()
        };
        for (field, start, end, value) in fields {
            if !identifier_name(&field) {
                continue;
            }
            let target = class_parts(&receiver_class).iter().find_map(|part| {
                self.project
                    .lookup_property(erase(part), &field, &mut BTreeSet::new())
            });
            let recv_expr = if receiver_expression.is_empty() {
                "$this"
            } else {
                receiver_expression
            };
            let mut meta = json!({
                "expression": format!("{field} via {member}"),
                "conditions": conditions,
                "member_name": field,
                "operation": "write",
                "receiver_class": receiver_class,
                "receiver_expression": recv_expr,
                "resolution": if target.is_some() { "resolved" } else { "unresolved" },
                "framework_evidence": "mass_assignment",
                "call_expression": self.unit.text(call.start, call.end),
            });
            if let Some(rhs) = &value {
                self.attach_assigned_value(&mut meta, rhs, env);
            }
            self.emit_effect(
                if target.is_some() {
                    "writes"
                } else {
                    "property_access"
                },
                "write_site",
                target,
                start,
                end,
                meta,
                conditions,
            )?;
        }
        Ok(())
    }

    /// `$model->attributes['field']` / `$model->attributes["field"] = …`
    fn attributes_bag_access(
        &mut self,
        base: &Expr,
        index: &Expr,
        write: bool,
        env: &Env,
        conditions: &[String],
        assigned: Option<&Expr>,
    ) -> Result<(), String> {
        let Kind::Property {
            receiver,
            member,
            dynamic,
            ..
        } = &base.kind
        else {
            return Ok(());
        };
        if *dynamic || !member.eq_ignore_ascii_case("attributes") {
            return Ok(());
        }
        let Some(field) = self.expr_string(index) else {
            return Ok(());
        };
        if !identifier_name(&field) {
            return Ok(());
        }
        let receiver_type = self.receiver_type(receiver, false, env);
        self.emit_attribute_effect(
            &field,
            &receiver_type.name,
            &self.unit.text(receiver.start, receiver.end),
            write,
            index.start,
            index.end,
            conditions,
            assigned,
            env,
            "attributes_bag",
        )
    }

    /// `$model['field']` ArrayAccess on ActiveRecord/Model.
    fn model_array_access(
        &mut self,
        base: &Expr,
        index: &Expr,
        write: bool,
        env: &Env,
        conditions: &[String],
        assigned: Option<&Expr>,
    ) -> Result<(), String> {
        // Only bare receivers (variable/property), not `$x->attributes[...]` (handled above).
        if matches!(&base.kind, Kind::Property { member, .. } if member.eq_ignore_ascii_case("attributes"))
        {
            return Ok(());
        }
        let Some(field) = self.expr_string(index) else {
            return Ok(());
        };
        if !identifier_name(&field) {
            return Ok(());
        }
        let receiver_type = match &base.kind {
            Kind::Variable(_) | Kind::Property { .. } | Kind::Call { .. } => {
                self.typed(base, env).name
            }
            _ => return Ok(()),
        };
        if !informative(&receiver_type) {
            return Ok(());
        }
        // Only when the field is a known (possibly framework) property of the class.
        let known = class_parts(&receiver_type).iter().any(|part| {
            self.project
                .lookup_property(erase(part), &field, &mut BTreeSet::new())
                .is_some()
        });
        if !known {
            return Ok(());
        }
        self.emit_attribute_effect(
            &field,
            &receiver_type,
            &self.unit.text(base.start, base.end),
            write,
            index.start,
            index.end,
            conditions,
            assigned,
            env,
            "array_access",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_attribute_effect(
        &mut self,
        field: &str,
        receiver_class: &str,
        receiver_expression: &str,
        write: bool,
        start: usize,
        end: usize,
        conditions: &[String],
        assigned: Option<&Expr>,
        env: &Env,
        evidence: &str,
    ) -> Result<(), String> {
        let target = class_parts(receiver_class).iter().find_map(|part| {
            self.project
                .lookup_property(erase(part), field, &mut BTreeSet::new())
        });
        let kind = if target.is_some() {
            if write {
                "writes"
            } else {
                "reads"
            }
        } else {
            "property_access"
        };
        let site_kind = if write { "write_site" } else { "read_site" };
        let mut meta = json!({
            "expression": format!("{field} via {evidence}"),
            "conditions": conditions,
            "member_name": field,
            "operation": if write { "write" } else { "read" },
            "receiver_class": receiver_class,
            "receiver_expression": clip_expr(receiver_expression.to_owned()),
            "resolution": if target.is_some() { "resolved" } else { "unresolved" },
            "framework_evidence": evidence,
        });
        if write {
            if let Some(rhs) = assigned {
                self.attach_assigned_value(&mut meta, rhs, env);
            }
        }
        self.emit_effect(kind, site_kind, target, start, end, meta, conditions)
    }

    /// `load($post)` / `save()` without a literal map → site per known model attribute.
    fn emit_known_attribute_writes(
        &mut self,
        member: &str,
        call: &Expr,
        receiver_class: &str,
        receiver_expression: &str,
        conditions: &[String],
        evidence: &str,
    ) -> Result<(), String> {
        self.emit_known_attribute_access(
            member,
            call,
            receiver_class,
            receiver_expression,
            conditions,
            evidence,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_known_attribute_access(
        &mut self,
        member: &str,
        call: &Expr,
        receiver_class: &str,
        receiver_expression: &str,
        conditions: &[String],
        evidence: &str,
        write: bool,
    ) -> Result<(), String> {
        let mut fields = BTreeSet::new();
        for part in class_parts(receiver_class) {
            let key = erase(&part);
            let Some(class) = self.project.classes.get(key) else {
                continue;
            };
            for prop in &class.properties {
                if identifier_name(&prop.name) {
                    fields.insert(prop.name.clone());
                }
            }
        }
        let recv_expr = if receiver_expression.is_empty() {
            "$this"
        } else {
            receiver_expression
        };
        for field in fields {
            let target = class_parts(receiver_class).iter().find_map(|part| {
                self.project
                    .lookup_property(erase(part), &field, &mut BTreeSet::new())
            });
            let meta = json!({
                "expression": format!("{field} via {member}"),
                "conditions": conditions,
                "member_name": field,
                "operation": if write { "write" } else { "read" },
                "receiver_class": receiver_class,
                "receiver_expression": recv_expr,
                "resolution": if target.is_some() { "resolved" } else { "unresolved" },
                "framework_evidence": evidence,
                "call_expression": self.unit.text(call.start, call.end),
                "mass_load": true,
            });
            let (edge_kind, site_kind) = if write {
                (
                    if target.is_some() {
                        "writes"
                    } else {
                        "property_access"
                    },
                    "write_site",
                )
            } else {
                (
                    if target.is_some() {
                        "reads"
                    } else {
                        "property_access"
                    },
                    "read_site",
                )
            };
            self.emit_effect(
                edge_kind, site_kind, target, call.start, call.end, meta, conditions,
            )?;
        }
        Ok(())
    }

    /// ActiveQuery/Query hash conditions and select lists → attribute R/W.
    #[allow(clippy::too_many_arguments)]
    fn query_builder_fields(
        &mut self,
        member: &str,
        args: &[Expr],
        call: &Expr,
        receiver_class: &str,
        receiver_expression: &str,
        env: &Env,
        conditions: &[String],
    ) -> Result<(), String> {
        let member_l = member.to_ascii_lowercase();
        let model = model_from_query_context(
            receiver_class,
            receiver_expression,
            self.function.class.as_deref().unwrap_or(""),
        );
        match member_l.as_str() {
            "where" | "andwhere" | "orwhere" | "filterwhere" | "andfilterwhere"
            | "orfilterwhere" | "oncondition" | "andoncondition" => {
                let Some(arg) = args.first() else {
                    return Ok(());
                };
                for (field, start, end) in self.condition_fields(arg) {
                    self.emit_attribute_effect(
                        &field,
                        &model,
                        receiver_expression,
                        false,
                        start,
                        end,
                        conditions,
                        None,
                        env,
                        "query_builder",
                    )?;
                }
            }
            "select" | "addselect" | "groupby" | "orderby" | "addorderby" => {
                let Some(arg) = args.first() else {
                    return Ok(());
                };
                for (field, start, end) in self.select_fields(arg) {
                    self.emit_attribute_effect(
                        &field,
                        &model,
                        receiver_expression,
                        false,
                        start,
                        end,
                        conditions,
                        None,
                        env,
                        "query_builder",
                    )?;
                }
            }
            "updateall" | "updateallcounters" => {
                // First arg writes already handled by mass_assignment; second = condition reads.
                if let Some(cond) = args.get(1) {
                    for (field, start, end) in self.condition_fields(cond) {
                        self.emit_attribute_effect(
                            &field,
                            &model,
                            receiver_expression,
                            false,
                            start,
                            end,
                            conditions,
                            None,
                            env,
                            "query_builder",
                        )?;
                    }
                }
            }
            "batchinsert" => {
                // batchInsert($table, ['a','b'], $rows)
                let cols = args.get(1).and_then(|a| match &a.kind {
                    Kind::Array(items) => Some(items),
                    _ => None,
                });
                if let Some(items) = cols {
                    for item in items {
                        if let Some(name) = self.expr_string(&item.value) {
                            if identifier_name(&name) {
                                self.emit_attribute_effect(
                                    &name,
                                    &model,
                                    receiver_expression,
                                    true,
                                    item.value.start,
                                    item.value.end,
                                    conditions,
                                    None,
                                    env,
                                    "batch_insert",
                                )?;
                            }
                        }
                    }
                }
                let _ = call;
            }
            "joinwith" | "innerjoinwith" | "with" => {
                let Some(arg) = args.first() else {
                    return Ok(());
                };
                for (rel, start, end) in self.relation_names(arg) {
                    self.emit_join_with_relation(
                        &rel,
                        &model,
                        receiver_expression,
                        start,
                        end,
                        conditions,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `joinWith('bonus')` / `with(['user', 'wallet'])` → relation property (+ getter).
    fn relation_names(&self, expr: &Expr) -> Vec<(String, usize, usize)> {
        match &expr.kind {
            Kind::Literal(_) => self
                .expr_string(expr)
                .filter(|n| identifier_name(n))
                .map(|n| vec![(n, expr.start, expr.end)])
                .unwrap_or_default(),
            Kind::Array(items) => {
                let mut out = Vec::new();
                for item in items {
                    // ['bonus', 'user' => fn…] — keys or bare string values
                    if let Some(key) = &item.key {
                        if let Some(name) = self.expr_string(key) {
                            if identifier_name(&name) {
                                out.push((name, key.start, key.end));
                            }
                        }
                    } else if let Some(name) = self.expr_string(&item.value) {
                        if identifier_name(&name) {
                            out.push((name, item.value.start, item.value.end));
                        }
                    }
                }
                out
            }
            _ => Vec::new(),
        }
    }

    fn emit_join_with_relation(
        &mut self,
        relation: &str,
        model: &str,
        receiver_expression: &str,
        start: usize,
        end: usize,
        conditions: &[String],
    ) -> Result<(), String> {
        let prop = class_parts(model).iter().find_map(|part| {
            self.project
                .lookup_property(erase(part), relation, &mut BTreeSet::new())
        });
        let getter_name = {
            let mut chars = relation.chars();
            let upper = chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default();
            format!("get{upper}")
        };
        let getter = class_parts(model).iter().find_map(|part| {
            self.project
                .lookup_method(erase(part), &getter_name, &mut BTreeSet::new())
        });
        if let Some(prop_id) = prop {
            let meta = json!({
                "expression": format!("joinWith/with {relation}"),
                "conditions": conditions,
                "member_name": relation,
                "operation": "read",
                "receiver_class": model,
                "receiver_expression": clip_expr(receiver_expression.to_owned()),
                "resolution": "resolved",
                "framework_evidence": "join_with",
            });
            self.emit_effect(
                "reads",
                "read_site",
                Some(prop_id),
                start,
                end,
                meta,
                conditions,
            )?;
        }
        if let Some(getter_id) = getter {
            let meta = json!({
                "expression": format!("joinWith/with {relation} → {getter_name}"),
                "conditions": conditions,
                "member_name": getter_name,
                "operation": "call",
                "receiver_class": model,
                "receiver_expression": clip_expr(receiver_expression.to_owned()),
                "resolution": "resolved",
                "framework_evidence": "join_with",
            });
            self.emit_effect(
                "calls",
                "call_site",
                Some(getter_id),
                start,
                end,
                meta,
                conditions,
            )?;
        }
        Ok(())
    }

    fn condition_fields(&self, expr: &Expr) -> Vec<(String, usize, usize)> {
        let Kind::Array(items) = &expr.kind else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // Hash: ['status' => 1, 'and', [...]]
        for item in items {
            if let Some(key) = &item.key {
                if let Some(name) = self.expr_string(key) {
                    if identifier_name(&name) {
                        out.push((name, key.start, key.end));
                    }
                }
                if let Kind::Array(_) = &item.value.kind {
                    out.extend(self.condition_fields(&item.value));
                }
            } else {
                match &item.value.kind {
                    Kind::Array(_) => out.extend(self.condition_fields(&item.value)),
                    Kind::Literal(_) => {
                        // Operator form: ['=', 'status', 1] — second string is column.
                    }
                    _ => {}
                }
            }
        }
        // Operator form without keys: ['=', 'col', val] or ['in', 'col', [...]]
        let values: Vec<&Expr> = items
            .iter()
            .filter(|i| i.key.is_none())
            .map(|i| &i.value)
            .collect();
        if values.len() >= 2 {
            if let Some(op) = self.expr_string(values[0]) {
                let op_l = op.to_ascii_lowercase();
                if matches!(
                    op_l.as_str(),
                    "=" | "=="
                        | "!="
                        | "<>"
                        | ">"
                        | ">="
                        | "<"
                        | "<="
                        | "in"
                        | "not in"
                        | "like"
                        | "ilike"
                        | "between"
                        | "not between"
                        | "is"
                        | "is not"
                ) {
                    if let Some(name) = self.expr_string(values[1]) {
                        if identifier_name(&name) {
                            out.push((name, values[1].start, values[1].end));
                        }
                    }
                }
            }
        }
        out
    }

    fn select_fields(&self, expr: &Expr) -> Vec<(String, usize, usize)> {
        match &expr.kind {
            Kind::Literal(_) => {
                if let Some(raw) = self.expr_string(expr) {
                    raw.split(',')
                        .filter_map(|part| {
                            let name = part
                                .split_whitespace()
                                .next()?
                                .rsplit('.')
                                .next()?
                                .trim_matches('`')
                                .trim_matches('"');
                            identifier_name(name).then_some((name.to_owned(), expr.start, expr.end))
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            }
            Kind::Array(items) => {
                let mut out = Vec::new();
                for item in items {
                    // ['amount', 'id'] or ['a' => 'amount']
                    if let Some(name) = self.expr_string(&item.value) {
                        let name = name.rsplit('.').next().unwrap_or(&name);
                        if identifier_name(name) {
                            out.push((name.to_owned(), item.value.start, item.value.end));
                        }
                    } else if let Some(key) = &item.key {
                        if let Some(name) = self.expr_string(key) {
                            if identifier_name(&name) {
                                out.push((name, key.start, key.end));
                            }
                        }
                    }
                }
                out
            }
            _ => Vec::new(),
        }
    }

    fn expr_string(&self, expr: &Expr) -> Option<String> {
        if !matches!(&expr.kind, Kind::Literal(_)) {
            return None;
        }
        literal_string(&self.unit.text(expr.start, expr.end))
    }

    /// `createCommand` / `findBySql` (and PDO `prepare`/`query`) with a SQL string.
    fn emit_embedded_sql(
        &mut self,
        member: &str,
        args: &[Expr],
        call: &Expr,
        env: &Env,
    ) -> Result<(), String> {
        if !sql_api(member) {
            return Ok(());
        }
        let Some(sql) = self.sql_argument(args, env) else {
            return Ok(());
        };
        if !crate::plugins::pgsql::looks_like_sql(&sql) {
            return Ok(());
        }
        let span = self.unit.span(call.start, call.end);
        let fragment = crate::plugins::pgsql::embed_sql(
            &sql,
            &self.unit.file.path,
            span.start_line,
            span.end_line,
            &self.function.id,
            self.project.options,
        )?;
        for node in fragment.nodes {
            if !self.project.nodes.contains_key(&node.id) {
                self.project.nodes.insert(node.id.clone(), node);
            }
        }
        for edge in fragment.edges {
            self.project.edges.entry(edge.id.clone()).or_insert(edge);
        }
        Ok(())
    }

    fn sql_argument(&self, args: &[Expr], env: &Env) -> Option<String> {
        let arg = args.first()?;
        if let Some(value) = self.expr_string(arg) {
            return Some(value);
        }
        if let Kind::Variable(name) = &arg.kind {
            return env.get(name).and_then(|b| b.string_value.clone());
        }
        // `"SELECT …".$id." …"` — only when every fragment is a recoverable string literal.
        self.concat_string_literals(arg, env)
    }

    /// Concatenate `.` / `.=` chains of string literals (and vars with known string_value).
    fn concat_string_literals(&self, expr: &Expr, env: &Env) -> Option<String> {
        match &expr.kind {
            Kind::Literal(_) => self.expr_string(expr),
            Kind::Variable(name) => env.get(name).and_then(|b| b.string_value.clone()),
            Kind::Binary(op, left, right) if op == "." => {
                let left = self.concat_string_literals(left, env)?;
                let right = self.concat_string_literals(right, env)?;
                Some(format!("{left}{right}"))
            }
            _ => None,
        }
    }

    fn array_field_writes(
        &self,
        items: &[expression::ArrayItem],
    ) -> Vec<(String, usize, usize, Option<Expr>)> {
        let mut out = Vec::new();
        for item in items {
            let Some(key) = &item.key else {
                continue;
            };
            if let Some(name) = self.expr_string(key) {
                out.push((name, key.start, key.end, Some(item.value.clone())));
            }
        }
        out
    }

    /// RHS of `$obj->field = …` / mass-assignment value → searchable write_site metadata.
    fn attach_assigned_value(&self, metadata: &mut Value, rhs: &Expr, env: &Env) {
        let text = self.unit.text(rhs.start, rhs.end);
        metadata["value_expression"] = json!(text);
        if let Some(constant) = constant_reference(rhs, &self.unit) {
            metadata["value_constant"] = json!(constant);
        }
        let typed = self.typed(rhs, env);
        if informative(&typed.name) {
            metadata["value_type"] = json!(typed.name);
            metadata["value_type_source"] = json!(super::type_source(typed.annotated));
        }
    }

    /// Analyzes a nested closure where it is defined: `use (...)` variables (or the
    /// whole scope for an arrow function) keep the types they have here.
    fn closure(&mut self, start: usize, env: &Env) -> Result<(), String> {
        let functions = self.project.functions;
        let Some(closure) = self
            .project
            .closures
            .get(&(self.function.unit, start))
            .and_then(|id| functions.get(id))
            .filter(|f| super::nested_closure(f))
        else {
            return Ok(());
        };
        let inherited: Env = if closure.arrow {
            env.clone()
        } else {
            closure
                .captures
                .iter()
                .filter_map(|name| env.get(name).map(|b| (name.clone(), b.clone())))
                .collect()
        };
        analyze_with(
            self.project,
            closure,
            inherited,
            self.annotated_vars.clone(),
        )
    }
    /// `Class::NAME` reads a class constant or enum case; `Class::class` is a string.
    fn constant(
        &mut self,
        expr: &Expr,
        receiver: &Expr,
        member: &str,
        env: &Env,
        conditions: &[String],
    ) -> Result<(), String> {
        if member.eq_ignore_ascii_case("class") {
            return Ok(());
        }
        let class = self.receiver_type(receiver, true, env).name;
        let target = class_parts(&class)
            .iter()
            .find_map(|part| self.project.lookup_constant(erase(part), member));
        let kind = if target.is_some() {
            "reads"
        } else {
            "constant_access"
        };
        let metadata = json!({
            "receiver_class": class,
            "receiver_expression": self.unit.text(receiver.start, receiver.end),
            "member_name": member,
            "operation": "constant",
            "expression": self.unit.text(expr.start, expr.end),
            "conditions": conditions,
            "type_source": "code",
        });
        self.emit_effect(
            kind,
            "read_site",
            target,
            expr.start,
            expr.end,
            metadata,
            conditions,
        )
    }
    // Keep graph evidence and its source context explicit at the insertion boundary.
    #[allow(clippy::too_many_arguments)]
    fn variable(
        &mut self,
        name: &str,
        start: usize,
        end: usize,
        env: &Env,
        write: bool,
        assignment: Option<&Expr>,
        conditions: &[String],
    ) -> Result<(), String> {
        let id = format!("variable:{}:${name}", self.function.id);
        let typed = self.variable_type(name, env);
        let type_name = typed.name.as_str();
        if !self.project.nodes.contains_key(&id) {
            self.project.options.check(self.project.elements() + 1)?;
            self.project.nodes.insert(id.clone(),Node{id:id.clone(),kind:"variable".into(),name:format!("${name}"),qualified_name:format!("{}:${name}",self.function.id),span:self.unit.span(start,end),metadata:json!({"fingerprint":tokens::fingerprint(["variable"]),"type":type_name,"scope":self.function.id,"assignments":[],"usages":[],"commented":false}),trust:1,revision:0});
            self.project.edge(
                "contains",
                &self.function.id,
                Some(id.clone()),
                self.function.unit,
                start,
                end,
                json!({}),
                "",
            )?;
        }
        let node = self.project.nodes.get_mut(&id).unwrap();
        node.span.start_line = node.span.start_line.min(self.unit.tokens[start].line);
        node.span.end_line = node.span.end_line.max(self.unit.tokens[end - 1].end_line);
        node.metadata["type"] = json!(union([
            node.metadata["type"].as_str().unwrap_or("unknown"),
            type_name
        ]));
        if typed.annotated {
            node.metadata["type_source"] = json!("annotation");
        } else if node.metadata.get("type_source").is_none() && informative(type_name) {
            node.metadata["type_source"] = json!("code");
        }
        node.metadata["usages"].as_array_mut().unwrap().push(json!({"span":self.unit.span(start,end),"access":if write{"write"}else{"read"},"conditions":conditions}));
        if let Some(rhs) = assignment {
            node.metadata["assignments"].as_array_mut().unwrap().push(json!({"span":self.unit.span(rhs.start,rhs.end),"expression":self.unit.text(rhs.start,rhs.end),"type":type_name,"conditions":conditions,"origins":env.get(name).map(|b|b.origins.clone()).unwrap_or_default()}));
        }
        let site_kind = if write { "write_site" } else { "read_site" };
        self.emit_effect(
            if write { "writes" } else { "reads" },
            site_kind,
            Some(id),
            start,
            end,
            json!({"expression":self.unit.text(start,end),"conditions":conditions}),
            conditions,
        )?;
        Ok(())
    }
    fn returned(
        &mut self,
        start: usize,
        end: usize,
        expr: Option<&Expr>,
        env: &Env,
        conditions: &[String],
        kind: &str,
    ) {
        let type_name = expr.map_or_else(|| "null".into(), |e| self.type_of(e, env));
        let origins = expr.map(|e| self.origins(e, env)).unwrap_or_default();
        // No multi-line `snippet`: it duplicated source into every return and blew 2 GiB.
        let expression = expr.map(|e| clip_expr(self.unit.text(e.start, e.end)));
        let value = json!({"line":self.unit.tokens[start].line,"span":self.unit.span(start,end),"expression":expression,"type":type_name,"conditions":conditions,"origins":origins,"kind":kind});
        self.project
            .nodes
            .get_mut(&self.function.id)
            .unwrap()
            .metadata["returns"]
            .as_array_mut()
            .unwrap()
            .push(value);
        let occurrence = self.occurrence_key("return_site", start, end);
        let _ = self.ensure_site(
            "return_site",
            start,
            end,
            &json!({"expression":expression,"type":type_name,"conditions":conditions,"kind":kind}),
            &occurrence,
        );
    }
    fn ensure_branch(
        &mut self,
        keyword: &str,
        start: usize,
        end: usize,
        condition: &str,
        path: &[String],
    ) -> Result<String, String> {
        let fp = self.unit.fp(start, end, None);
        let id = format!("branch:{}:{fp}", self.function.id);
        if !self.project.nodes.contains_key(&id) {
            self.project.options.check(self.project.elements() + 1)?;
            self.project.nodes.insert(
                id.clone(),
                Node {
                    id: id.clone(),
                    kind: "branch".into(),
                    name: keyword.into(),
                    qualified_name: id.clone(),
                    span: self.unit.span(start, end),
                    metadata: json!({
                        "fingerprint": fp,
                        "keyword": keyword,
                        "condition": condition,
                        "conditions": path,
                        "commented": false
                    }),
                    trust: 1,
                    revision: 0,
                },
            );
            self.project.edge(
                "contains",
                &self.function.id,
                Some(id.clone()),
                self.function.unit,
                start,
                end,
                json!({}),
                &fp,
            )?;
        }
        Ok(id)
    }
    #[allow(clippy::too_many_arguments)]
    fn emit_effect(
        &mut self,
        edge_kind: &str,
        site_kind: &str,
        to: Option<String>,
        start: usize,
        end: usize,
        mut metadata: Value,
        conditions: &[String],
    ) -> Result<(), String> {
        if metadata.get("conditions").is_none() {
            metadata["conditions"] = json!(conditions);
        }
        metadata["commented"] = json!(false);
        let occurrence = self.occurrence_key(site_kind, start, end);
        let site_id = self.ensure_site(site_kind, start, end, &metadata, &occurrence)?;
        self.project.edge(
            edge_kind,
            &site_id,
            to,
            self.function.unit,
            start,
            end,
            metadata,
            &occurrence,
        )?;
        Ok(())
    }
    fn ensure_site(
        &mut self,
        site_kind: &str,
        start: usize,
        end: usize,
        metadata: &Value,
        occurrence: &str,
    ) -> Result<String, String> {
        let id = format!("{site_kind}:{}:{occurrence}", self.function.id);
        if !self.project.nodes.contains_key(&id) {
            self.project.options.check(self.project.elements() + 1)?;
            let expression = metadata
                .get("expression")
                .cloned()
                .unwrap_or_else(|| json!(self.unit.text(start, end)));
            let label = expression
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(site_kind);
            self.project.nodes.insert(
                id.clone(),
                Node {
                    id: id.clone(),
                    kind: site_kind.into(),
                    name: label.chars().take(80).collect(),
                    qualified_name: id.clone(),
                    span: self.unit.span(start, end),
                    // Keep receiver_* on the site node so symbol search
                    // `Owner::$member` can filter without walking edges.
                    metadata: json!({
                        "fingerprint": self.unit.fp(start, end, None),
                        "expression": expression,
                        "conditions": metadata.get("conditions").cloned().unwrap_or_else(|| json!([])),
                        "commented": false,
                        "member_name": metadata.get("member_name").cloned().unwrap_or(Value::Null),
                        "operation": metadata.get("operation").cloned().unwrap_or(Value::Null),
                        "resolution": metadata.get("resolution").cloned().unwrap_or(Value::Null),
                        "receiver_class": metadata.get("receiver_class").cloned().unwrap_or(Value::Null),
                        "receiver_root_class": metadata.get("receiver_root_class").cloned().unwrap_or(Value::Null),
                        "receiver_expression": metadata.get("receiver_expression").cloned().unwrap_or(Value::Null),
                        "value_expression": metadata.get("value_expression").cloned().unwrap_or(Value::Null),
                        "value_constant": metadata.get("value_constant").cloned().unwrap_or(Value::Null),
                        "value_type": metadata.get("value_type").cloned().unwrap_or(Value::Null),
                    }),
                    trust: 1,
                    revision: 0,
                },
            );
            if metadata["undeclared"] == true {
                if let Some(site) = self.project.nodes.get_mut(&id) {
                    site.metadata["undeclared"] = json!(true);
                }
            }
            self.project.edge(
                "contains",
                &self.function.id,
                Some(id.clone()),
                self.function.unit,
                start,
                end,
                json!({}),
                occurrence,
            )?;
        }
        Ok(id)
    }
    fn occurrence_key(&mut self, kind: &str, start: usize, end: usize) -> String {
        let key = format!("{kind}:{}", self.unit.fp(start, end, None));
        let count = self.occurrences.entry(key.clone()).or_default();
        *count += 1;
        format!("{key}:{}", *count)
    }
    fn index_comments(&mut self) -> Result<(), String> {
        let Some((body_start, body_end)) = self.function.body else {
            return Ok(());
        };
        if body_start >= body_end || body_start >= self.unit.tokens.len() {
            return Ok(());
        }
        let range_start = self.unit.tokens[body_start].start;
        let range_end = self.unit.tokens[body_end.saturating_sub(1).max(body_start)].end;
        let comments = self.unit.comments.clone();
        for comment in comments {
            if comment.end <= range_start || comment.start >= range_end {
                continue;
            }
            if !comment_signal(&comment.text) {
                continue;
            }
            self.emit_commented_site(&comment)?;
        }
        Ok(())
    }
    fn emit_commented_site(&mut self, comment: &tokens::Comment) -> Result<(), String> {
        let text = comment.text.trim().trim_end_matches(';').trim();
        let site_kind = if text.contains("->") || text.contains("::") {
            "call_site"
        } else if text.to_ascii_lowercase().starts_with("return") {
            "return_site"
        } else {
            return Ok(());
        };
        let line = comment.line.to_string();
        let fp = tokens::fingerprint([comment.text.as_str(), line.as_str()]);
        let id = format!("{site_kind}:{}:comment:{fp}", self.function.id);
        if self.project.nodes.contains_key(&id) {
            return Ok(());
        }
        self.project.options.check(self.project.elements() + 1)?;
        self.project.nodes.insert(
            id.clone(),
            Node {
                id: id.clone(),
                kind: site_kind.into(),
                name: site_kind.into(),
                qualified_name: id.clone(),
                span: crate::domain::SourceSpan {
                    path: self.unit.file.path.clone(),
                    start_line: comment.line,
                    end_line: comment.end_line,
                },
                metadata: json!({
                    "fingerprint": fp,
                    "expression": text,
                    "conditions": [],
                    "commented": true
                }),
                trust: 1,
                revision: 0,
            },
        );
        self.project.edge(
            "contains",
            &self.function.id,
            Some(id),
            self.function.unit,
            self.function.start,
            self.function.end,
            json!({"commented": true}),
            &fp,
        )?;
        Ok(())
    }
    fn type_of(&self, expr: &Expr, env: &Env) -> String {
        self.typed(expr, env).name
    }
    /// Reads `/** @var … */` comments that precede the statement at `token`.
    fn read_annotations(&mut self, token: usize) {
        let Some(position) = self.unit.tokens.get(token).map(|t| t.start) else {
            return;
        };
        while let Some(comment) = self.unit.comments.get(self.comment_cursor) {
            if comment.start >= position {
                break;
            }
            self.comment_cursor += 1;
            if !comment.text.starts_with('*') || !comment.text.contains("@var") {
                continue;
            }
            let names = &self.function.names;
            let doc = DocBlock::parse(&comment.text, &|raw| names.resolve(raw));
            for var in doc.vars {
                // `@var Type` without a name annotates `$x = …` in this statement.
                let name = var.name.or_else(|| {
                    (self.unit.tokens[token].kind == TokenKind::Variable
                        && self.unit.is(token + 1, "="))
                    .then(|| {
                        self.unit.tokens[token]
                            .text
                            .trim_start_matches('$')
                            .to_owned()
                    })
                });
                if let Some(name) = name {
                    self.annotated_vars.insert(name, var.type_name);
                }
            }
        }
    }
    /// Code binding first; an inline `@var` only fills an uninformative code type.
    fn variable_type(&self, name: &str, env: &Env) -> Typed {
        let code = env.get(name).map(|b| Typed {
            name: b.type_name.clone(),
            annotated: b.annotated,
            exact: b.exact,
        });
        match (code, self.annotated_vars.get(name)) {
            (Some(code), _) if informative(&code.name) => code,
            (_, Some(annotation)) => Typed {
                name: annotation.clone(),
                annotated: true,
                exact: false,
            },
            (Some(code), None) => code,
            (None, None) => Typed::unknown(),
        }
    }
    fn receiver_type(&self, receiver: &Expr, static_: bool, env: &Env) -> Typed {
        match (&receiver.kind, static_) {
            (Kind::Name(name), true) => Typed::code(self.resolve_class(name)),
            _ => self.typed(receiver, env),
        }
    }
    /// `property_type`, but a framework member typed per application (`types_by_area`)
    /// takes the type of the area this function's file belongs to.
    fn property_type_here(&self, id: &str) -> Typed {
        let mut typed = self.property_type(id);
        let area = crate::domain::area_of(&self.project.options.areas, &self.unit.file.path)
            .filter(|area| !area.shared);
        if let Some(area) = area {
            if let Some(ty) =
                self.project.nodes[id].metadata["types_by_area"][&area.prefix].as_str()
            {
                typed.name = ty.to_owned();
            }
        }
        typed
    }
    fn property_type(&self, id: &str) -> Typed {
        let node = &self.project.nodes[id];
        Typed {
            name: node.metadata["type"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
            annotated: node.metadata["type_source"] == "annotation",
            exact: false,
        }
    }
    fn typed(&self, expr: &Expr, env: &Env) -> Typed {
        match &expr.kind {
            Kind::Variable(name) => self.variable_type(name, env),
            Kind::Literal(type_name) => Typed::code(type_name.clone()),
            Kind::New(class, _) => self.constructed_type(class, env),
            Kind::Array(_) => Typed::code("array"),
            Kind::Closure(_) => Typed::code("Closure"),
            Kind::Assign(_, rhs) | Kind::CompoundAssign(_, _, rhs) => self.typed(rhs, env),
            Kind::Cast { type_name, .. } => Typed::code(type_name.clone()),
            Kind::Index { base, .. } => {
                let base = self.typed(base, env);
                Typed {
                    name: element_type(&base.name),
                    annotated: base.annotated,
                    exact: false,
                }
            }
            Kind::Ternary {
                condition,
                then_branch,
                else_branch,
            } => {
                let then_ty = then_branch
                    .as_ref()
                    .map(|e| self.typed(e, env))
                    .unwrap_or_else(|| self.typed(condition, env));
                let else_ty = self.typed(else_branch, env);
                Typed {
                    name: union([then_ty.name.as_str(), else_ty.name.as_str()]),
                    annotated: then_ty.annotated || else_ty.annotated,
                    exact: false,
                }
            }
            Kind::Unary(op, _) if op == "!" => Typed::code("bool"),
            Kind::Unary(_, rhs) => self.typed(rhs, env),
            Kind::Binary(op, a, b) => {
                if [
                    "===",
                    "!==",
                    "==",
                    "!=",
                    "<",
                    ">",
                    "<=",
                    ">=",
                    "<=>",
                    "&&",
                    "||",
                    "and",
                    "or",
                    "instanceof",
                ]
                .contains(&op.as_str())
                {
                    Typed::code("bool")
                } else if op == "." {
                    Typed::code("string")
                } else if op == "??" {
                    let (a, b) = (self.typed(a, env), self.typed(b, env));
                    Typed {
                        name: union([a.name.as_str(), b.name.as_str()]),
                        annotated: a.annotated || b.annotated,
                        exact: false,
                    }
                } else if op != "/"
                    && self.type_of(a, env) == "int"
                    && self.type_of(b, env) == "int"
                {
                    Typed::code("int")
                } else {
                    Typed::unknown()
                }
            }
            Kind::Property {
                receiver,
                member,
                static_,
                ..
            } if *static_ && !member.starts_with('$') => {
                if member.eq_ignore_ascii_case("class") {
                    Typed::code("string")
                } else {
                    let class = match &receiver.kind {
                        Kind::Name(name) => self.resolve_class(name),
                        _ => String::new(),
                    };
                    self.project
                        .lookup_constant(&class, member)
                        .map(|id| {
                            let node = &self.project.nodes[&id];
                            if let Some(values) = node.metadata["class_string_values"].as_array() {
                                let parts: Vec<String> = values
                                    .iter()
                                    .filter_map(|v| v.as_str())
                                    .map(|c| format!("class-string<{c}>[]"))
                                    .collect();
                                if !parts.is_empty() {
                                    return Typed::code(parts.join("|"));
                                }
                            }
                            if let Some(ty) =
                                node.metadata["type"].as_str().filter(|t| informative(t))
                            {
                                return Typed::code(ty.to_owned());
                            }
                            Typed::code("unknown")
                        })
                        .unwrap_or_else(Typed::unknown)
                }
            }
            Kind::Property {
                receiver,
                member,
                static_,
                ..
            } => {
                let receiver = self.receiver_type(receiver, *static_, env);
                let name = member.trim_start_matches('$');
                // Every part of a union receiver may be the runtime object: the property
                // type is the union of each part's type, not the first part's.
                let mut types: Vec<String> = Vec::new();
                let mut annotated = receiver.annotated;
                for part in class_parts(&receiver.name) {
                    let Some(id) =
                        self.project
                            .lookup_property(erase(&part), name, &mut BTreeSet::new())
                    else {
                        continue;
                    };
                    let typed = self.property_type_here(&id);
                    annotated |= typed.annotated;
                    for ty in split_top(&typed.name, '|') {
                        let ty = ty.trim();
                        if !ty.is_empty() && !types.iter().any(|known| known == ty) {
                            types.push(ty.to_owned());
                        }
                    }
                }
                if types.is_empty() {
                    Typed::unknown()
                } else {
                    Typed::code(types.join("|")).tainted(annotated)
                }
            }
            Kind::Call {
                receiver,
                member,
                args,
                ..
            } => {
                let call = self.resolve(expr, env);
                let typed = if let Some(refined) = self.framework_return(expr, &call) {
                    refined
                } else if let Some((_, returns)) = &call.builtin {
                    Typed::code(returns.clone())
                } else if let Some(target) = &call.target {
                    self.return_type(target, &call.matched)
                        .tainted(call.annotated)
                } else if !call.candidates.is_empty() {
                    let types: Vec<Typed> = call
                        .candidates
                        .iter()
                        .map(|c| self.return_type(c, &call.matched))
                        .collect();
                    Typed {
                        name: union(types.iter().map(|t| t.name.as_str())),
                        annotated: call.annotated || types.iter().any(|t| t.annotated),
                        exact: false,
                    }
                } else if receiver.is_none() {
                    Typed::code(match member.to_ascii_lowercase().as_str() {
                        "fopen" | "tmpfile" | "popen" | "fsockopen" => "resource|false",
                        "strlen" | "count" | "intval" | "ord" => "int",
                        "strval" | "sprintf" | "trim" | "strtolower" | "strtoupper" | "substr" => {
                            "string"
                        }
                        "is_int" | "is_string" | "is_null" | "is_array" | "isset" | "empty"
                        | "is_resource" => "bool",
                        "explode" | "array_keys" | "array_values" => "array",
                        _ => "unknown",
                    })
                } else {
                    Typed::unknown()
                };
                // Yii::createObject(X::class) names the instance; prefer that over
                // createObject's own object/mixed/null return. Otherwise useMaster
                // (@return mixed) takes the Closure argument's return.
                let typed = if let Some(created) = self.create_object_return(member, args, env) {
                    created
                } else if informative(&typed.name) {
                    typed
                } else {
                    self.callable_arg_return(args).unwrap_or(typed)
                };
                // Fluent Query methods keep ActiveQuery<T>; batch/each unwrap T.
                let typed = self.query_fluent_return(member, &call, typed);
                self.query_batch_return(member, &call, typed)
            }
            Kind::Match { arms, .. } => {
                if arms.is_empty() {
                    Typed::unknown()
                } else {
                    let types: Vec<Typed> = arms.iter().map(|arm| self.typed(arm, env)).collect();
                    Typed {
                        name: union(types.iter().map(|t| t.name.as_str())),
                        annotated: types.iter().any(|t| t.annotated),
                        exact: false,
                    }
                }
            }
            _ => Typed::unknown(),
        }
    }
    /// `ActiveQuery<T>::where/select/…` → keep `ActiveQuery<T>` (not bare ActiveQuery).
    fn query_fluent_return(&self, member: &str, call: &Call, typed: Typed) -> Typed {
        const FLUENT: &[&str] = &[
            "where",
            "andwhere",
            "orwhere",
            "filterwhere",
            "andfilterwhere",
            "orfilterwhere",
            "select",
            "addselect",
            "orderby",
            "addorderby",
            "groupby",
            "having",
            "andhaving",
            "orhaving",
            "limit",
            "offset",
            "indexby",
            "joinwith",
            "innerjoinwith",
            "with",
            "join",
            "innerjoin",
            "leftjoin",
            "rightjoin",
            "oncondition",
            "andoncondition",
            "asarray",
            "withquery",
        ];
        let name = member.to_ascii_lowercase();
        if !FLUENT.contains(&name.as_str()) {
            return typed;
        }
        let source = if informative(&call.matched.name) {
            call.matched.name.as_str()
        } else {
            call.receiver_class.as_str()
        };
        let preserved = class_parts(source).into_iter().find_map(|part| {
            let arg = type_argument(&part)?;
            if !informative(arg) {
                return None;
            }
            let open = part.find('<')?;
            Some(format!("{}<{arg}>", &part[..open]))
        });
        let Some(query_ty) = preserved else {
            return typed;
        };
        // Only override when we would otherwise lose the type argument.
        let loses_arg = class_parts(&typed.name).iter().any(|p| {
            let lower = p.to_ascii_lowercase();
            (lower.contains("activequery") || lower.ends_with("\\query") || lower == "query")
                && type_argument(p).is_none()
        }) || !informative(&typed.name);
        if loses_arg {
            Typed {
                name: query_ty,
                annotated: typed.annotated || call.annotated,
                exact: true,
            }
        } else {
            typed
        }
    }

    /// `ActiveQuery<Model>::batch()` → `Model[][]`; `each()` → `Model[]` for foreach.
    fn query_batch_return(&self, member: &str, call: &Call, typed: Typed) -> Typed {
        let name = member.to_ascii_lowercase();
        if name != "batch" && name != "each" {
            return typed;
        }
        let source = if informative(&call.matched.name) {
            call.matched.name.as_str()
        } else {
            call.receiver_class.as_str()
        };
        let model = class_parts(source)
            .into_iter()
            .find_map(|part| type_argument(&part).map(str::to_owned))
            .filter(|m| informative(m));
        let Some(model) = model else {
            return typed;
        };
        Typed {
            name: if name == "batch" {
                format!("{model}[][]")
            } else {
                format!("{model}[]")
            },
            annotated: typed.annotated || call.annotated,
            exact: false,
        }
    }
    /// `Yii::createObject(Foo::class)` / `createObject($c)` when `$c` is `class-string<Foo>`.
    fn create_object_return(&self, member: &str, args: &[Expr], env: &Env) -> Option<Typed> {
        if !member.eq_ignore_ascii_case("createObject") {
            return None;
        }
        let first = args.first()?;
        if let Kind::Property {
            receiver,
            member: prop,
            static_: true,
            ..
        } = &first.kind
        {
            if prop.eq_ignore_ascii_case("class") {
                if let Kind::Name(name) = &receiver.kind {
                    let class = self.resolve_class(name);
                    if informative(&class) {
                        return Some(Typed {
                            exact: true,
                            ..Typed::code(class)
                        });
                    }
                }
            }
        }
        let arg = self.typed(first, env);
        let targets = class_string_targets(&arg.name);
        if targets.is_empty() {
            return None;
        }
        Some(Typed {
            name: union(targets.iter().map(String::as_str)),
            annotated: arg.annotated,
            exact: targets.len() == 1 && !arg.annotated,
        })
    }
    /// `new Foo` or `new $class` when `$class` is `class-string<Foo>|class-string<Bar>`.
    fn constructed_type(&self, class: &str, env: &Env) -> Typed {
        if let Some(var) = class.strip_prefix('$') {
            let binding = self.variable_type(var, env);
            let targets = class_string_targets(&binding.name);
            if !targets.is_empty() {
                return Typed {
                    name: union(targets.iter().map(String::as_str)),
                    annotated: binding.annotated,
                    exact: false,
                };
            }
            return Typed::unknown().tainted(binding.annotated);
        }
        Typed {
            exact: true,
            ..Typed::code(self.resolve_class(class))
        }
    }
    /// Return type of the first Closure argument, if it has an informative return.
    /// Used when the callee itself returns `mixed`/`unknown` (e.g. `Connection::useMaster`).
    fn callable_arg_return(&self, args: &[Expr]) -> Option<Typed> {
        for arg in args {
            let Kind::Closure(start) = &arg.kind else {
                continue;
            };
            let id = self.project.closures.get(&(self.function.unit, *start))?;
            let typed = self.return_type(id, &Typed::unknown());
            if informative(&typed.name) {
                return Some(typed);
            }
        }
        None
    }
    /// A framework's return type for a call on a class (e.g. Yii's `X::find()` → `Q<X>`,
    /// or `$user->getWallets()` → `ActiveQuery<Wallet>`); the call target stays real.
    fn framework_return(&self, expr: &Expr, call: &Call) -> Option<Typed> {
        let Kind::Call {
            receiver: Some(receiver),
            member,
            static_,
            ..
        } = &expr.kind
        else {
            return None;
        };
        let class = if *static_ {
            let Kind::Name(name) = &receiver.kind else {
                return None;
            };
            self.resolve_class(name)
        } else {
            // Instance refinements (relation getters) need a typed receiver.
            class_parts(&call.matched.name).into_iter().next()?
        };
        let (type_name, exact) = self.framework_return_of(&class, member)?;
        // Apply even when the call target is unresolved: e.g. User::find() while the
        // library parent that declares find() is outside declaration_paths — Yii still
        // attached ActiveQuery<User> on the model class.
        Some(Typed {
            name: type_name,
            annotated: call.annotated,
            exact,
        })
    }
    /// `framework_returns` on `class` or a parent that declares the refinement.
    fn framework_return_of(&self, class: &str, member: &str) -> Option<(String, bool)> {
        let member = member.to_ascii_lowercase();
        let mut current = Some(erase(class).to_owned());
        let mut seen = BTreeSet::new();
        while let Some(name) = current {
            if !seen.insert(name.clone()) {
                break;
            }
            let key = if self.project.classes.contains_key(&name) {
                name
            } else if let Some(key) = self.project.index.canonical(&name) {
                key.clone()
            } else {
                break;
            };
            if let Some((ty, exact)) = self.project.classes[&key].framework_returns.get(&member) {
                return Some((ty.clone(), *exact));
            }
            current = self.project.classes[&key].parent.clone();
        }
        None
    }
    /// Return type of `target` seen from `receiver`: code type refined by `@return`,
    /// with `self`/`parent`/`static`/`$this` and class `@template` parameters substituted.
    fn return_type(&self, target: &str, receiver: &Typed) -> Typed {
        let Some(f) = self.project.functions.get(target) else {
            return Typed::unknown();
        };
        let (declared, annotated) = refine(&f.return_type, f.doc_return.as_deref());
        if !informative(&declared) {
            if let Some(inferred) = f.inferred_return.as_deref().filter(|ty| informative(ty)) {
                return Typed::code(inferred.to_owned());
            }
            let types = self.project.nodes[target].metadata["returns"]
                .as_array()
                .map(|r| {
                    r.iter()
                        .filter_map(|r| r["type"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Typed::code(union(types));
        }
        let class = f.class.as_ref().and_then(|c| self.project.classes.get(c));
        let substitution = Substitution {
            self_class: f.class.clone(),
            parent: class.and_then(|c| c.parent.clone()),
            receiver: class_parts(&receiver.name)
                .into_iter()
                .next()
                .or_else(|| f.class.clone()),
            templates: class.map(|c| c.templates.clone()).unwrap_or_default(),
        };
        Typed {
            name: substitution.apply(&declared),
            annotated,
            // `return $this` on an exactly known receiver stays exact.
            exact: receiver.exact && declared.trim().eq_ignore_ascii_case("static"),
        }
    }
    /// Every receiver part is an indexed class and neither it nor any descendant declares
    /// `member` (callers already ruled out ancestors, built-ins and `__call`).
    fn declared_nowhere(&self, parts: &[String], member: &str) -> bool {
        // A class whose ancestry leaves the index may be an unseen descendant.
        let lower = member.to_ascii_lowercase();
        let hidden_declarer = self.project.classes.iter().any(|(name, class)| {
            class.methods.contains_key(&lower)
                && super::unindexed_ancestor(self.project.classes, self.project.index, name)
                    .is_some()
        });
        !hidden_declarer
            && !parts.is_empty()
            && parts.iter().all(|part| {
                let class = erase(part);
                self.project.classes.contains_key(class)
                    && self.project.index.descendants(class).iter().all(|child| {
                        self.project
                            .lookup_method(child, member, &mut BTreeSet::new())
                            .is_none()
                    })
            })
    }

    fn resolve(&self, expr: &Expr, env: &Env) -> Call {
        let mut result = Call {
            target: None,
            candidates: Vec::new(),
            resolution: "unresolved".into(),
            receiver_class: "unknown".into(),
            receiver_expression: String::new(),
            receiver_root_class: String::new(),
            dispatcher: None,
            dynamic: false,
            annotated: false,
            matched: Typed::unknown(),
            unindexed_ancestor: None,
            undeclared: false,
            builtin: None,
        };
        let Kind::Call {
            receiver,
            member,
            dynamic,
            static_,
            ..
        } = &expr.kind
        else {
            return result;
        };
        result.dynamic = *dynamic;
        if receiver.is_none() {
            if *dynamic {
                let name = member.trim_start_matches('$');
                if let Some(binding) = env.get(name) {
                    for origin in &binding.origins {
                        if let Some(id) = origin["symbol_id"].as_str() {
                            if self.project.functions.get(id).is_some_and(|f| f.closure) {
                                result.target = Some(id.into());
                                result.dynamic = false;
                            }
                        }
                    }
                }
            } else {
                let resolved = self.function.names.resolve(member);
                // PHP falls back to the global function: the one visible from this area.
                let global = self.function.names.global(member);
                for candidate in [resolved, global] {
                    let id = format!("function:{candidate}");
                    if self.project.functions.contains_key(&id) {
                        result.target = Some(id);
                        break;
                    }
                }
            }
            result.resolution = if result.target.is_some() {
                "resolved"
            } else if !*dynamic {
                "external"
            } else {
                "unresolved"
            }
            .into();
            return result;
        }
        let receiver = receiver.as_ref().unwrap();
        result.receiver_expression = self.unit.text(receiver.start, receiver.end);
        result.receiver_root_class = self.root_class(receiver);
        let receiver_type = self.receiver_type(receiver, *static_, env);
        result.annotated = receiver_type.annotated;
        result.receiver_class = receiver_type.name.clone();
        if *dynamic {
            return result;
        }
        let exact = *static_
            || receiver_type.exact
            || matches!(&receiver.kind, Kind::New(..))
            || matches!(&receiver.kind,Kind::Variable(name)if env.get(name).is_some_and(|b|b.exact));
        let parts = class_parts(&receiver_type.name);
        // Each class of a union is a possible receiver; distinct targets are candidates.
        let mut found: Vec<(String, String)> = Vec::new();
        for part in &parts {
            if let Some(target) =
                self.project
                    .lookup_method(erase(part), member, &mut BTreeSet::new())
            {
                if !found.iter().any(|(t, _)| *t == target) {
                    found.push((target, part.clone()));
                }
            }
        }
        let class = parts
            .first()
            .map(|p| erase(p).to_owned())
            .unwrap_or_else(|| receiver_type.name.clone());
        match found.len() {
            0 => {
                result.unindexed_ancestor = parts.iter().find_map(|part| {
                    super::unindexed_ancestor(self.project.classes, self.project.index, erase(part))
                });
                // The hierarchy ends in a PHP built-in class that declares the method.
                result.builtin = result
                    .unindexed_ancestor
                    .as_deref()
                    .filter(|class| super::builtins::is_builtin(class))
                    .and_then(|class| super::builtins::method(class, member))
                    .map(|(class, returns)| (format!("{class}::{member}"), returns.to_owned()));
                if result.builtin.is_some() {
                    result.resolution = "external".into();
                }
                result.dispatcher = parts.iter().find_map(|part| {
                    self.project.lookup_method(
                        erase(part),
                        if *static_ { "__callStatic" } else { "__call" },
                        &mut BTreeSet::new(),
                    )
                });
                result.undeclared = result.unindexed_ancestor.is_none()
                    && result.builtin.is_none()
                    && result.dispatcher.is_none()
                    && self.declared_nowhere(&parts, member);
                return result;
            }
            1 => {
                let (target, part) = found.remove(0);
                // A method that exists only as a `@method` tag is itself an annotation.
                let magic = self.project.functions.get(&target).is_some_and(|f| f.magic);
                result.annotated |= magic;
                result.target = Some(target);
                result.matched = Typed {
                    exact,
                    ..Typed::code(part).tainted(receiver_type.annotated)
                };
                result.resolution = if result.annotated {
                    "annotated"
                } else {
                    "resolved"
                }
                .into();
            }
            _ => {
                result.matched = receiver_type.clone();
                result.candidates = found.into_iter().map(|(t, _)| t).collect();
                result.resolution = "candidate".into();
                return result;
            }
        }
        let interface = self
            .project
            .classes
            .get(&class)
            .is_some_and(|c| c.kind == "interface");
        if !exact || interface {
            for c in self.project.index.descendants(&class) {
                if let Some(target) = self.project.lookup_method(c, member, &mut BTreeSet::new()) {
                    if !result.candidates.contains(&target) {
                        result.candidates.push(target);
                    }
                }
            }
            if interface || result.candidates.len() > 1 {
                result.resolution = "candidate".into();
            }
        }
        result
    }
    fn origins(&self, expr: &Expr, env: &Env) -> Vec<Value> {
        if let Kind::Variable(name) = &expr.kind {
            if let Some(binding) = env.get(name) {
                return binding.origins.clone();
            }
        }
        let mut origin = json!({"kind":"expression","span":self.unit.span(expr.start,expr.end),"expression":self.unit.text(expr.start,expr.end)});
        match &expr.kind {
            Kind::Call { .. } => {
                let call = self.resolve(expr, env);
                origin["kind"] = json!("call");
                origin["symbol_id"] = json!(call.target);
                origin["candidates"] = json!(call.candidates);
                origin["resolution"] = json!(call.resolution);
            }
            Kind::Closure(start) => {
                origin["symbol_id"] =
                    json!(self.project.closures.get(&(self.function.unit, *start)))
            }
            _ => {}
        }
        vec![origin]
    }
    fn root_class(&self, expr: &Expr) -> String {
        match &expr.kind {
            Kind::Name(name) => self.resolve_class(name),
            Kind::Property { receiver, .. }
            | Kind::Call {
                receiver: Some(receiver),
                ..
            }
            | Kind::Cast { expr: receiver, .. }
            | Kind::Index { base: receiver, .. } => self.root_class(receiver),
            _ => String::new(),
        }
    }
    fn resolve_class(&self, name: &str) -> String {
        if ["self", "parent", "static"].contains(&name.to_ascii_lowercase().as_str()) {
            self.class_type(name)
        } else {
            self.function.names.resolve(name)
        }
    }
    fn class_type(&self, name: &str) -> String {
        match name.to_ascii_lowercase().as_str() {
            "self" | "static" => self
                .function
                .class
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            "parent" => self
                .function
                .class
                .as_ref()
                .and_then(|c| self.project.classes[c].parent.clone())
                .unwrap_or_else(|| "unknown".into()),
            _ => name.into(),
        }
    }
}
/// `self::STATUS_REFUSE` / `UserBonus::STATUS_REFUSE` → `self::STATUS_REFUSE`.
fn constant_reference(expr: &Expr, unit: &super::syntax::Unit) -> Option<String> {
    let Kind::Property {
        receiver,
        member,
        static_: true,
        dynamic: false,
        ..
    } = &expr.kind
    else {
        return None;
    };
    if member.starts_with('$') || member.is_empty() {
        return None;
    }
    let recv = unit.text(receiver.start, receiver.end);
    let recv = recv.trim();
    if recv.is_empty() {
        return None;
    }
    Some(format!("{recv}::{member}"))
}

fn comment_signal(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('$')
        || text.contains("->")
        || text.contains("::")
        || lower.contains("function")
        || lower.contains("class")
        || lower.contains("return")
}
fn union<'a>(types: impl IntoIterator<Item = &'a str>) -> String {
    let mut parts = Vec::new();
    for ty in types {
        for part in ty.split('|') {
            if !part.is_empty() && !parts.contains(&part) {
                parts.push(part);
            }
        }
    }
    if parts.is_empty() {
        "unknown".into()
    } else {
        parts.join("|")
    }
}
fn merge<'a>(environments: impl IntoIterator<Item = &'a Env>) -> Env {
    let environments: Vec<_> = environments.into_iter().collect();
    let names: BTreeSet<_> = environments
        .iter()
        .flat_map(|e| e.keys().cloned())
        .collect();
    names
        .into_iter()
        .map(|name| {
            let mut types = Vec::new();
            let mut origins = Vec::new();
            let mut exact = true;
            let mut annotated = false;
            let mut string_value = None;
            for env in &environments {
                if let Some(binding) = env.get(&name) {
                    types.push(binding.type_name.as_str());
                    exact &= binding.exact;
                    annotated |= binding.annotated;
                    match (&string_value, &binding.string_value) {
                        (None, Some(value)) => string_value = Some(value.clone()),
                        (Some(prev), Some(next)) if prev == next => {}
                        (Some(_), Some(_)) => string_value = None,
                        _ => {}
                    }
                    for origin in &binding.origins {
                        if !origins.contains(origin) {
                            origins.push(origin.clone());
                        }
                    }
                } else {
                    types.push("unknown");
                    exact = false;
                    string_value = None;
                }
            }
            let type_name = union(types);
            exact &= !type_name.contains('|');
            (
                name,
                Binding {
                    type_name,
                    exact,
                    origins,
                    annotated,
                    string_value,
                },
            )
        })
        .collect()
}

/// Class types of a union: no scalars, `null`, `false`, arrays or unknown.
fn class_parts(ty: &str) -> Vec<String> {
    split_top(ty, '|')
        .into_iter()
        .map(str::trim)
        .filter(|part| {
            !part.is_empty()
                && !part.ends_with("[]")
                && !matches!(
                    part.to_ascii_lowercase().as_str(),
                    "unknown"
                        | "mixed"
                        | "null"
                        | "false"
                        | "true"
                        | "int"
                        | "float"
                        | "string"
                        | "bool"
                        | "array"
                        | "object"
                        | "void"
                        | "never"
                        | "callable"
                        | "iterable"
                        | "resource"
                        | "self"
                        | "static"
                        | "parent"
                )
        })
        .map(str::to_owned)
        .collect()
}
/// `Base<Arg>` → `Base`.
fn erase(part: &str) -> &str {
    part.split('<').next().unwrap_or(part)
}

fn literal_string(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.len() >= 2 {
        let bytes = t.as_bytes();
        if (bytes[0] == b'\'' || bytes[0] == b'"') && bytes[0] == bytes[t.len() - 1] {
            return Some(t[1..t.len() - 1].to_owned());
        }
    }
    None
}

fn sql_api(member: &str) -> bool {
    matches!(
        member.to_ascii_lowercase().as_str(),
        "createcommand" | "findbysql" | "prepare"
    )
}

/// `ActiveQuery<Model>` / `Model|null` / `static::find()->…` → Model for attribute binding.
fn model_from_query_context(ty: &str, receiver_expression: &str, enclosing_class: &str) -> String {
    let from_ty = model_from_receiver(ty);
    if is_model_class(&from_ty) {
        return from_ty;
    }
    if let Some(from_expr) = class_before_find(receiver_expression) {
        if from_expr.eq_ignore_ascii_case("static")
            || from_expr.eq_ignore_ascii_case("self")
            || from_expr.eq_ignore_ascii_case("parent")
        {
            if is_model_class(enclosing_class) {
                return enclosing_class.to_owned();
            }
        } else if is_model_class(&from_expr) {
            return from_expr;
        }
    }
    if is_model_class(enclosing_class) {
        return enclosing_class.to_owned();
    }
    from_ty
}

fn is_model_class(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && !lower.contains("activequery")
        && !lower.ends_with("\\query")
        && lower != "query"
        && !matches!(
            lower.as_str(),
            "null" | "false" | "true" | "mixed" | "unknown" | "array"
        )
}

fn class_before_find(expr: &str) -> Option<String> {
    let lower = expr.to_ascii_lowercase();
    let idx = lower.find("::find")?;
    let before = expr[..idx].trim();
    // take last namespace/class token
    let name = before
        .rsplit(|c: char| c == '(' || c == '>' || c == '=' || c == ',' || c.is_whitespace())
        .next()?
        .trim()
        .trim_start_matches('\\');
    (!name.is_empty()).then(|| name.to_owned())
}

/// `ActiveQuery<Model>` / `Model|null` → Model for attribute binding.
fn model_from_receiver(ty: &str) -> String {
    for part in class_parts(ty) {
        if let Some(inner) = part
            .split_once('<')
            .and_then(|(_, rest)| rest.strip_suffix('>'))
        {
            let model = erase(inner.split('|').next().unwrap_or(inner).trim());
            if !model.is_empty() && model != "array" {
                return model.to_owned();
            }
        }
    }
    class_parts(ty)
        .into_iter()
        .map(|p| erase(&p).to_owned())
        .find(|p| is_model_class(p))
        .unwrap_or_else(|| {
            erase(ty.split('|').next().unwrap_or(ty).trim())
                .split('<')
                .next()
                .unwrap_or("")
                .to_owned()
        })
}

fn identifier_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// `$this` inside a trait method is an instance of a class that `use`s the trait.
fn trait_receiver_type(classes: &BTreeMap<String, Class>, class: &str) -> String {
    let Some(decl) = classes.get(class) else {
        return class.to_owned();
    };
    if decl.kind != "trait" {
        return class.to_owned();
    }
    let users: Vec<String> = classes
        .values()
        .filter(|c| c.kind == "class" && c.traits.iter().any(|t| t == class))
        .map(|c| c.qualified.clone())
        .collect();
    if users.is_empty() {
        class.to_owned()
    } else {
        users.join("|")
    }
}
/// `Base<Arg>` → `Arg`.
fn type_argument(part: &str) -> Option<&str> {
    let open = part.find('<')?;
    part.strip_suffix('>').map(|p| &p[open + 1..])
}
/// Element type of `X[]` parts of a union (`unknown` when none).
fn element_type(ty: &str) -> String {
    union(
        split_top(ty, '|')
            .into_iter()
            .filter_map(|part| part.trim().strip_suffix("[]")),
    )
}

/// Replaces late-bound and template names in a declared return type.
struct Substitution {
    self_class: Option<String>,
    parent: Option<String>,
    /// The receiver class part, possibly generic (`ActiveQuery<Wallet>`).
    receiver: Option<String>,
    templates: Vec<String>,
}
impl Substitution {
    fn apply(&self, ty: &str) -> String {
        union(
            split_top(ty, '|')
                .into_iter()
                .map(|part| self.part(part.trim()))
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str),
        )
    }
    fn part(&self, part: &str) -> String {
        if let Some(inner) = part.strip_suffix("[]") {
            let inner = self.part(inner);
            return if inner == "unknown" {
                "array".into()
            } else {
                split_top(&inner, '|')
                    .into_iter()
                    .map(|p| format!("{}[]", p.trim()))
                    .collect::<Vec<_>>()
                    .join("|")
            };
        }
        if let (Some(argument), Some(open)) = (type_argument(part), part.find('<')) {
            return format!("{}<{}>", &part[..open], self.apply(argument));
        }
        let unknown = || "unknown".to_owned();
        match part.to_ascii_lowercase().as_str() {
            "self" => self.self_class.clone().unwrap_or_else(unknown),
            "parent" => self.parent.clone().unwrap_or_else(unknown),
            "static" => self
                .receiver
                .clone()
                .or_else(|| self.self_class.clone())
                .unwrap_or_else(unknown),
            _ if self.templates.iter().any(|t| t == part) => self
                .receiver
                .as_deref()
                .and_then(type_argument)
                .map(str::to_owned)
                .unwrap_or_else(unknown),
            _ => part.to_owned(),
        }
    }
}

/// Cap stored expression text so occurrence metadata does not retain whole statements.
fn clip_expr(text: String) -> String {
    clip_text(text, 160)
}

/// Early-exit guards stored per function (`metadata.guards`).
const GUARDS_PER_FUNCTION: usize = 16;
/// Stored width of one guard condition.
const GUARD_CONDITION_MAX: usize = 120;

/// Stored width of one argument value; names are never clipped, only values.
const ARG_VALUE_MAX: usize = 32;

fn clip_text(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// PHP 8 named argument label (`bonusType: $x`); the expression parser drops it.
fn argument_label(unit: &Unit, arg: &Expr) -> Option<String> {
    let label = arg.start.checked_sub(2)?;
    (unit.tokens[label].kind == TokenKind::Ident && unit.is(arg.start - 1, ":"))
        .then(|| unit.tokens[label].text.clone())
}
