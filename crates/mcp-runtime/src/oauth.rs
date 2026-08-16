use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::Query;
use axum::routing::get;
use axum::Router;
use mcp_core::validate_remote_url;
use mcp_platform::{server_bearer_key, server_oauth_key, BrowserOpener, SecretStore};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error(transparent)]
    RemoteUrl(#[from] mcp_core::RemoteUrlError),
    #[error("oauth failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait OAuthHttp: Send + Sync {
    async fn get_json(&self, url: &str) -> Result<Value, OAuthError>;
    async fn post_json(&self, url: &str, body: Value) -> Result<Value, OAuthError>;
    async fn post_form(
        &self,
        url: &str,
        form: HashMap<String, String>,
    ) -> Result<Value, OAuthError>;
}

pub struct ReqwestOAuthHttp {
    client: reqwest::Client,
}

impl ReqwestOAuthHttp {
    pub fn new() -> Result<Self, OAuthError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .map_err(|err| OAuthError::Failed(err.to_string()))?,
        })
    }
}

#[async_trait]
impl OAuthHttp for ReqwestOAuthHttp {
    async fn get_json(&self, url: &str) -> Result<Value, OAuthError> {
        self.client
            .get(url)
            .send()
            .await
            .map_err(|err| OAuthError::Failed(err.to_string()))?
            .error_for_status()
            .map_err(|err| OAuthError::Failed(err.to_string()))?
            .json()
            .await
            .map_err(|err| OAuthError::Failed(err.to_string()))
    }

    async fn post_json(&self, url: &str, body: Value) -> Result<Value, OAuthError> {
        self.client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|err| OAuthError::Failed(err.to_string()))?
            .error_for_status()
            .map_err(|err| OAuthError::Failed(err.to_string()))?
            .json()
            .await
            .map_err(|err| OAuthError::Failed(err.to_string()))
    }

    async fn post_form(
        &self,
        url: &str,
        form: HashMap<String, String>,
    ) -> Result<Value, OAuthError> {
        self.client
            .post(url)
            .form(&form)
            .send()
            .await
            .map_err(|err| OAuthError::Failed(err.to_string()))?
            .error_for_status()
            .map_err(|err| OAuthError::Failed(err.to_string()))?
            .json()
            .await
            .map_err(|err| OAuthError::Failed(err.to_string()))
    }
}

pub struct OAuthFlow {
    http: Arc<dyn OAuthHttp>,
    browser: Arc<dyn BrowserOpener>,
    secrets: Arc<dyn SecretStore>,
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: String,
    state: String,
}

fn fail(message: impl Into<String>) -> OAuthError {
    OAuthError::Failed(message.into())
}

fn base64url_nopad(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let a = u32::from(chunk[0]);
        let b = u32::from(chunk.get(1).copied().unwrap_or(0));
        let c = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (a << 16) | (b << 8) | c;
        let keep = match chunk.len() {
            1 => 2,
            2 => 3,
            _ => 4,
        };
        for shift in [18, 12, 6, 0].into_iter().take(keep) {
            out.push(TABLE[((n >> shift) & 63) as usize] as char);
        }
    }
    out
}

fn random_b64(nbytes: usize) -> String {
    let mut bytes = vec![0u8; nbytes];
    rand::rng().fill_bytes(&mut bytes);
    base64url_nopad(&bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    base64url_nopad(&Sha256::digest(verifier.as_bytes()))
}

fn well_known_url(mcp_url: &Url) -> Result<String, OAuthError> {
    let mut url = mcp_url.clone();
    url.set_path("/.well-known/oauth-authorization-server");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

impl OAuthFlow {
    pub fn new(
        http: Arc<dyn OAuthHttp>,
        browser: Arc<dyn BrowserOpener>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            http,
            browser,
            secrets,
        }
    }

    pub async fn authorize(&self, server_id: &str, mcp_url: &str) -> Result<(), OAuthError> {
        let mcp_url = validate_remote_url(mcp_url)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|err| fail(err.to_string()))?;
        let addr = listener.local_addr().map_err(|err| fail(err.to_string()))?;
        let redirect_uri = format!("http://127.0.0.1:{}/oauth/callback", addr.port());

        let (tx, rx) = oneshot::channel::<CallbackQuery>();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let app = Router::new().route(
            "/oauth/callback",
            get({
                let tx = tx.clone();
                move |Query(query): Query<CallbackQuery>| {
                    let tx = tx.clone();
                    async move {
                        if let Some(sender) = tx.lock().ok().and_then(|mut slot| slot.take()) {
                            let _ = sender.send(query);
                        }
                        "Authorization complete. You can close this window."
                    }
                }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let metadata = self.http.get_json(&well_known_url(&mcp_url)?).await?;
        let authorization_endpoint = metadata
            .get("authorization_endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| fail("missing authorization_endpoint"))?;
        let token_endpoint = metadata
            .get("token_endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| fail("missing token_endpoint"))?;
        let registration_endpoint = metadata
            .get("registration_endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| fail("missing registration_endpoint"))?;

        let registration = self
            .http
            .post_json(
                registration_endpoint,
                json!({
                    "client_name": "mcp-manager",
                    "redirect_uris": [redirect_uri],
                    "grant_types": ["authorization_code", "refresh_token"],
                    "response_types": ["code"],
                    "token_endpoint_auth_method": "none"
                }),
            )
            .await?;
        let client_id = registration
            .get("client_id")
            .and_then(Value::as_str)
            .ok_or_else(|| fail("missing client_id"))?
            .to_string();
        let client_secret = registration
            .get("client_secret")
            .and_then(Value::as_str)
            .map(str::to_string);

        let verifier = random_b64(32);
        let state = random_b64(16);
        let mut authorize =
            Url::parse(authorization_endpoint).map_err(|err| fail(err.to_string()))?;
        authorize
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("state", &state)
            .append_pair("code_challenge", &pkce_challenge(&verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("resource", mcp_url.as_str());
        self.browser
            .open_url(authorize.as_str())
            .map_err(|err| fail(err.to_string()))?;

        let callback = tokio::time::timeout(std::time::Duration::from_secs(180), rx)
            .await
            .map_err(|_| fail("timed out waiting for oauth callback"))?
            .map_err(|_| fail("callback closed"))?;
        if callback.state != state {
            return Err(fail("oauth state mismatch"));
        }

        let mut form = HashMap::from([
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), callback.code),
            ("redirect_uri".into(), redirect_uri),
            ("client_id".into(), client_id.clone()),
            ("code_verifier".into(), verifier),
        ]);
        if let Some(secret) = client_secret {
            form.insert("client_secret".into(), secret);
        }
        let tokens = self.http.post_form(token_endpoint, form).await?;
        let access = tokens
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| fail("missing access_token"))?;
        self.secrets
            .set(&server_bearer_key(server_id), access)
            .map_err(|err| fail(err.to_string()))?;
        self.secrets
            .set(&server_oauth_key(server_id, "access_token"), access)
            .map_err(|err| fail(err.to_string()))?;
        self.secrets
            .set(&server_oauth_key(server_id, "client_id"), &client_id)
            .map_err(|err| fail(err.to_string()))?;
        if let Some(refresh) = tokens.get("refresh_token").and_then(Value::as_str) {
            self.secrets
                .set(&server_oauth_key(server_id, "refresh_token"), refresh)
                .map_err(|err| fail(err.to_string()))?;
        }
        Ok(())
    }
}
