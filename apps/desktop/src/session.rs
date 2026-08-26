use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use mcp_core::{
    is_tool_public, Aggregator, BackendConnector, IssuedToken, ServerConfig, ServerRegistry,
    ServerState, ServerType, TokenRecord, TokenService,
};
use mcp_platform::{
    server_bearer_key, server_env_key, server_oauth_key, BrowserOpener, SecretStore,
    SecretStoreError,
};
use mcp_runtime::OAuthFlow;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Clone)]
pub struct Session {
    tokens: Arc<Mutex<TokenService>>,
    registry: Arc<AsyncMutex<ServerRegistry>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
    secrets: Arc<dyn SecretStore>,
    browser: Arc<dyn BrowserOpener>,
}

#[derive(Debug, Clone)]
pub struct AddServerRequest {
    pub name: String,
    pub server_type: ServerType,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub remote_url: Option<String>,
    pub auto_start: bool,
    pub bearer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerToolView {
    pub name: String,
    pub public: bool,
}

pub fn parse_env(raw: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        if eq == 0 {
            continue;
        }
        env.insert(trimmed[..eq].to_string(), trimmed[eq + 1..].to_string());
    }
    env
}

fn new_server_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("srv-{nanos:x}")
}

fn load_secrets(
    store: &dyn SecretStore,
    config: &ServerConfig,
) -> Result<HashMap<String, String>, SecretStoreError> {
    let mut secrets = HashMap::new();
    for key in &config.env_keys {
        if let Some(value) = store.get(&server_env_key(&config.id, key))? {
            secrets.insert(key.clone(), value);
        }
    }
    if let Some(bearer) = store.get(&server_bearer_key(&config.id))? {
        secrets.insert("BEARER_TOKEN".into(), bearer);
    }
    Ok(secrets)
}

fn persist_secrets(
    store: &dyn SecretStore,
    id: &str,
    env: &HashMap<String, String>,
    bearer: Option<&str>,
) -> Result<(), SecretStoreError> {
    for (key, value) in env {
        store.set(&server_env_key(id, key), value)?;
    }
    if let Some(token) = bearer.filter(|value| !value.is_empty()) {
        store.set(&server_bearer_key(id), token)?;
    }
    Ok(())
}

fn delete_secrets(store: &dyn SecretStore, config: &ServerConfig) {
    for key in &config.env_keys {
        let _ = store.delete(&server_env_key(&config.id, key));
    }
    let _ = store.delete(&server_bearer_key(&config.id));
    let _ = store.delete(&server_oauth_key(&config.id, "access_token"));
    let _ = store.delete(&server_oauth_key(&config.id, "refresh_token"));
    let _ = store.delete(&server_oauth_key(&config.id, "client_id"));
    let _ = store.delete(&server_oauth_key(&config.id, "credentials"));
}

impl Session {
    pub fn open(
        data_dir: &Path,
        connector: Arc<dyn BackendConnector>,
        secrets: Arc<dyn SecretStore>,
        browser: Arc<dyn BrowserOpener>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let tokens =
            TokenService::open_sqlite(&data_dir.join("tokens.db")).map_err(|e| e.to_string())?;
        let registry = ServerRegistry::open_sqlite(&data_dir.join("state.db"), connector)
            .map_err(|e| e.to_string())?;
        let aggregator = registry.aggregator();
        Ok(Self {
            tokens: Arc::new(Mutex::new(tokens)),
            registry: Arc::new(AsyncMutex::new(registry)),
            aggregator,
            secrets,
            browser,
        })
    }

    pub fn tokens(&self) -> Arc<Mutex<TokenService>> {
        self.tokens.clone()
    }

    pub fn aggregator(&self) -> Arc<AsyncMutex<Aggregator>> {
        self.aggregator.clone()
    }

    pub fn aggregator_endpoint() -> String {
        format!(
            "http://{}{}",
            mcp_core::DEFAULT_HTTP_BIND,
            mcp_core::DEFAULT_MCP_PATH
        )
    }

    pub fn issue_token(&self, client_name: &str) -> Result<IssuedToken, String> {
        let name = client_name.trim();
        if name.is_empty() {
            return Err("client name is required".into());
        }
        self.tokens
            .lock()
            .map_err(|e| e.to_string())
            .map(|mut svc| svc.issue(name))
    }

    pub fn list_tokens(&self) -> Result<Vec<TokenRecord>, String> {
        self.tokens
            .lock()
            .map_err(|e| e.to_string())
            .map(|svc| svc.list())
    }

