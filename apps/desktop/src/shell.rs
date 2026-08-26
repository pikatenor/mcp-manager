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
    match id {
        "open" => Some(TrayCommand::Open),
        "copy-endpoint" => Some(TrayCommand::CopyEndpoint),
        "quit" => Some(TrayCommand::Quit),
        _ => None,
    }
}

pub fn on_tray(window_open: bool, command: TrayCommand) -> ShellAction {
    match command {
        TrayCommand::Open => {
            if window_open {
                ShellAction::FocusWindow
            } else {
                ShellAction::OpenWindow
            }
        }
        TrayCommand::CopyEndpoint => ShellAction::CopyEndpoint,
        TrayCommand::Quit => ShellAction::Exit,
    }
}

pub fn on_close_requested() -> ShellAction {
    ShellAction::CloseWindow
}
