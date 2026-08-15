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
