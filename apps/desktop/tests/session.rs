use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{
    AggregatorError, BackendConnector, McpBackend, RegistryError, ServerConfig, ServerStatus,
    ServerType, Tool,
};
use mcp_manager::session::{parse_env, AddServerRequest, ImportOutcome, Session};
use mcp_platform::{
    server_bearer_key, server_env_key, server_oauth_client_id_key,
    server_oauth_client_secret_key, server_oauth_key, BrowserError, BrowserOpener,
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
        oauth_client_id: None,
        oauth_client_secret: None,
    }
}

fn remote_add(name: &str) -> AddServerRequest {
    AddServerRequest {
        name: name.into(),
        server_type: ServerType::RemoteStreamable,
        command: None,
        args: vec![],
        env: HashMap::new(),
        remote_url: Some("https://example.com/mcp".into()),
        auto_start: true,
        bearer: None,
        oauth_client_id: None,
        oauth_client_secret: None,
    }
}

#[derive(Default)]
struct SpyConnector {
    seen: std::sync::Mutex<Vec<(String, HashMap<String, String>)>>,
}

#[async_trait]
impl BackendConnector for SpyConnector {
    async fn connect(
        &self,
        config: &ServerConfig,
        secrets: &HashMap<String, String>,
    ) -> Result<Arc<dyn McpBackend>, RegistryError> {
        self.seen
            .lock()
            .unwrap()
            .push((config.id.clone(), secrets.clone()));
        Ok(Arc::new(StaticBackend { tools: vec![] }))
    }
}

struct ToggleConnector {
    fail: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl BackendConnector for ToggleConnector {
    async fn connect(
        &self,
        _config: &ServerConfig,
        _secrets: &HashMap<String, String>,
    ) -> Result<Arc<dyn McpBackend>, RegistryError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            Err(RegistryError::Backend("connect failed".into()))
        } else {
            Ok(Arc::new(StaticBackend { tools: vec![] }))
        }
    }
}

fn open_session(
    connector: Arc<dyn BackendConnector>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let session = Session::open(dir.path(), connector, secrets.clone(), Arc::new(NoopBrowser))
        .unwrap();
    Fixture {
        session,
        secrets,
        _dir: dir,
    }
}

fn spy_session() -> (Fixture, Arc<SpyConnector>) {
    let spy = Arc::new(SpyConnector::default());
    (open_session(spy.clone()), spy)
}

fn toggle_session() -> (Fixture, Arc<ToggleConnector>) {
    let toggle = Arc::new(ToggleConnector {
        fail: std::sync::atomic::AtomicBool::new(false),
    });
    (open_session(toggle.clone()), toggle)
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
async fn add_server_stores_oauth_client_credentials() {
    let fx = session(vec![]);
    let mut request = remote_add("remote");
    request.oauth_client_id = Some("cid-static".into());
    request.oauth_client_secret = Some("sec-static".into());
    let config = fx.session.add_server(request).await.unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_id_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("cid-static")
    );
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_secret_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("sec-static")
    );
}

#[tokio::test]
async fn add_server_rejects_client_secret_without_client_id() {
    let fx = session(vec![]);
    let mut request = remote_add("remote");
    request.oauth_client_secret = Some("sec-orphan".into());
    let err = fx.session.add_server(request).await.unwrap_err();
    assert!(err.contains("client id"));
    assert!(fx.session.list_servers().await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_server_removes_secrets() {
    let fx = session(vec![]);
    let mut request = local_add("everything");
    request.env.insert("API_TOKEN".into(), "sk-secret".into());
    request.bearer = Some("tok-1".into());
    request.oauth_client_id = Some("cid-static".into());
    request.oauth_client_secret = Some("sec-static".into());
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
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_id_key(&config.id))
            .unwrap(),
        None
    );
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_secret_key(&config.id))
            .unwrap(),
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

#[tokio::test]
async fn add_server_rejects_duplicate_name() {
    let fx = session(vec![]);
    fx.session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    let err = fx
        .session
        .add_server(local_add("everything"))
        .await
        .unwrap_err();
    assert!(err.contains("already exists"));
    assert_eq!(fx.session.list_servers().await.unwrap().len(), 1);
}

