use crate::{
    domain::{
        AtlasError, CallersRequest, ContextRequest, Detail, Direction, ErrorKind, Expansion,
        NodeSearch, SearchMode, SearchPage,
    },
    grep_search::{GrepOutput, GrepPage, GrepRequest},
    mcp_text,
    project::SourceWalk,
    service::{AtlasService, SupplementRequest},
};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const MCP_VERSION: &str = "2025-11-25";
static DELIVERY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct HttpConfig {
    pub agent_token: String,
    pub review_token: String,
    pub allowed_origins: Vec<String>,
    pub max_request_bytes: usize,
    pub max_context_nodes: usize,
    pub max_context_depth: usize,
    /// Project files root for optional `withGrep` supplement on atlas_search.
    pub project_root: PathBuf,
    /// Same discovery knobs as indexing so withGrep sees config files without graph nodes.
    pub watch_dirs: Vec<String>,
    pub excludes: Vec<String>,
    pub include_vendor: bool,
    pub declaration_paths: Vec<String>,
    pub language_plugins: Vec<String>,
    pub framework_plugins: Vec<String>,
    pub max_file_bytes: u64,
    /// Size limit of text search reads (`ATLAS_GREP_MAX_FILE_BYTES`).
    pub grep_max_file_bytes: u64,
    /// Test-code globs left out of agent views (`ATLAS_TEST_PATHS`).
    pub test_paths: Vec<String>,
    /// Text search walk exclusions (`exclude` + `grep.exclude`).
    pub grep_excludes: Vec<String>,
    /// Paths `atlas_read` refuses besides the built-in ones.
    pub read_excludes: Vec<String>,
    /// Application folders (`area` argument).
    pub areas: Vec<crate::domain::Area>,
}

#[derive(Clone)]
struct HttpState {
    service: Arc<Mutex<AtlasService>>,
    progress: crate::progress::IndexingProgress,
    config: HttpConfig,
}

/// Transport adapters deliberately delegate every use case to AtlasService.
pub fn router(service: Arc<Mutex<AtlasService>>, config: HttpConfig) -> Router {
    let progress = service
        .lock()
        .expect("service available during router setup")
        .indexing_progress();
    let state = HttpState {
        service,
        config,
        progress,
    };
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route(
            "/assets/styles.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/styles.css"),
                )
            }),
        )
        .route(
            "/assets/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/assets/model.mjs",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/model.mjs"),
                )
            }),
        )
        .route(
            "/assets/icon.svg",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/svg+xml")],
                    include_str!("../web/icon.svg"),
                )
            }),
        )
        .route(
            "/favicon.ico",
            get(|| async { Redirect::permanent("/assets/icon.svg") }),
        )
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","service":"code-atlas"})) }),
        )
        .route(
            "/assets/explore.mjs",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/explore.mjs"),
                )
            }),
        )
        .route(
            "/assets/indexing.mjs",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/indexing.mjs"),
                )
            }),
        )
        .route(
            "/mcp",
            post(mcp).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .route("/api/reviews", get(reviews))
        .route("/api/reviews/:id/decision", post(review_decision))
        .route("/api/analytics", get(analytics))
        .route("/api/indexing", get(indexing))
        .route("/api/reindex", post(reindex))
        .route("/api/atlas/reset", post(reset_atlas))
        .route("/api/search", get(search))
        .route("/api/context/:id", get(context))
        .layer(DefaultBodyLimit::max(state.config.max_request_bytes))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Queues a full reindex without waiting for the service lock held by publication.
async fn reindex(State(state): State<HttpState>) -> (StatusCode, Json<Value>) {
    state.progress.request_reindex();
    (
        StatusCode::ACCEPTED,
        Json(json!({"reindex_queued": state.progress.snapshot().reindex_queued})),
    )
}

#[derive(Deserialize)]
struct ResetRequest {
    confirm: Option<String>,
}
/// Reviewer-only full reset; the explicit confirmation guards against accidental calls.
async fn reset_atlas(
    State(state): State<HttpState>,
    Json(request): Json<ResetRequest>,
) -> Result<Json<Value>, ApiError> {
    if request.confirm.as_deref() != Some("reset") {
        return Err(ApiError(AtlasError::invalid(
            "confirm must be \"reset\" to delete the atlas",
        )));
    }
    let progress = state.progress.clone();
    service_call(state, |service| service.reset()).await?;
    progress.request_reindex();
    Ok(Json(json!({
        "reset": true,
        "reindex_queued": progress.snapshot().reindex_queued
    })))
}

async fn indexing(State(state): State<HttpState>) -> Json<crate::progress::ProgressSnapshot> {
    Json(state.progress.snapshot())
}

async fn guard(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let token = if path == "/mcp" {
        Some(&state.config.agent_token)
    } else if path.starts_with("/api/") {
        Some(&state.config.review_token)
    } else {
        None
    };
    if let Some(expected) = token {
        // Bundled public assets must load before the browser can authenticate.
        // Apply the configured Origin policy only to protected API routes.
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            if !origin.to_str().is_ok_and(|value| {
                state
                    .config
                    .allowed_origins
                    .iter()
                    .any(|allowed| allowed == value)
            }) {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error":"Origin is not allowed"})),
                )
                    .into_response();
            }
        }
        let provided = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .unwrap_or("");
        if !same_token(provided, expected) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(json!({"error":"Authentication required"})),
            )
                .into_response();
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'"));
    response
}

