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
        return Err(RegistryError::InvalidConfig(
            "server name is required".into(),
        ));
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

fn validate_startable(config: &ServerConfig) -> Result<(), RegistryError> {
    validate_config(config)?;
    if config.server_type == ServerType::Local && config.command.is_none() {
        return Err(RegistryError::InvalidConfig(
            "command is required for local servers".into(),
        ));
    }
    Ok(())
}

/// Re-fetches one server's tool list into the cache. Never holds the
/// aggregator across the upstream call, and never touches the registry lock.
async fn refresh_cached_tools(aggregator: &Arc<AsyncMutex<Aggregator>>, id: &str) {
    let Some(backend) = aggregator.lock().await.running_backend(id) else {
        return;
    };
    // On failure keep the last known list; the next tick retries.
    if let Ok(tools) = backend.list_tools().await {
        aggregator.lock().await.set_cached_tools(id, tools);
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

    /// Persists `config` and pushes permission changes into the running
    /// aggregator. Awaited (not `try_lock`) so a change is never silently
    /// dropped while an inbound request holds the aggregator; safe because
    /// lock ordering is always registry → aggregator.
    pub async fn update(&mut self, config: ServerConfig) -> Result<ServerConfig, RegistryError> {
        validate_config(&config)?;
        self.ensure_unique_name(&config.name, Some(&config.id))?;
        let updated = self.store.update(config)?;
        self.aggregator
            .lock()
            .await
            .set_tool_permissions(&updated.id, updated.tool_permissions.clone());
        Ok(updated)
    }

    fn ensure_unique_name(
        &self,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<(), RegistryError> {
        let name = name.trim();
        if self
            .store
            .list()?
            .iter()
            .any(|config| Some(config.id.as_str()) != exclude_id && config.name.trim() == name)
        {
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

    pub fn connector(&self) -> Arc<dyn BackendConnector> {
        self.connector.clone()
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

    /// Validate and mark the server `Starting` without connecting.
    pub fn begin_start(&mut self, id: &str) -> Result<ServerConfig, RegistryError> {
        let config = self
            .store
            .get(id)?
            .ok_or_else(|| RegistryError::UnknownServer(id.to_string()))?;
        if let Err(err) = validate_startable(&config) {
            self.statuses.insert(config.id.clone(), ServerStatus::Error);
            self.errors.insert(config.id.clone(), err.to_string());
            return Err(err);
        }
        self.statuses
            .insert(config.id.clone(), ServerStatus::Starting);
        Ok(config)
    }

    /// Record the result of `connector.connect` as `Running` or `Error`.
    pub async fn finish_start(
        &mut self,
        config: &ServerConfig,
        result: Result<Arc<dyn McpBackend>, RegistryError>,
    ) -> Result<(), RegistryError> {
        match result {
            Ok(backend) => {
                self.aggregator
                    .lock()
                    .await
                    .upsert_server(RegisteredServer {
                        id: config.id.clone(),
                        name: config.name.clone(),
                        running: true,
                        tool_permissions: config.tool_permissions.clone(),
                        cached_tools: None,
                        backend: backend.clone(),
                    });
                // Spawned so start latency stays at connect cost; until the
                // first fetch lands the aggregator serves live lists.
                if let Some(mut watcher) = backend.tool_list_watcher() {
                    let aggregator = self.aggregator.clone();
                    let id = config.id.clone();
                    tokio::spawn(async move {
                        refresh_cached_tools(&aggregator, &id).await;
                        // The backend dropping on restart or stop ends this
                        // loop via a closed watch channel.
                        while watcher.changed().await.is_ok() {
                            refresh_cached_tools(&aggregator, &id).await;
                        }
                    });
                }
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

    pub async fn start(
        &mut self,
        id: &str,
        secrets: HashMap<String, String>,
    ) -> Result<(), RegistryError> {
        let config = self.begin_start(id)?;
        let result = self.connector.connect(&config, &secrets).await;
        self.finish_start(&config, result).await
    }

    /// Mark every `auto_start && !disabled` server `Starting` without connecting.
    pub fn begin_auto_start(&mut self) -> Result<Vec<String>, RegistryError> {
        let ids: Vec<String> = self
            .store
            .list()?
            .into_iter()
            .filter(|config| config.auto_start && !config.disabled)
            .map(|config| config.id)
            .collect();
        for id in &ids {
            self.statuses.insert(id.clone(), ServerStatus::Starting);
        }
        Ok(ids)
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
        // Snapshot under a short lock; the fan-out runs without it.
        let servers = self.aggregator.lock().await.listed_servers();
        Aggregator::resolve_listed_tools(servers).await
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

    /// Backend whose tool list mutates at runtime and that can signal a
    /// refresh, mimicking an upstream pushing `tools/list_changed`.
    struct MutableBackend {
        tools: Mutex<Vec<Tool>>,
        changes: tokio::sync::watch::Sender<u64>,
    }

    impl MutableBackend {
        fn new(tools: Vec<Tool>) -> Arc<Self> {
            let (changes, _) = tokio::sync::watch::channel(0);
            Arc::new(Self {
                tools: Mutex::new(tools),
                changes,
            })
        }

        fn set_tools(&self, tools: Vec<Tool>) {
            *self.tools.lock().unwrap() = tools;
        }

        /// Emulates what rmcp's notification handler does on a real
        /// `tools/list_changed` push.
        fn tick(&self) {
            self.changes.send_modify(|tick| *tick += 1);
        }
    }

    #[async_trait]
    impl McpBackend for MutableBackend {
        async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError> {
            Ok(self.tools.lock().unwrap().clone())
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> Result<serde_json::Value, AggregatorError> {
            Ok(serde_json::json!({ "ok": true }))
        }

        fn tool_list_watcher(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
            Some(self.changes.subscribe())
        }
    }

    struct MutableConnector(Arc<MutableBackend>);

    #[async_trait]
    impl BackendConnector for MutableConnector {
        async fn connect(
            &self,
            _config: &ServerConfig,
            _secrets: &HashMap<String, String>,
        ) -> Result<Arc<dyn McpBackend>, RegistryError> {
            Ok(self.0.clone())
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
            title: None,
            description: None,
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
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
    async fn tool_cache_serves_upstream_snapshot_until_refreshed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let backend = MutableBackend::new(vec![tool("echo")]);
        let mut registry =
            ServerRegistry::open_sqlite(&path, Arc::new(MutableConnector(backend.clone())))
                .unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        let aggregator = registry.aggregator();
        let mut changes = aggregator.lock().await.subscribe_tool_list_changes();

        registry.start("srv-1", HashMap::new()).await.unwrap();
        // First broadcast is the start's upsert, the second is the
        // refresher's initial fetch landing in the cache.
        changes.recv().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), changes.recv())
            .await
            .unwrap()
            .unwrap();

        backend.set_tools(vec![tool("echo"), tool("delete")]);
        // Until the upstream signals a change, the snapshot stays served.
        let names: Vec<_> = registry
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["everything__echo"]);

        backend.tick();
        tokio::time::timeout(std::time::Duration::from_secs(1), changes.recv())
            .await
            .unwrap()
            .unwrap();
        let names: Vec<_> = registry
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["everything__echo", "everything__delete"]);
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
        registry.update(config).await.unwrap();
        let names: Vec<_> = registry
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["everything__echo"]);
    }

    #[tokio::test]
    async fn update_applies_permissions_and_notifies_while_aggregator_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let registry = Arc::new(AsyncMutex::new(
            ServerRegistry::open_sqlite(
                &path,
                RecordingConnector::with_tools(vec![tool("echo"), tool("delete")]),
            )
            .unwrap(),
        ));
        let aggregator = registry.lock().await.aggregator();
        // Subscribe before start so the start broadcast can be drained below.
        let mut changes = aggregator.lock().await.subscribe_tool_list_changes();
        {
            let mut registry = registry.lock().await;
            registry.add(local_config("srv-1", "everything")).unwrap();
            registry.start("srv-1", HashMap::new()).await.unwrap();
        }
        // Drain the start broadcast so the only pending signal would be the
        // permission change itself.
        changes.recv().await.unwrap();

        // Hold the aggregator the way an in-flight inbound tools/list does;
        // the update must wait for it, not silently skip.
        let held = aggregator.lock().await;
        let update = tokio::spawn({
            let registry = registry.clone();
            async move {
                let mut config = registry.lock().await.list().unwrap()[0].config.clone();
                config.tool_permissions.insert("delete".into(), false);
                registry.lock().await.update(config).await.unwrap();
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        drop(held);
        update.await.unwrap();

        let names: Vec<_> = registry
            .lock()
            .await
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, vec!["everything__echo"]);
        assert!(changes.try_recv().is_ok());
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
        let err = registry
            .add(local_config("srv-1", "bad__name"))
            .unwrap_err();
        assert!(matches!(err, RegistryError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn update_allows_keeping_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        let config = registry.list().unwrap()[0].config.clone();
        registry.update(config).await.unwrap();
        assert_eq!(registry.list().unwrap()[0].config.name, "everything");
    }

    #[tokio::test]
    async fn update_rejects_rename_onto_existing_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        registry.add(local_config("srv-2", "other")).unwrap();
        let mut config = registry.list().unwrap()[0].config.clone();
        config.name = "other".into();
        let err = registry.update(config).await.unwrap_err();
        assert!(matches!(err, RegistryError::DuplicateName(_)));
        assert_eq!(registry.list().unwrap()[0].config.name, "everything");
    }

    struct GatedConnector {
        gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        tools: Vec<Tool>,
    }

    #[async_trait]
    impl BackendConnector for GatedConnector {
        async fn connect(
            &self,
            _config: &ServerConfig,
            _secrets: &HashMap<String, String>,
        ) -> Result<Arc<dyn McpBackend>, RegistryError> {
            let rx = self.gate.lock().unwrap().take();
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            Ok(Arc::new(StaticBackend {
                tools: self.tools.clone(),
            }))
        }
    }

    #[test]
    fn begin_start_marks_server_starting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();

        let config = registry.begin_start("srv-1").unwrap();
        assert_eq!(config.id, "srv-1");
        let listed = registry.list().unwrap();
        assert_eq!(listed[0].status, ServerStatus::Starting);
        assert!(listed[0].last_error.is_none());
    }

    #[tokio::test]
    async fn finish_start_records_backend_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        let config = registry.begin_start("srv-1").unwrap();

        let err = registry
            .finish_start(
                &config,
                Err(RegistryError::Backend("handshake failed".into())),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::Backend(_)));
        let listed = registry.list().unwrap();
        assert_eq!(listed[0].status, ServerStatus::Error);
        assert_eq!(
            listed[0].last_error.as_deref(),
            Some("backend error: handshake failed")
        );
    }

    #[test]
    fn begin_auto_start_marks_only_flagged_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();

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

        let ids = registry.begin_auto_start().unwrap();
        assert_eq!(ids, vec!["auto".to_string()]);
        let by_id: HashMap<_, _> = registry
            .list()
            .unwrap()
            .into_iter()
            .map(|s| (s.config.id, s.status))
            .collect();
        assert_eq!(by_id.get("auto"), Some(&ServerStatus::Starting));
        assert_eq!(by_id.get("disabled"), Some(&ServerStatus::Stopped));
        assert_eq!(by_id.get("manual"), Some(&ServerStatus::Stopped));
    }

    #[tokio::test]
    async fn list_observes_starting_while_connect_is_in_flight() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let (release, gate) = tokio::sync::oneshot::channel();
        let connector = Arc::new(GatedConnector {
            gate: Mutex::new(Some(gate)),
            tools: vec![tool("echo")],
        });
        let registry = Arc::new(AsyncMutex::new(
            ServerRegistry::open_sqlite(&path, connector).unwrap(),
        ));
        {
            let mut registry = registry.lock().await;
            registry.add(local_config("srv-1", "everything")).unwrap();
        }

        let (config, connector) = {
            let mut registry = registry.lock().await;
            let config = registry.begin_start("srv-1").unwrap();
            (config, registry.connector())
        };
        assert_eq!(
            registry.lock().await.list().unwrap()[0].status,
            ServerStatus::Starting
        );

        let connect = tokio::spawn({
            let config = config.clone();
            async move { connector.connect(&config, &HashMap::new()).await }
        });
        tokio::task::yield_now().await;
        assert_eq!(
            registry.lock().await.list().unwrap()[0].status,
            ServerStatus::Starting
        );

        release.send(()).unwrap();
        let backend = connect.await.unwrap().unwrap();
        registry
            .lock()
            .await
            .finish_start(&config, Ok(backend))
            .await
            .unwrap();
        assert_eq!(
            registry.lock().await.list().unwrap()[0].status,
            ServerStatus::Running
        );
    }

    #[tokio::test]
    async fn failed_finish_start_leaves_sibling_starting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut registry =
            ServerRegistry::open_sqlite(&path, RecordingConnector::with_tools(vec![])).unwrap();
        let mut first = local_config("first", "first");
        first.env_keys.clear();
        registry.add(first).unwrap();
        let mut second = local_config("second", "second");
        second.env_keys.clear();
        registry.add(second).unwrap();

        let ids = registry.begin_auto_start().unwrap();
        assert_eq!(ids.len(), 2);
        let first_config = registry
            .list()
            .unwrap()
            .into_iter()
            .find(|s| s.config.id == "first")
            .unwrap()
            .config;
        registry
            .finish_start(
                &first_config,
                Err(RegistryError::Backend("handshake failed".into())),
            )
            .await
            .unwrap_err();

        let by_id: HashMap<_, _> = registry
            .list()
            .unwrap()
            .into_iter()
            .map(|s| (s.config.id, s.status))
            .collect();
        assert_eq!(by_id.get("first"), Some(&ServerStatus::Error));
        assert_eq!(by_id.get("second"), Some(&ServerStatus::Starting));
    }
}
