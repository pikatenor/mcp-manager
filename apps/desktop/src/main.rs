fn main() -> iced::Result {
    mcp_platform::fix_path_for_children();
    mcp_manager::run()
}
