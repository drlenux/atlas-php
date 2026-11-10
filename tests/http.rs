use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use code_atlas::{
    domain::GraphSnapshot,
    http::{router, HttpConfig},
    repository::MemoryRepository,
    service::AtlasService,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn config() -> HttpConfig {
    HttpConfig {
        agent_token: "agent-secret".into(),
        review_token: "review-secret".into(),
        allowed_origins: vec!["http://localhost:8787".into()],
        max_request_bytes: 4096,
        max_context_nodes: 50,
        max_context_depth: 3,
        project_root: std::env::temp_dir(),
        watch_dirs: vec![".".into()],
        excludes: vec![],
        include_vendor: false,
        declaration_paths: vec![],
        language_plugins: vec!["php".into()],
        framework_plugins: vec![],
        max_file_bytes: 2_000_000,
        grep_max_file_bytes: 2_000_000,
        test_paths: vec!["tests/*".into(), "*/tests/*".into(), "*Test.php".into()],
        grep_excludes: vec![],
        read_excludes: vec![],
        areas: vec![],
    }
}

/// MCP tool result text (grep-style lines, no JSON payload).
fn mcp_text(reply: &Value) -> String {
    reply["result"]["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_owned()
}

/// Entity ids of node lines (`… | id=<id> …`), in output order; edge lines are skipped.
fn text_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.contains("-> "))
        .filter_map(|line| line.split_once(" | id=").map(|(_, rest)| rest))
        .map(|rest| rest.split(' ').next().unwrap().to_owned())
        .collect()
}

/// The node line whose identity is `id`.
fn line_of<'a>(text: &'a str, id: &str) -> &'a str {
    text.lines()
        .filter(|line| !line.contains("-> "))
        .find(|line| {
            line.split_once(" | id=")
                .is_some_and(|(_, rest)| rest.split(' ').next() == Some(id))
        })
        .unwrap_or_else(|| panic!("no line for {id}: {text}"))
}

/// `key=value` token of a line (value up to the next space).
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    line.split(' ')
        .find_map(|token| token.strip_prefix(prefix.as_str()))
}

fn service() -> Arc<Mutex<AtlasService>> {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let snapshot: GraphSnapshot = serde_json::from_value(json!({"nodes":[
        {"id":"a", "kind":"method", "name":"pay", "qualified_name":"A::pay", "span":{"path":"a.php","start_line":2,"end_line":4}, "metadata":{"note":"original"}},
        {"id":"b", "kind":"class", "name":"A", "qualified_name":"A", "span":{"path":"a.php","start_line":1,"end_line":5}}
    ], "edges":[{"id":"e","kind":"contains","from":"b","to":"a","span":{"path":"a.php","start_line":2,"end_line":4}}]})).unwrap();
    service
        .index(snapshot, vec!["a.php".to_owned()], vec![])
        .unwrap();
    Arc::new(Mutex::new(service))
}

#[tokio::test]
async fn indexing_status_requires_review_token_and_does_not_wait_for_service() {
    let shared = service();
    let app = router(shared.clone(), config());
    // Model a publication holding the service lock. Status must still answer.
    let (release, wait) = std::sync::mpsc::channel();
    let (locked, observed) = std::sync::mpsc::channel();
    let publication = std::thread::spawn(move || {
        let _guard = shared.lock().unwrap();
        locked.send(()).unwrap();
        let _ = wait.recv_timeout(std::time::Duration::from_secs(5));
    });
    observed.recv().unwrap();
    for (token, expected) in [
        ("", StatusCode::UNAUTHORIZED),
        ("agent-secret", StatusCode::UNAUTHORIZED),
        ("review-secret", StatusCode::OK),
    ] {
        let response = tokio::time::timeout(
            std::time::Duration::from_millis(300),
            app.clone().oneshot(
                Request::get("/api/indexing")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .expect("status must not wait for graph publication")
        .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["state"], "waiting");
            assert!(value["eta_seconds"].is_null());
            assert!(value["total"].is_null());
        }
    }
    release.send(()).unwrap();
    publication.join().unwrap();
}

