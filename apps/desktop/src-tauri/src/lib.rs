use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use mcp_core::{
    is_tool_public, IssuedToken, ServerConfig, ServerRegistry, ServerState, ServerType,
    TokenRecord, TokenService,
};
use mcp_platform::{
    server_bearer_key, server_env_key, server_oauth_key, NativeBrowserOpener, SecretStore,
    SecretStoreError,
};
#[cfg(target_os = "macos")]
use mcp_platform::KeychainSecretStore;
#[cfg(not(target_os = "macos"))]
use mcp_platform::MemorySecretStore;
use mcp_runtime::{McpConnector, OAuthFlow, ReqwestOAuthHttp};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, State};
use tokio::sync::Mutex as AsyncMutex;

struct AppTokens(Arc<Mutex<TokenService>>);
struct AppRegistry(Arc<AsyncMutex<ServerRegistry>>);
struct AppSecrets(Arc<dyn SecretStore>);

#[derive(Debug, Serialize)]
struct ServerToolView {
    name: String,
    public: bool,
}

#[derive(Debug, Deserialize)]
struct AddServerRequest {
    name: String,
    server_type: ServerType,
    command: Option<String>,
    args: Vec<String>,
    env: HashMap<String, String>,
    remote_url: Option<String>,
    auto_start: bool,
    bearer: Option<String>,
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
}

#[tauri::command]
fn aggregator_endpoint() -> String {
    format!(
        "http://{}{}",
        mcp_core::DEFAULT_HTTP_BIND,
        mcp_core::DEFAULT_MCP_PATH
    )
}

#[tauri::command]
fn issue_token(tokens: State<AppTokens>, client_name: String) -> Result<IssuedToken, String> {
    let name = client_name.trim();
    if name.is_empty() {
        return Err("client name is required".into());
    }
    tokens
        .0
        .lock()
        .map_err(|e| e.to_string())
        .map(|mut svc| svc.issue(name))
}

#[tauri::command]
fn list_tokens(tokens: State<AppTokens>) -> Result<Vec<TokenRecord>, String> {
    tokens
        .0
        .lock()
        .map_err(|e| e.to_string())
        .map(|svc| svc.list())
}