fn same_token(provided: &str, expected: &str) -> bool {
    if expected.is_empty() || provided.len() != expected.len() {
        return false;
    }
    provided
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |different, (a, b)| different | (a ^ b))
        == 0
}

async fn service_call<T, F>(state: HttpState, action: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&mut AtlasService) -> crate::domain::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let mut service = state
            .service
            .lock()
            .map_err(|_| AtlasError::storage("Service lock unavailable"))?;
        action(&mut service)
    })
    .await
    .map_err(|_| ApiError(AtlasError::storage("Service task failed")))?
    .map_err(ApiError)
}

struct ApiError(AtlasError);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.kind {
            ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::Conflict => StatusCode::CONFLICT,
            ErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ErrorKind::Storage => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = if self.0.kind == ErrorKind::Storage {
            "Storage operation failed".to_owned()
        } else {
            self.0.message
        };
        (
            status,
            Json(json!({"error":{"kind":self.0.kind,"message":message}})),
        )
            .into_response()
    }
}

#[derive(Deserialize)]
struct PageQuery {
    offset: Option<usize>,
    limit: Option<usize>,
}

async fn reviews(
    State(state): State<HttpState>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    let offset = page.offset.unwrap_or(0);
    let limit = page.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(ApiError(AtlasError::invalid(
            "limit must be between 1 and 100",
        )));
    }
    let fetch = offset
        .checked_add(limit)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| ApiError(AtlasError::invalid("offset is too large")))?;
    let items = service_call(state, move |service| service.pending_reviews(fetch)).await?;
    let has_more = items.len() > offset.saturating_add(limit);
    let items: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
    Ok(Json(
        json!({"items":items,"offset":offset,"limit":limit,"has_more":has_more}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewRequest {
    decision: String,
    expected_revision: u64,
}

async fn review_decision(
    State(state): State<HttpState>,
    Path(id): Path<String>,
    Json(body): Json<ReviewRequest>,
) -> Result<Json<Value>, ApiError> {
    let approve = match body.decision.as_str() {
        "approve" => true,
        "decline" => false,
        _ => {
            return Err(ApiError(AtlasError::invalid(
                "decision must be approve or decline",
            )))
        }
    };
    let correction = service_call(state, move |service| {
        service.review(&id, body.expected_revision, approve)
    })
    .await?;
    Ok(Json(json!(correction)))
}

#[derive(Deserialize)]
struct AnalyticsQuery {
    from: Option<i64>,
    to: Option<i64>,
}
async fn analytics(
    State(state): State<HttpState>,
    Query(query): Query<AnalyticsQuery>,
) -> Result<Json<Value>, ApiError> {
    let to = query.to.unwrap_or_else(now);
    let from = query.from.unwrap_or_else(|| to.saturating_sub(7 * 86400));
    let result = service_call(state, move |service| service.analytics(from, to, 86400)).await?;
    Ok(Json(json!(result)))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    limit: Option<usize>,
    /// Comma-separated node kinds.
    kinds: Option<String>,
    cursor: Option<String>,
    mode: Option<SearchMode>,
    #[serde(default, alias = "withGrep")]
    with_grep: bool,
    #[serde(
        default,
        alias = "grepCursor",
        alias = "grep_next_cursor",
        alias = "grep_cursor"
    )]
    grep_cursor: Option<String>,
}
async fn search(
    State(state): State<HttpState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(ApiError(AtlasError::invalid(
            "limit must be between 1 and 100",
        )));
    }
    let with_grep = query.with_grep;
    let mut grep_cursor = query.grep_cursor.filter(|c| !c.is_empty());
    let mut node_cursor = query.cursor.filter(|cursor| !cursor.is_empty());
    // path:line in `cursor` while withGrep → grep page, not node keyset.
    if with_grep && grep_cursor.is_none() {
        if let Some(c) = node_cursor.as_deref().filter(|c| looks_like_grep_cursor(c)) {
            grep_cursor = Some(c.to_owned());
            node_cursor = None;
        }
    }
    let http_config = state.config.clone();
    let search = NodeSearch {
        text: query.q,
        kinds: query
            .kinds
            .map(|kinds| kinds.split(',').map(str::to_owned).collect())
            .unwrap_or_default(),
        after: node_cursor,
        limit,
        // Reviewers type partial names; agents get whole-identifier `text` by default.
        mode: query.mode.unwrap_or(SearchMode::Substring),
        // The reviewer sees tests; agent views leave them out by default.
        exclude_paths: None,
        include_paths: None,
    };
    let value = service_call(state, move |service| {
        let page = service.search_page(&search, Detail::Summary)?;
        search_payload(
            service,
            &http_config,
            &search,
            with_grep,
            grep_cursor.as_deref(),
            page,
        )
    })
    .await?;
    Ok(Json(value))
}

