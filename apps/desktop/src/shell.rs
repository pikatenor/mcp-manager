#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Open,
    CopyEndpoint,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellAction {
    OpenWindow,
    FocusWindow,
    CloseWindow,
    Exit,
    CopyEndpoint,
}

pub fn parse_tray_menu_id(id: &str) -> Option<TrayCommand> {
    let _ = id;
    unimplemented!("parse_tray_menu_id")
}

pub fn on_tray(window_open: bool, command: TrayCommand) -> ShellAction {
    let _ = (window_open, command);
    unimplemented!("on_tray")
}

pub fn on_close_requested() -> ShellAction {
    unimplemented!("on_close_requested")
}
