use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Query, RawForm};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use mcp_platform::{
    server_bearer_key, server_oauth_key, BrowserError, BrowserOpener, MemorySecretStore,
    SecretStore,
};
use mcp_runtime::{OAuthError, OAuthFlow};
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// Opens the authorization URL like a real browser: GET it and follow the
/// redirect onto the flow's loopback listener.
struct HeadlessBrowser;

#[async_trait]
impl BrowserOpener for HeadlessBrowser {
    fn open_url(&self, url: &str) -> Result<(), BrowserError> {
        let url = url.to_string();
        tokio::spawn(async move {
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let _ = client.get(&url).send().await;
        });
        Ok(())
    }
}

/// Fake authorization server speaking real HTTP so rmcp's manager drives the
/// full flow: discovery, dynamic registration, PKCE authorize, token exchange.
async fn spawn_fake_as() -> (SocketAddr, Arc<Mutex<Vec<HashMap<String, String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let token_requests: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::default();
    let requests = token_requests.clone();
    let metadata = json!({
        // The path-inserted discovery form implies an issuer with the same path.
        "issuer": format!("{origin}/mcp"),
        "authorization_endpoint": format!("{origin}/authorize"),
        "token_endpoint": format!("{origin}/token"),
        "registration_endpoint": format!("{origin}/register"),
    });
    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get(move || async move { axum::Json(metadata.clone()) }),
        )
        .route(
            "/register",
            post(|body: String| async move {
                let request: Value = serde_json::from_str(&body).unwrap_or(json!({}));
                axum::Json(json!({
                    "client_id": "cid-1",
                    "redirect_uris": request["redirect_uris"].clone(),
                }))
            }),
        )
        .route(
            "/authorize",
            get(|Query(params): Query<HashMap<String, String>>| async move {
                let redirect = params.get("redirect_uri").cloned().unwrap_or_default();
                let state = params.get("state").cloned().unwrap_or_default();
                Response::builder()
                    .status(StatusCode::FOUND)
                    .header(
                        header::LOCATION,
                        format!("{redirect}?code=auth-code&state={state}"),
                    )
                    .body(Body::empty())
                    .unwrap()
            }),
        )
        .route(
            "/token",
            post(
                move |RawForm(form): RawForm| async move {
                    let pairs: HashMap<String, String> =
                        url::form_urlencoded::parse(&form).into_owned().collect();
                    requests.lock().unwrap().push(pairs);
                    axum::Json(json!({
                        "access_token": "at-123",
                        "refresh_token": "rt-123",
                        "token_type": "Bearer",
                        "expires_in": 3600
                    }))
                },
            ),
        );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, token_requests)
}

#[tokio::test]
async fn authorize_stores_credentials_and_mirrors_bearer() {
    let (as_addr, token_requests) = spawn_fake_as().await;
    let secrets = Arc::new(MemorySecretStore::new());
    let flow = OAuthFlow::new(Arc::new(HeadlessBrowser), secrets.clone());
    flow.authorize("srv-1", &format!("http://{as_addr}/mcp"))
        .await
        .expect("authorize should succeed");

    // The code exchange carried the authorization code and PKCE verifier.
    let exchange = token_requests.lock().unwrap()[0].clone();
    assert_eq!(exchange["grant_type"], "authorization_code");
    assert_eq!(exchange["code"], "auth-code");
    assert!(exchange.contains_key("code_verifier"));

    // rmcp's credential blob landed in the store with both tokens.
    let blob = secrets
        .get(&server_oauth_key("srv-1", "credentials"))
        .expect("store read")
        .expect("credentials blob stored");
    let parsed: Value = serde_json::from_str(&blob).unwrap();
    assert_eq!(parsed["client_id"], "cid-1");
    assert_eq!(parsed["token_response"]["access_token"], "at-123");
    assert_eq!(parsed["token_response"]["refresh_token"], "rt-123");

    // The access token is mirrored for the UI and the legacy bearer path.
    assert_eq!(
        secrets.get(&server_bearer_key("srv-1")).unwrap().as_deref(),
        Some("at-123")
    );
    assert_eq!(
        secrets
            .get(&server_oauth_key("srv-1", "access_token"))
            .unwrap()
            .as_deref(),
        Some("at-123")
    );
}

#[tokio::test]
async fn authorize_rejects_plain_http_public_host() {
    let secrets = Arc::new(MemorySecretStore::new());
    let flow = OAuthFlow::new(Arc::new(HeadlessBrowser), secrets);
    let err = match flow.authorize("srv-1", "http://example.com/mcp").await {
        Err(err) => err,
        Ok(()) => panic!("expected url rejection"),
    };
    assert!(matches!(err, OAuthError::RemoteUrl(_)));
}
