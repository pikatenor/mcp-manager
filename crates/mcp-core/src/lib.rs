//! OS-free MCP aggregator core.
//!
//! This crate must not contain `#[cfg(target_os = ...)]`. Platform I/O belongs
//! in `mcp-platform`.

/// Delimiter used when prefixing aggregated tool names: `{server}__{tool}`.
pub const TOOL_DELIMITER: &str = "__";

/// Default bind address for the aggregator HTTP endpoint.
pub const DEFAULT_HTTP_BIND: &str = "127.0.0.1:8757";

/// Default public MCP path.
pub const DEFAULT_MCP_PATH: &str = "/mcp";