    pub fn revoke_token(&self, id: &str) -> Result<(), String> {
        self.tokens
            .lock()
            .map_err(|e| e.to_string())?
            .revoke(id)
            .map_err(|e| e.to_string())
    }

    pub async fn list_servers(&self) -> Result<Vec<ServerState>, String> {
        self.registry.lock().await.list().map_err(|e| e.to_string())
    }

    pub async fn add_server(&self, request: AddServerRequest) -> Result<ServerConfig, String> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err("server name is required".into());
        }
        let id = new_server_id();
        let env_keys: Vec<String> = request.env.keys().cloned().collect();
        persist_secrets(
            self.secrets.as_ref(),
            &id,
            &request.env,
            request.bearer.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        let config = ServerConfig {
            id,
            name: name.to_string(),
            server_type: request.server_type,
            command: request.command.filter(|c| !c.trim().is_empty()),
            args: request.args,
            env_keys,
            remote_url: request.remote_url.filter(|u| !u.trim().is_empty()),
            auto_start: request.auto_start,
            disabled: false,
            tool_permissions: HashMap::new(),
        };
        self.registry
            .lock()
            .await
            .add(config)
            .map_err(|e| e.to_string())
    }

    pub async fn delete_server(&self, id: &str) -> Result<bool, String> {
        let mut registry = self.registry.lock().await;
        let config = registry
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|state| state.config.id == id)
            .map(|state| state.config);
        if let Some(config) = config {
            delete_secrets(self.secrets.as_ref(), &config);
        }
        registry.delete(id).await.map_err(|e| e.to_string())
    }

    pub async fn start_server(&self, id: &str) -> Result<(), String> {
        let mut registry = self.registry.lock().await;
        let config = registry
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|state| state.config.id == id)
            .ok_or_else(|| format!("unknown server: {id}"))?
            .config;
        let server_secrets =
            load_secrets(self.secrets.as_ref(), &config).map_err(|e| e.to_string())?;
        registry
            .start(id, server_secrets)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn stop_server(&self, id: &str) -> Result<(), String> {
        self.registry
            .lock()
            .await
            .stop(id)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn list_server_tools(&self, id: &str) -> Result<Vec<ServerToolView>, String> {
        let registry = self.registry.lock().await;
        let config = registry
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|state| state.config.id == id)
            .ok_or_else(|| format!("unknown server: {id}"))?
            .config;
        let tools = registry
            .aggregator()
            .lock()
            .await
            .origin_tools(id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(tools
            .into_iter()
            .map(|tool| ServerToolView {
                public: is_tool_public(&config.tool_permissions, &tool.name),
                name: tool.name,
            })
            .collect())
    }

    pub async fn set_tool_permission(
        &self,
        id: &str,
        tool_name: &str,
        public: bool,
    ) -> Result<(), String> {
        let mut registry = self.registry.lock().await;
        let mut config = registry
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|state| state.config.id == id)
            .ok_or_else(|| format!("unknown server: {id}"))?
            .config;
        config
            .tool_permissions
            .insert(tool_name.to_string(), public);
        registry.update(config).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn oauth_connect(&self, id: &str) -> Result<(), String> {
        let remote_url = {
            let registry = self.registry.lock().await;
            let state = registry
                .list()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|state| state.config.id == id)
                .ok_or_else(|| format!("unknown server: {id}"))?;
            if state.config.server_type == ServerType::Local {
                return Err("OAuth is only for remote servers".into());
            }
            state
                .config
                .remote_url
                .ok_or_else(|| "remote_url is required".to_string())?
        };
        let flow = OAuthFlow::new(self.browser.clone(), self.secrets.clone());
        flow.authorize(id, &remote_url)
            .await
            .map_err(|e| e.to_string())
    }

    pub fn oauth_connected(&self, id: &str) -> Result<bool, String> {
        Ok(self
            .secrets
            .get(&server_oauth_key(id, "access_token"))
            .map_err(|e| e.to_string())?
            .filter(|value| !value.is_empty())
            .is_some())
    }

    pub async fn auto_start(&self) -> Result<(), String> {
        let mut registry = self.registry.lock().await;
        let mut all_secrets = HashMap::new();
        if let Ok(listed) = registry.list() {
            for state in listed {
                match load_secrets(self.secrets.as_ref(), &state.config) {
                    Ok(server_secrets) => {
                        all_secrets.insert(state.config.id.clone(), server_secrets);
                    }
                    Err(error) => {
                        eprintln!("secret load failed for {}: {error}", state.config.id);
                    }
                }
            }
        }
        registry
            .auto_start(all_secrets)
            .await
            .map_err(|e| e.to_string())
    }
}