#[tokio::test]
async fn update_server_rewrites_config_and_preserves_permissions() {
    let fx = session(vec![tool("echo"), tool("delete")]);
    let config = fx
        .session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    fx.session
        .set_tool_permission(&config.id, "delete", false)
        .await
        .unwrap();

    let mut request = local_add("renamed");
    request.command = Some("docker".into());
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();

    let listed = fx.session.list_servers().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].config.id, config.id);
    assert_eq!(listed[0].config.name, "renamed");
    assert_eq!(listed[0].config.command.as_deref(), Some("docker"));
    assert_eq!(
        listed[0].config.tool_permissions.get("delete"),
        Some(&false)
    );
}

#[tokio::test]
async fn update_server_replaces_env_secrets() {
    let fx = session(vec![]);
    let mut request = local_add("everything");
    request.env.insert("A".into(), "1".into());
    request.env.insert("B".into(), "2".into());
    let config = fx.session.add_server(request).await.unwrap();

    let mut request = local_add("everything");
    request.env.insert("A".into(), "3".into());
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();

    assert_eq!(
        fx.secrets
            .get(&server_env_key(&config.id, "A"))
            .unwrap()
            .as_deref(),
        Some("3")
    );
    assert_eq!(
        fx.secrets.get(&server_env_key(&config.id, "B")).unwrap(),
        None
    );
    let listed = fx.session.list_servers().await.unwrap();
    assert_eq!(listed[0].config.env_keys, vec!["A".to_string()]);
}

#[tokio::test]
async fn update_server_keeps_bearer_when_blank_and_overwrites_when_set() {
    let fx = session(vec![]);
    let mut request = remote_add("remote");
    request.bearer = Some("tok-1".into());
    let config = fx.session.add_server(request).await.unwrap();

    let request = remote_add("remote");
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_bearer_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("tok-1")
    );

    let mut request = remote_add("remote");
    request.bearer = Some("tok-2".into());
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_bearer_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("tok-2")
    );
}

#[tokio::test]
async fn update_server_overwrites_and_clears_oauth_client_config() {
    let fx = session(vec![]);
    let mut request = remote_add("remote");
    request.oauth_client_id = Some("cid-1".into());
    request.oauth_client_secret = Some("sec-1".into());
    let config = fx.session.add_server(request).await.unwrap();

    // The secret is never echoed back into the form, so a blank secret on
    // update means "keep the stored one" — even when the id changes.
    let mut request = remote_add("remote");
    request.oauth_client_id = Some("cid-2".into());
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_id_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("cid-2")
    );
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_secret_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("sec-1")
    );

    // A blank client id clears the whole static client config.
    let request = remote_add("remote");
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_id_key(&config.id))
            .unwrap(),
        None
    );
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_secret_key(&config.id))
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn update_server_invalidates_oauth_tokens_when_client_config_changes() {
    let fx = session(vec![]);
    let mut request = remote_add("remote");
    request.oauth_client_id = Some("cid-1".into());
    let config = fx.session.add_server(request).await.unwrap();
    let seed_tokens = |secrets: &MemorySecretStore, id: &str| {
        for (key, value) in [
            (server_oauth_key(id, "credentials"), "blob".to_string()),
            (server_bearer_key(id), "at-1".to_string()),
            (server_oauth_key(id, "access_token"), "at-1".to_string()),
            (server_oauth_key(id, "refresh_token"), "rt-1".to_string()),
        ] {
            secrets.set(&key, &value).unwrap();
        }
    };
    seed_tokens(fx.secrets.as_ref(), &config.id);

    // Tokens minted under one client identity cannot refresh under another,
    // so a config change drops the guaranteed-broken token state.
    let mut request = remote_add("remote");
    request.oauth_client_id = Some("cid-1".into());
    request.oauth_client_secret = Some("sec-rotated".into());
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_secret_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("sec-rotated")
    );
    for key in [
        server_oauth_key(&config.id, "credentials"),
        server_bearer_key(&config.id),
        server_oauth_key(&config.id, "access_token"),
        server_oauth_key(&config.id, "refresh_token"),
    ] {
        assert_eq!(fx.secrets.get(&key).unwrap(), None, "{key}");
    }

    // A byte-identical save (id + re-typed secret) is not a change and keeps
    // the token state.
    seed_tokens(fx.secrets.as_ref(), &config.id);
    let mut request = remote_add("remote");
    request.oauth_client_id = Some("cid-1".into());
    request.oauth_client_secret = Some("sec-rotated".into());
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();
    assert_eq!(
        fx.secrets
            .get(&server_oauth_key(&config.id, "credentials"))
            .unwrap()
            .as_deref(),
        Some("blob")
    );
}

