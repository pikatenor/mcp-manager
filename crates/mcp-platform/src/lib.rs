//! OS-dependent adapters.
//!
//! Core and HTTP crates depend only on these traits. macOS is implemented in
//! v1; Linux Secret Service / XDG paths can be added later without touching
//! aggregator logic.

mod app_paths;
mod browser;
mod secrets;

pub use app_paths::{AppPaths, AppPathsError};
pub use browser::{BrowserError, BrowserOpener};
pub use secrets::{
    server_bearer_key, server_env_key, server_oauth_key, SecretStore, SecretStoreError,
};
