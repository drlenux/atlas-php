//! Indexes examples/probe-project and checks PLUGIN-05 graph evidence without PostgreSQL.
use code_atlas::{
    config::Config, indexer::IndexCoordinator, repository::MemoryRepository, service::AtlasService,
};
use serde_json::Value;
use std::{collections::BTreeMap, env, path::PathBuf, process};

fn main() {
    if let Err(error) = run() {
        eprintln!("probe_verify: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("examples/probe-project"));
    let include_vendor = env::args().nth(2).as_deref() == Some("--vendor");
    let config = Config::from_map(&BTreeMap::from([
        (
            "ATLAS_PROJECT_ROOT".into(),
            root.canonicalize()?.display().to_string(),
        ),
        ("ATLAS_INCLUDE_VENDOR".into(), include_vendor.to_string()),
        ("ATLAS_INDEX_WORKERS".into(), "4".into()),
        ("ATLAS_DATABASE_URL".into(), "dbname=unused".into()),
        ("ATLAS_AGENT_TOKEN".into(), "probe-agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "probe-reviewer".into()),
    ]))?;
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let report = IndexCoordinator::new(config).index(&mut service)?;
    let method = "method:admin\\controllers\\TestController::actionProbe";
    let context = service.context(method, 3, 200)?;
    let diagnostics = context
        .diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "path": d.path,
                "line": d.line,
                "code": d.code,
                "message": d.message,
            })
        })
        .collect::<Vec<_>>();
    let calls = context
        .edges
        .iter()
        .filter(|e| e.from == method && e.kind == "calls")
        .cloned()
        .collect::<Vec<_>>();
    let method_node = context
        .nodes
        .iter()
        .find(|n| n.id == method)
        .ok_or("missing actionProbe node")?;
    let limitations = diagnostics
        .iter()
        .filter(|d| d["code"] == "analysis_limitation")
        .cloned()
        .collect::<Vec<_>>();
    let cascade = limitations.iter().any(|d| {
        let message = d["message"].as_str().unwrap_or("");
        message.contains("Expression semantics are unknown: ::")
            || message.contains("Expression semantics are unknown: ->")
            || message.contains("Expression semantics are unknown: :")
            || message.contains("Yii::$app->request->post")
                && message.contains("analysis_limitation")
            || message.ends_with(": ::")
            || message.ends_with(": ->")
            || message.contains("unknown: ::")
            || message.contains("unknown: ->")
            || message.contains("unknown: :")
    });
    let post_calls = calls
        .iter()
        .filter(|e| e.metadata["member_name"] == "post")
        .count();
    let delete_all = calls
        .iter()
        .filter(|e| {
            e.to.as_deref() == Some("method:App\\Counter::deleteAll")
                || e.metadata["member_name"] == "deleteAll"
        })
        .count();
    let load_a = calls
        .iter()
        .any(|e| e.to.as_deref() == Some("function:App\\loadA"));
    let load_b = calls
        .iter()
        .any(|e| e.to.as_deref() == Some("function:App\\loadB"));
    let fallback = calls
        .iter()
        .filter(|e| e.to.as_deref() == Some("function:App\\fallback"))
        .count();
    let data_var = context
        .nodes
        .iter()
        .find(|n| n.id == format!("variable:{method}:$data"));
    let deleted_total = context
        .nodes
        .iter()
        .find(|n| n.id == format!("variable:{method}:$deletedTotal"));
    let returns = method_node.metadata["returns"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut failures = Vec::new();
    if post_calls < 3 {
        failures.push(format!("expected >=3 post calls, got {post_calls}"));
    }
    if cascade {
        failures.push(format!(
            "operator/fragment cascade remains: {limitations:?}"
        ));
    }
    if !load_a || !load_b {
        failures.push("ternary branch calls loadA/loadB missing".into());
    }
    if fallback < 2 {
        failures.push(format!(
            "expected >=2 fallback calls from ?: and ??, got {fallback}"
        ));
    }
    if delete_all != 2 {
        failures.push(format!("expected 2 deleteAll calls, got {delete_all}"));
    }
    if data_var.map(|n| n.metadata["usages"].as_array().map(|u| !u.is_empty()) == Some(true))
        != Some(true)
    {
        failures.push("$data usages missing after index read".into());
    }
    if deleted_total
        .and_then(|n| n.metadata["assignments"].as_array())
        .map(|items| {
            items
                .iter()
                .any(|a| a["expression"].as_str().unwrap_or("").contains("+="))
        })
        != Some(true)
    {
        failures.push("$deletedTotal += assignment missing".into());
    }
    if returns.is_empty() {
        failures.push("actionProbe returns missing".into());
    }
    let summary = serde_json::json!({
        "project": root.display().to_string(),
        "include_vendor": include_vendor,
        "report": report,
        "method": method,
        "post_calls": post_calls,
        "delete_all_calls": delete_all,
        "fallback_calls": fallback,
        "load_a": load_a,
        "load_b": load_b,
        "limitation_count": limitations.len(),
        "limitations": limitations,
        "call_members": calls.iter().map(|e| Value::String(e.metadata["member_name"].as_str().unwrap_or("?").into())).collect::<Vec<_>>(),
        "failures": failures,
        "ok": failures.is_empty(),
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if !failures.is_empty() {
        return Err(format!("probe failed: {}", failures.join("; ")).into());
    }
    Ok(())
}
