use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use mcp_core::{
    is_tool_public, Aggregator, BackendConnector, CallLog, ImportedServer, IssuedToken,
    ServerConfig, ServerRegistry, ServerState, ServerStatus, ServerType, TokenRecord, TokenService,
    ToolCallEntry,
};
use mcp_platform::{
    server_bearer_key, server_env_key, server_oauth_client_id_key,
    server_oauth_client_secret_key, server_oauth_key, BrowserOpener, SecretStore,
    SecretStoreError,
};
use mcp_runtime::OAuthFlow;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Clone)]
pub struct Session {
    tokens: Arc<Mutex<TokenService>>,
    registry: Arc<AsyncMutex<ServerRegistry>>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
    call_log: Arc<CallLog>,
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
    /// Pre-registered OAuth client id. `None`/blank clears the static client
    /// config on update; the form prefills it, so blank means emptied.
    pub oauth_client_id: Option<String>,
    /// Pre-registered OAuth client secret. `None`/blank keeps the stored one:
    /// the form never echoes it back, so blank means unchanged.
    pub oauth_client_secret: Option<String>,
}

/// Env values, bearer, and OAuth client id for one server, for edit-form
/// prefill. The client secret is deliberately not read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSecretValues {
    pub env: HashMap<String, String>,
    pub bearer: Option<String>,
    pub oauth_client_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerToolView {
    pub name: String,
    pub public: bool,
}

/// Report of one JSON import: what was added, skipped, and why others failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOutcome {
    pub added: Vec<String>,
    pub skipped: Vec<String>,
    pub invalid: Vec<String>,
}

impl ImportOutcome {
    /// One-line report for the notice banner.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!(
                "imported {} ({})",
                self.added.len(),
                self.added.join(", ")
            ));
        }
        if !self.skipped.is_empty() {
            parts.push(format!(
                "skipped {} (already exist: {})",
                self.skipped.len(),
                self.skipped.join(", ")
            ));
        }
        if !self.invalid.is_empty() {
            parts.push(format!(
                "{} invalid ({})",
                self.invalid.len(),
                self.invalid.join("; ")
            ));
        }
        if parts.is_empty() {
            "No servers found to import".into()
        } else {
            parts.join("; ")
        }
    }
}