async fn call(app: axum::Router, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::post(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2025-11-25")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

#[tokio::test]
async fn mcp_handshake_tools_and_context_are_real_protocol_results() {
    let app = router(service(), config());
    let (status, init) = call(app.clone(), "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    let (_, tools) = call(
        app.clone(),
        "/mcp",
        "agent-secret",
        json!({"jsonrpc":"2.0","id":"list","method":"tools/list"}),
    )
    .await;
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "atlas_search",
            "atlas_context",
            "atlas_outline",
            "atlas_grep",
            "atlas_read",
            "atlas_correct",
            "atlas_batch"
        ]
    );
    let (_, context) = call(app, "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"atlas_context","arguments":{"entity_id":"a"}}})).await;
    assert_eq!(context["result"]["isError"], false);
    let text = mcp_text(&context);
    assert!(text.starts_with("# index state="), "{text}");
    assert!(text.contains("# context root=[1] nodes=2 "), "{text}");
    assert_eq!(text_ids(&text)[0], "a");
}

#[tokio::test]
async fn mcp_context_and_outline_deliveries_increment_analytics() {
    let shared = service();
    let app = router(shared.clone(), config());
    for path in ["/api/context/a", "/api/analytics?from=0&to=4102444800"] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer review-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .analytics(0, 4102444800, 86400)
            .unwrap()
            .total_responses,
        0
    );
    call(app.clone(), "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"atlas_context","arguments":{"entity_id":"a"}}})).await;
    call(app.clone(), "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"atlas_outline","arguments":{"entity_id":"a"}}})).await;
    call(app, "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"atlas_search","arguments":{"query":"A","limit":5}}})).await;
    let stats = shared
        .lock()
        .unwrap()
        .analytics(0, 4102444800, 86400)
        .unwrap();
    assert_eq!(stats.total_responses, 3);
    assert!(stats.unique_nodes >= 1, "{stats:?}");
    // Outline of method `a` reads the contains edge from class `b`.
    assert!(stats.unique_edges >= 1, "{stats:?}");
}

#[tokio::test]
async fn auth_roles_origin_and_body_size_are_enforced() {
    let app = router(service(), config());
    let (status, _) = call(
        app.clone(),
        "/mcp",
        "review-secret",
        json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/reviews")
                .header("authorization", "Bearer agent-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("origin", "https://hostile.example")
                .header("authorization", "Bearer agent-secret")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .oneshot(
            Request::post("/mcp")
                .header("authorization", "Bearer agent-secret")
                .header("content-type", "application/json")
                .body(Body::from("x".repeat(5000)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn public_web_modules_load_with_an_origin_on_the_published_port() {
    let app = router(service(), config()); // API origin remains localhost:8787.
    for (path, mime) in [
        ("/", "text/html"),
        ("/assets/app.js", "text/javascript"),
        ("/assets/model.mjs", "text/javascript"),
        ("/assets/indexing.mjs", "text/javascript"),
        ("/assets/explore.mjs", "text/javascript"),
        ("/assets/icon.svg", "image/svg+xml"),
        ("/assets/styles.css", "text/css"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("host", "localhost:8085")
                    .header("origin", "http://localhost:8085")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "public resource {path}");
        assert!(response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with(mime));
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
        assert!(!response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty());
    }
}

#[tokio::test]
async fn public_assets_do_not_relax_the_indexing_api_origin_or_role_checks() {
    let app = router(service(), config());
    for (origin, token, expected) in [
        (
            "http://localhost:8085",
            "review-secret",
            StatusCode::FORBIDDEN,
        ),
        ("http://localhost:8787", "", StatusCode::UNAUTHORIZED),
        (
            "http://localhost:8787",
            "agent-secret",
            StatusCode::UNAUTHORIZED,
        ),
        ("http://localhost:8787", "review-secret", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/indexing")
                    .header("origin", origin)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn invalid_arguments_notifications_and_unknown_methods_are_distinct() {
    let app = router(service(), config());
    let (_, result) = call(app.clone(), "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"atlas_context","arguments":{"entity_id":"a","depth":999}}})).await;
    assert_eq!(result["error"]["code"], -32602);
    let (_, result) = call(
        app.clone(),
        "/mcp",
        "agent-secret",
        json!({"jsonrpc":"2.0","id":2,"method":"unknown"}),
    )
    .await;
    assert_eq!(result["error"]["code"], -32601);
    let (status, body) = call(
        app,
        "/mcp",
        "agent-secret",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body, Value::Null);
}

#[tokio::test]
async fn agent_correction_is_visible_and_reviewer_approval_protects_it() {
    let app = router(service(), config());
    let (_, correction) = call(app.clone(), "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"atlas_correct","arguments":{"kind":"node","id":"a","expected_revision":1,"patch":{"metadata":{"note":"corrected"}},"reason":"Checked the implementation"}}})).await;
    assert_eq!(correction["result"]["isError"], false);
    let correction = mcp_text(&correction);
    let id = correction
        .lines()
        .find_map(|l| l.strip_prefix("id="))
        .unwrap()
        .to_owned();
    let id = id.as_str();
    let revision: u64 = correction
        .lines()
        .find_map(|l| l.strip_prefix("revision="))
        .unwrap()
        .parse()
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/reviews")
                .header("authorization", "Bearer review-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let pending: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(pending["items"][0]["id"], id);
    assert_eq!(
        pending["items"][0]["before"]["metadata"]["note"],
        "original"
    );
    let path = format!("/api/reviews/{id}/decision");
    let (status, _) = call(
        app.clone(),
        &path,
        "agent-secret",
        json!({"decision":"approve","expected_revision":revision}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, approved) = call(
        app.clone(),
        &path,
        "review-secret",
        json!({"decision":"approve","expected_revision":revision}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["status"], "approved");
    let (_, context) = call(app.clone(), "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"atlas_context","arguments":{"entity_id":"a"}}})).await;
    let context_text = mcp_text(&context);
    let node = line_of(&context_text, "a");
    assert_eq!(field(node, "trust"), Some("3"), "{node}");
    let node_revision: u64 = field(node, "rev").unwrap().parse().unwrap();
    let (_, rejected) = call(app, "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"atlas_correct","arguments":{"kind":"node","id":"a","expected_revision":node_revision,"patch":{"metadata":{"note":"overwrite"}},"reason":"Another guess"}}})).await;
    assert_eq!(rejected["result"]["isError"], true);
}

#[tokio::test]
async fn invalid_json_protocol_version_and_missing_entities_are_reported() {
    let app = router(service(), config());
    let response = app
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("authorization", "Bearer agent-secret")
                .header("content-type", "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let parsed: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(parsed["error"]["code"], -32700);
    let response = app
        .clone()
        .oneshot(
            Request::post("/mcp")
                .header("authorization", "Bearer agent-secret")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2099-01-01")
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (_, missing) = call(app.clone(), "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"atlas_context","arguments":{"entity_id":"missing"}}})).await;
    assert_eq!(missing["result"]["isError"], true);
    let (_, invalid) = call(app, "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"atlas_correct","arguments":{"kind":"node","id":"a","expected_revision":1,"patch":{"trust":3},"reason":"forged trust"}}})).await;
    assert_eq!(invalid["result"]["isError"], true);
}

fn lean_service() -> Arc<Mutex<AtlasService>> {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let snapshot: GraphSnapshot = serde_json::from_value(json!({"nodes":[
        {"id":"method:pay", "kind":"method", "name":"pay", "qualified_name":"Wallet::pay", "span":{"path":"w.php","start_line":2,"end_line":9}, "metadata":{"returns":[{"snippet":"return 1;"}],"imports":{"X":"Y"}}},
        {"id":"write_site:pay:1", "kind":"write_site", "name":"$wallet->dep_tax_control = 0", "span":{"path":"w.php","start_line":3,"end_line":3}, "metadata":{"conditions":["$a > 0"]}},
        {"id":"write_site:pay:2", "kind":"write_site", "name":"$wallet->dep_tax_control = 1", "span":{"path":"w.php","start_line":4,"end_line":4}, "metadata":{"conditions":[]}},
        {"id":"read_site:pay:1", "kind":"read_site", "name":"$wallet->dep_tax_control", "span":{"path":"w.php","start_line":5,"end_line":5}, "metadata":{"conditions":[]}},
        {"id":"method:caller", "kind":"method", "name":"caller", "qualified_name":"Game::caller", "span":{"path":"w.php","start_line":10,"end_line":12}},
        {"id":"call_site:caller:1", "kind":"call_site", "name":"$wallet->pay()", "span":{"path":"w.php","start_line":11,"end_line":11}}
    ], "edges":[
        {"id":"c1","kind":"contains","from":"method:pay","to":"write_site:pay:1","span":{"path":"w.php","start_line":3,"end_line":3}},
        {"id":"c2","kind":"contains","from":"method:caller","to":"call_site:caller:1","span":{"path":"w.php","start_line":11,"end_line":11}},
        {"id":"k1","kind":"calls","from":"call_site:caller:1","to":"method:pay","span":{"path":"w.php","start_line":11,"end_line":11}}
    ], "diagnostics":[{"path":"w.php","line":3,"code":"unresolved_call","message":"cannot resolve"}]})).unwrap();
    service
        .index(snapshot, vec!["w.php".to_owned()], vec![])
        .unwrap();
    Arc::new(Mutex::new(service))
}
async fn tool(app: axum::Router, name: &str, arguments: Value) -> Value {
    let (_, reply) = call(app, "/mcp", "agent-secret", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}})).await;
    reply
}

#[tokio::test]
async fn tool_schemas_declare_lean_response_arguments() {
    let app = router(lean_service(), config());
    let (_, tools) = call(
        app,
        "/mcp",
        "agent-secret",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    let tools = tools["result"]["tools"].as_array().unwrap();
    let search = &tools[0]["inputSchema"]["properties"];
    assert_eq!(search["kinds"]["type"], "array");
    assert_eq!(search["cursor"]["type"], "string");
    assert_eq!(search["withGrep"]["type"], "boolean");
    let outline = &tools[2]["inputSchema"]["properties"];
    assert_eq!(outline["sections"]["type"], "array");
    assert_eq!(tools[3]["name"], "atlas_grep");
    assert_eq!(tools[4]["name"], "atlas_read");
    let context = &tools[1]["inputSchema"]["properties"];
    assert_eq!(context["direction"]["enum"], json!(["both", "in", "out"]));
    assert_eq!(context["detail"]["enum"], json!(["summary", "full"]));
    assert_eq!(context["include_diagnostics"]["type"], "boolean");
}

#[tokio::test]
async fn mcp_grep_and_read_cover_live_sources() {
    let dir = std::env::temp_dir().join(format!(
        "atlas-mcp-grep-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("common/config")).unwrap();
    std::fs::write(
        dir.join("common/config/params.php"),
        "<?php\nreturn ['piastrixInvoiceRefund' => 'https://x'];\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("w.php"),
        "<?php\nclass Wallet {\n  function pay() { return 1; }\n}\n",
    )
    .unwrap();
    let mut cfg = config();
    cfg.project_root = dir.clone();
    let app = router(lean_service(), cfg);
    let grep = tool(
        app.clone(),
        "atlas_grep",
        json!({"query":"piastrixInvoiceRefund","path_prefix":"common/config"}),
    )
    .await;
    let page = mcp_text(&grep);
    assert!(
        page.lines()
            .any(|l| l.starts_with("common/config/params.php:2:")),
        "{page}"
    );
    let header = page.lines().nth(1).unwrap();
    assert!(header.starts_with("# grep hits=1 "), "{header}");
    assert!(!header.contains("incomplete"), "{header}");
    let read = tool(
        app,
        "atlas_read",
        json!({"path":"w.php","start_line":2,"end_line":3}),
    )
    .await;
    let body = mcp_text(&read);
    assert_eq!(
        body.lines().skip(1).collect::<Vec<_>>(),
        [
            "# w.php:2-3",
            "2: class Wallet {",
            "3:   function pay() { return 1; }"
        ]
    );
}

#[tokio::test]
async fn mcp_search_with_grep_adds_hits_missing_from_atlas() {
    let dir = std::env::temp_dir().join(format!(
        "atlas-with-grep-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("w.php"),
        "<?php\nclass Wallet { function pay() { $wallet->dep_tax_control = 0; } }\n// GREP_ONLY_MARKER in comment\n",
    )
    .unwrap();
    let mut cfg = config();
    cfg.project_root = dir;
    let app = router(lean_service(), cfg);
    let without = tool(
        app.clone(),
        "atlas_search",
        json!({"query":"GREP_ONLY_MARKER"}),
    )
    .await;
    let page = mcp_text(&without);
    assert!(!page.contains("# grep supplement"), "{page}");
    assert!(page.contains("# search nodes=0 complete"), "{page}");

    let with = tool(
        app,
        "atlas_search",
        json!({"query":"GREP_ONLY_MARKER","withGrep":true}),
    )
    .await;
    let page = mcp_text(&with);
    assert!(page.contains("# grep supplement hits=1 "), "{page}");
    assert!(
        page.lines()
            .any(|l| l.starts_with("w.php:3:") && l.contains("GREP_ONLY_MARKER")),
        "{page}"
    );
}

#[tokio::test]
async fn mcp_with_grep_scans_php_files_without_graph_nodes() {
    // Config-only files never become atlas nodes; withGrep must still find them.
    let dir = std::env::temp_dir().join(format!(
        "atlas-grep-config-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("common/config")).unwrap();
    std::fs::write(
        dir.join("common/config/params.php"),
        "<?php\nreturn [\n    'piastrixInvoiceRefund' => 'https://example.test/refund',\n];\n",
    )
    .unwrap();
    let mut cfg = config();
    cfg.project_root = dir;
    let app = router(lean_service(), cfg);
    let with = tool(
        app,
        "atlas_search",
        json!({"query":"piastrixInvoiceRefund","withGrep":true}),
    )
    .await;
    let page = mcp_text(&with);
    assert!(
        page.lines()
            .any(|l| l.starts_with("common/config/params.php:3:")
                && l.contains("piastrixInvoiceRefund")),
        "{page}"
    );
}

#[tokio::test]
async fn mcp_outline_sections_returns_only_called_by() {
    let app = router(lean_service(), config());
    let full = tool(
        app.clone(),
        "atlas_outline",
        json!({"entity_id":"method:pay"}),
    )
    .await;
    let full_outline = mcp_text(&full);
    assert!(
        full_outline
            .lines()
            .any(|l| l == "calls:" || l == "called_by:" || l == "members:"),
        "{full_outline}"
    );

    let lean = tool(
        app,
        "atlas_outline",
        json!({"entity_id":"method:pay","sections":["called_by"]}),
    )
    .await;
    let outline = mcp_text(&lean);
    let lines: Vec<&str> = outline.lines().collect();
    assert!(lines[1].ends_with("| id=method:pay"), "{outline}");
    for absent in ["calls:", "members:", "parents:"] {
        assert!(!lines.contains(&absent), "{outline}");
    }
    let section = lines.iter().position(|l| *l == "called_by:").unwrap();
    let callers: Vec<_> = lines[section + 1..]
        .iter()
        .take_while(|l| l.starts_with("  "))
        .collect();
    assert_eq!(callers.len(), 1);
    assert!(callers[0].contains("caller"));

    let content = lean["result"]["content"][0]["text"].as_str().unwrap();
    let structured = lean["result"].get("structuredContent");
    assert!(
        structured.is_none() || structured == Some(&serde_json::Value::Null),
        "avoid duplicating the full payload in structuredContent"
    );
    assert!(content.contains("called_by:"));
    assert!(!content.contains('{'), "no JSON payload: {content}");
}

#[tokio::test]
async fn mcp_search_filters_kinds_pages_and_omits_heavy_metadata() {
    let app = router(lean_service(), config());
    let first = tool(
        app.clone(),
        "atlas_search",
        json!({"query":"dep_tax_control","kinds":["write_site"],"limit":1}),
    )
    .await;
    let page = mcp_text(&first);
    let header = page.lines().nth(1).unwrap();
    assert!(
        header.starts_with("# search nodes=1 next_cursor="),
        "{header}"
    );
    assert!(
        page.lines().nth(2).unwrap().contains(": write_site "),
        "{page}"
    );
    let cursor = field(header, "next_cursor").unwrap().to_owned();
    let second = tool(
        app.clone(),
        "atlas_search",
        json!({"query":"dep_tax_control","kinds":["write_site"],"limit":1,"cursor":cursor}),
    )
    .await;
    let page = mcp_text(&second);
    assert_eq!(text_ids(&page), ["write_site:pay:2"]);
    assert!(page.contains("# search nodes=1 complete"), "{page}");

    let method = tool(app.clone(), "atlas_search", json!({"query":"Wallet::pay"})).await;
    let method_text = mcp_text(&method);
    let node = line_of(&method_text, "method:pay");
    assert!(!node.contains("returns="), "{node}");
    assert!(!node.contains("imports="), "{node}");

    let invalid = tool(
        app,
        "atlas_search",
        json!({"query":"x","kinds":["Write-Site"]}),
    )
    .await;
    assert_eq!(invalid["result"]["isError"], true);
}

#[tokio::test]
async fn mcp_context_defaults_are_lean_and_flags_restore_full_data() {
    let app = router(lean_service(), config());
    let lean = tool(
        app.clone(),
        "atlas_context",
        json!({"entity_id":"method:pay","depth":1}),
    )
    .await;
    let context = mcp_text(&lean);
    assert!(!context.contains("# diagnostics"), "{context}");
    assert!(!line_of(&context, "method:pay").contains("returns="));

    let callers = tool(
        app.clone(),
        "atlas_context",
        json!({"entity_id":"method:pay","depth":1,"direction":"in"}),
    )
    .await;
    // direction in is a callers tree: the caller, not the method's own write sites.
    let callers = mcp_text(&callers);
    assert!(callers.contains("└ caller L11 "), "{callers}");
    assert!(!callers.contains("dep_tax_control"), "{callers}");

    let full = tool(
        app.clone(),
        "atlas_context",
        json!({"entity_id":"method:pay","depth":0,"detail":"full","include_diagnostics":true}),
    )
    .await;
    let context = mcp_text(&full);
    assert!(
        context.contains("# diagnostics\nw.php:3: unresolved_call"),
        "{context}"
    );
    assert!(line_of(&context, "method:pay").contains("returns="));

    let invalid = tool(
        app,
        "atlas_context",
        json!({"entity_id":"method:pay","direction":"sideways"}),
    )
    .await;
    assert_eq!(invalid["error"]["code"], -32602);
}

#[tokio::test]
async fn reviewer_web_api_keeps_full_metadata_and_diagnostics() {
    let app = router(lean_service(), config());
    for (path, check) in [("/api/context/method:pay", "context")] {
        let response = app
            .clone()
            .oneshot(
                Request::get(path)
                    .header("authorization", "Bearer review-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let node = body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "method:pay")
            .unwrap();
        assert!(node["metadata"].get("returns").is_some(), "{check}");
        if check == "context" {
            assert_eq!(body["diagnostics"].as_array().unwrap().len(), 1);
        }
    }
}

async fn send(app: axum::Router, method: &str, path: &str, token: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn reviewer_can_queue_a_reindex_and_see_it_in_the_indexing_status() {
    let app = router(lean_service(), config());
    for token in ["", "agent-secret"] {
        let (status, _) = send(app.clone(), "POST", "/api/reindex", token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (_, indexing) = send(app.clone(), "GET", "/api/indexing", "review-secret").await;
    assert_eq!(indexing["reindex_queued"], false);
    let (status, body) = send(app.clone(), "POST", "/api/reindex", "review-secret").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["reindex_queued"], true);
    let (_, indexing) = send(app.clone(), "GET", "/api/indexing", "review-secret").await;
    assert_eq!(indexing["reindex_queued"], true);
    let (status, _) = send(app, "GET", "/api/reindex", "review-secret").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn web_search_pages_by_kind_and_returns_compact_nodes() {
    let app = router(lean_service(), config());
    let (status, first) = send(
        app.clone(),
        "GET",
        "/api/search?q=dep_tax_control&kinds=write_site&limit=1",
        "review-secret",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["nodes"][0]["id"], "write_site:pay:1");
    let cursor = first["next_cursor"].as_str().unwrap();
    let cursor_enc = cursor.replace('|', "%7C");
    let (_, second) = send(
        app.clone(),
        "GET",
        &format!("/api/search?q=dep_tax_control&kinds=write_site&limit=1&cursor={cursor_enc}"),
        "review-secret",
    )
    .await;
    assert_eq!(second["nodes"][0]["id"], "write_site:pay:2");
    assert!(second["next_cursor"].is_null());
    let (_, method) = send(
        app.clone(),
        "GET",
        "/api/search?q=Wallet::pay",
        "review-secret",
    )
    .await;
    assert!(method["nodes"][0]["metadata"].get("returns").is_none());
    let (status, _) = send(
        app,
        "GET",
        "/api/search?q=x&kinds=Bad-Kind",
        "review-secret",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn web_context_accepts_view_parameters_and_keeps_full_defaults() {
    let app = router(lean_service(), config());
    let (_, full) = send(
        app.clone(),
        "GET",
        "/api/context/method:pay",
        "review-secret",
    )
    .await;
    assert_eq!(full["diagnostics"].as_array().unwrap().len(), 1);
    let (status, lean) = send(
        app.clone(),
        "GET",
        "/api/context/method:pay?depth=1&direction=in&detail=summary&include_diagnostics=false",
        "review-secret",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lean["diagnostics"], json!([]));
    let ids: Vec<_> = lean["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"method:caller"));
    assert!(!ids.contains(&"write_site:pay:1"));
    let root = lean["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "method:pay")
        .unwrap();
    assert!(root["metadata"].get("returns").is_none());
    let (status, _) = send(
        app.clone(),
        "GET",
        "/api/context/method:pay?depth=99",
        "review-secret",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(
        app,
        "GET",
        "/api/context/method:pay?direction=up",
        "review-secret",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mcp_outline_symbol_search_and_freshness() {
    let app = router(lean_service(), config());
    let outline = tool(
        app.clone(),
        "atlas_outline",
        json!({"entity_id":"method:pay"}),
    )
    .await;
    let result = mcp_text(&outline);
    let lines: Vec<&str> = result.lines().collect();
    assert!(lines[1].ends_with("| id=method:pay"), "{result}");
    assert!(
        !lines.contains(&"members:"),
        "default method outline omits redundant members line"
    );
    assert!(lines.contains(&"called_by:"));
    assert!(field(lines[0], "indexed_at").is_some(), "{}", lines[0]);
    assert!(!lines[0].contains("changes_pending"));

    let missing = tool(app.clone(), "atlas_outline", json!({"entity_id":"missing"})).await;
    assert_eq!(missing["result"]["isError"], true);

    let text = tool(
        app.clone(),
        "atlas_search",
        json!({"query":"pay","kinds":["call_site"]}),
    )
    .await;
    let text = mcp_text(&text);
    assert!(text.contains("# search nodes=1 "), "{text}");
    assert!(field(text.lines().next().unwrap(), "indexed_at").is_some());
    let symbol = tool(
        app.clone(),
        "atlas_search",
        json!({"query":"pay","mode":"symbol"}),
    )
    .await;
    let ids = text_ids(&mcp_text(&symbol));
    assert_eq!(
        ids,
        ["method:pay"],
        "call site has no member_name in this fixture"
    );
    let invalid = tool(
        app.clone(),
        "atlas_search",
        json!({"query":"pay","mode":"fuzzy"}),
    )
    .await;
    assert_eq!(invalid["error"]["code"], -32602);

    let context = tool(
        app,
        "atlas_context",
        json!({"entity_id":"method:pay","depth":0}),
    )
    .await;
    let context = mcp_text(&context);
    assert!(context.starts_with("# index state="), "{context}");
    assert!(
        context.contains(" truncated=false depth_limited=true"),
        "{context}"
    );
}

#[tokio::test]
async fn reviewer_reset_requires_confirmation_and_queues_a_reindex() {
    let app = router(lean_service(), config());
    let reset = |token: &'static str, body: Value| {
        let app = app.clone();
        async move { call(app, "/api/atlas/reset", token, body).await }
    };
    let (status, _) = reset("agent-secret", json!({"confirm":"reset"})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = reset("review-secret", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = reset("review-secret", json!({"confirm":"yes"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, still) = send(app.clone(), "GET", "/api/search?q=pay", "review-secret").await;
    assert!(
        !still["nodes"].as_array().unwrap().is_empty(),
        "not reset yet"
    );

    let (status, body) = reset("review-secret", json!({"confirm":"reset"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"reset": true, "reindex_queued": true}));
    let (_, empty) = send(app.clone(), "GET", "/api/search?q=pay", "review-secret").await;
    assert!(empty["nodes"].as_array().unwrap().is_empty());
    let (_, indexing) = send(app.clone(), "GET", "/api/indexing", "review-secret").await;
    assert_eq!(indexing["reindex_queued"], true);
    let (_, tools) = call(
        app,
        "/mcp",
        "agent-secret",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    assert!(!tools.to_string().contains("reset"), "no MCP reset tool");
}

/// Wallet owns pay (root) and sibling refund; used to compare MCP and web context.
fn owner_service() -> Arc<Mutex<AtlasService>> {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let snapshot: GraphSnapshot = serde_json::from_value(json!({"nodes":[
        {"id":"class:Wallet", "kind":"class", "name":"Wallet", "span":{"path":"w.php","start_line":1,"end_line":20}},
        {"id":"method:pay", "kind":"method", "name":"pay", "span":{"path":"w.php","start_line":2,"end_line":5}},
        {"id":"method:refund", "kind":"method", "name":"refund", "span":{"path":"w.php","start_line":6,"end_line":9}}
    ], "edges":[
        {"id":"o1","kind":"contains","from":"class:Wallet","to":"method:pay","span":{"path":"w.php","start_line":2,"end_line":5}},
        {"id":"o2","kind":"contains","from":"class:Wallet","to":"method:refund","span":{"path":"w.php","start_line":6,"end_line":9}}
    ]})).unwrap();
    service
        .index(snapshot, vec!["w.php".to_owned()], vec![])
        .unwrap();
    Arc::new(Mutex::new(service))
}

fn node_ids(body: &Value) -> Vec<String> {
    body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn agent_context_omits_owner_members_while_reviewer_keeps_them() {
    let app = router(owner_service(), config());
    let agent = mcp_text(
        &tool(
            app.clone(),
            "atlas_context",
            json!({"entity_id":"method:pay"}),
        )
        .await,
    );
    let agent_ids = text_ids(&agent);
    assert!(
        agent_ids.contains(&"class:Wallet".to_owned()),
        "{agent_ids:?}"
    );
    assert!(
        !agent_ids.contains(&"method:refund".to_owned()),
        "{agent_ids:?}"
    );

    let response = app
        .oneshot(
            Request::get("/api/context/method:pay")
                .header("authorization", "Bearer review-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let reviewer: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(node_ids(&reviewer).contains(&"method:refund".to_owned()));
}

#[tokio::test]
async fn reviewer_search_defaults_to_substring_while_agent_text_needs_whole_identifier() {
    let app = router(lean_service(), config());
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/search?q=dep_tax")
                .header("authorization", "Bearer review-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let reviewer: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        !reviewer["nodes"].as_array().unwrap().is_empty(),
        "{reviewer}"
    );
    let agent = mcp_text(&tool(app, "atlas_search", json!({"query":"dep_tax"})).await);
    assert!(agent.contains("# search nodes=0 complete"), "{agent}");
}

#[tokio::test]
async fn batch_runs_several_tools_in_one_request_and_isolates_errors() {
    let app = router(lean_service(), config());
    let reply = tool(
        app,
        "atlas_batch",
        json!({"calls":[
            {"tool":"atlas_outline","arguments":{"entity_id":"method:pay","sections":["called_by"]}},
            {"tool":"atlas_outline","arguments":{"entity_id":"missing"}},
            {"tool":"atlas_search","arguments":{"query":"dep_tax_control","kinds":["write_site"]}}
        ]}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let text = mcp_text(&reply);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("# index state="), "{text}");
    assert_eq!(text.matches("# index state=").count(), 1, "{text}");
    assert_eq!(lines[1], "## [1] atlas_outline");
    assert!(text.contains("\n## [2] atlas_outline error: "), "{text}");
    assert!(
        text.contains("\n## [3] atlas_search\n# search nodes=2"),
        "{text}"
    );
    let app = router(lean_service(), config());
    let found = mcp_text(&tool(app.clone(), "atlas_search", json!({"query":"Wallet::pay"})).await);
    let revision: u64 = field(line_of(&found, "method:pay"), "rev")
        .unwrap()
        .parse()
        .unwrap();
    let nested = tool(
        app,
        "atlas_batch",
        json!({"calls":[
            {"tool":"atlas_batch","arguments":{"calls":[]}},
            {"tool":"atlas_correct","arguments":{"kind":"node","id":"method:pay","expected_revision":revision,"patch":{"metadata":{"note":"x"}},"reason":"batch"}}
        ]}),
    )
    .await;
    let nested = mcp_text(&nested);
    assert!(nested.contains("## [1] atlas_batch error: "), "{nested}");
    assert!(
        nested.contains("## [2] atlas_correct error: "),
        "batch is read-only: {nested}"
    );
}

fn tests_service() -> Arc<Mutex<AtlasService>> {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let snapshot: GraphSnapshot = serde_json::from_value(json!({"nodes":[
        {"id":"method:pay", "kind":"method", "name":"pay", "span":{"path":"app/Wallet.php","start_line":2,"end_line":4}},
        {"id":"method:testPay", "kind":"method", "name":"testPay", "span":{"path":"common/tests/WalletTest.php","start_line":2,"end_line":6}},
        {"id":"call_site:t:1", "kind":"call_site", "name":"$w->pay()", "span":{"path":"common/tests/WalletTest.php","start_line":4,"end_line":4}, "metadata":{"member_name":"pay"}},
        {"id":"method:run", "kind":"method", "name":"run", "span":{"path":"app/Game.php","start_line":2,"end_line":6}},
        {"id":"call_site:r:1", "kind":"call_site", "name":"$w->pay()", "span":{"path":"app/Game.php","start_line":4,"end_line":4}, "metadata":{"member_name":"pay"}}
    ], "edges":[
        {"id":"c1","kind":"contains","from":"method:testPay","to":"call_site:t:1","span":{"path":"common/tests/WalletTest.php","start_line":4,"end_line":4}},
        {"id":"k1","kind":"calls","from":"call_site:t:1","to":"method:pay","span":{"path":"common/tests/WalletTest.php","start_line":4,"end_line":4}},
        {"id":"c2","kind":"contains","from":"method:run","to":"call_site:r:1","span":{"path":"app/Game.php","start_line":4,"end_line":4}},
        {"id":"k2","kind":"calls","from":"call_site:r:1","to":"method:pay","span":{"path":"app/Game.php","start_line":4,"end_line":4}}
    ]})).unwrap();
    service
        .index(
            snapshot,
            vec![
                "app/Wallet.php".into(),
                "app/Game.php".into(),
                "common/tests/WalletTest.php".into(),
            ],
            vec![],
        )
        .unwrap();
    Arc::new(Mutex::new(service))
}

#[tokio::test]
async fn agent_search_excludes_tests_unless_asked() {
    let app = router(tests_service(), config());
    let lean = mcp_text(
        &tool(
            app.clone(),
            "atlas_search",
            json!({"query":"pay","kinds":["call_site"]}),
        )
        .await,
    );
    assert_eq!(text_ids(&lean), ["call_site:r:1"], "{lean}");
    assert!(
        lean.lines().nth(1).unwrap().contains(" tests=excluded"),
        "{lean}"
    );
    let all = mcp_text(
        &tool(
            app,
            "atlas_search",
            json!({"query":"pay","kinds":["call_site"],"include_tests":true}),
        )
        .await,
    );
    assert_eq!(text_ids(&all).len(), 2, "{all}");
    assert!(!all.contains("tests=excluded"), "{all}");
}

#[tokio::test]
async fn agent_context_in_is_a_callers_tree_without_tests() {
    let app = router(tests_service(), config());
    let tree = mcp_text(
        &tool(
            app.clone(),
            "atlas_context",
            json!({"entity_id":"method:pay","direction":"in"}),
        )
        .await,
    );
    let lines: Vec<&str> = tree.lines().collect();
    assert!(
        lines[1].starts_with("# callers method:pay levels=3 complete excluded.tests=1"),
        "{tree}"
    );
    assert_eq!(lines[2], "pay app/Wallet.php:2");
    assert!(lines[3].starts_with("└ run L4 "), "{tree}");
    let with_tests = mcp_text(
        &tool(
            app,
            "atlas_context",
            json!({"entity_id":"method:pay","direction":"in","include_tests":true}),
        )
        .await,
    );
    assert!(with_tests.contains("testPay L4"), "{with_tests}");
}

#[tokio::test]
async fn outline_callers_exclude_tests_and_say_so() {
    let app = router(tests_service(), config());
    let outline = mcp_text(
        &tool(
            app.clone(),
            "atlas_outline",
            json!({"entity_id":"method:pay","sections":["called_by"]}),
        )
        .await,
    );
    let lines: Vec<&str> = outline.lines().collect();
    assert_eq!(lines[2], "called_by (excluded.tests=1):", "{outline}");
    assert!(lines[3].starts_with("  run L4: "), "{outline}");
    assert_eq!(lines.len(), 4, "{outline}");
    let all = mcp_text(
        &tool(
            app,
            "atlas_outline",
            json!({"entity_id":"method:pay","sections":["called_by"],"include_tests":true}),
        )
        .await,
    );
    assert!(all.contains("testPay L4"), "{all}");
}

#[tokio::test]
async fn class_context_defaults_to_member_usages() {
    let app = router(owner_service(), config());
    let text = mcp_text(&tool(app, "atlas_context", json!({"entity_id":"class:Wallet"})).await);
    assert!(
        text.lines()
            .nth(1)
            .unwrap()
            .starts_with("# callers class:Wallet "),
        "{text}"
    );
    assert!(text.contains("members_without_usages=2"), "{text}");
}

#[tokio::test]
async fn agent_grep_excludes_tests_unless_asked() {
    let dir = std::env::temp_dir().join(format!("atlas-grep-tests-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("common/tests")).unwrap();
    std::fs::write(
        dir.join("common/tests/WalletTest.php"),
        "<?php ONLY_IN_TEST\n",
    )
    .unwrap();
    let mut cfg = config();
    cfg.project_root = dir;
    let app = router(lean_service(), cfg);
    let lean = mcp_text(&tool(app.clone(), "atlas_grep", json!({"query":"ONLY_IN_TEST"})).await);
    assert!(lean.contains(" excluded.tests=1"), "{lean}");
    assert!(!lean.contains("WalletTest.php:"), "{lean}");
    let all = mcp_text(
        &tool(
            app,
            "atlas_grep",
            json!({"query":"ONLY_IN_TEST","include_tests":true}),
        )
        .await,
    );
    assert!(all.contains("common/tests/WalletTest.php:1:"), "{all}");
}

#[tokio::test]
async fn grep_uses_its_own_scope_and_reads_refuse_secrets_and_excluded_paths() {
    let dir = std::env::temp_dir().join(format!("atlas-scopes-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("generated")).unwrap();
    std::fs::create_dir_all(dir.join("private")).unwrap();
    std::fs::write(dir.join("generated/map.php"), "<?php SCOPE_MARKER\n").unwrap();
    std::fs::write(dir.join("private/keys.php"), "<?php SCOPE_MARKER\n").unwrap();
    std::fs::write(dir.join(".env"), "SCOPE_MARKER=secret\n").unwrap();
    let mut cfg = config();
    cfg.project_root = dir;
    cfg.excludes = vec!["generated/*".into(), "private/*".into()];
    cfg.grep_excludes = vec!["private/*".into()];
    cfg.read_excludes = vec!["private/*".into()];
    let app = router(lean_service(), cfg);
    let grep = mcp_text(&tool(app.clone(), "atlas_grep", json!({"query":"SCOPE_MARKER"})).await);
    assert!(
        grep.contains("generated/map.php:1:"),
        "index-only exclusion: {grep}"
    );
    assert!(!grep.contains("private/keys.php"), "{grep}");
    assert!(!grep.contains(".env"), "{grep}");
    for path in [".env", "private/keys.php"] {
        let read = tool(
            app.clone(),
            "atlas_read",
            json!({"path": path, "start_line": 1}),
        )
        .await;
        assert_eq!(read["result"]["isError"], true, "{path}: {read}");
        assert!(
            mcp_text(&read).contains("outside the readable scope"),
            "{read}"
        );
    }
}

#[tokio::test]
async fn grep_exclusions_also_apply_to_indexed_paths() {
    let dir = std::env::temp_dir().join(format!("atlas-grep-indexed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("w.php"), "<?php $wallet->dep_tax_control = 0;\n").unwrap();
    let mut cfg = config();
    cfg.project_root = dir;
    cfg.grep_excludes = vec!["w.php".into()];
    let app = router(lean_service(), cfg);
    let grep = mcp_text(&tool(app, "atlas_grep", json!({"query":"dep_tax_control"})).await);
    assert!(!grep.contains("w.php:"), "{grep}");
}

fn areas_service() -> Arc<Mutex<AtlasService>> {
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let snapshot: GraphSnapshot = serde_json::from_value(json!({"nodes":[
        {"id":"method:pay", "kind":"method", "name":"pay", "span":{"path":"lib/P.php","start_line":2,"end_line":3}},
        {"id":"method:p\\A::run", "kind":"method", "name":"run", "span":{"path":"partner/A.php","start_line":2,"end_line":6}, "metadata":{"area":"partner"}},
        {"id":"call_site:p:1", "kind":"call_site", "name":"$w->pay()", "span":{"path":"partner/A.php","start_line":4,"end_line":4}, "metadata":{"area":"partner","member_name":"pay"}},
        {"id":"method:a\\B::run", "kind":"method", "name":"run", "span":{"path":"admin/B.php","start_line":2,"end_line":6}, "metadata":{"area":"admin"}},
        {"id":"call_site:a:1", "kind":"call_site", "name":"$w->pay()", "span":{"path":"admin/B.php","start_line":4,"end_line":4}, "metadata":{"area":"admin","member_name":"pay"}}
    ], "edges":[
        {"id":"c1","kind":"contains","from":"method:p\\A::run","to":"call_site:p:1","span":{"path":"partner/A.php","start_line":4,"end_line":4}},
        {"id":"k1","kind":"calls","from":"call_site:p:1","to":"method:pay","span":{"path":"partner/A.php","start_line":4,"end_line":4}},
        {"id":"c2","kind":"contains","from":"method:a\\B::run","to":"call_site:a:1","span":{"path":"admin/B.php","start_line":4,"end_line":4}},
        {"id":"k2","kind":"calls","from":"call_site:a:1","to":"method:pay","span":{"path":"admin/B.php","start_line":4,"end_line":4}}
    ]})).unwrap();
    service
        .index(
            snapshot,
            vec![
                "lib/P.php".into(),
                "partner/A.php".into(),
                "admin/B.php".into(),
            ],
            vec![],
        )
        .unwrap();
    Arc::new(Mutex::new(service))
}

#[tokio::test]
async fn area_argument_limits_search_grep_outline_and_callers() {
    let dir = std::env::temp_dir().join(format!("atlas-areas-{}", std::process::id()));
    for folder in ["partner", "admin"] {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
        std::fs::write(dir.join(format!("{folder}/note.txt")), "AREA_MARKER\n").unwrap();
    }
    let mut cfg = config();
    cfg.project_root = dir;
    cfg.areas = vec![
        code_atlas::domain::Area {
            prefix: "partner".into(),
            paths: vec!["partner".into()],
            shared: false,
        },
        code_atlas::domain::Area {
            prefix: "admin".into(),
            paths: vec!["admin".into()],
            shared: false,
        },
    ];
    let app = router(areas_service(), cfg);
    let search = mcp_text(
        &tool(
            app.clone(),
            "atlas_search",
            json!({"query":"run","area":"admin"}),
        )
        .await,
    );
    assert_eq!(text_ids(&search), ["method:a\\B::run"], "{search}");
    assert!(search.contains(": [admin] method run"), "{search}");
    let grep = mcp_text(
        &tool(
            app.clone(),
            "atlas_grep",
            json!({"query":"AREA_MARKER","area":["partner"]}),
        )
        .await,
    );
    assert!(
        grep.contains("partner/note.txt:1:") && !grep.contains("admin/note.txt"),
        "{grep}"
    );
    let outline = mcp_text(
        &tool(
            app.clone(),
            "atlas_outline",
            json!({"entity_id":"method:pay","sections":["called_by"],"area":"partner"}),
        )
        .await,
    );
    assert!(
        outline.contains("p\\A::run L4") && !outline.contains("a\\B::run"),
        "{outline}"
    );
    let tree = mcp_text(
        &tool(
            app.clone(),
            "atlas_context",
            json!({"entity_id":"method:pay","direction":"in","area":"admin"}),
        )
        .await,
    );
    assert!(
        tree.contains("└ [admin] a\\B::run L4") && !tree.contains("p\\A::run"),
        "{tree}"
    );
    let unknown = tool(app, "atlas_search", json!({"query":"run","area":"nope"})).await;
    let message = unknown["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        message.contains("nope") && message.contains("partner"),
        "{unknown}"
    );
}
