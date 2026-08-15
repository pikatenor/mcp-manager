use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::aggregator::{AggregatedTool, McpBackend};
use super::servers::ServerConfig;
use super::store::StoreError;

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
    _private: (),
}

impl ServerRegistry {
    pub fn open_sqlite(
        path: &Path,
        _connector: Arc<dyn BackendConnector>,
    ) -> Result<Self, RegistryError> {
        unimplemented!("ServerRegistry::open_sqlite")
    }

    pub fn add(&mut self, _config: ServerConfig) -> Result<ServerConfig, RegistryError> {
        unimplemented!("ServerRegistry::add")
    }

    pub fn list(&self) -> Result<Vec<ServerState>, RegistryError> {
        unimplemented!("ServerRegistry::list")
    }

    pub async fn start(
        &mut self,
        _id: &str,
        _secrets: HashMap<String, String>,
    ) -> Result<(), RegistryError> {
        unimplemented!("ServerRegistry::start")
    }

    pub async fn stop(&mut self, _id: &str) -> Result<(), RegistryError> {
        unimplemented!("ServerRegistry::stop")
    }

    pub async fn auto_start(
        &mut self,
        _secrets: HashMap<String, HashMap<String, String>>,
    ) -> Result<(), RegistryError> {
        unimplemented!("ServerRegistry::auto_start")
    }

    pub async fn list_tools(&self) -> Result<Vec<AggregatedTool>, super::aggregator::AggregatorError> {
        unimplemented!("ServerRegistry::list_tools")
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
        config
            .tool_permissions
            .insert("delete".into(), false);
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
        let mut registry = ServerRegistry::open_sqlite(
            &path,
            RecordingConnector::with_tools(vec![tool("echo")]),
        )
        .unwrap();
        registry.add(local_config("srv-1", "everything")).unwrap();
        registry.start("srv-1", HashMap::new()).await.unwrap();
        registry.stop("srv-1").await.unwrap();
        assert_eq!(registry.list().unwrap()[0].status, ServerStatus::Stopped);
        assert!(registry.list_tools().await.unwrap().is_empty());
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
}
