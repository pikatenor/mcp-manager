//! Real TCP + SQLite end-to-end checks for POST /mcp.

use std::sync::{Arc, Mutex};

use mcp_core::{Aggregator, CallLog, TokenService};
use mcp_http::{serve_with_listener, serve_with_listener_and_aggregator};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;

async fn spawn_server(tokens: Arc<Mutex<TokenService>>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        serve_with_listener(listener, tokens).await.unwrap();
    });
    addr
}

async fn post_mcp(
    addr: std::net::SocketAddr,
    authorization: Option<&str>,
    body: &str,
) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(value) = authorization {
        request.push_str(&format!("Authorization: {value}\r\n"));
    }
    request.push_str("Connection: close\r\n\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let resp_body = text
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string();
    (status, resp_body)
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#;

#[tokio::test]
async fn e2e_http_auth_over_tcp() {
    let mut tokens = TokenService::new();
    let issued = tokens.issue("e2e");
    let addr = spawn_server(Arc::new(Mutex::new(tokens))).await;

    let (missing, _) = post_mcp(addr, None, INITIALIZE).await;
    assert_eq!(missing, 401);

    let (invalid, _) = post_mcp(addr, Some("Bearer mcpm_nope"), INITIALIZE).await;
    assert_eq!(invalid, 401);

    let (ok, body) = post_mcp(
        addr,
        Some(&format!("Bearer {}", issued.plaintext)),
        INITIALIZE,
    )
    .await;
    assert_eq!(ok, 200);
    assert!(body.contains("mcp-manager"), "{body}");
    assert!(body.contains("jsonrpc"), "{body}");
}

#[tokio::test]
async fn e2e_sqlite_token_survives_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokens.db");
    let issued = {
        let mut tokens = TokenService::open_sqlite(&db).unwrap();
        tokens.issue("cursor")
    };

    let tokens = TokenService::open_sqlite(&db).unwrap();
    let addr = spawn_server(Arc::new(Mutex::new(tokens))).await;
    let (status, body) = post_mcp(
        addr,
        Some(&format!("Bearer {}", issued.plaintext)),
        INITIALIZE,
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("mcp-manager"), "{body}");
}

#[tokio::test]
async fn e2e_revoked_token_is_rejected_over_tcp() {
    let mut tokens = TokenService::new();
    let issued = tokens.issue("cursor");
    tokens.revoke(&issued.id).unwrap();
    let addr = spawn_server(Arc::new(Mutex::new(tokens))).await;
    let (status, _) = post_mcp(
        addr,
        Some(&format!("Bearer {}", issued.plaintext)),
        INITIALIZE,
    )
    .await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn e2e_call_log_records_tools_call_over_tcp() {
    let dir = tempfile::tempdir().unwrap();
    let mut tokens = TokenService::new();
    let issued = tokens.issue("cursor");
    let call_log = Arc::new(CallLog::open_sqlite(&dir.path().join("calls.db")).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tokens = Arc::new(Mutex::new(tokens));
    let aggregator = Arc::new(AsyncMutex::new(Aggregator::new()));
    tokio::spawn(async move {
        serve_with_listener_and_aggregator(listener, tokens, aggregator, call_log)
            .await
            .unwrap();
    });

    // No servers registered: the call fails as unknown, and that failure is
    // what gets recorded with the bearer client's name.
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"docs__search","arguments":{}}}"#;
    let (status, body) = post_mcp(
        addr,
        Some(&format!("Bearer {}", issued.plaintext)),
        call,
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("error"), "{body}");

    // Metadata persists in calls.db, readable after reopening the file.
    let log = CallLog::open_sqlite(&dir.path().join("calls.db")).unwrap();
    let rows = log.list_recent(10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].server, "docs");
    assert_eq!(rows[0].tool, "search");
    assert_eq!(rows[0].client, "cursor");
    assert!(!rows[0].ok);
    assert_eq!(rows[0].error_kind.as_deref(), Some("unknown_tool"));
    assert!(rows[0].called_at > 0);
    assert!(rows[0].duration_ms >= 0);
}
