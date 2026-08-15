use std::path::PathBuf;

/// Locations for SQLite and other non-secret state.
pub trait AppPaths: Send + Sync {
    fn data_dir(&self) -> PathBuf;
}

#[derive(Debug, thiserror::Error)]
pub enum AppPathsError {
    #[error("data directory is unavailable: {0}")]
    Unavailable(String),
}

/// macOS: `~/Library/Application Support/mcp-manager`
/// Linux (later): `~/.local/share/mcp-manager`
pub struct NativeAppPaths;

impl AppPaths for NativeAppPaths {
    fn data_dir(&self) -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mcp-manager")
    }
}
