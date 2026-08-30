use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::aggregator::{
    AggregatedTool, Aggregator, AggregatorError, McpBackend, RegisteredServer,
};
use super::remote_url::validate_remote_url;
use super::servers::{ServerConfig, ServerStore, ServerType};
use super::store::StoreError;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerState {
    pub config: ServerConfig,
    pub status: ServerStatus,
    pub last_error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    RemoteUrl(#[from] super::remote_url::RemoteUrlError),
    #[error("unknown server: {0}")]
    UnknownServer(String),
    #[error("server name already exists: {0}")]
    DuplicateName(String),
    #[error("invalid server config: {0}")]
    InvalidConfig(String),
    #[error("backend error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait BackendConnector: Send + Sync {
    async fn connect(
        &self,
        config: &ServerConfig,
        secrets: &HashMap<String, String>,
    ) -> Result<Arc<dyn McpBackend>, RegistryError>;
}

pub struct ServerRegistry {
    store: ServerStore,
    connector: Arc<dyn BackendConnector>,
    aggregator: Arc<AsyncMutex<Aggregator>>,
    statuses: HashMap<String, ServerStatus>,
    errors: HashMap<String, String>,
}

fn validate_config(config: &ServerConfig) -> Result<(), RegistryError> {
    let name = config.name.trim();
    if name.is_empty() {
        return Err(RegistryError::InvalidConfig("server name is required".into()));
    }
    if name.contains(crate::naming::TOOL_DELIMITER) {
        return Err(RegistryError::InvalidConfig(format!(
            "server names must not contain \"{}\"",
            crate::naming::TOOL_DELIMITER
        )));
    }
    match config.server_type {
        ServerType::Local => Ok(()),
        ServerType::Remote | ServerType::RemoteStreamable => {
            let url = config
                .remote_url
                .as_deref()
                .ok_or_else(|| RegistryError::InvalidConfig("remote_url is required".into()))?;
            validate_remote_url(url)?;
            Ok(())
        }
    }
}

impl ServerRegistry {
    pub fn open_sqlite(
        path: &Path,
        connector: Arc<dyn BackendConnector>,
    ) -> Result<Self, RegistryError> {
        Ok(Self {
            store: ServerStore::open_sqlite(path)?,
            connector,
            aggregator: Arc::new(AsyncMutex::new(Aggregator::new())),
            statuses: HashMap::new(),
            errors: HashMap::new(),
        })
    }

    pub fn add(&mut self, config: ServerConfig) -> Result<ServerConfig, RegistryError> {
        validate_config(&config)?;
        self.ensure_unique_name(&config.name, None)?;
        Ok(self.store.add(config)?)
    }

    pub fn update(&mut self, config: ServerConfig) -> Result<ServerConfig, RegistryError> {
        validate_config(&config)?;
        self.ensure_unique_name(&config.name, Some(&config.id))?;
        let updated = self.store.update(config)?;
        if let Ok(mut aggregator) = self.aggregator.try_lock() {
            aggregator.set_tool_permissions(&updated.id, updated.tool_permissions.clone());
        }
        Ok(updated)
    }

    fn ensure_unique_name(
        &self,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<(), RegistryError> {
        let name = name.trim();
        if self.store.list()?.iter().any(|config| {
            Some(config.id.as_str()) != exclude_id && config.name.trim() == name
        }) {
            return Err(RegistryError::DuplicateName(name.to_string()));
        }
        Ok(())
    }

    pub async fn delete(&mut self, id: &str) -> Result<bool, RegistryError> {
        if self.store.get(id)?.is_none() {
            return Ok(false);
        }
        let _ = self.stop(id).await;
        Ok(self.store.delete(id)?)
    }

    pub fn aggregator(&self) -> Arc<AsyncMutex<Aggregator>> {
        self.aggregator.clone()
    }

    pub fn list(&self) -> Result<Vec<ServerState>, RegistryError> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .map(|config| {
                let id = &config.id;
                ServerState {
                    status: self
                        .statuses
                        .get(id)
                        .copied()
                        .unwrap_or(ServerStatus::Stopped),
                    last_error: self.errors.get(id).cloned(),
                    config,
                }
            })
            .collect())
    }

    pub async fn start(
        &mut self,
        id: &str,
        secrets: HashMap<String, String>,
    ) -> Result<(), RegistryError> {
        let config = self
            .store
            .get(id)?
            .ok_or_else(|| RegistryError::UnknownServer(id.to_string()))?;
        validate_config(&config)?;
        if config.server_type == ServerType::Local && config.command.is_none() {
            return Err(RegistryError::InvalidConfig(
                "command is required for local servers".into(),
            ));
        }

        self.statuses
            .insert(config.id.clone(), ServerStatus::Starting);
        match self.connector.connect(&config, &secrets).await {
            Ok(backend) => {
                self.aggregator
                    .lock()
                    .await
                    .upsert_server(RegisteredServer {
                        id: config.id.clone(),
                        name: config.name.clone(),
                        running: true,
                        tool_permissions: config.tool_permissions.clone(),
                        backend,
                    });
                self.statuses
                    .insert(config.id.clone(), ServerStatus::Running);
                self.errors.remove(&config.id);
                Ok(())
            }
            Err(err) => {
                self.statuses.insert(config.id.clone(), ServerStatus::Error);
                self.errors.insert(config.id.clone(), err.to_string());
                Err(err)
            }
        }
    }

    pub async fn stop(&mut self, id: &str) -> Result<(), RegistryError> {
        let config = self
            .store
            .get(id)?
            .ok_or_else(|| RegistryError::UnknownServer(id.to_string()))?;
        self.statuses
            .insert(config.id.clone(), ServerStatus::Stopping);
        self.aggregator.lock().await.set_running(&config.id, false);
        self.statuses
            .insert(config.id.clone(), ServerStatus::Stopped);
        self.errors.remove(&config.id);
        Ok(())
    }

    pub async fn auto_start(
        &mut self,
        mut secrets: HashMap<String, HashMap<String, String>>,
    ) -> Result<(), RegistryError> {
        let ids: Vec<String> = self
            .store
            .list()?
            .into_iter()
            .filter(|config| config.auto_start && !config.disabled)
            .map(|config| config.id)
            .collect();
        for id in ids {
            let server_secrets = secrets.remove(&id).unwrap_or_default();
            self.start(&id, server_secrets).await?;
        }
        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<AggregatedTool>, AggregatorError> {
        self.aggregator.lock().await.list_tools().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregator::{AggregatorError, Tool};
    use crate::remote_url::RemoteUrlError;
    use crate::servers::ServerType;
    use std::sync::Mutex;

    struct StaticBackend {
        tools: Vec<Tool>,
    }

    #[async_trait]
    impl McpBackend for StaticBackend {
        async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError> {
            Ok(self.tools.clone())
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> Result<serde_json::Value, AggregatorError> {
            Ok(serde_json::json!({ "ok": true }))
        }
    }

    #[derive(Default)]
    struct RecordingConnector {
        seen: Mutex<Vec<(String, HashMap<String, String>)>>,
        tools: Mutex<Vec<Tool>>,
    }

    impl RecordingConnector {
        fn with_tools(tools: Vec<Tool>) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
                tools: Mutex::new(tools),
            })
        }
    }

    #[async_trait]
    impl BackendConnector for RecordingConnector {
        async fn connect(
            &self,
            config: &ServerConfig,
            secrets: &HashMap<String, String>,
        ) -> Result<Arc<dyn McpBackend>, RegistryError> {
            self.seen
                .lock()
                .unwrap()
                .push((config.id.clone(), secrets.clone()));
            let tools = self.tools.lock().unwrap().clone();
            Ok(Arc::new(StaticBackend { tools }))
        }
    }

    fn local_config(id: &str, name: &str) -> ServerConfig {
        ServerConfig {
            id: id.into(),
            name: name.into(),
            server_type: ServerType::Local,
            command: Some("npx".into()),
            args: vec!["-y".into(), "mcp-server".into()],
            env_keys: vec!["API_TOKEN".into()],
            remote_url: None,
            auto_start: true,
            disabled: false,
            tool_permissions: HashMap::new(),
        }
    }

    fn tool(name: &str) -> Tool {
        Tool {
            name: name.into(),
            description: None,
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        }
    }

    #[test]
    fn persisted_servers_load_as_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let connector = RecordingConnector::with_tools(vec![]);
        {
            let mut registry = ServerRegistry::open_sqlite(&path, connector.clone()).unwrap();
            registry.add(local_config("srv-1", "everything")).unwrap();
        }
        let registry = ServerRegistry::open_sqlite(&path, connector).unwrap();
        let listed = registry.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].config.name, "everything");
        assert_eq!(listed[0].status, ServerStatus::Stopped);
        assert!(listed[0].last_error.is_none());
    }

    #[test]
    fn add_rejects_plain_http_remote() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        let err = registry
            .add(ServerConfig {
                id: "bad".into(),
                name: "bad".into(),
                server_type: ServerType::Remote,
                command: None,
                args: vec![],
                env_keys: vec![],
                remote_url: Some("http://example.com/sse".into()),
                auto_start: false,
                disabled: false,
                tool_permissions: HashMap::new(),
            })
            .unwrap_err();
        assert!(matches!(
            err,
            RegistryError::RemoteUrl(RemoteUrlError::Scheme)
        ));
    }

    #[tokio::test]
    async fn start_injects_secrets_and_exposes_public_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let connector = RecordingConnector::with_tools(vec![tool("echo"), tool("delete")]);
        let mut registry = ServerRegistry::open_sqlite(&path, connector.clone()).unwrap();
        let mut config = local_config("srv-1", "everything");
        config.tool_permissions.insert("delete".into(), false);
        registry.add(config).unwrap();

        let mut secrets = HashMap::new();
        secrets.insert("API_TOKEN".into(), "sk-secret".into());
        registry.start("srv-1", secrets).await.unwrap();

        let listed = registry.list().unwrap();
        assert_eq!(listed[0].status, ServerStatus::Running);
        let names: Vec<_> = registry
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["everything__echo"]);

        let seen = connector.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "srv-1");
        assert_eq!(seen[0].1.get("API_TOKEN"), Some(&"sk-secret".to_string()));
    }

    #[tokio::test]
    async fn stop_hides_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![tool("echo")]))
                .unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        registry.start("srv-1", HashMap::new()).await.unwrap();
        registry.stop("srv-1").await.unwrap();
        assert_eq!(registry.list().unwrap()[0].status, ServerStatus::Stopped);
        assert!(registry.list_tools().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn start_and_stop_notify_tool_list_subscribers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![tool("echo")]))
                .unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        let aggregator = registry.aggregator();
        let mut changes = aggregator.lock().await.subscribe_tool_list_changes();

        registry.start("srv-1", HashMap::new()).await.unwrap();
        changes.recv().await.unwrap();

        registry.stop("srv-1").await.unwrap();
        changes.recv().await.unwrap();
    }

    #[tokio::test]
    async fn auto_start_skips_disabled_and_manual_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let connector = RecordingConnector::with_tools(vec![tool("echo")]);
        let mut registry = ServerRegistry::open_sqlite(&path, connector.clone()).unwrap();

        let mut auto = local_config("auto", "auto");
        auto.env_keys.clear();
        registry.add(auto).unwrap();

        let mut disabled = local_config("disabled", "disabled");
        disabled.disabled = true;
        disabled.env_keys.clear();
        registry.add(disabled).unwrap();

        let mut manual = local_config("manual", "manual");
        manual.auto_start = false;
        manual.env_keys.clear();
        registry.add(manual).unwrap();

        registry.auto_start(HashMap::new()).await.unwrap();
        let by_id: HashMap<_, _> = registry
            .list()
            .unwrap()
            .into_iter()
            .map(|s| (s.config.id, s.status))
            .collect();
        assert_eq!(by_id.get("auto"), Some(&ServerStatus::Running));
        assert_eq!(by_id.get("disabled"), Some(&ServerStatus::Stopped));
        assert_eq!(by_id.get("manual"), Some(&ServerStatus::Stopped));
        assert_eq!(connector.seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn shared_aggregator_handle_lists_started_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![tool("echo")]))
                .unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        registry.start("srv-1", HashMap::new()).await.unwrap();
        let tools = registry
            .aggregator()
            .lock()
            .await
            .list_tools()
            .await
            .unwrap();
        assert_eq!(tools[0].name, "everything__echo");
    }

    #[tokio::test]
    async fn delete_stops_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![tool("echo")]))
                .unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        registry.start("srv-1", HashMap::new()).await.unwrap();
        assert!(registry.delete("srv-1").await.unwrap());
        assert!(registry.list().unwrap().is_empty());
        assert!(registry.list_tools().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn update_permissions_hides_running_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry = ServerRegistry::open_sqlite(
            &path,
            RecordingConnector::with_tools(vec![tool("echo"), tool("delete")]),
        )
        .unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        registry.start("srv-1", HashMap::new()).await.unwrap();
        let names: Vec<_> = registry
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["everything__echo", "everything__delete"]);

        let mut config = registry.list().unwrap()[0].config.clone();
        config.tool_permissions.insert("delete".into(), false);
        registry.update(config).unwrap();
        let names: Vec<_> = registry
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["everything__echo"]);
    }

    #[test]
    fn add_rejects_duplicate_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        let err = registry
            .add(local_config("srv-2", "everything"))
            .unwrap_err();
        assert!(matches!(err, RegistryError::DuplicateName(_)));
        assert_eq!(registry.list().unwrap().len(), 1);
    }

    #[test]
    fn add_rejects_blank_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        let err = registry.add(local_config("srv-1", "  ")).unwrap_err();
        assert!(matches!(err, RegistryError::InvalidConfig(_)));
    }

    #[test]
    fn add_rejects_delimiter_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        let err = registry.add(local_config("srv-1", "bad__name")).unwrap_err();
        assert!(matches!(err, RegistryError::InvalidConfig(_)));
    }

    #[test]
    fn update_allows_keeping_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        let config = registry.list().unwrap()[0].config.clone();
        registry.update(config).unwrap();
        assert_eq!(registry.list().unwrap()[0].config.name, "everything");
    }

    #[test]
    fn update_rejects_rename_onto_existing_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        registry.add(local_config("srv-2", "other")).unwrap();
        let mut config = registry.list().unwrap()[0].config.clone();
        config.name = "other".into();
        let err = registry.update(config).unwrap_err();
        assert!(matches!(err, RegistryError::DuplicateName(_)));
        assert_eq!(registry.list().unwrap()[0].config.name, "everything");
    }
}
