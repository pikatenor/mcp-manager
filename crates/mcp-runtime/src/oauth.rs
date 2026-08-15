use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_platform::{BrowserOpener, SecretStore};
use serde_json::Value;

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

pub struct OAuthFlow {
    http: Arc<dyn OAuthHttp>,
    browser: Arc<dyn BrowserOpener>,
    secrets: Arc<dyn SecretStore>,
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

    pub async fn authorize(&self, _server_id: &str, _mcp_url: &str) -> Result<(), OAuthError> {
        unimplemented!("OAuthFlow::authorize")
    }
}
