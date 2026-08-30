use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Json, RawForm};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use mcp_core::{BackendConnector, ServerConfig, ServerType};
use mcp_platform::{
    server_bearer_key, server_oauth_client_id_key, server_oauth_client_secret_key,
    server_oauth_key, MemorySecretStore, SecretStore,
};
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
                "inputSchema": {
                    "type": "object",
                    "properties": { "q": { "type": "string", "description": "query" } },
                    "required": ["q"]
                }
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
    let backend = McpConnector::default().connect(&config, &secrets).await.unwrap();
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
    let backend = McpConnector::default()
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
    let backend = McpConnector::default()
        .connect(&config, &HashMap::new())
        .await
        .unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");
}

fn assert_echo_schema(tool: &mcp_core::Tool) {
    assert_eq!(tool.input_schema["type"], "object");
    assert_eq!(tool.input_schema["properties"]["q"]["type"], "string");
    assert_eq!(tool.input_schema["properties"]["q"]["description"], "query");
    assert_eq!(tool.input_schema["required"][0], "q");
}

#[tokio::test]
async fn streamable_http_preserves_input_schema() {
    let addr = spawn_streamable(None).await;
    let config = remote_config(ServerType::RemoteStreamable, format!("http://{addr}/mcp"));
    let backend = McpConnector::default()
        .connect(&config, &HashMap::new())
        .await
        .unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_echo_schema(&tools[0]);
}

#[tokio::test]
async fn sse_remote_preserves_input_schema() {
    let addr = spawn_sse().await;
    let config = remote_config(ServerType::Remote, format!("http://{addr}/sse"));
    let backend = McpConnector::default()
        .connect(&config, &HashMap::new())
        .await
        .unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_echo_schema(&tools[0]);
}

#[tokio::test]
async fn local_stdio_preserves_input_schema() {
    let config = ServerConfig {
        id: "local".into(),
        name: "fixture".into(),
        server_type: ServerType::Local,
        command: Some(env!("CARGO_BIN_EXE_stdio_fixture").into()),
        args: vec![],
        env_keys: vec![],
        remote_url: None,
        auto_start: false,
        disabled: false,
        tool_permissions: HashMap::new(),
    };
    let backend = McpConnector::default()
        .connect(&config, &HashMap::new())
        .await
        .unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_echo_schema(&tools[0]);
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
    let backend = McpConnector::default().connect(&config, &secrets).await.unwrap();
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
    let err = match McpConnector::default().connect(&config, &HashMap::new()).await {
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
    let err = match McpConnector::default().connect(&config, &HashMap::new()).await {
        Err(err) => err,
        Ok(_) => panic!("expected RemoteUrl"),
    };
    assert!(matches!(
        err,
        mcp_core::RegistryError::RemoteUrl(mcp_core::RemoteUrlError::Scheme)
    ));
}

struct AuthStreamable {
    addr: SocketAddr,
    current_token: Arc<Mutex<String>>,
    refresh_count: Arc<Mutex<u32>>,
    token_auth_headers: Arc<Mutex<Vec<Option<String>>>>,
}

/// Fake AS + guarded MCP endpoint on one host: `/mcp` only accepts the
/// server's current token and answers 401 with a challenge otherwise.
async fn spawn_auth_streamable() -> (SocketAddr, Arc<Mutex<String>>, Arc<Mutex<u32>>) {
    let fake = spawn_auth_streamable_opts(None).await;
    (fake.addr, fake.current_token, fake.refresh_count)
}

/// `required_client_auth` makes the token endpoint reject refresh requests
/// that lack `Authorization: Basic base64(id:secret)` for that client, the
/// way a confidential client would be validated.
async fn spawn_auth_streamable_opts(
    required_client_auth: Option<(&'static str, &'static str)>,
) -> AuthStreamable {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let current_token: Arc<Mutex<String>> = Arc::new(Mutex::new("at-initial".into()));
    let refresh_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let token_auth_headers: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let metadata = json!({
        // The path-inserted discovery form implies an issuer with the same path.
        "issuer": format!("{origin}/mcp"),
        "authorization_endpoint": format!("{origin}/authorize"),
        "token_endpoint": format!("{origin}/token"),
        "registration_endpoint": format!("{origin}/register"),
    });
    let token_state = (current_token.clone(), refresh_count.clone());
    let auth_headers = token_auth_headers.clone();
    let guarded_token = current_token.clone();
    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get(move || async move { axum::Json(metadata.clone()) }),
        )
        .route(
            "/token",
            post(move |headers: HeaderMap, RawForm(form): RawForm| {
                let (current_token, refresh_count) = token_state.clone();
                let auth_headers = auth_headers.clone();
                async move {
                    auth_headers
                        .lock()
                        .unwrap()
                        .push(headers.get(header::AUTHORIZATION).map(|value| value.to_str().unwrap().to_string()));
                    let pairs: HashMap<String, String> =
                        url::form_urlencoded::parse(&form).into_owned().collect();
                    if pairs.get("grant_type").map(String::as_str) == Some("refresh_token") {
                        if let Some((client_id, client_secret)) = required_client_auth {
                            let expected = format!(
                                "Basic {}",
                                base64::engine::general_purpose::STANDARD
                                    .encode(format!("{client_id}:{client_secret}"))
                            );
                            let received = auth_headers.lock().unwrap().last().cloned().flatten();
                            if received.as_deref() != Some(expected.as_str()) {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    axum::Json(json!({ "error": "invalid_client" })),
                                )
                                    .into_response();
                            }
                        }
                        let mut count = refresh_count.lock().unwrap();
                        *count += 1;
                        let token = format!("at-refresh-{count}");
                        *current_token.lock().unwrap() = token.clone();
                        return axum::Json(json!({
                            "access_token": token,
                            "refresh_token": format!("rt-{count}"),
                            "token_type": "Bearer",
                            "expires_in": 3600
                        }))
                        .into_response();
                    }
                    axum::Json(json!({ "access_token": "at-initial", "token_type": "Bearer" }))
                        .into_response()
                }
            }),
        )
        .route(
            "/mcp",
            post(move |request: Request<Body>| {
                let current = guarded_token.clone();
                async move {
                    let authorization = request
                        .headers()
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    if authorization != format!("Bearer {}", current.lock().unwrap()) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            [("www-authenticate", "Bearer realm=\"test\"")],
                            "invalid_token",
                        )
                            .into_response();
                    }
                    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    let body: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
                    handle_mcp(body)
                }
            }),
        );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    AuthStreamable {
        addr,
        current_token,
        refresh_count,
        token_auth_headers,
    }
}

fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Write an rmcp credentials blob for server id `srv` (see
/// `KeychainCredentialStore` for the layout).
fn seed_credentials(
    secrets: &MemorySecretStore,
    access_token: &str,
    expires_in: u64,
    received_at: u64,
) {
    seed_credentials_as(secrets, "cid-1", access_token, expires_in, received_at);
}

fn seed_credentials_as(
    secrets: &MemorySecretStore,
    client_id: &str,
    access_token: &str,
    expires_in: u64,
    received_at: u64,
) {
    let blob = json!({
        "client_id": client_id,
        "token_response": {
            "access_token": access_token,
            "token_type": "Bearer",
            "expires_in": expires_in,
            "refresh_token": "rt-1"
        },
        "granted_scopes": [],
        "token_received_at": received_at
    });
    secrets
        .set(&server_oauth_key("srv", "credentials"), &blob.to_string())
        .unwrap();
}

#[tokio::test]
async fn connect_refreshes_expired_token_before_initialize() {
    let (addr, _current, refresh_count) = spawn_auth_streamable().await;
    let secrets = Arc::new(MemorySecretStore::new());
    let two_hours_ago = epoch_now().saturating_sub(7200);
    seed_credentials(secrets.as_ref(), "at-initial", 3600, two_hours_ago);

    let connector = McpConnector::new(secrets.clone());
    let config = remote_config(ServerType::RemoteStreamable, format!("http://{addr}/mcp"));
    let backend = connector.connect(&config, &HashMap::new()).await.unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");

    assert_eq!(*refresh_count.lock().unwrap(), 1);
    // The refreshed token is persisted, including the bearer mirror.
    assert_eq!(
        secrets.get(&server_bearer_key("srv")).unwrap().as_deref(),
        Some("at-refresh-1")
    );
}

#[tokio::test]
async fn call_tool_refreshes_after_mid_session_rejection() {
    let (addr, current_token, refresh_count) = spawn_auth_streamable().await;
    let secrets = Arc::new(MemorySecretStore::new());
    seed_credentials(secrets.as_ref(), "at-initial", 3600, epoch_now());

    let connector = McpConnector::new(secrets.clone());
    let config = remote_config(ServerType::RemoteStreamable, format!("http://{addr}/mcp"));
    let backend = connector.connect(&config, &HashMap::new()).await.unwrap();
    assert_eq!(*refresh_count.lock().unwrap(), 0, "fresh token needs no refresh");

    // The server rotates its token underneath the live connection.
    *current_token.lock().unwrap() = "at-rotated".into();

    let result = backend.call_tool("echo", json!({ "q": "hi" })).await.unwrap();
    let text = result.to_string();
    assert!(text.contains("echo"), "{text}");
    assert_eq!(*refresh_count.lock().unwrap(), 1);
}

#[tokio::test]
async fn connect_refreshes_with_client_authentication_for_confidential_clients() {
    let fake = spawn_auth_streamable_opts(Some(("cid-static", "sec-static"))).await;
    let secrets = Arc::new(MemorySecretStore::new());
    let two_hours_ago = epoch_now().saturating_sub(7200);
    seed_credentials_as(secrets.as_ref(), "cid-static", "at-initial", 3600, two_hours_ago);
    // The confidential client identity survives restarts in the keychain;
    // the restore path alone only recovers the public half of it.
    secrets
        .set(&server_oauth_client_id_key("srv"), "cid-static")
        .unwrap();
    secrets
        .set(&server_oauth_client_secret_key("srv"), "sec-static")
        .unwrap();

    let connector = McpConnector::new(secrets.clone());
    let config = remote_config(
        ServerType::RemoteStreamable,
        format!("http://{}/mcp", fake.addr),
    );
    let backend = connector.connect(&config, &HashMap::new()).await.unwrap();
    let tools = backend.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "echo");

    assert_eq!(*fake.refresh_count.lock().unwrap(), 1);
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("cid-static:sec-static")
    );
    assert_eq!(
        fake.token_auth_headers.lock().unwrap()[0].as_deref(),
        Some(expected.as_str())
    );
    // The refreshed token is persisted, including the bearer mirror.
    assert_eq!(
        secrets.get(&server_bearer_key("srv")).unwrap().as_deref(),
        Some("at-refresh-1")
    );
}
