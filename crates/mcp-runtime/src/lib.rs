//! Upstream MCP backends used by the desktop registry.

mod connector;
mod oauth;

pub use connector::McpConnector;
pub use oauth::{KeychainCredentialStore, OAuthError, OAuthFlow};
