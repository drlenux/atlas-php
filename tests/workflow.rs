use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use code_atlas::{
    config::Config,
    http::{router, HttpConfig},
    indexer::IndexCoordinator,
    repository::MemoryRepository,
    service::AtlasService,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

async fn request(app: axum::Router, path: &str, token: &str, body: Value) -> Value {
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
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn native_php_to_mcp_correction_reindex_review_and_analytics() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("Domain.php"),
        include_str!("fixtures/php/app/Domain.php"),
    )
    .unwrap();
    let config = Config::from_map(&BTreeMap::from([
        (
            "ATLAS_PROJECT_ROOT".into(),
            root.path().display().to_string(),
        ),
        ("ATLAS_DATABASE_URL".into(), "dbname=test".into()),
        ("ATLAS_AGENT_TOKEN".into(), "agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "reviewer".into()),
    ]))
    .unwrap();
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let mut indexer = IndexCoordinator::new(config);
    let report = indexer.index(&mut service).unwrap();
    assert!(report.nodes > 20 && report.edges > 20);
    let shared = Arc::new(Mutex::new(service));
    let app = router(
        shared.clone(),
        HttpConfig {
            agent_token: "agent".into(),
            review_token: "reviewer".into(),
            allowed_origins: vec![],
            max_request_bytes: 16384,
            max_context_nodes: 100,
            max_context_depth: 3,
            project_root: std::env::temp_dir(),
            watch_dirs: vec![".".into()],
            excludes: vec![],
            include_vendor: false,
            declaration_paths: vec![],
            language_plugins: vec!["php".into()],
            framework_plugins: vec!["yii2".into()],
            max_file_bytes: 2_000_000,
            grep_max_file_bytes: 2_000_000,
            test_paths: vec![],
            grep_excludes: vec![],
            read_excludes: vec![],
            areas: vec![],
        },
    );
    let id = "method:App\\Shop::choose";
    let context=request(app.clone(),"/mcp","agent",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"atlas_context","arguments":{"entity_id":id,"detail":"full"}}})).await;
    assert_eq!(context["result"]["isError"], false);
    let text = context["result"]["content"][0]["text"].as_str().unwrap();
    let node = text
        .lines()
        .find(|l| l.contains(&format!(" | id={id}")) && !l.contains("-> "))
        .unwrap_or_else(|| panic!("no node line: {text}"));
    // `returns=` carries compact JSON; parse the value that starts there.
    let returns_start = node.find("returns=").expect("returns field") + "returns=".len();
    let returns: serde_json::Value = serde_json::Deserializer::from_str(&node[returns_start..])
        .into_iter()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(returns.as_array().unwrap().len(), 3);
    let revision: u64 = node
        .split(' ')
        .find_map(|t| t.strip_prefix("rev="))
        .unwrap()
        .parse()
        .unwrap();
    let correction=request(app.clone(),"/mcp","agent",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"atlas_correct","arguments":{"kind":"node","id":id,"expected_revision":revision,"patch":{"metadata":{"note":"Reviewed mechanism"}},"reason":"test investigation"}}})).await;
    assert_eq!(correction["result"]["isError"], false);
    {
        let mut service = shared.lock().unwrap();
        indexer.index(&mut service).unwrap();
        let root = service.context(id, 0, 1).unwrap().nodes.remove(0);
        assert_eq!(root.trust, 2);
        assert_eq!(root.metadata["note"], "Reviewed mechanism");
    }
    let pending = shared
        .lock()
        .unwrap()
        .pending_reviews(10)
        .unwrap()
        .remove(0);
    request(
        app.clone(),
        &format!("/api/reviews/{}/decision", pending.id),
        "reviewer",
        json!({"decision":"approve","expected_revision":pending.revision}),
    )
    .await;
    let mut service = shared.lock().unwrap();
    assert_eq!(service.context(id, 0, 1).unwrap().nodes[0].trust, 3);
    assert_eq!(
        service
            .analytics(0, 4102444800, 86400)
            .unwrap()
            .total_responses,
        1
    );
}
