use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Query, RawForm};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use mcp_platform::{
    server_bearer_key, server_oauth_client_id_key, server_oauth_client_secret_key,
    server_oauth_key, BrowserError, BrowserOpener, MemorySecretStore, SecretStore,
};
use mcp_runtime::{KeychainCredentialStore, OAuthError, OAuthFlow};
use rmcp::transport::auth::CredentialStore;
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

struct FakeAs {
    addr: SocketAddr,
    token_requests: Arc<Mutex<Vec<HashMap<String, String>>>>,
    token_auth_headers: Arc<Mutex<Vec<Option<String>>>>,
    authorize_params: Arc<Mutex<Vec<HashMap<String, String>>>>,
    register_hits: Arc<Mutex<usize>>,
}

/// Fake authorization server speaking real HTTP so rmcp's manager drives the
/// full flow: discovery, dynamic registration, PKCE authorize, token exchange.
async fn spawn_fake_as() -> (SocketAddr, Arc<Mutex<Vec<HashMap<String, String>>>>) {
    let fake = spawn_fake_as_opts(true).await;
    (fake.addr, fake.token_requests)
}

/// `with_registration_endpoint = false` omits `registration_endpoint` from
/// the discovery metadata; the `/register` route stays mounted so a DCR
/// attempt remains observable through `register_hits`.
async fn spawn_fake_as_opts(with_registration_endpoint: bool) -> FakeAs {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let token_requests: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::default();
    let token_auth_headers: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let authorize_params: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::default();
    let register_hits: Arc<Mutex<usize>> = Arc::default();
    let requests = token_requests.clone();
    let auth_headers = token_auth_headers.clone();
    let authorize_queries = authorize_params.clone();
    let registrations = register_hits.clone();
    let mut metadata = json!({
        // The path-inserted discovery form implies an issuer with the same path.
        "issuer": format!("{origin}/mcp"),
        "authorization_endpoint": format!("{origin}/authorize"),
        "token_endpoint": format!("{origin}/token"),
    });
    if with_registration_endpoint {
        metadata["registration_endpoint"] = json!(format!("{origin}/register"));
    }
    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get(move || {
                let metadata = metadata.clone();
                async move { axum::Json(metadata) }
            }),
        )
        .route(
            "/register",
            post(move |body: String| async move {
                *registrations.lock().unwrap() += 1;
                let request: Value = serde_json::from_str(&body).unwrap_or(json!({}));
                axum::Json(json!({
                    "client_id": "cid-1",
                    "redirect_uris": request["redirect_uris"].clone(),
                }))
            }),
        )
        .route(
            "/authorize",
            get(move |Query(params): Query<HashMap<String, String>>| async move {
                let redirect = params.get("redirect_uri").cloned().unwrap_or_default();
                let state = params.get("state").cloned().unwrap_or_default();
                authorize_queries.lock().unwrap().push(params);
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
                move |headers: HeaderMap, RawForm(form): RawForm| async move {
                    let pairs: HashMap<String, String> =
                        url::form_urlencoded::parse(&form).into_owned().collect();
                    requests.lock().unwrap().push(pairs);
                    auth_headers.lock().unwrap().push(
                        headers
                            .get(header::AUTHORIZATION)
                            .map(|value| value.to_str().unwrap().to_string()),
                    );
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
    FakeAs {
        addr,
        token_requests,
        token_auth_headers,
        authorize_params,
        register_hits,
    }
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

fn seed_static_client(store: &MemorySecretStore, client_id: &str, client_secret: Option<&str>) {
    store
        .set(&server_oauth_client_id_key("srv-1"), client_id)
        .unwrap();
    if let Some(secret) = client_secret {
        store
            .set(&server_oauth_client_secret_key("srv-1"), secret)
            .unwrap();
    }
}

#[tokio::test]
async fn authorize_uses_preregistered_client_without_dynamic_registration() {
    let fake = spawn_fake_as_opts(false).await;
    let secrets = Arc::new(MemorySecretStore::new());
    seed_static_client(&secrets, "cid-static", Some("sec-static"));
    let flow = OAuthFlow::new(Arc::new(HeadlessBrowser), secrets.clone());
    flow.authorize("srv-1", &format!("http://{}/mcp", fake.addr))
        .await
        .expect("authorize should succeed with a pre-registered client");

    // No dynamic registration was attempted; the static identity was used.
    assert_eq!(*fake.register_hits.lock().unwrap(), 0);
    let authorize = fake.authorize_params.lock().unwrap()[0].clone();
    assert_eq!(authorize["client_id"], "cid-static");

    // The code exchange authenticated the confidential client and still
    // carried the PKCE verifier.
    let exchange = fake.token_requests.lock().unwrap()[0].clone();
    assert_eq!(exchange["grant_type"], "authorization_code");
    assert_eq!(exchange["code"], "auth-code");
    assert!(exchange.contains_key("code_verifier"));
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("cid-static:sec-static")
    );
    assert_eq!(
        fake.token_auth_headers.lock().unwrap()[0].as_deref(),
        Some(expected.as_str())
    );

    // Tokens still landed in the store, under the static client identity.
    let blob = secrets
        .get(&server_oauth_key("srv-1", "credentials"))
        .expect("store read")
        .expect("credentials blob stored");
    let parsed: Value = serde_json::from_str(&blob).unwrap();
    assert_eq!(parsed["client_id"], "cid-static");
    assert_eq!(parsed["token_response"]["access_token"], "at-123");
    assert_eq!(
        secrets.get(&server_bearer_key("srv-1")).unwrap().as_deref(),
        Some("at-123")
    );
}

#[tokio::test]
async fn authorize_with_client_id_only_keeps_client_public() {
    let fake = spawn_fake_as_opts(false).await;
    let secrets = Arc::new(MemorySecretStore::new());
    seed_static_client(&secrets, "cid-public", None);
    let flow = OAuthFlow::new(Arc::new(HeadlessBrowser), secrets.clone());
    flow.authorize("srv-1", &format!("http://{}/mcp", fake.addr))
        .await
        .expect("authorize should succeed with a public pre-registered client");

    assert_eq!(*fake.register_hits.lock().unwrap(), 0);
    let authorize = fake.authorize_params.lock().unwrap()[0].clone();
    assert_eq!(authorize["client_id"], "cid-public");

    // Without a secret the token exchange carries no client authentication.
    assert_eq!(fake.token_auth_headers.lock().unwrap()[0].as_deref(), None);
    assert_eq!(
        secrets.get(&server_bearer_key("srv-1")).unwrap().as_deref(),
        Some("at-123")
    );
}

#[tokio::test]
async fn credential_store_clear_preserves_user_client_config() {
    let secrets = Arc::new(MemorySecretStore::new());
    for key in [
        server_oauth_key("srv-1", "credentials"),
        server_bearer_key("srv-1"),
        server_oauth_key("srv-1", "access_token"),
        server_oauth_key("srv-1", "refresh_token"),
        server_oauth_client_id_key("srv-1"),
        server_oauth_client_secret_key("srv-1"),
    ] {
        secrets.set(&key, "value").unwrap();
    }
    let store = KeychainCredentialStore::new(secrets.clone(), "srv-1");
    store.clear().await.unwrap();

    for key in [
        server_oauth_key("srv-1", "credentials"),
        server_bearer_key("srv-1"),
        server_oauth_key("srv-1", "access_token"),
        server_oauth_key("srv-1", "refresh_token"),
    ] {
        assert_eq!(secrets.get(&key).unwrap(), None, "{key} should be cleared");
    }
    // The static client identity is user config, not token state; rmcp calls
    // `clear` on issuer change and must not wipe it.
    assert_eq!(
        secrets
            .get(&server_oauth_client_id_key("srv-1"))
            .unwrap()
            .as_deref(),
        Some("value")
    );
    assert_eq!(
        secrets
            .get(&server_oauth_client_secret_key("srv-1"))
            .unwrap()
            .as_deref(),
        Some("value")
    );
}
