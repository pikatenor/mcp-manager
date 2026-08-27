//! OS-free MCP aggregator core.
//!
//! This crate must not contain `#[cfg(target_os = ...)]`. Platform I/O belongs
//! in `mcp-platform`.

pub mod aggregator;
pub mod mcp_json;
pub mod naming;
pub mod permissions;
pub mod registry;
pub mod remote_url;
pub mod servers;
pub mod sse;
pub mod store;
pub mod token;

pub use aggregator::{
    AggregatedTool, Aggregator, AggregatorError, McpBackend, RegisteredServer, Tool,
};
pub use mcp_json::{parse_mcp_servers, ImportedServer, ParsedImport};
pub use naming::{prefix_tool_name, strip_server_prefix, TOOL_DELIMITER};
pub use permissions::is_tool_public;
pub use registry::{BackendConnector, RegistryError, ServerRegistry, ServerState, ServerStatus};
pub use remote_url::{validate_remote_url, RemoteUrlError};
pub use servers::{ServerConfig, ServerStore, ServerType};
pub use sse::{
    parse_sse_endpoint_event, parse_sse_message_events, SseClientTransport, SseTransportError,
};
pub use store::StoreError;
pub use token::{IssuedToken, TokenError, TokenRecord, TokenService};

/// Default bind address for the aggregator HTTP endpoint.
pub const DEFAULT_HTTP_BIND: &str = "127.0.0.1:8757";

/// Default public MCP path.
pub const DEFAULT_MCP_PATH: &str = "/mcp";