#[derive(Deserialize)]
struct ContextQuery {
    depth: Option<usize>,
    max_nodes: Option<usize>,
    direction: Option<Direction>,
    detail: Option<Detail>,
    include_diagnostics: Option<bool>,
}
async fn context(
    State(state): State<HttpState>,
    Path(id): Path<String>,
    query: Result<Query<ContextQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError(AtlasError::invalid(error.body_text())))?;
    let config = &state.config;
    let within = |value: usize, max: usize, name: &str| {
        if value > max {
            Err(ApiError(AtlasError::invalid(format!(
                "{name} must be at most {max}"
            ))))
        } else {
            Ok(value)
        }
    };
    // Defaults keep the reviewer's full view; the shipped page asks for lean views.
    let request = ContextRequest {
        root_id: id,
        depth: within(
            query.depth.unwrap_or(config.max_context_depth.min(2)),
            config.max_context_depth,
            "depth",
        )?,
        max_nodes: within(
            query.max_nodes.unwrap_or(config.max_context_nodes),
            config.max_context_nodes,
            "max_nodes",
        )?,
        direction: query.direction.unwrap_or(Direction::Both),
        detail: query.detail.unwrap_or(Detail::Full),
        include_diagnostics: query.include_diagnostics.unwrap_or(true),
        expansion: Expansion::Full,
    };
    let result = service_call(state, move |service| service.context_view(&request)).await?;
    Ok(Json(json!(result)))
}

async fn mcp(State(state): State<HttpState>, headers: HeaderMap, bytes: Bytes) -> Response {
    let parsed = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc_error(Value::Null, -32700, "Parse error")),
            )
                .into_response()
        }
    };
    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    let json_content = headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| h.split(';').next().unwrap_or("").trim() == "application/json");
    if !json_content {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(rpc_error(
                id,
                -32600,
                "Content-Type must be application/json",
            )),
        )
            .into_response();
    }
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !accept
        .split(',')
        .any(|v| v.trim().starts_with("application/json"))
        || !accept
            .split(',')
            .any(|v| v.trim().starts_with("text/event-stream"))
    {
        return (
            StatusCode::NOT_ACCEPTABLE,
            Json(rpc_error(
                id,
                -32600,
                "Accept must include application/json and text/event-stream",
            )),
        )
            .into_response();
    }
    let version = headers
        .get("mcp-protocol-version")
        .and_then(|h| h.to_str().ok());
    if version.is_some_and(|v| v != MCP_VERSION)
        || (version.is_none() && parsed.get("method").and_then(Value::as_str) != Some("initialize"))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_error(
                id,
                -32600,
                "Supported MCP-Protocol-Version: 2025-11-25",
            )),
        )
            .into_response();
    }
    let config = state.config.clone();
    match service_call(state, move |service| {
        Ok(dispatch_rpc(service, parsed, &config))
    })
    .await
    {
        Ok(RpcReply::Response(value)) => Json(value).into_response(),
        Ok(RpcReply::Notification) => StatusCode::ACCEPTED.into_response(),
        Err(error) => error.into_response(),
    }
}

/// Shared JSON-RPC adapter for HTTP and a newline-delimited stdio transport.
/// HTTP authenticates before calling this; stdio is trusted by its launching process.
pub enum RpcReply {
    Response(Value),
    Notification,
}

