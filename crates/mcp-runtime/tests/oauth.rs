use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mcp_platform::{
    server_bearer_key, server_oauth_key, BrowserError, BrowserOpener, MemorySecretStore,
    SecretStore,
};
use mcp_runtime::{OAuthError, OAuthFlow, OAuthHttp};
use serde_json::{json, Value};
use tokio::sync::Notify;
use url::Url;

struct FakeHttp {
    well_known: Value,
    client_id: String,
    access_token: String,
    refresh_token: String,
    posts: Mutex<Vec<(String, Value)>>,
    forms: Mutex<Vec<(String, HashMap<String, String>)>>,
}

#[async_trait]
impl OAuthHttp for FakeHttp {
    async fn get_json(&self, url: &str) -> Result<Value, OAuthError> {
        if url.contains("/.well-known/oauth-authorization-server") {
            return Ok(self.well_known.clone());
        }
        Err(OAuthError::Failed(format!("unexpected GET {url}")))
    }

    async fn post_json(&self, url: &str, body: Value) -> Result<Value, OAuthError> {
        self.posts.lock().unwrap().push((url.to_string(), body));
        if url.ends_with("/register") {
            return Ok(json!({
                "client_id": self.client_id,
                "client_secret": "cs"
            }));
        }
        Err(OAuthError::Failed(format!("unexpected POST JSON {url}")))
    }

    async fn post_form(
        &self,
        url: &str,
        form: HashMap<String, String>,
    ) -> Result<Value, OAuthError> {
        self.forms
            .lock()
            .unwrap()
            .push((url.to_string(), form.clone()));
        if url.ends_with("/token") {
            assert_eq!(
                form.get("grant_type").map(String::as_str),
                Some("authorization_code")
            );
            assert_eq!(form.get("code").map(String::as_str), Some("auth-code"));
            assert!(form.contains_key("code_verifier"));
            return Ok(json!({
                "access_token": self.access_token,
                "refresh_token": self.refresh_token,
                "token_type": "Bearer"
            }));
        }
        Err(OAuthError::Failed(format!("unexpected POST form {url}")))
    }
}

struct RecordingBrowser {
    url: Mutex<Option<String>>,
    notify: Notify,
}

impl BrowserOpener for RecordingBrowser {
    fn open_url(&self, url: &str) -> Result<(), BrowserError> {
        *self.url.lock().unwrap() = Some(url.to_string());
        self.notify.notify_waiters();
        Ok(())
    }
}

impl RecordingBrowser {
    async fn wait_url(&self) -> String {
        loop {
            if let Some(url) = self.url.lock().unwrap().clone() {
                return url;
            }
            self.notify.notified().await;
        }
    }
}

fn metadata() -> Value {
    json!({
        "issuer": "https://auth.example",
        "authorization_endpoint": "https://auth.example/authorize",
        "token_endpoint": "https://auth.example/token",
        "registration_endpoint": "https://auth.example/register"
    })
}

#[tokio::test]
async fn authorize_pkce_loopback_stores_tokens() {
    let http = Arc::new(FakeHttp {
        well_known: metadata(),
        client_id: "cid-1".into(),
        access_token: "at-123".into(),
        refresh_token: "rt-123".into(),
        posts: Mutex::new(Vec::new()),
        forms: Mutex::new(Vec::new()),
    });
    let browser = Arc::new(RecordingBrowser {
        url: Mutex::new(None),
        notify: Notify::new(),
    });
    let secrets = Arc::new(MemorySecretStore::new());
    let flow = OAuthFlow::new(http.clone(), browser.clone(), secrets.clone());
    let task = tokio::spawn({
        let flow = OAuthFlow::new(http.clone(), browser.clone(), secrets.clone());
        async move { flow.authorize("srv-1", "https://mcp.example.com/mcp").await }
    });
    drop(flow);

    let auth_url = tokio::time::timeout(std::time::Duration::from_secs(2), browser.wait_url())
        .await
        .expect("browser opened");
    let parsed = Url::parse(&auth_url).unwrap();
    assert_eq!(parsed.host_str(), Some("auth.example"));
    let pairs: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(pairs.get("client_id").map(String::as_str), Some("cid-1"));
    assert_eq!(
        pairs.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert!(pairs.contains_key("code_challenge"));
    let state = pairs.get("state").cloned().unwrap();
    let redirect = pairs.get("redirect_uri").cloned().unwrap();
    assert!(redirect.starts_with("http://127.0.0.1:"));
    assert!(redirect.contains("/oauth/callback"));

    let (register_url, register_body) = http.posts.lock().unwrap()[0].clone();
    assert!(register_url.ends_with("/register"));
    let redirect_uris = register_body["redirect_uris"].as_array().unwrap();
    assert_eq!(redirect_uris[0].as_str(), Some(redirect.as_str()));

    let callback = format!("{redirect}?code=auth-code&state={state}");
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(&callback)
        .send()
        .await
        .unwrap();

    task.await.unwrap().unwrap();
    assert_eq!(
        secrets.get(&server_bearer_key("srv-1")).unwrap().as_deref(),
        Some("at-123")
    );
    assert_eq!(
        secrets
            .get(&server_oauth_key("srv-1", "refresh_token"))
            .unwrap()
            .as_deref(),
        Some("rt-123")
    );
}

#[tokio::test]
async fn authorize_rejects_plain_http_public_host() {
    let http = Arc::new(FakeHttp {
        well_known: metadata(),
        client_id: "cid-1".into(),
        access_token: "at".into(),
        refresh_token: "rt".into(),
        posts: Mutex::new(Vec::new()),
        forms: Mutex::new(Vec::new()),
    });
    let browser = Arc::new(RecordingBrowser {
        url: Mutex::new(None),
        notify: Notify::new(),
    });
    let secrets = Arc::new(MemorySecretStore::new());
    let flow = OAuthFlow::new(http, browser, secrets);
    let err = match flow.authorize("srv-1", "http://example.com/mcp").await {
        Err(err) => err,
        Ok(()) => panic!("expected url rejection"),
    };
    assert!(matches!(err, OAuthError::RemoteUrl(_)));
}