#[tokio::test]
async fn update_server_rejects_client_secret_without_client_id_and_keeps_config() {
    let fx = session(vec![]);
    let mut request = remote_add("remote");
    request.oauth_client_id = Some("cid-1".into());
    request.oauth_client_secret = Some("sec-1".into());
    let config = fx.session.add_server(request).await.unwrap();

    let mut request = remote_add("remote");
    request.oauth_client_secret = Some("sec-orphan".into());
    let err = fx
        .session
        .update_server(&config.id, request)
        .await
        .unwrap_err();
    assert!(err.contains("client id"));
    // The rejected update must not touch the stored client identity.
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_id_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("cid-1")
    );
    assert_eq!(
        fx.secrets
            .get(&server_oauth_client_secret_key(&config.id))
            .unwrap()
            .as_deref(),
        Some("sec-1")
    );
}

#[tokio::test]
async fn update_server_restarts_running_server() {
    let (fx, spy) = spy_session();
    let mut request = local_add("everything");
    request.env.insert("API_TOKEN".into(), "sk-1".into());
    let config = fx.session.add_server(request).await.unwrap();
    fx.session.start_server(&config.id).await.unwrap();

    let mut request = local_add("everything");
    request.env.insert("API_TOKEN".into(), "sk-2".into());
    fx.session
        .update_server(&config.id, request)
        .await
        .unwrap();

    assert_eq!(
        fx.session.list_servers().await.unwrap()[0].status,
        ServerStatus::Running
    );
    let seen = spy.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[1].1.get("API_TOKEN").map(String::as_str),
        Some("sk-2")
    );
}

#[tokio::test]
async fn update_server_reports_unknown_id() {
    let fx = session(vec![]);
    let err = fx
        .session
        .update_server("srv-none", local_add("x"))
        .await
        .unwrap_err();
    assert!(err.contains("unknown server"));
}

#[tokio::test]
async fn update_server_saves_when_restart_fails() {
    let (fx, toggle) = toggle_session();
    let config = fx
        .session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    fx.session.start_server(&config.id).await.unwrap();
    toggle
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);

    fx.session
        .update_server(&config.id, local_add("renamed"))
        .await
        .unwrap();

    let listed = fx.session.list_servers().await.unwrap();
    assert_eq!(listed[0].config.name, "renamed");
    assert_eq!(listed[0].status, ServerStatus::Error);
    assert!(listed[0].last_error.is_some());
}

#[tokio::test]
async fn update_server_rejects_rename_onto_existing() {
    let fx = session(vec![]);
    fx.session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    let other = fx.session.add_server(local_add("other")).await.unwrap();

    let err = fx
        .session
        .update_server(&other.id, local_add("everything"))
        .await
        .unwrap_err();
    assert!(err.contains("already exists"));
    let listed = fx.session.list_servers().await.unwrap();
    assert_eq!(listed[1].config.name, "other");
}

#[tokio::test]
async fn server_secret_values_returns_env_bearer_and_oauth_client_id() {
    let fx = session(vec![]);
    let mut remote = remote_add("remote");
    remote.env.insert("A".into(), "1".into());
    remote.bearer = Some("tok-1".into());
    remote.oauth_client_id = Some("cid-static".into());
    remote.oauth_client_secret = Some("sec-static".into());
    let config = fx.session.add_server(remote).await.unwrap();
    let values = fx.session.server_secret_values(&config.id).await.unwrap();
    assert_eq!(values.env.get("A").map(String::as_str), Some("1"));
    assert_eq!(values.bearer.as_deref(), Some("tok-1"));
    // The id prefills the edit form; the secret never does.
    assert_eq!(values.oauth_client_id.as_deref(), Some("cid-static"));

    let mut local = local_add("everything");
    local.env.insert("B".into(), "2".into());
    let config = fx.session.add_server(local).await.unwrap();
    let values = fx.session.server_secret_values(&config.id).await.unwrap();
    assert_eq!(values.env.get("B").map(String::as_str), Some("2"));
    assert_eq!(values.bearer, None);
    assert_eq!(values.oauth_client_id, None);
}

