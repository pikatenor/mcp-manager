use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{
    AggregatorError, BackendConnector, McpBackend, RegistryError, ServerConfig, ServerStatus,
    ServerType, Tool,
};
use mcp_manager::session::{parse_env, AddServerRequest, Session};
use mcp_platform::{
    server_bearer_key, server_env_key, server_oauth_key, BrowserError, BrowserOpener,
    MemorySecretStore, SecretStore,
};

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

struct RecordingConnector {
    tools: Vec<Tool>,
}

#[async_trait]
impl BackendConnector for RecordingConnector {
    async fn connect(
        &self,
        _config: &ServerConfig,
        _secrets: &HashMap<String, String>,
    ) -> Result<Arc<dyn McpBackend>, RegistryError> {
        Ok(Arc::new(StaticBackend {
            tools: self.tools.clone(),
        }))
    }
}

struct NoopBrowser;

impl BrowserOpener for NoopBrowser {
    fn open_url(&self, _url: &str) -> Result<(), BrowserError> {
        Ok(())
    }
}

struct Fixture {
    session: Session,
    secrets: Arc<MemorySecretStore>,
    _dir: tempfile::TempDir,
}

fn session(tools: Vec<Tool>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let session = Session::open(
        dir.path(),
        Arc::new(RecordingConnector { tools }),
        secrets.clone(),
        Arc::new(NoopBrowser),
    )
    .unwrap();
    Fixture {
        session,
        secrets,
        _dir: dir,
    }
}

fn tool(name: &str) -> Tool {
    Tool {
        name: name.into(),
        description: None,
    }
}

fn local_add(name: &str) -> AddServerRequest {
    AddServerRequest {
        name: name.into(),
        server_type: ServerType::Local,
        command: Some("npx".into()),
        args: vec!["-y".into(), "mcp-server".into()],
        env: HashMap::new(),
        remote_url: None,
        auto_start: true,
        bearer: None,
    }
}

#[test]
fn aggregator_endpoint_is_localhost_streamable_http() {
    assert_eq!(Session::aggregator_endpoint(), "http://127.0.0.1:8757/mcp");
}

#[test]
fn parse_env_splits_lines_on_first_equals() {
    let env = parse_env("API_TOKEN=secret\n\nPATH=/bin=/usr\n=skip\nNOEQ\n");
    assert_eq!(env.get("API_TOKEN"), Some(&"secret".to_string()));
    assert_eq!(env.get("PATH"), Some(&"/bin=/usr".to_string()));
    assert!(!env.contains_key("NOEQ"));
    assert_eq!(env.len(), 2);
}

#[test]
fn issue_token_requires_client_name() {
    let fx = session(vec![]);
    let err = fx.session.issue_token("  ").unwrap_err();
    assert!(err.contains("client name"));
}

#[test]
fn issue_token_returns_plaintext_once_and_lists_without_it() {
    let fx = session(vec![]);
    let issued = fx.session.issue_token("cursor").unwrap();
    assert!(issued.plaintext.starts_with("mcpm_"));
    let listed = fx.session.list_tokens().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, issued.id);
    assert_eq!(listed[0].client_name, "cursor");
    let encoded = serde_json::to_string(&listed[0]).unwrap();
    assert!(!encoded.contains(&issued.plaintext));
}

#[test]
fn revoke_token_marks_record() {
    let fx = session(vec![]);
    let issued = fx.session.issue_token("cursor").unwrap();
    fx.session.revoke_token(&issued.id).unwrap();
    let listed = fx.session.list_tokens().unwrap();
    assert!(listed[0].revoked_at.is_some());
}

#[tokio::test]
async fn add_server_requires_name() {
    let fx = session(vec![]);
    let err = fx.session.add_server(local_add("  ")).await.unwrap_err();
    assert!(err.contains("server name"));
}

#[tokio::test]
async fn add_server_keeps_env_values_in_secret_store() {
    let fx = session(vec![]);
    let mut request = local_add("everything");
    request.env.insert("API_TOKEN".into(), "sk-secret".into());
    let config = fx.session.add_server(request).await.unwrap();
    assert_eq!(config.env_keys, vec!["API_TOKEN".to_string()]);
    assert_eq!(
        fx.secrets
            .get(&server_env_key(&config.id, "API_TOKEN"))
            .unwrap()
            .as_deref(),
        Some("sk-secret")
    );
    let listed = fx.session.list_servers().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].config.name, "everything");
    assert_eq!(listed[0].status, ServerStatus::Stopped);
}

