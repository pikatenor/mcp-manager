use std::sync::{Arc, Mutex};

use mcp_core::{IssuedToken, TokenRecord, TokenService};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, State};

struct AppTokens(Arc<Mutex<TokenService>>);

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            aggregator_endpoint,
            issue_token,
            list_tokens,
            revoke_token
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let tokens = TokenService::open_sqlite(&data_dir.join("tokens.db"))?;
            let tokens = Arc::new(Mutex::new(tokens));
            app.manage(AppTokens(tokens.clone()));

            tauri::async_runtime::spawn(async move {
                if let Err(error) = mcp_http::serve(tokens).await {
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
                    "copy-endpoint" => {}
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
