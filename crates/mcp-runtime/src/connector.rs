use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{BackendConnector, McpBackend, RegistryError, ServerConfig};

/// Connects to local stdio, remote Streamable HTTP, or legacy HTTP+SSE servers.
#[derive(Debug, Default, Clone, Copy)]
pub struct McpConnector;

#[async_trait]
impl BackendConnector for McpConnector {
    async fn connect(
        &self,
        _config: &ServerConfig,
        _secrets: &HashMap<String, String>,
    ) -> Result<Arc<dyn McpBackend>, RegistryError> {
        unimplemented!("McpConnector::connect")
    }
}
