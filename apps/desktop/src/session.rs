use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use mcp_core::{
    BackendConnector, IssuedToken, ServerConfig, ServerRegistry, ServerState, ServerType,
    TokenRecord, TokenService,
};
use mcp_platform::{BrowserOpener, SecretStore};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Clone)]
pub struct Session {
    #[allow(dead_code)]
    tokens: Arc<Mutex<TokenService>>,
    #[allow(dead_code)]
    registry: Arc<AsyncMutex<ServerRegistry>>,
    #[allow(dead_code)]
    secrets: Arc<dyn SecretStore>,
    #[allow(dead_code)]
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

pub fn parse_env(_raw: &str) -> HashMap<String, String> {
    unimplemented!("parse_env")
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
        Ok(Self {
            tokens: Arc::new(Mutex::new(tokens)),
            registry: Arc::new(AsyncMutex::new(registry)),
            secrets,
            browser,
        })
    }

    pub fn aggregator_endpoint() -> String {
        unimplemented!("aggregator_endpoint")
    }

    pub fn issue_token(&self, client_name: &str) -> Result<IssuedToken, String> {
        let _ = client_name;
        unimplemented!("issue_token")
    }

    pub fn list_tokens(&self) -> Result<Vec<TokenRecord>, String> {
        unimplemented!("list_tokens")
    }

    pub fn revoke_token(&self, id: &str) -> Result<(), String> {
        let _ = id;
        unimplemented!("revoke_token")
    }

    pub async fn list_servers(&self) -> Result<Vec<ServerState>, String> {
        unimplemented!("list_servers")
    }

    pub async fn add_server(&self, request: AddServerRequest) -> Result<ServerConfig, String> {
        let _ = request;
        unimplemented!("add_server")
    }

    pub async fn delete_server(&self, id: &str) -> Result<bool, String> {
        let _ = id;
        unimplemented!("delete_server")
    }

    pub async fn start_server(&self, id: &str) -> Result<(), String> {
        let _ = id;
        unimplemented!("start_server")
    }

    pub async fn stop_server(&self, id: &str) -> Result<(), String> {
        let _ = id;
        unimplemented!("stop_server")
    }

    pub async fn list_server_tools(&self, id: &str) -> Result<Vec<ServerToolView>, String> {
        let _ = id;
        unimplemented!("list_server_tools")
    }

    pub async fn set_tool_permission(
        &self,
        id: &str,
        tool_name: &str,
        public: bool,
    ) -> Result<(), String> {
        let _ = (id, tool_name, public);
        unimplemented!("set_tool_permission")
    }

    pub async fn oauth_connect(&self, id: &str) -> Result<(), String> {
        let _ = id;
        unimplemented!("oauth_connect")
    }

    pub fn oauth_connected(&self, id: &str) -> Result<bool, String> {
        let _ = id;
        unimplemented!("oauth_connected")
    }

    pub async fn auto_start(&self) -> Result<(), String> {
        unimplemented!("auto_start")
    }
}
