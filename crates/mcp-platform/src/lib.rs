//! OS-dependent adapters.
//!
//! Core and HTTP crates depend only on these traits. macOS is implemented in
//! v1; Linux Secret Service / XDG paths can be added later without touching
//! aggregator logic.

mod app_paths;
mod browser;
mod keychain;
mod memory;
mod secrets;
mod shell_env;

pub use app_paths::{AppPaths, AppPathsError, NativeAppPaths};
pub use browser::{BrowserError, BrowserOpener, NativeBrowserOpener};
pub use keychain::KeychainSecretStore;
pub use memory::MemorySecretStore;
pub use secrets::{
    server_bearer_key, server_env_key, server_oauth_key, SecretStore, SecretStoreError,
};
pub use shell_env::fix_path_for_children;
