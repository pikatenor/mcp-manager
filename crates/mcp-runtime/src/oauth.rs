use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::Query;
use axum::routing::get;
use axum::Router;
use mcp_core::validate_remote_url;
use mcp_platform::{server_bearer_key, server_oauth_key, BrowserOpener, SecretStore};
use oauth2::TokenResponse;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession, CredentialStore,
    StoredCredentials,
};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error(transparent)]
    RemoteUrl(#[from] mcp_core::RemoteUrlError),
    #[error("oauth failed: {0}")]
    Failed(String),
}

fn auth_error(error: AuthError) -> OAuthError {
    OAuthError::Failed(error.to_string())
}

fn store_error(error: mcp_platform::SecretStoreError) -> AuthError {
    AuthError::InternalError(error.to_string())
}

/// Credentials blob persisted under `server:{id}:oauth:credentials` so rmcp's
/// `AuthorizationManager` can round-trip tokens across restarts. The access
/// token is mirrored to the server bearer (and the legacy access-token key)
/// so `McpConnector` and the UI keep working without knowing about the blob.
const CREDENTIALS_SUFFIX: &str = "credentials";

pub struct KeychainCredentialStore {
    secrets: Arc<dyn SecretStore>,
    server_id: String,
}

impl KeychainCredentialStore {
    pub fn new(secrets: Arc<dyn SecretStore>, server_id: impl Into<String>) -> Self {
        Self {
            secrets,
            server_id: server_id.into(),
        }
    }

    fn blob_key(&self) -> String {
        server_oauth_key(&self.server_id, CREDENTIALS_SUFFIX)
    }

    fn mirror_tokens(&self, credentials: &StoredCredentials) -> Result<(), AuthError> {
        let Some(response) = credentials.token_response.as_ref() else {
            return Ok(());
        };
        let access = response.access_token().secret();
        self.secrets
            .set(&server_bearer_key(&self.server_id), access)
            .map_err(store_error)?;
        self.secrets
            .set(&server_oauth_key(&self.server_id, "access_token"), access)
            .map_err(store_error)?;
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for KeychainCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let blob = self.secrets.get(&self.blob_key()).map_err(store_error)?;
        match blob {
            Some(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|err| AuthError::InternalError(err.to_string())),
            None => Ok(None),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let json = serde_json::to_string(&credentials)
            .map_err(|err| AuthError::InternalError(err.to_string()))?;
        self.secrets
            .set(&self.blob_key(), &json)
            .map_err(store_error)?;
        self.mirror_tokens(&credentials)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        for key in [
            self.blob_key(),
            server_bearer_key(&self.server_id),
            server_oauth_key(&self.server_id, "access_token"),
            server_oauth_key(&self.server_id, "refresh_token"),
            server_oauth_key(&self.server_id, "client_id"),
        ] {
            self.secrets.delete(&key).map_err(store_error)?;
        }
        Ok(())
    }
}

pub struct OAuthFlow {
    browser: Arc<dyn BrowserOpener>,
    secrets: Arc<dyn SecretStore>,
}

#[derive(Debug, thiserror::Error)]
enum CallbackWait {
    #[error("timed out waiting for oauth callback")]
    Timeout,
    #[error("callback channel closed")]
    Closed,
}

impl OAuthFlow {
    pub fn new(browser: Arc<dyn BrowserOpener>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { browser, secrets }
    }

    pub async fn authorize(&self, server_id: &str, mcp_url: &str) -> Result<(), OAuthError> {
        let mcp_url = validate_remote_url(mcp_url)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|err| OAuthError::Failed(err.to_string()))?;
        let addr = listener
            .local_addr()
            .map_err(|err| OAuthError::Failed(err.to_string()))?;
        let redirect_uri = format!("http://127.0.0.1:{}/oauth/callback", addr.port());

        // Capture the whole redirect URL: rmcp validates the CSRF state and
        // the optional RFC 9207 `iss` parameter from it. The raw query pairs
        // are re-serialized so unknown parameters survive.
        let (tx, rx) = oneshot::channel::<String>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
        let redirect = redirect_uri.clone();
        let app = Router::new().route(
            "/oauth/callback",
            get({
                let tx = tx.clone();
                move |Query(query): Query<Vec<(String, String)>>| {
                    let tx = tx.clone();
                    let redirect = redirect.clone();
                    async move {
                        if let Some(sender) =
                            tx.lock().ok().and_then(|mut slot| slot.take())
                        {
                            let _ = sender.send(callback_url(&redirect, &query));
                        }
                        "Authorization complete. You can close this window."
                    }
                }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let mut manager =
            AuthorizationManager::new(mcp_url.to_string()).await.map_err(auth_error)?;
        manager.set_credential_store(KeychainCredentialStore::new(
            self.secrets.clone(),
            server_id,
        ));
        let resolution = manager.resolve_metadata().await.map_err(auth_error)?;
        manager.set_metadata(resolution.metadata);

        let session = AuthorizationSession::new(
            manager,
            AuthorizationRequest::new(redirect_uri).with_client_name("mcp-manager"),
        )
        .await
        .map_err(|(_, error)| auth_error(error))?;
        self.browser
            .open_url(session.get_authorization_url())
            .map_err(|err| OAuthError::Failed(err.to_string()))?;

        let callback = tokio::time::timeout(Duration::from_secs(180), rx)
            .await
            .map_err(|_| CallbackWait::Timeout)
            .and_then(|result| result.map_err(|_| CallbackWait::Closed))
            .map_err(|err| OAuthError::Failed(err.to_string()))?;
        // Exchanges the code (PKCE + resource) and saves credentials through
        // the keychain-backed CredentialStore configured above.
        session
            .handle_callback_url(&callback)
            .await
            .map_err(auth_error)?;
        Ok(())
    }
}

fn callback_url(redirect_uri: &str, query: &[(String, String)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in query {
        serializer.append_pair(key, value);
    }
    format!("{redirect_uri}?{}", serializer.finish())
}