#[tokio::test]
async fn import_json_adds_local_and_remote_servers() {
    let fx = session(vec![]);
    let outcome = fx
        .session
        .import_json(
            r#"{
                "mcpServers": {
                    "everything": {
                        "command": "npx",
                        "args": ["-y", "server-everything"],
                        "env": { "API_TOKEN": "sk-1" }
                    },
                    "atlas": {
                        "url": "https://example.com/mcp",
                        "headers": { "Authorization": "Bearer tok-1" }
                    }
                }
            }"#,
        )
        .await
        .unwrap();
    assert!(outcome.skipped.is_empty());
    assert!(outcome.invalid.is_empty());
    assert_eq!(outcome.added.len(), 2);
    assert!(outcome.added.contains(&"everything".to_string()));
    assert!(outcome.added.contains(&"atlas".to_string()));

    let listed = fx.session.list_servers().await.unwrap();
    assert_eq!(listed.len(), 2);
    let by_name: HashMap<_, _> = listed
        .into_iter()
        .map(|state| (state.config.name.clone(), state.config))
        .collect();
    let everything = by_name.get("everything").unwrap();
    assert_eq!(everything.command.as_deref(), Some("npx"));
    assert!(!everything.auto_start);
    assert_eq!(
        fx.secrets
            .get(&server_env_key(&everything.id, "API_TOKEN"))
            .unwrap()
            .as_deref(),
        Some("sk-1")
    );
    let atlas = by_name.get("atlas").unwrap();
    assert_eq!(atlas.server_type, ServerType::RemoteStreamable);
    assert_eq!(
        fx.secrets
            .get(&server_bearer_key(&atlas.id))
            .unwrap()
            .as_deref(),
        Some("tok-1")
    );
}

#[tokio::test]
async fn import_json_skips_existing_names() {
    let fx = session(vec![]);
    fx.session
        .add_server(local_add("everything"))
        .await
        .unwrap();
    let outcome = fx
        .session
        .import_json(
            r#"{ "mcpServers": {
                "everything": { "command": "npx" },
                "atlas": { "url": "https://example.com/mcp" }
            } }"#,
        )
        .await
        .unwrap();
    assert_eq!(outcome.skipped, vec!["everything".to_string()]);
    assert_eq!(outcome.added, vec!["atlas".to_string()]);
    assert!(outcome.invalid.is_empty());
    assert_eq!(fx.session.list_servers().await.unwrap().len(), 2);
}

#[tokio::test]
async fn import_json_reports_invalid_entries_without_aborting() {
    let fx = session(vec![]);
    let outcome = fx
        .session
        .import_json(r#"{ "mcpServers": { "good": { "command": "npx" }, "bad": {} } }"#)
        .await
        .unwrap();
    assert_eq!(outcome.added, vec!["good".to_string()]);
    assert_eq!(outcome.invalid.len(), 1);
    assert!(outcome.invalid[0].starts_with("bad: "));
    assert_eq!(fx.session.list_servers().await.unwrap().len(), 1);
}

#[tokio::test]
async fn import_json_invalid_document_adds_nothing() {
    let fx = session(vec![]);
    let outcome = fx.session.import_json("not json").await.unwrap();
    assert!(outcome.added.is_empty());
    assert!(outcome.skipped.is_empty());
    assert_eq!(outcome.invalid.len(), 1);
    assert!(fx.session.list_servers().await.unwrap().is_empty());
}

#[test]
fn import_outcome_summary_lists_counts() {
    let outcome = ImportOutcome {
        added: vec!["a".into()],
        skipped: vec!["b".into(), "c".into()],
        invalid: vec!["d: bad".into()],
    };
    let summary = outcome.summary();
    assert!(summary.contains("imported 1 (a)"));
    assert!(summary.contains("skipped 2 (already exist: b, c)"));
    assert!(summary.contains("1 invalid (d: bad)"));

    let empty = ImportOutcome::default();
    assert!(empty.summary().contains("No servers"));
}
