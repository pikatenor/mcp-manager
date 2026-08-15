//! Real TCP + SQLite end-to-end checks for POST /mcp.

use std::sync::{Arc, Mutex};

use mcp_core::TokenService;
use mcp_http::serve_with_listener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn spawn_server(tokens: Arc<Mutex<TokenService>>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        serve_with_listener(listener, tokens).await.unwrap();
    });
    addr
}

async fn post_mcp(addr: std::net::SocketAddr, authorization: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut request = format!("POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\n");
    if let Some(value) = authorization {
        request.push_str(&format!("Authorization: {value}\r\n"));
    }
    request.push_str("Connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string();
    (status, body)
}

#[tokio::test]
async fn e2e_http_auth_over_tcp() {
    let mut tokens = TokenService::new();
    let issued = tokens.issue("e2e");
    let addr = spawn_server(Arc::new(Mutex::new(tokens))).await;

    let (missing, _) = post_mcp(addr, None).await;
    assert_eq!(missing, 401);

    let (invalid, _) = post_mcp(addr, Some("Bearer mcpm_nope")).await;
    assert_eq!(invalid, 401);

    let (ok, body) = post_mcp(addr, Some(&format!("Bearer {}", issued.plaintext))).await;
    assert_eq!(ok, 200);
    assert!(body.contains("\"ok\":true"), "{body}");
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
    let (status, body) = post_mcp(addr, Some(&format!("Bearer {}", issued.plaintext))).await;
    assert_eq!(status, 200);
    assert!(body.contains("\"ok\":true"), "{body}");
}

#[tokio::test]
async fn e2e_revoked_token_is_rejected_over_tcp() {
    let mut tokens = TokenService::new();
    let issued = tokens.issue("cursor");
    tokens.revoke(&issued.id).unwrap();
    let addr = spawn_server(Arc::new(Mutex::new(tokens))).await;
    let (status, _) = post_mcp(addr, Some(&format!("Bearer {}", issued.plaintext))).await;
    assert_eq!(status, 401);
}
