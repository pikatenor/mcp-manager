//! Localhost Streamable HTTP endpoint for the aggregator.
//!
//! Inbound clients speak Streamable HTTP only. SSE is not served here.

mod auth;
mod server;

pub use auth::extract_bearer;
pub use mcp_core::{DEFAULT_HTTP_BIND, DEFAULT_MCP_PATH};
pub use server::{router, router_with_aggregator, serve, serve_with_listener};