pub fn dispatch_rpc(service: &mut AtlasService, request: Value, config: &HttpConfig) -> RpcReply {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let valid_id = id.is_string() || id.as_i64().is_some() || id.as_u64().is_some();
    let method = request.get("method").and_then(Value::as_str);
    if !request.is_object()
        || request.get("jsonrpc") != Some(&json!("2.0"))
        || method.is_none()
        || (request.get("id").is_some() && !valid_id)
    {
        return RpcReply::Response(rpc_error(Value::Null, -32600, "Invalid JSON-RPC request"));
    }
    let method = method.unwrap();
    // Notifications never invoke tools or mutate graph/analytics state.
    if request.get("id").is_none() {
        return RpcReply::Notification;
    }
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return RpcReply::Response(rpc_error(id, -32602, "params must be an object"));
    }
    let result = match method {
        "initialize" => {
            if params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .is_none()
            {
                return RpcReply::Response(rpc_error(id, -32602, "protocolVersion is required"));
            }
            json!({"protocolVersion":MCP_VERSION,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"code-atlas","version":env!("CARGO_PKG_VERSION")},"instructions":"Results are grep-style text, not JSON. Lines may carry an [area] label (application folder from .atlas/setup.yaml); area:\"admin\" limits search, grep, outline callers and callers trees. atlas_batch runs up to 10 read-only calls at once. atlas_context direction in = callers tree (call levels); atlas_outline sections:[\"called_by\"] for direct callers; atlas_search (withGrep adds source lines atlas missed); atlas_grep full-text over every text file; atlas_read narrow slices. Tests are excluded unless include_tests. Graph resolution is not proof of unused code. Returned context is measured as delivery, not actual agent usage."})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":tool_definitions(config)}),
        "tools/call" => {
            let name = match params.get("name").and_then(Value::as_str) {
                Some(name) => name,
                None => return RpcReply::Response(rpc_error(id, -32602, "Tool name is required")),
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call_tool(service, name, arguments, config) {
                Ok(text) => tool_text(text),
                Err(ToolError::Arguments(message)) => {
                    return RpcReply::Response(rpc_error(id, -32602, &message))
                }
                Err(ToolError::Domain(error)) => {
                    json!({"content":[{"type":"text","text":domain_message(error)}],"isError":true})
                }
            }
        }
        _ => return RpcReply::Response(rpc_error(id, -32601, "Method not found")),
    };
    RpcReply::Response(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn delivery_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "http-{}-{nanos}-{}",
        std::process::id(),
        DELIVERY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

enum ToolError {
    Arguments(String),
    Domain(AtlasError),
}
impl From<AtlasError> for ToolError {
    fn from(error: AtlasError) -> Self {
        Self::Domain(error)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    kinds: Vec<String>,
    limit: Option<usize>,
    cursor: Option<String>,
    #[serde(default)]
    mode: SearchMode,
    /// When true, append literal file hits not already covered by returned nodes.
    #[serde(default, alias = "withGrep")]
    with_grep: bool,
    /// Resume grep after this `path:line` from a previous `grep.next_cursor`.
    #[serde(
        default,
        alias = "grepCursor",
        alias = "grep_next_cursor",
        alias = "grep_cursor"
    )]
    grep_cursor: Option<String>,
    /// Keep nodes under `ATLAS_TEST_PATHS` (left out by default).
    #[serde(default, alias = "includeTests")]
    include_tests: bool,
    /// Only these areas (`.atlas/setup.yaml` `areas`).
    #[serde(default)]
    area: OneOrMany,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextArgs {
    entity_id: String,
    depth: Option<usize>,
    max_nodes: Option<usize>,
    #[serde(default)]
    direction: Direction,
    #[serde(default)]
    detail: Detail,
    #[serde(default)]
    include_diagnostics: bool,
    /// Keep callers under `ATLAS_TEST_PATHS` (direction `in`).
    #[serde(default, alias = "includeTests")]
    include_tests: bool,
    /// Only callers in these areas.
    #[serde(default)]
    area: OneOrMany,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutlineArgs {
    entity_id: String,
    /// When non-empty, keep only these outline fields (e.g. ["called_by"] for callers).
    #[serde(default)]
    sections: Vec<String>,
    /// Keep `called_by` sites under `ATLAS_TEST_PATHS`.
    #[serde(default, alias = "includeTests")]
    include_tests: bool,
    /// Only `called_by` sites in these areas.
    #[serde(default)]
    area: OneOrMany,
}
/// A string or a list of strings (`path_glob: "*.sh"` or `["*.php", "!*/tests/*"]`).
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum OneOrMany {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(one) => vec![one],
            Self::Many(many) => many,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrepArgs {
    query: String,
    #[serde(default)]
    regex: bool,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default)]
    word: bool,
    path_prefix: Option<String>,
    #[serde(default)]
    path_glob: OneOrMany,
    #[serde(default)]
    include_vendor: bool,
    /// Keep files under `ATLAS_TEST_PATHS` (left out by default).
    #[serde(default, alias = "includeTests")]
    include_tests: bool,
    /// Only files in these areas.
    #[serde(default)]
    area: OneOrMany,
    #[serde(default)]
    output: GrepOutput,
    #[serde(default)]
    context: Option<usize>,
    limit: Option<usize>,
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: Option<String>,
    entity_id: Option<String>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    around_line: Option<usize>,
    #[serde(default)]
    context: Option<usize>,
}
fn default_true() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectArgs {
    kind: String,
    id: String,
    expected_revision: u64,
    patch: Value,
    reason: String,
}

fn arguments<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|error| ToolError::Arguments(error.to_string()))
}
fn bounded(value: usize, min: usize, max: usize, name: &str) -> Result<usize, ToolError> {
    if value < min || value > max {
        Err(ToolError::Arguments(format!(
            "{name} must be between {min} and {max}"
        )))
    } else {
        Ok(value)
    }
}
/// Agent-visible text of a domain error; storage details stay in the server log.
fn domain_message(error: AtlasError) -> String {
    if error.kind == ErrorKind::Storage {
        "Storage operation failed".to_owned()
    } else {
        error.message
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchArgs {
    calls: Vec<BatchCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchCall {
    tool: String,
    #[serde(default)]
    arguments: Value,
}

/// Calls per `atlas_batch` request.
const MAX_BATCH_CALLS: usize = 10;
/// Read-only tools a batch may run (no nesting, no corrections).
const BATCH_TOOLS: [&str; 5] = [
    "atlas_search",
    "atlas_context",
    "atlas_outline",
    "atlas_grep",
    "atlas_read",
];

/// `atlas_batch`: each call runs through `call_tool` (same validation and analytics);
/// one index line on top, then `## [n] tool` sections; a failing call becomes an
/// error section and does not stop the others.
fn run_batch(
    service: &mut AtlasService,
    args: Value,
    config: &HttpConfig,
) -> Result<String, ToolError> {
    let args: BatchArgs = arguments(args)?;
    bounded(args.calls.len(), 1, MAX_BATCH_CALLS, "calls")?;
    let mut sections = Vec::new();
    for (index, call) in args.calls.into_iter().enumerate() {
        let n = index + 1;
        let tool = call.tool;
        let result = if !BATCH_TOOLS.contains(&tool.as_str()) {
            Err(ToolError::Arguments(format!(
                "{tool} is not allowed in atlas_batch (read-only tools: {})",
                BATCH_TOOLS.join(", ")
            )))
        } else {
            let arguments = if call.arguments.is_null() {
                json!({})
            } else {
                call.arguments
            };
            call_tool(service, &tool, arguments, config)
        };
        sections.push(match result {
            Ok(text) => format!("## [{n}] {tool}\n{}", mcp_text::without_index(&text)),
            Err(ToolError::Arguments(message)) => format!("## [{n}] {tool} error: {message}"),
            Err(ToolError::Domain(error)) => {
                format!("## [{n}] {tool} error: {}", domain_message(error))
            }
        });
    }
    let index = mcp_text::index_line(&service.freshness()?);
    Ok(format!("{index}\n{}", sections.join("\n")))
}

/// Successful tool result: one grep-style text block (see `mcp_text`).
fn tool_text(text: String) -> Value {
    json!({"content":[{"type":"text","text":text}],"isError":false})
}

/// Runs one tool and returns its text result (see `mcp_text`).
fn call_tool(
    service: &mut AtlasService,
    name: &str,
    args: Value,
    config: &HttpConfig,
) -> Result<String, ToolError> {
    match name {
        "atlas_batch" => run_batch(service, args, config),
        "atlas_search" => {
            let mut args: SearchArgs = arguments(args)?;
            let mut grep_cursor = args.grep_cursor.filter(|c| !c.is_empty());
            let mut node_cursor = args.cursor.filter(|c| !c.is_empty());
            if args.with_grep && grep_cursor.is_none() {
                if let Some(c) = node_cursor.as_deref().filter(|c| looks_like_grep_cursor(c)) {
                    grep_cursor = Some(c.to_owned());
                    node_cursor = None;
                }
            }
            let search = NodeSearch {
                text: args.query.clone(),
                kinds: args.kinds,
                after: node_cursor,
                limit: bounded(args.limit.unwrap_or(20), 1, 100, "limit")?,
                mode: args.mode,
                exclude_paths: test_exclusion(config, args.include_tests),
                include_paths: area_filter(config, std::mem::take(&mut args.area))?,
            };
            let page = service.search_page(&search, Detail::Summary)?;
            let nodes: Vec<_> = page.nodes.iter().map(|n| n.id.clone()).collect();
            let grep = if args.with_grep {
                grep_supplement(service, config, &search, grep_cursor.as_deref())?
            } else {
                None
            };
            let result = mcp_text::search(
                &service.freshness()?,
                &page,
                grep.as_ref(),
                search.exclude_paths.is_some(),
            );
            if !nodes.is_empty() {
                service.record_delivery(&delivery_id(), &nodes, &[], now())?;
            }
            Ok(result)
        }
        "atlas_context" => {
            let args: ContextArgs = arguments(args)?;
            // Callers tree for direction in; a class answers "who uses it" unless `out`.
            if args.direction == Direction::In
                || (args.direction == Direction::Both && service.is_class_like(&args.entity_id)?)
            {
                return callers_text(service, args, config);
            }
            let depth = bounded(
                args.depth.unwrap_or(config.max_context_depth.min(2)),
                0,
                config.max_context_depth,
                "depth",
            )?;
            let limit = bounded(
                args.max_nodes.unwrap_or(config.max_context_nodes),
                1,
                config.max_context_nodes,
                "max_nodes",
            )?;
            let context = service.context_view(&ContextRequest {
                root_id: args.entity_id,
                depth,
                max_nodes: limit,
                direction: args.direction,
                detail: args.detail,
                include_diagnostics: args.include_diagnostics,
                expansion: Expansion::Focused,
            })?;
            let nodes: Vec<_> = context.nodes.iter().map(|n| n.id.clone()).collect();
            let edges: Vec<_> = context.edges.iter().map(|e| e.id.clone()).collect();
            // Serialize successfully before recording. This records an issued response,
            // not a TCP acknowledgement or proof that the agent used its contents.
            let result = mcp_text::context(&service.freshness()?, &context);
            service.record_delivery(&delivery_id(), &nodes, &edges, now())?;
            Ok(result)
        }
        "atlas_outline" => {
            let args: OutlineArgs = arguments(args)?;
            let area = area_filter(config, args.area)?;
            let (outline, nodes, edges) = service.outline_view(
                &args.entity_id,
                &args.sections,
                test_exclusion(config, args.include_tests).as_deref(),
                area.as_deref(),
            )?;
            let result = mcp_text::outline(&service.freshness()?, &outline);
            // Count the opened entity and the graph edges/nodes the outline actually read.
            service.record_delivery(&delivery_id(), &nodes, &edges, now())?;
            Ok(result)
        }
        "atlas_grep" => {
            let mut args: GrepArgs = arguments(args)?;
            if args.query.is_empty() {
                return Err(ToolError::Arguments("query must not be empty".into()));
            }
            let limit = bounded(args.limit.unwrap_or(50), 1, 200, "limit")?;
            let context = bounded(args.context.unwrap_or(0), 0, 5, "context")?;
            let path_globs = std::mem::take(&mut args.path_glob).into_vec();
            let tests = test_exclusion(config, args.include_tests);
            let area = area_filter(config, std::mem::take(&mut args.area))?;
            let include_vendor = config.include_vendor || args.include_vendor;
            let walk = SourceWalk {
                include_vendor,
                ..source_walk(config)
            };
            let page = service.grep_source(
                &config.project_root,
                walk,
                GrepRequest {
                    query: &args.query,
                    limit,
                    after: args.cursor.as_deref(),
                    kinds: &[],
                    symbol_mode: false,
                    regex: args.regex,
                    case_sensitive: args.case_sensitive,
                    word: args.word,
                    path_prefix: args.path_prefix.as_deref(),
                    path_globs: &path_globs,
                    exclude_paths: tests.as_deref(),
                    include_paths: area.as_deref(),
                    include_vendor: args.include_vendor || config.include_vendor,
                    output: args.output,
                    context,
                    max_file_bytes: config.grep_max_file_bytes,
                },
            )?;
            Ok(mcp_text::grep(&service.freshness()?, &page))
        }
        "atlas_read" => {
            let args: ReadArgs = arguments(args)?;
            let slice = service.read_source(
                &config.project_root,
                &crate::service::ReadSourceRequest {
                    path: args.path.as_deref(),
                    entity_id: args.entity_id.as_deref(),
                    start_line: args.start_line,
                    end_line: args.end_line,
                    around_line: args.around_line,
                    context: args.context.unwrap_or(20),
                    max_lines: 400,
                    read_excludes: &config.read_excludes,
                },
            )?;
            Ok(mcp_text::read(&service.freshness()?, &slice))
        }
        "atlas_correct" => {
            let args: CorrectArgs = arguments(args)?;
            if !matches!(args.kind.as_str(), "node" | "edge") {
                return Err(ToolError::Arguments("kind must be node or edge".into()));
            }
            Ok(mcp_text::correction(&service.correct(
                &args.kind,
                &args.id,
                args.expected_revision,
                args.patch,
                &args.reason,
            )?))
        }
        _ => Err(ToolError::Arguments("Unknown tool".into())),
    }
}

/// Adds how current the atlas is, so the agent can judge the answer's freshness.
/// Atlas page plus the optional `withGrep` supplement (lines atlas did not return).
fn search_payload(
    service: &mut AtlasService,
    config: &HttpConfig,
    search: &NodeSearch,
    with_grep: bool,
    grep_cursor: Option<&str>,
    page: SearchPage,
) -> crate::domain::Result<Value> {
    let grep = if with_grep {
        grep_supplement(service, config, search, grep_cursor)?
    } else {
        None
    };
    let mut value = json!(page);
    if let Some(g) = grep {
        value["grep"] = json!(g);
        // Flat aliases for clients that expect the previous shape.
        value["grep_truncated"] = json!(g.truncated);
        value["grep_next_cursor"] = json!(g.next_cursor);
    }
    Ok(value)
}

fn grep_supplement(
    service: &mut AtlasService,
    config: &HttpConfig,
    search: &NodeSearch,
    grep_cursor: Option<&str>,
) -> crate::domain::Result<Option<GrepPage>> {
    service.grep_supplement(
        &config.project_root,
        source_walk(config),
        &SupplementRequest {
            search,
            grep_cursor,
            include_vendor: config.include_vendor,
            max_file_bytes: config.grep_max_file_bytes,
        },
    )
}

/// `atlas_context direction:in`: callers tree, depth = call levels.
fn callers_text(
    service: &mut AtlasService,
    args: ContextArgs,
    config: &HttpConfig,
) -> Result<String, ToolError> {
    let levels = bounded(
        args.depth.unwrap_or(config.max_context_depth.min(3)).max(1),
        1,
        config.max_context_depth.max(1),
        "depth",
    )?;
    let max_nodes = bounded(
        args.max_nodes.unwrap_or(config.max_context_nodes),
        1,
        config.max_context_nodes,
        "max_nodes",
    )?;
    let tree = service.callers_tree(&CallersRequest {
        root_id: args.entity_id,
        levels,
        max_nodes,
        exclude_paths: test_exclusion(config, args.include_tests),
        include_paths: area_filter(config, args.area)?,
    })?;
    let text = mcp_text::callers(&service.freshness()?, &tree);
    let mut nodes = vec![tree.root_id.clone()];
    collect_caller_ids(&tree.children, &mut nodes);
    service.record_delivery(&delivery_id(), &nodes, &[], now())?;
    Ok(text)
}

fn collect_caller_ids(children: &[crate::domain::CallerNode], ids: &mut Vec<String>) {
    for child in children {
        if let Some(id) = &child.id {
            ids.push(id.clone());
        }
        collect_caller_ids(&child.children, ids);
    }
}

/// Anchored path regex of the named areas; unknown names are argument errors.
fn area_filter(config: &HttpConfig, names: OneOrMany) -> Result<Option<String>, ToolError> {
    let names = names.into_vec();
    if names.is_empty() {
        return Ok(None);
    }
    let mut folders = Vec::new();
    for name in &names {
        let Some(area) = config.areas.iter().find(|a| a.prefix == *name) else {
            let known: Vec<&str> = config.areas.iter().map(|a| a.prefix.as_str()).collect();
            return Err(ToolError::Arguments(format!(
                "unknown area {name}; configured areas: {}",
                if known.is_empty() {
                    "none (set areas in .atlas/setup.yaml)".to_owned()
                } else {
                    known.join(", ")
                }
            )));
        };
        folders.extend(
            area.paths
                .iter()
                .map(|p| regex::escape(p.trim_end_matches('/'))),
        );
    }
    Ok(Some(format!("^(?:{})(?:/|$)", folders.join("|"))))
}

/// Path regex for test code left out of agent views, unless the agent asked for tests.
fn test_exclusion(config: &HttpConfig, include_tests: bool) -> Option<String> {
    if include_tests {
        None
    } else {
        crate::glob::any_regex(&config.test_paths)
    }
}

fn source_walk(config: &HttpConfig) -> SourceWalk<'_> {
    SourceWalk {
        project_root: &config.project_root,
        watch_dirs: &config.watch_dirs,
        // Text search has its own scope (`exclude` + `grep.exclude`).
        excludes: &config.grep_excludes,
        include_vendor: config.include_vendor,
        declaration_paths: &config.declaration_paths,
        language_plugins: &config.language_plugins,
        framework_plugins: &config.framework_plugins,
        max_file_bytes: config.max_file_bytes,
    }
}

fn looks_like_grep_cursor(cursor: &str) -> bool {
    cursor.contains('/')
        && cursor
            .rsplit_once(':')
            .is_some_and(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Graph paths plus filesystem discovery (configs with no nodes still need withGrep).
fn tool_definitions(config: &HttpConfig) -> Vec<Value> {
    vec![
        json!({"name":"atlas_search","description":"Find code entities by name, qualified name, occurrence expression or id. Text result: `# search nodes=N complete|next_cursor=C [tests=excluded]`, then one line per node: `path:line[-end]: kind name [resolution] if(condition)... key=value... | id=ID rev=N`. Ranked exact name > declarations > other > occurrence sites; pass next_cursor as cursor until complete. mode text = whole identifier (like grep -w), never inside string literals; substring = partial; symbol = exact declaration/member. Narrow with kinds (class, method, call_site, write_site, config_key...). Test code is left out unless include_tests. withGrep=true appends `# grep supplement`: source lines matching the same query that atlas did not return (comments, strings, configs, scripts); continue it with grepCursor.","inputSchema":{"type":"object","properties":{"area":{"type":["string","array"],"items":{"type":"string"},"description":"Only these areas (prefixes from .atlas/setup.yaml areas, e.g. partner, admin)"},"include_tests":{"type":"boolean","default":false,"description":"Keep code under ATLAS_TEST_PATHS (left out by default)"},"query":{"type":"string"},"kinds":{"type":"array","items":{"type":"string","pattern":"^[a-z0-9_]+$"},"minItems":1,"maxItems":20,"description":"Node kinds to keep (class, method, config_key, write_site, call_site, …). For withGrep, write_site/writes/magic_set/assignment also restrict grep to assignment-like lines."},"limit":{"type":"integer","minimum":1,"maximum":100,"default":20},"cursor":{"type":"string","description":"next_cursor from the previous node page (or path:line grep cursor when withGrep)"},"mode":{"type":"string","enum":["text","substring","symbol"],"default":"text","description":"text: whole identifier (like grep -w) in names and occurrence expressions; substring: partial match in the same fields; symbol: exact declaration/member/config_key match; withGrep uses word-boundary short name"},"withGrep":{"type":"boolean","default":false,"description":"Append literal grep hits missing from atlas results (scans watched source tree, not only paths already in the graph)"},"grepCursor":{"type":"string","description":"Continue truncated withGrep: pass grep.next_cursor / grep_next_cursor from the previous response"}},"required":["query"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
        json!({"name":"atlas_context","description":"Who calls / uses an entity, or its local graph. direction in (and the default for a class): callers tree `# callers ID levels=N complete|truncated [excluded.tests=N]`, root line, then `├`/`└` lines `Caller::method L12 L30 [resolved] if(cond): expression`; depth = call levels; a class lists its members used from outside, then their callers. direction both/out on a method: numbered nodes `[n] path:line: kind name ... | id=ID rev=N` and edges `[n] -kind-> [m] path:line ...` (edges omit what their source line already shows); declaration members are not listed (omitted_members; use atlas_outline). Test callers are left out unless include_tests. resolution: resolved = from code types; annotated = only via PHPDoc, verify; candidate = several targets; unresolved/external = unknown or outside the index.","inputSchema":{"type":"object","properties":{"area":{"type":["string","array"],"items":{"type":"string"},"description":"Only these areas (prefixes from .atlas/setup.yaml areas, e.g. partner, admin)"},"include_tests":{"type":"boolean","default":false,"description":"Keep code under ATLAS_TEST_PATHS (left out by default)"},"entity_id":{"type":"string"},"depth":{"type":"integer","minimum":0,"maximum":config.max_context_depth},"max_nodes":{"type":"integer","minimum":1,"maximum":config.max_context_nodes},"direction":{"type":"string","enum":["both","in","out"],"default":"both"},"detail":{"type":"string","enum":["summary","full"],"default":"summary"},"include_diagnostics":{"type":"boolean","default":false}},"required":["entity_id"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
        json!({"name":"atlas_outline","description":"Compact structure of a class, interface, trait or method as text: header `kind name path:line | id=ID`, then sections (`called_by:`, `calls:`, `members:`, `parents:`, `overridden_by:`...) with one indented line each; a requested empty section prints `<section>: none`. Interface methods list implementations under overridden_by. Class members: `L<line> <kind> <name>(<params>): <type> [magic|annotation|framework]`. calls: `L<line> <expression> -> <target> [resolution]`; called_by: `<caller> L<line>: <expression> [resolution]` (test callers left out unless include_tests, counted as `called_by (excluded.tests=N):`). sections:[\"called_by\"] for callers only.","inputSchema":{"type":"object","properties":{"area":{"type":["string","array"],"items":{"type":"string"},"description":"Only these areas (prefixes from .atlas/setup.yaml areas, e.g. partner, admin)"},"include_tests":{"type":"boolean","default":false,"description":"Keep code under ATLAS_TEST_PATHS (left out by default)"},"entity_id":{"type":"string"},"sections":{"type":"array","items":{"type":"string","enum":["owner","parents","interfaces","traits","children","implementations","overrides","overridden_by","calls","called_by","members"]},"minItems":1,"maxItems":11,"description":"Keep only these outline fields; omit for the default lean method / full class outline. Use [\"called_by\"] when listing callers."}},"required":["entity_id"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
        json!({"name":"atlas_grep","description":"Full-text search over live sources (PHP, configs, comments, templates), including files without graph nodes; prefer over shell rg. Literal or regex, case_sensitive, word (identifier boundaries), path_prefix, path_glob, include_vendor. Text result in ripgrep layout: `# grep hits=N scanned=F complete|next_cursor=C [incomplete] [excluded.<reason>=N]`, matches `path:line:col: text`, context `path-line- text`, files_with_matches one path per line, count `path:N`, `# skipped path reason`, `# error path: message`. Test code is left out unless include_tests; .git/.idea/.vscode are never scanned. No match lines and no `incomplete` means no match.","inputSchema":{"type":"object","properties":{"area":{"type":["string","array"],"items":{"type":"string"},"description":"Only these areas (prefixes from .atlas/setup.yaml areas, e.g. partner, admin)"},"include_tests":{"type":"boolean","default":false,"description":"Keep files under ATLAS_TEST_PATHS (left out by default, counted as excluded.tests)"},"query":{"type":"string","minLength":1},"regex":{"type":"boolean","default":false},"case_sensitive":{"type":"boolean","default":true},"word":{"type":"boolean","default":false},"path_prefix":{"type":"string"},"path_glob":{"type":["string","array"],"items":{"type":"string"},"description":"Glob(s) on the relative path: * ** ? [abc] [!abc] {a,b}; entries starting with ! exclude, e.g. [\"*.php\",\"!*/tests/*\"]"},"include_vendor":{"type":"boolean","default":false},"output":{"type":"string","enum":["content","files_with_matches","count"],"default":"content"},"context":{"type":"integer","minimum":0,"maximum":5,"default":0},"limit":{"type":"integer","minimum":1,"maximum":200,"default":50},"cursor":{"type":"string","description":"next_cursor from previous page (path:line)"}},"required":["query"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
        json!({"name":"atlas_read","description":"Read live source lines: path+start_line/end_line, around_line+context, or entity_id (graph span, current file text). Max 400 lines. Text result: `# path:start-end [truncated]`, then `line: text`. Use after atlas_grep/outline instead of external Read.","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"entity_id":{"type":"string"},"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1},"around_line":{"type":"integer","minimum":1},"context":{"type":"integer","minimum":0,"maximum":50,"default":20,"description":"Lines around around_line, or ignored when start/end set"}},"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
        json!({"name":"atlas_correct","description":"Correct existing node or edge knowledge at agent trust level 2. Requires current revision (rev= on the node/edge line); cannot overwrite level 3 or create/delete objects. Patch contains metadata and optional edge to target. Text result: `# correction` and key=value lines.","inputSchema":{"type":"object","properties":{"kind":{"type":"string","enum":["node","edge"]},"id":{"type":"string"},"expected_revision":{"type":"integer","minimum":0},"patch":{"type":"object","properties":{"metadata":{"type":"object"},"to":{"type":["string","null"]}},"additionalProperties":false},"reason":{"type":"string","minLength":1}},"required":["kind","id","expected_revision","patch","reason"],"additionalProperties":false},"annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false}}),
        json!({"name":"atlas_batch","description":"Run 1-10 atlas tool calls in one request (like chaining shell commands). Result: one `# index` line, then `## [n] <tool>` followed by that tool's text, or `## [n] <tool> error: ...`; a failing call does not stop the others. Use it to combine independent lookups (several outlines, a grep and a read).","inputSchema":{"type":"object","properties":{"calls":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"object","properties":{"tool":{"type":"string","enum":["atlas_search","atlas_context","atlas_outline","atlas_grep","atlas_read"]},"arguments":{"type":"object"}},"required":["tool","arguments"],"additionalProperties":false}}},"required":["calls"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
    ]
}