#[tokio::test]
async fn add_server_stores_optional_bearer() {
    let fx = session(vec![]);
    let mut request = local_add("remote-ish");
    request.server_type = ServerType::RemoteStreamable;
    request.command = None;
    request.args = vec![];
    request.remote_url = Some("https://example.com/mcp".into());
    request.bearer = Some("tok-1".into());
    let config = fx.session.add_server(request).await.unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_bearer_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("tok-1")
    );
}

#[tokio::test]
async fn delete_server_removes_secrets() {
    let fx = session(vec![]);
    let mut request = local_add("everything");
    request.env.insert("API_TOKEN".into(), "sk-secret".into());
    request.bearer = Some("tok-1".into());
    let config = fx.session.add_server(request).await.unwrap();
    assert!(fx.session.delete_server(&config.id).await.unwrap());
    assert_eq!(
        fx.secrets
            .get(&server_env_key(&config.id, "API_TOKEN"))
            .unwrap(),
        None
    );
    assert_eq!(
        fx.secrets.get(&server_bearer_key(&config.id)).unwrap(),
        None
    );
    assert!(fx.session.list_servers().await.unwrap().is_empty());
}

#[tokio::test]
async fn start_and_stop_server() {
    let fx = session(vec![tool("echo")]);
    let config = fx
        .session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    fx.session.start_server(&config.id).await.unwrap();
    assert_eq!(
        fx.session.list_servers().await.unwrap()[0].status,
        ServerStatus::Running
    );
    fx.session.stop_server(&config.id).await.unwrap();
    assert_eq!(
        fx.session.list_servers().await.unwrap()[0].status,
        ServerStatus::Stopped
    );
}

#[tokio::test]
async fn list_server_tools_defaults_public() {
    let fx = session(vec![tool("echo"), tool("delete")]);
    let config = fx
        .session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    fx.session.start_server(&config.id).await.unwrap();
    let tools = fx.session.list_server_tools(&config.id).await.unwrap();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| tool.public));
}

#[tokio::test]
async fn set_tool_permission_hides_tool() {
    let fx = session(vec![tool("echo"), tool("delete")]);
    let config = fx
        .session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    fx.session.start_server(&config.id).await.unwrap();
    fx.session
        .set_tool_permission(&config.id, "delete", false)
        .await
        .unwrap();
    let tools = fx.session.list_server_tools(&config.id).await.unwrap();
    let delete = tools.iter().find(|tool| tool.name == "delete").unwrap();
    assert!(!delete.public);
}

#[tokio::test]
async fn oauth_connect_rejects_local_servers() {
    let fx = session(vec![]);
    let config = fx
        .session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    let err = fx.session.oauth_connect(&config.id).await.unwrap_err();
    assert!(err.contains("OAuth"));
}

#[tokio::test]
async fn oauth_connected_reads_access_token() {
    let fx = session(vec![]);
    let mut request = local_add("remote");
    request.server_type = ServerType::RemoteStreamable;
    request.command = None;
    request.args = vec![];
    request.remote_url = Some("https://example.com/mcp".into());
    let config = fx.session.add_server(request).await.unwrap();
    assert!(!fx.session.oauth_connected(&config.id).unwrap());
    fx.secrets
        .set(&server_oauth_key(&config.id, "access_token"), "at")
        .unwrap();
    assert!(fx.session.oauth_connected(&config.id).unwrap());
}

#[tokio::test]
async fn auto_start_starts_flagged_servers() {
    let fx = session(vec![tool("echo")]);
    let mut auto = local_add("auto");
    auto.auto_start = true;
    let mut manual = local_add("manual");
    manual.auto_start = false;
    fx.session.add_server(auto).await.unwrap();
    fx.session.add_server(manual).await.unwrap();
    fx.session.auto_start().await.unwrap();
    let listed = fx.session.list_servers().await.unwrap();
    let by_name: HashMap<_, _> = listed
        .into_iter()
        .map(|state| (state.config.name, state.status))
        .collect();
    assert_eq!(by_name.get("auto"), Some(&ServerStatus::Running));
    assert_eq!(by_name.get("manual"), Some(&ServerStatus::Stopped));
}
