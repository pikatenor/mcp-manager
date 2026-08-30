use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Extension, Json, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::stream;
use mcp_core::{strip_server_prefix, Aggregator, AggregatorError, CallLog, TokenService};
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::auth::extract_bearer;

/// Bearer-derived client identity for the call log; never the token hash.
#[derive(Clone)]
pub struct ClientName(pub String);

#[derive(Clone)]
pub struct AppState {
    pub tokens: Arc<Mutex<TokenService>>,
    pub aggregator: Arc<AsyncMutex<Aggregator>>,
    pub call_log: Arc<CallLog>,
}

pub fn router(tokens: Arc<Mutex<TokenService>>) -> Router {
    router_with_aggregator(
        tokens,
        Arc::new(AsyncMutex::new(Aggregator::new())),
        Arc::new(CallLog::memory().expect("in-memory call log")),
    )
}

pub fn router_with_aggregator(
    tokens: Arc<Mutex<TokenService>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
    call_log: Arc<CallLog>,
) -> Router {
    let state = AppState {
        tokens,
        aggregator,
        call_log,
    };
    Router::new()
        .route("/mcp", get(mcp_listen).post(mcp_handler))
        .route_layer(from_fn_with_state(state.clone(), require_token))
        .with_state(state)
}

pub async fn serve(tokens: Arc<Mutex<TokenService>>) -> std::io::Result<()> {
    serve_with_aggregator(
        tokens,
        Arc::new(AsyncMutex::new(Aggregator::new())),
        Arc::new(CallLog::memory().expect("in-memory call log")),
    )
    .await
}

pub async fn serve_with_aggregator(
    tokens: Arc<Mutex<TokenService>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
    call_log: Arc<CallLog>,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(mcp_core::DEFAULT_HTTP_BIND).await?;
    serve_with_listener_and_aggregator(listener, tokens, aggregator, call_log).await
}

pub async fn serve_with_listener(
    listener: tokio::net::TcpListener,
    tokens: Arc<Mutex<TokenService>>,
) -> std::io::Result<()> {
    serve_with_listener_and_aggregator(
        listener,
        tokens,
        Arc::new(AsyncMutex::new(Aggregator::new())),
        Arc::new(CallLog::memory().expect("in-memory call log")),
    )
    .await
}

pub async fn serve_with_listener_and_aggregator(
    listener: tokio::net::TcpListener,
    tokens: Arc<Mutex<TokenService>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
    call_log: Arc<CallLog>,
) -> std::io::Result<()> {
    axum::serve(listener, router_with_aggregator(tokens, aggregator, call_log)).await
}

async fn require_token(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let Some(token) = extract_bearer(header) else {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "authentication required" })),
        )
            .into_response();
    };
    let record = state
        .tokens
        .lock()
        .ok()
        .and_then(|svc| svc.validate(&token).ok());
    let Some(record) = record else {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "invalid token" })),
        )
            .into_response();
    };
    req.extensions_mut().insert(ClientName(record.client_name));
    next.run(req).await
}

fn jsonrpc_ok(id: Value, result: Value) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })),
    )
        .into_response()
}

fn jsonrpc_err(id: Value, code: i64, message: String) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        })),
    )
        .into_response()
}

/// Coarse failure category for the call log; raw error strings are not stored.
fn error_kind_of(err: &AggregatorError) -> &'static str {
    match err {
        AggregatorError::UnknownTool(_) => "unknown_tool",
        AggregatorError::PrivateTool(_) => "private_tool",
        AggregatorError::Backend(_) => "backend_error",
    }
}

