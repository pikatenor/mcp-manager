use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Json, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use mcp_core::{Aggregator, AggregatorError, TokenService};
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::auth::extract_bearer;

#[derive(Clone)]
pub struct AppState {
    pub tokens: Arc<Mutex<TokenService>>,
    pub aggregator: Arc<AsyncMutex<Aggregator>>,
}

pub fn router(tokens: Arc<Mutex<TokenService>>) -> Router {
    router_with_aggregator(tokens, Arc::new(AsyncMutex::new(Aggregator::new())))
}

pub fn router_with_aggregator(
    tokens: Arc<Mutex<TokenService>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
) -> Router {
    let state = AppState {
        tokens,
        aggregator,
    };
    Router::new()
        .route("/mcp", post(mcp_handler))
        .route_layer(from_fn_with_state(state.clone(), require_token))
        .with_state(state)
}

pub async fn serve(tokens: Arc<Mutex<TokenService>>) -> std::io::Result<()> {
    serve_with_aggregator(tokens, Arc::new(AsyncMutex::new(Aggregator::new()))).await
}

pub async fn serve_with_aggregator(
    tokens: Arc<Mutex<TokenService>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(mcp_core::DEFAULT_HTTP_BIND).await?;
    serve_with_listener_and_aggregator(listener, tokens, aggregator).await
}

pub async fn serve_with_listener(
    listener: tokio::net::TcpListener,
    tokens: Arc<Mutex<TokenService>>,
) -> std::io::Result<()> {
    serve_with_listener_and_aggregator(
        listener,
        tokens,
        Arc::new(AsyncMutex::new(Aggregator::new())),
    )
    .await
}

pub async fn serve_with_listener_and_aggregator(
    listener: tokio::net::TcpListener,
    tokens: Arc<Mutex<TokenService>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
) -> std::io::Result<()> {
    axum::serve(listener, router_with_aggregator(tokens, aggregator)).await
}

async fn require_token(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
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
    let valid = state
        .tokens
        .lock()
        .ok()
        .and_then(|svc| svc.validate(&token).ok())
        .is_some();
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "invalid token" })),
        )
            .into_response();
    }
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

async fn mcp_handler(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = body.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => jsonrpc_ok(
            id,
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
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
            let aggregator = state.aggregator.lock().await;
            match aggregator.call_tool(name, arguments).await {
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
            }
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
    use mcp_core::{Aggregator, AggregatorError, McpBackend, RegisteredServer, TokenService, Tool};
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

    fn app_with_docs_server() -> (Router, String) {
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
        (
            router_with_aggregator(
                Arc::new(Mutex::new(tokens)),
                Arc::new(AsyncMutex::new(aggregator)),
            ),
            plaintext,
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
        let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
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
        assert!(json["result"]["capabilities"]["tools"].is_object());
        assert!(json["result"]["protocolVersion"].as_str().is_some());
    }

    #[tokio::test]
    async fn tools_list_returns_prefixed_names() {
        let (app, token) = app_with_docs_server();
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

    #[tokio::test]
    async fn tools_call_routes_to_backend() {
        let (app, token) = app_with_docs_server();
        let (status, json) = post_mcp(
            app,
            &token,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "docs__search",
                    "arguments": { "q": "hello" }
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["result"]["isError"], false);
        let text = json["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("search"), "{text}");
        assert!(text.contains("hello"), "{text}");
    }
}