impl From<ImportedServer> for AddServerRequest {
    fn from(server: ImportedServer) -> Self {
        // Imported servers start stopped; auto-start is a per-server choice.
        Self {
            name: server.name,
            server_type: server.server_type,
            command: server.command,
            args: server.args,
            env: server.env,
            remote_url: server.remote_url,
            auto_start: false,
            bearer: server.bearer,
            oauth_client_id: None,
            oauth_client_secret: None,
        }
    }
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

/// Set-or-clear the static OAuth client identity. A blank id clears both keys
/// (the form prefills it, so blank means emptied); a blank secret keeps the
/// stored one (the form never echoes it back).
fn persist_oauth_client(
    store: &dyn SecretStore,
    id: &str,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<(), SecretStoreError> {
    let Some(client_id) = client_id.map(str::trim).filter(|value| !value.is_empty()) else {
        store.delete(&server_oauth_client_id_key(id))?;
        store.delete(&server_oauth_client_secret_key(id))?;
        return Ok(());
    };
    store.set(&server_oauth_client_id_key(id), client_id)?;
    if let Some(secret) = client_secret.map(str::trim).filter(|value| !value.is_empty()) {
        store.set(&server_oauth_client_secret_key(id), secret)?;
    }
    Ok(())
}

/// Shared add/update validation, run before any keychain write.
fn validate_oauth_client(request: &AddServerRequest) -> Result<(), String> {
    let has_id = request
        .oauth_client_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_secret = request
        .oauth_client_secret
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if has_secret && !has_id {
        return Err("oauth client secret requires a client id".into());
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
    let _ = store.delete(&server_oauth_client_id_key(&config.id));
    let _ = store.delete(&server_oauth_client_secret_key(&config.id));
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
        let call_log =
            CallLog::open_sqlite(&data_dir.join("calls.db")).map_err(|e| e.to_string())?;
        Ok(Self {
            tokens: Arc::new(Mutex::new(tokens)),
            registry: Arc::new(AsyncMutex::new(registry)),
            aggregator,
            call_log: Arc::new(call_log),
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

    /// The same call log the HTTP server records into.
    pub fn call_log(&self) -> Arc<CallLog> {
        self.call_log.clone()
    }

    pub fn list_tool_calls(&self, limit: usize) -> Result<Vec<ToolCallEntry>, String> {
        self.call_log.list_recent(limit).map_err(|e| e.to_string())
    }

    pub fn clear_tool_calls(&self) -> Result<(), String> {
        self.call_log.clear();
        Ok(())
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
        validate_oauth_client(&request)?;
        // Pre-check before persisting secrets so a rejected add cannot orphan
        // keychain entries; the registry check stays the authority.
        {
            let registry = self.registry.lock().await;
            let duplicate = registry
                .list()
                .map_err(|e| e.to_string())?
                .iter()
                .any(|state| state.config.name.trim() == name);
            if duplicate {
                return Err(format!("server name already exists: {name}"));
            }
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
        persist_oauth_client(
            self.secrets.as_ref(),
            &id,
            request.oauth_client_id.as_deref(),
            request.oauth_client_secret.as_deref(),
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

    /// Env values, bearer, and OAuth client id for one server, for edit-form
    /// prefill. The client secret is never read back out of the keychain.
    pub async fn server_secret_values(&self, id: &str) -> Result<ServerSecretValues, String> {
        let config = self
            .registry
            .lock()
            .await
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|state| state.config.id == id)
            .ok_or_else(|| format!("unknown server: {id}"))?
            .config;
        let mut env = HashMap::new();
        for key in &config.env_keys {
            if let Some(value) = self
                .secrets
                .get(&server_env_key(&config.id, key))
                .map_err(|e| e.to_string())?
            {
                env.insert(key.clone(), value);
            }
        }
        let bearer = self
            .secrets
            .get(&server_bearer_key(&config.id))
            .map_err(|e| e.to_string())?
            .filter(|value| !value.is_empty());
        let oauth_client_id = self
            .secrets
            .get(&server_oauth_client_id_key(&config.id))
            .map_err(|e| e.to_string())?
            .filter(|value| !value.is_empty());
        Ok(ServerSecretValues {
            env,
            bearer,
            oauth_client_id,
        })
    }

    /// Rewrite one server's config in place, keeping its id, disabled flag,
    /// and tool permissions. A running server is restarted on the new config.
    pub async fn update_server(&self, id: &str, request: AddServerRequest) -> Result<(), String> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err("server name is required".into());
        }
        validate_oauth_client(&request)?;
        let (existing, was_running) = {
            // Scoped lock: update validates and persists before any keychain
            // mutation, and the lock must be released before stop/start.
            let mut registry = self.registry.lock().await;
            let state = registry
                .list()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|state| state.config.id == id)
                .ok_or_else(|| format!("unknown server: {id}"))?;
            let was_running = state.status == ServerStatus::Running;
            let mut updated = state.config.clone();
            updated.name = name.to_string();
            updated.server_type = request.server_type;
            updated.command = request.command.filter(|c| !c.trim().is_empty());
            updated.args = request.args;
            updated.env_keys = request.env.keys().cloned().collect();
            updated.remote_url = request.remote_url.filter(|u| !u.trim().is_empty());
            updated.auto_start = request.auto_start;
            registry.update(updated).map_err(|e| e.to_string())?;
            (state.config, was_running)
        };
        let old_client_id = self
            .secrets
            .get(&server_oauth_client_id_key(id))
            .map_err(|e| e.to_string())?
            .filter(|value| !value.is_empty());
        let old_client_secret = self
            .secrets
            .get(&server_oauth_client_secret_key(id))
            .map_err(|e| e.to_string())?
            .filter(|value| !value.is_empty());
        for key in &existing.env_keys {
            if !request.env.contains_key(key) {
                let _ = self.secrets.delete(&server_env_key(id, key));
            }
        }
        persist_secrets(self.secrets.as_ref(), id, &request.env, request.bearer.as_deref())
            .map_err(|e| e.to_string())?;
        persist_oauth_client(
            self.secrets.as_ref(),
            id,
            request.oauth_client_id.as_deref(),
            request.oauth_client_secret.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        // Tokens minted under one client identity cannot refresh under
        // another, so a changed static client config invalidates the
        // flow-derived token state (a re-typed identical config does not).
        let new_client_id = request
            .oauth_client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let new_client_secret = request
            .oauth_client_secret
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let client_config_changed = new_client_id != old_client_id.as_deref()
            || new_client_secret.is_some_and(|secret| Some(secret) != old_client_secret.as_deref());
        if client_config_changed {
            for key in [
                server_oauth_key(id, "credentials"),
                server_bearer_key(id),
                server_oauth_key(id, "access_token"),
                server_oauth_key(id, "refresh_token"),
            ] {
                let _ = self.secrets.delete(&key);
            }
        }
        if was_running {
            let _ = self.stop_server(id).await;
            if let Err(error) = self.start_server(id).await {
                // The save itself succeeded; the failed restart surfaces as
                // the server's Error status and last_error.
                eprintln!("restart after update failed for {id}: {error}");
            }
        }
        Ok(())
    }

    /// Import servers from a Cursor/Claude Desktop style config document.
    /// Same-name entries are skipped; invalid entries are reported, not fatal.
    pub async fn import_json(&self, raw: &str) -> Result<ImportOutcome, String> {
        let parsed = mcp_core::parse_mcp_servers(raw);
        let mut known: HashSet<String> = self
            .registry
            .lock()
            .await
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|state| state.config.name.trim().to_string())
            .collect();
        let mut outcome = ImportOutcome::default();
        for server in parsed.servers {
            let name = server.name.clone();
            if known.contains(&name) {
                outcome.skipped.push(name);
                continue;
            }
            match self.add_server(server.into()).await {
                Ok(_) => {
                    known.insert(name.clone());
                    outcome.added.push(name);
                }
                Err(error) => outcome.invalid.push(format!("{name}: {error}")),
            }
        }
        outcome.invalid.extend(parsed.errors);
        Ok(outcome)
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