async fn mcp_listen(State(state): State<AppState>) -> Response {
    let changes = state.aggregator.lock().await.subscribe_tool_list_changes();
    let events = stream::unfold(changes, |mut changes| async move {
        loop {
            match changes.recv().await {
                Ok(()) => {
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/tools/list_changed"
                    });
                    let event = Event::default().data(notification.to_string());
                    return Some((Ok::<_, Infallible>(event), changes));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(events).into_response()
}

async fn mcp_handler(
    State(state): State<AppState>,
    // Always present: require_token runs first in the route layer.
    Extension(client): Extension<ClientName>,
    Json(body): Json<Value>,
) -> Response {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = body.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => jsonrpc_ok(
            id,
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": {
                    "name": "mcp-manager",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "ping" => jsonrpc_ok(id, json!({})),
        "tools/list" => {
            let aggregator = state.aggregator.lock().await;
            match aggregator.list_tools().await {
                Ok(tools) => {
                    let tools: Vec<Value> = tools
                        .into_iter()
                        .map(|tool| {
                            json!({
                                "name": tool.name,
                                "description": tool.description.unwrap_or_default(),
                                "inputSchema": { "type": "object", "properties": {} }
                            })
                        })
                        .collect();
                    jsonrpc_ok(id, json!({ "tools": tools }))
                }
                Err(err) => jsonrpc_err(id, -32603, err.to_string()),
            }
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let started = Instant::now();
            // Scoped block: drop the aggregator guard before the blocking log write.
            let outcome = {
                let aggregator = state.aggregator.lock().await;
                aggregator.call_tool(name, arguments).await
            };
            let duration_ms = started.elapsed().as_millis() as i64;
            let (server, tool) = strip_server_prefix(name).unwrap_or(("", name));
            let ok = outcome.is_ok();
            let error_kind = outcome.as_ref().err().map(error_kind_of);
            let response = match outcome {
                Ok(value) => jsonrpc_ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": value.to_string() }],
                        "isError": false
                    }),
                ),
                Err(AggregatorError::UnknownTool(name) | AggregatorError::PrivateTool(name)) => {
                    jsonrpc_err(id, -32601, name)
                }
                Err(err) => jsonrpc_ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": err.to_string() }],
                        "isError": true
                    }),
                ),
            };
            state
                .call_log
                .record(server, tool, &client.0, ok, error_kind, duration_ms);
            response
        }
        "" => jsonrpc_err(id, -32600, "invalid request".into()),
        other => jsonrpc_err(id, -32601, format!("method not found: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use http_body_util::BodyExt;
    use mcp_core::{
        Aggregator, AggregatorError, CallLog, McpBackend, RegisteredServer, TokenService, Tool,
    };
    use serde_json::{json, Value};
    use tower::ServiceExt;

    struct FakeBackend {
        tools: Vec<Tool>,
    }

    #[async_trait]
    impl McpBackend for FakeBackend {
        async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError> {
            Ok(self.tools.clone())
        }

        async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, AggregatorError> {
            Ok(json!({ "ok": true, "tool": name, "arguments": arguments }))
        }
    }

    struct FailingBackend;

    #[async_trait]
    impl McpBackend for FailingBackend {
        async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError> {
            Ok(vec![Tool {
                name: "boom".into(),
                description: None,
            }])
        }

        async fn call_tool(&self, _name: &str, _arguments: Value) -> Result<Value, AggregatorError> {
            Err(AggregatorError::Backend("upstream refused".into()))
        }
    }

    fn initialize_body() -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }
        })
    }

    fn app_with_token() -> (Router, String) {
        let mut tokens = TokenService::new();
        let issued = tokens.issue("cursor");
        let plaintext = issued.plaintext.clone();
        (router(Arc::new(Mutex::new(tokens))), plaintext)
    }

    fn app_with_docs_server() -> (Router, String, Arc<CallLog>) {
        let mut tokens = TokenService::new();
        let issued = tokens.issue("cursor");
        let plaintext = issued.plaintext.clone();
        let mut aggregator = Aggregator::new();
        aggregator.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: true,
            tool_permissions: Default::default(),
            backend: Arc::new(FakeBackend {
                tools: vec![Tool {
                    name: "search".into(),
                    description: Some("search docs".into()),
                }],
            }),
        });
        let call_log = Arc::new(CallLog::memory().unwrap());
        (
            router_with_aggregator(
                Arc::new(Mutex::new(tokens)),
                Arc::new(AsyncMutex::new(aggregator)),
                call_log.clone(),
            ),
            plaintext,
            call_log,
        )
    }

    fn app_with_docs_server_and_change_handle() -> (Router, String, Arc<AsyncMutex<Aggregator>>) {
        let mut tokens = TokenService::new();
        let issued = tokens.issue("cursor");
        let plaintext = issued.plaintext.clone();
        let mut aggregator = Aggregator::new();
        aggregator.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: true,
            tool_permissions: Default::default(),
            backend: Arc::new(FakeBackend {
                tools: vec![Tool {
                    name: "search".into(),
                    description: Some("search docs".into()),
                }],
            }),
        });
        let aggregator = Arc::new(AsyncMutex::new(aggregator));
        let app = router_with_aggregator(
            Arc::new(Mutex::new(tokens)),
            aggregator.clone(),
            Arc::new(CallLog::memory().unwrap()),
        );
        (app, plaintext, aggregator)
    }

    fn app_with_failing_server() -> (Router, String, Arc<CallLog>) {
        let mut tokens = TokenService::new();
        let issued = tokens.issue("cursor");
        let plaintext = issued.plaintext.clone();
        let mut aggregator = Aggregator::new();
        aggregator.add_server(RegisteredServer {
            id: "1".into(),
            name: "flaky".into(),
            running: true,
            tool_permissions: Default::default(),
            backend: Arc::new(FailingBackend),
        });
        let call_log = Arc::new(CallLog::memory().unwrap());
        (
            router_with_aggregator(
                Arc::new(Mutex::new(tokens)),
                Arc::new(AsyncMutex::new(aggregator)),
                call_log.clone(),
            ),
            plaintext,
            call_log,
        )
    }

    async fn post_mcp(app: Router, token: &str, body: Value) -> (StatusCode, Value) {
        let res = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
        (status, json)
    }

    #[tokio::test]
    async fn missing_token_is_unauthorized() {
        let (app, _) = app_with_token();
        let res = app
            .oneshot(Request::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_token_is_unauthorized() {
        let (app, _) = app_with_token();
        let res = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer mcpm_nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoked_token_is_unauthorized() {
        let mut tokens = TokenService::new();
        let issued = tokens.issue("cursor");
        tokens.revoke(&issued.id).unwrap();
        let app = router(Arc::new(Mutex::new(tokens)));
        let res = app
            .oneshot(
                Request::post("/mcp")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", issued.plaintext),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn initialize_returns_server_info() {
        let (app, token) = app_with_token();
        let (status, json) = post_mcp(app, &token, initialize_body()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["result"]["serverInfo"]["name"], "mcp-manager");
        assert_eq!(json["result"]["capabilities"]["tools"]["listChanged"], true);
        assert!(json["result"]["protocolVersion"].as_str().is_some());
    }

    #[tokio::test]
    async fn get_stream_emits_tool_list_changed_notification() {
        let (app, token, aggregator) = app_with_docs_server_and_change_handle();
        let response = app
            .oneshot(
                Request::get("/mcp")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header(header::ACCEPT, "text/event-stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );

        aggregator.lock().await.set_running("1", false);

        let frame = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            response.into_body().frame(),
        )
        .await
        .expect("notification timed out")
        .expect("notification stream ended")
        .expect("notification body failed");
        let data = frame.into_data().expect("expected notification data");
        let event = String::from_utf8(data.to_vec()).unwrap();
        assert!(
            event.contains(r#""method":"notifications/tools/list_changed""#),
            "{event}"
        );
    }

    #[tokio::test]
    async fn tools_list_returns_prefixed_names() {
        let (app, token, _) = app_with_docs_server();
        let (status, json) = post_mcp(
            app,
            &token,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<&str> = json["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["docs__search"]);
    }

    fn call_body(id: i64, name: &str, arguments: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
    }

    #[tokio::test]
    async fn tools_call_routes_to_backend() {
        let (app, token, _) = app_with_docs_server();
        let (status, json) = post_mcp(app, &token, call_body(3, "docs__search", json!({ "q": "hello" })))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["result"]["isError"], false);
        let text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("search"), "{text}");
        assert!(text.contains("hello"), "{text}");
    }

    #[tokio::test]
    async fn tools_call_records_metadata_for_success() {
        let (app, token, call_log) = app_with_docs_server();
        let (status, json) =
            post_mcp(app, &token, call_body(3, "docs__search", json!({ "q": "hello" }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["result"]["isError"], false);
        let rows = call_log.list_recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].server, "docs");
        assert_eq!(rows[0].tool, "search");
        assert_eq!(rows[0].client, "cursor");
        assert!(rows[0].ok);
        assert_eq!(rows[0].error_kind, None);
        assert!(rows[0].duration_ms >= 0);
        assert!(rows[0].called_at > 0);
    }

    #[tokio::test]
    async fn tools_call_records_unknown_tool_failure() {
        let (app, token, call_log) = app_with_docs_server();
        let (status, json) = post_mcp(app, &token, call_body(4, "docs__missing", json!({}))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["error"].is_object());
        let rows = call_log.list_recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].server, "docs");
        assert_eq!(rows[0].tool, "missing");
        assert!(!rows[0].ok);
        assert_eq!(rows[0].error_kind.as_deref(), Some("unknown_tool"));
    }

    #[tokio::test]
    async fn tools_call_records_backend_error_kind() {
        let (app, token, call_log) = app_with_failing_server();
        let (status, json) = post_mcp(app, &token, call_body(5, "flaky__boom", json!({}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["result"]["isError"], true);
        let rows = call_log.list_recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].ok);
        assert_eq!(rows[0].error_kind.as_deref(), Some("backend_error"));
        assert_eq!(rows[0].server, "flaky");
        assert_eq!(rows[0].tool, "boom");
    }

    #[tokio::test]
    async fn tools_call_without_delimiter_logs_empty_server() {
        let (app, token, call_log) = app_with_docs_server();
        let (_, json) = post_mcp(app, &token, call_body(6, "nodelimiter", json!({}))).await;
        assert!(json["error"].is_object());
        let rows = call_log.list_recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].server, "");
        assert_eq!(rows[0].tool, "nodelimiter");
        assert!(!rows[0].ok);
        assert_eq!(rows[0].error_kind.as_deref(), Some("unknown_tool"));
    }
}