#[tauri::command]
fn revoke_token(tokens: State<AppTokens>, id: String) -> Result<(), String> {
    tokens
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .revoke(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_servers(registry: State<'_, AppRegistry>) -> Result<Vec<ServerState>, String> {
    registry.0.lock().await.list().map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_server(
    registry: State<'_, AppRegistry>,
    secrets: State<'_, AppSecrets>,
    request: AddServerRequest,
) -> Result<ServerConfig, String> {
    let name = request.name.trim();
    if name.is_empty() {
        return Err("server name is required".into());
    }
    let id = new_server_id();
    let env_keys: Vec<String> = request.env.keys().cloned().collect();
    persist_secrets(secrets.0.as_ref(), &id, &request.env, request.bearer.as_deref())
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
    registry
        .0
        .lock()
        .await
        .add(config)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_server(
    registry: State<'_, AppRegistry>,
    secrets: State<'_, AppSecrets>,
    id: String,
) -> Result<bool, String> {
    let mut registry = registry.0.lock().await;
    let config = registry
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|state| state.config.id == id)
        .map(|state| state.config);
    if let Some(config) = config {
        delete_secrets(secrets.0.as_ref(), &config);
    }
    registry.delete(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_server(
    registry: State<'_, AppRegistry>,
    secrets: State<'_, AppSecrets>,
    id: String,
) -> Result<(), String> {
    let mut registry = registry.0.lock().await;
    let config = registry
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|state| state.config.id == id)
        .ok_or_else(|| format!("unknown server: {id}"))?
        .config;
    let server_secrets = load_secrets(secrets.0.as_ref(), &config).map_err(|e| e.to_string())?;
    registry
        .start(&id, server_secrets)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_server(registry: State<'_, AppRegistry>, id: String) -> Result<(), String> {
    registry
        .0
        .lock()
        .await
        .stop(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_server_tools(
    registry: State<'_, AppRegistry>,
    id: String,
) -> Result<Vec<ServerToolView>, String> {
    let registry = registry.0.lock().await;
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
        .origin_tools(&id)
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

#[tauri::command]
async fn set_tool_permission(
    registry: State<'_, AppRegistry>,
    id: String,
    tool_name: String,
    public: bool,
) -> Result<(), String> {
    let mut registry = registry.0.lock().await;
    let mut config = registry
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|state| state.config.id == id)
        .ok_or_else(|| format!("unknown server: {id}"))?
        .config;
    config.tool_permissions.insert(tool_name, public);
    registry.update(config).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn oauth_connect(
    registry: State<'_, AppRegistry>,
    secrets: State<'_, AppSecrets>,
    id: String,
) -> Result<(), String> {
    let remote_url = {
        let registry = registry.0.lock().await;
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
    let flow = OAuthFlow::new(
        Arc::new(ReqwestOAuthHttp::new().map_err(|e| e.to_string())?),
        Arc::new(NativeBrowserOpener),
        secrets.0.clone(),
    );
    flow.authorize(&id, &remote_url)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn oauth_connected(secrets: State<AppSecrets>, id: String) -> Result<bool, String> {
    Ok(secrets
        .0
        .get(&server_oauth_key(&id, "access_token"))
        .map_err(|e| e.to_string())?
        .filter(|value| !value.is_empty())
        .is_some())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // GUI launches get launchd's minimal PATH; stdio servers need the user's
    // login-shell PATH (npx, uvx, asdf shims). Must run before any spawns.
    mcp_platform::fix_path_for_children();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            aggregator_endpoint,
            issue_token,
            list_tokens,
            revoke_token,
            list_servers,
            add_server,
            delete_server,
            start_server,
            stop_server,
            list_server_tools,
            set_tool_permission,
            oauth_connect,
            oauth_connected
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let tokens = TokenService::open_sqlite(&data_dir.join("tokens.db"))?;
            let tokens = Arc::new(Mutex::new(tokens));
            let registry = ServerRegistry::open_sqlite(
                &data_dir.join("state.db"),
                Arc::new(McpConnector),
            )?;
            let aggregator = registry.aggregator();
            let registry = Arc::new(AsyncMutex::new(registry));
            #[cfg(target_os = "macos")]
            let secrets: Arc<dyn SecretStore> = Arc::new(KeychainSecretStore::new());
            // Linux Secret Service wiring is deferred; MemorySecretStore keeps
            // the Ubuntu CI compile/test path working until then.
            #[cfg(not(target_os = "macos"))]
            let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
            app.manage(AppTokens(tokens.clone()));
            app.manage(AppRegistry(registry.clone()));
            app.manage(AppSecrets(secrets.clone()));

            tauri::async_runtime::spawn({
                let registry = registry.clone();
                let secrets = secrets.clone();
                async move {
                    let mut registry = registry.lock().await;
                    let mut all_secrets = HashMap::new();
                    if let Ok(listed) = registry.list() {
                        for state in listed {
                            match load_secrets(secrets.as_ref(), &state.config) {
                                Ok(server_secrets) => {
                                    all_secrets.insert(state.config.id.clone(), server_secrets);
                                }
                                Err(error) => {
                                    eprintln!("secret load failed for {}: {error}", state.config.id)
                                }
                            }
                        }
                    }
                    if let Err(error) = registry.auto_start(all_secrets).await {
                        eprintln!("auto-start failed: {error}");
                    }
                }
            });

            tauri::async_runtime::spawn(async move {
                if let Err(error) = mcp_http::serve_with_aggregator(tokens, aggregator).await {
                    eprintln!("mcp http server failed: {error}");
                }
            });

            let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
            let copy =
                MenuItem::with_id(app, "copy-endpoint", "Copy endpoint", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &copy, &quit])?;

            let icon = app
                .default_window_icon()
                .cloned()
                .expect("app icon is required for the tray");

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    "copy-endpoint" => {
                        let endpoint = format!(
                            "http://{}{}",
                            mcp_core::DEFAULT_HTTP_BIND,
                            mcp_core::DEFAULT_MCP_PATH
                        );
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(endpoint);
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
