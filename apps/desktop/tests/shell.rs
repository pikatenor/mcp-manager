use mcp_manager::shell::{
    on_close_requested, on_tray, parse_tray_menu_id, ShellAction, TrayCommand,
};

#[test]
fn tray_menu_ids_match_product_labels() {
    assert_eq!(parse_tray_menu_id("open"), Some(TrayCommand::Open));
    assert_eq!(
        parse_tray_menu_id("copy-endpoint"),
        Some(TrayCommand::CopyEndpoint)
    );
    assert_eq!(parse_tray_menu_id("quit"), Some(TrayCommand::Quit));
    assert_eq!(parse_tray_menu_id("nope"), None);
}

#[test]
fn tray_open_opens_or_focuses() {
    assert_eq!(on_tray(false, TrayCommand::Open), ShellAction::OpenWindow);
    assert_eq!(on_tray(true, TrayCommand::Open), ShellAction::FocusWindow);
}

#[test]
fn tray_copy_and_quit() {
    assert_eq!(
        on_tray(false, TrayCommand::CopyEndpoint),
        ShellAction::CopyEndpoint
    );
    assert_eq!(on_tray(true, TrayCommand::Quit), ShellAction::Exit);
}

#[test]
fn close_requested_hides_to_tray() {
    assert_eq!(on_close_requested(), ShellAction::CloseWindow);
}
