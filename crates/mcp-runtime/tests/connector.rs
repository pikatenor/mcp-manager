use std::collections::HashMap;
use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::Json;
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use mcp_core::{BackendConnector, ServerConfig, ServerType};
use mcp_runtime::McpConnector;
use serde_json::{json, Value};
use tokio::net::TcpListener;

fn handle_mcp(body: Value) -> Response {
    if body.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "stub", "version": "0" }
        }),
        "ping" => json!({}),
        "tools/list" => json!({
            "tools": [{
                "name": "echo",
                "description": "echo",
                "inputSchema": { "type": "object", "properties": {} }
            }]
        }),
        "tools/call" => json!({
            "content": [{
                "type": "text",
                "text": format!("called {}", body["params"]["name"])
            }],
            "isError": false
        }),
        _ => {
            return (
                StatusCode::OK,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "method not found" }
                })),
            )
                .into_response();
        }
    };
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

async fn spawn_streamable(expected_bearer: Option<&'static str>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/mcp",
        post(move |request: Request<Body>| async move {
            if let Some(token) = expected_bearer {
                let header = request
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                if header != format!("Bearer {token}") {
                    return (StatusCode::UNAUTHORIZED, "nope").into_response();
                }
            }
            let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
            handle_mcp(body)
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn spawn_sse() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/sse",
            get(|| async {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body("event: endpoint\ndata: /messages\n\n".to_string())
                    .unwrap()
            }),
        )
        .route(
            "/messages",
            post(|Json(body): Json<Value>| async move { handle_mcp(body) }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn remote_config(server_type: ServerType, url: String) -> ServerConfig {
    ServerConfig {
        id: "srv".into(),
        name: "upstream".into(),
        server_type,
        command: None,
        args: vec![],
        env_keys: vec![],
        remote_url: Some(url),
        auto_start: false,
        disabled: false,
        tool_permissions: HashMap::new(),
    }
}

#[tokio::test]
async fn streamable_http_lists_and_calls_with_bearer() {
    let addr = spawn_streamable(Some("secret-token")).await;
    let config = remote_config(ServerType::RemoteStreamable, format!("http://{addr}/mcp"));
    let mut secrets = HashMap::new();
    secrets.insert("BEARER_TOKEN".into(), "secret-token".into());
    let backend = McpConnector.connect(&config, &secrets).await.unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");
    let result = backend
        .call_tool("echo", json!({ "q": "hi" }))
        .await
        .unwrap();
    let text = result.to_string();
    assert!(text.contains("echo"), "{text}");
}

fn sse_json(value: Value) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(format!("event: message\ndata: {value}\n\n")))
        .unwrap()
}

/// Spec-strict streamable server, modeled on Atlassian's: POSTs must accept
/// both `application/json` and `text/event-stream`, replies may be
/// SSE-framed, and the `Mcp-Session-Id` from initialize must be echoed.
async fn spawn_streamable_spec() -> SocketAddr {
    const NOT_ACCEPTABLE: &str = "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32000,\"message\":\"Not Acceptable: Client must accept both application/json and text/event-stream\"},\"id\":null}";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/mcp",
        post(move |request: Request<Body>| async move {
            let accept = request
                .headers()
                .get(header::ACCEPT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            if !(accept.contains("application/json") && accept.contains("text/event-stream")) {
                return (StatusCode::NOT_ACCEPTABLE, NOT_ACCEPTABLE.to_string()).into_response();
            }
            let session = request
                .headers()
                .get("mcp-session-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
            let id = body.get("id").cloned();
            let method = body
                .get("method")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_none() {
                if session.as_deref() != Some("sess-1") {
                    return (StatusCode::NOT_FOUND, "unknown session".to_string()).into_response();
                }
                return StatusCode::ACCEPTED.into_response();
            }
            let id = id.unwrap();
            match method.as_str() {
                "initialize" => {
                    let mut response = sse_json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2025-03-26",
                            "capabilities": { "tools": {} },
                            "serverInfo": { "name": "spec-stub", "version": "0" }
                        }
                    }));
                    response
                        .headers_mut()
                        .insert("mcp-session-id", "sess-1".parse().unwrap());
                    response
                }
                "tools/list" | "tools/call" => {
                    if session.as_deref() != Some("sess-1") {
                        return (StatusCode::NOT_FOUND, "unknown session".to_string())
                            .into_response();
                    }
                    handle_mcp(body)
                }
                _ => sse_json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "method not found" }
                })),
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

#[tokio::test]
async fn streamable_http_sends_accept_and_parses_sse_responses() {
    let addr = spawn_streamable_spec().await;
    let config = remote_config(ServerType::RemoteStreamable, format!("http://{addr}/mcp"));
    let backend = McpConnector
        .connect(&config, &HashMap::new())
        .await
        .unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");
    let result = backend
        .call_tool("echo", json!({ "q": "hi" }))
        .await
        .unwrap();
    let text = result.to_string();
    assert!(text.contains("echo"), "{text}");
}

#[tokio::test]
async fn sse_remote_lists_tools() {
    let addr = spawn_sse().await;
    let config = remote_config(ServerType::Remote, format!("http://{addr}/sse"));
    let backend = McpConnector
        .connect(&config, &HashMap::new())
        .await
        .unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");
}

#[tokio::test]
async fn local_stdio_injects_env_and_lists_tools() {
    let config = ServerConfig {
        id: "local".into(),
        name: "fixture".into(),
        server_type: ServerType::Local,
        command: Some(env!("CARGO_BIN_EXE_stdio_fixture").into()),
        args: vec![],
        env_keys: vec!["TEST_SECRET".into()],
        remote_url: None,
        auto_start: false,
        disabled: false,
        tool_permissions: HashMap::new(),
    };
    let mut secrets = HashMap::new();
    secrets.insert("TEST_SECRET".into(), "sk-secret".into());
    let backend = McpConnector.connect(&config, &secrets).await.unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");
    let result = backend.call_tool("echo", json!({ "n": 1 })).await.unwrap();
    let text = result.to_string();
    assert!(text.contains("sk-secret"), "{text}");
}

#[tokio::test]
async fn local_requires_command() {
    let config = ServerConfig {
        id: "local".into(),
        name: "fixture".into(),
        server_type: ServerType::Local,
        command: None,
        args: vec![],
        env_keys: vec![],
        remote_url: None,
        auto_start: false,
        disabled: false,
        tool_permissions: HashMap::new(),
    };
    let err = match McpConnector.connect(&config, &HashMap::new()).await {
        Err(err) => err,
        Ok(_) => panic!("expected InvalidConfig"),
    };
    assert!(matches!(err, mcp_core::RegistryError::InvalidConfig(_)));
}

#[tokio::test]
async fn remote_rejects_plain_http_to_public_host() {
    let config = remote_config(
        ServerType::RemoteStreamable,
        "http://example.com/mcp".into(),
    );
    let err = match McpConnector.connect(&config, &HashMap::new()).await {
        Err(err) => err,
        Ok(_) => panic!("expected RemoteUrl"),
    };
    assert!(matches!(
        err,
        mcp_core::RegistryError::RemoteUrl(mcp_core::RemoteUrlError::Scheme)
    ));
}
