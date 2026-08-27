//! Parsing of Cursor/Claude Desktop style `mcpServers` JSON for import.
//!
//! The format is a JSON object mapping a server name to its entry:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "everything": {
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-everything"],
//!       "env": { "API_TOKEN": "sk-1" }
//!     },
//!     "atlas": { "url": "https://example.com/mcp" }
//!   }
//! }
//! ```
//!
//! The top-level `mcpServers` wrapper is optional; a bare name-to-entry map
//! is accepted as well. Invalid entries are collected as report lines and
//! never abort the rest of the import.

use std::collections::HashMap;

use super::servers::ServerType;

/// One server entry parsed from an `mcpServers` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedServer {
    pub name: String,
    pub server_type: ServerType,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub remote_url: Option<String>,
    pub bearer: Option<String>,
}

/// Result of parsing an import document: valid servers plus per-entry errors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedImport {
    pub servers: Vec<ImportedServer>,
    pub errors: Vec<String>,
}

/// Parse a Cursor/Claude Desktop style MCP config document.
pub fn parse_mcp_servers(raw: &str) -> ParsedImport {
    let mut parsed = ParsedImport::default();
    let root: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(error) => {
            parsed.errors.push(format!("invalid JSON: {error}"));
            return parsed;
        }
    };
    let Some(entries) = root
        .get("mcpServers")
        .and_then(|value| value.as_object())
        .or_else(|| root.as_object())
    else {
        parsed
            .errors
            .push("invalid JSON: expected an object of server entries".into());
        return parsed;
    };
    for (name, entry) in entries {
        let name = name.trim();
        if name.is_empty() {
            parsed.errors.push("name is required".into());
            continue;
        }
        match parse_entry(entry) {
            Ok(mut server) => {
                server.name = name.to_string();
                parsed.servers.push(server);
            }
            Err(reason) => parsed.errors.push(format!("{name}: {reason}")),
        }
    }
    parsed
}

fn parse_entry(entry: &serde_json::Value) -> Result<ImportedServer, String> {
    if !entry.is_object() {
        return Err("entry must be an object".into());
    }
    let command = match entry.get("command") {
        Some(value) => Some(
            value
                .as_str()
                .map(str::trim)
                .filter(|command| !command.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "command must be a non-empty string".to_string())?,
        ),
        None => None,
    };
    let url = match entry.get("url") {
        Some(value) => Some(
            value
                .as_str()
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "url must be a non-empty string".to_string())?,
        ),
        None => None,
    };
    let (server_type, remote_url) = match (&command, &url) {
        // A command wins: the entry is a local stdio server and url is ignored.
        (Some(_), _) => (ServerType::Local, None),
        (None, Some(url)) => (
            if url.ends_with("/sse") {
                ServerType::Remote
            } else {
                ServerType::RemoteStreamable
            },
            Some(url.clone()),
        ),
        (None, None) => return Err("entry needs a command or a url".into()),
    };

    let mut args = Vec::new();
    if let Some(value) = entry.get("args") {
        let items = value
            .as_array()
            .ok_or_else(|| "args must be an array of strings".to_string())?;
        for item in items {
            let item = item
                .as_str()
                .ok_or_else(|| "args must be an array of strings".to_string())?;
            args.push(item.to_string());
        }
    }

    let mut env = HashMap::new();
    if let Some(value) = entry.get("env") {
        let map = value
            .as_object()
            .ok_or_else(|| "env must be an object of strings".to_string())?;
        for (key, value) in map {
            let value = value
                .as_str()
                .ok_or_else(|| format!("env value for {key} must be a string"))?;
            env.insert(key.clone(), value.to_string());
        }
    }

    let mut bearer = None;
    if let Some(value) = entry.get("headers") {
        let headers = value
            .as_object()
            .ok_or_else(|| "headers must be an object".to_string())?;
        if let Some(authorization) = headers.get("Authorization") {
            let authorization = authorization
                .as_str()
                .ok_or_else(|| "Authorization header must be a string".to_string())?;
            let token = authorization.strip_prefix("Bearer ").ok_or(
                "only Bearer Authorization headers are supported",
            )?;
            bearer = Some(token.to_string());
        }
    }

    Ok(ImportedServer {
        name: String::new(),
        server_type,
        command,
        args,
        env,
        remote_url,
        bearer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cursor_local_entry() {
        let parsed = parse_mcp_servers(
            r#"{
                "mcpServers": {
                    "everything": {
                        "command": "npx",
                        "args": ["-y", "@modelcontextprotocol/server-everything"],
                        "env": { "API_TOKEN": "sk-1" }
                    }
                }
            }"#,
        );
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.servers.len(), 1);
        let server = &parsed.servers[0];
        assert_eq!(server.name, "everything");
        assert_eq!(server.server_type, ServerType::Local);
        assert_eq!(server.command.as_deref(), Some("npx"));
        assert_eq!(
            server.args,
            vec!["-y", "@modelcontextprotocol/server-everything"]
        );
        assert_eq!(
            server.env.get("API_TOKEN").map(String::as_str),
            Some("sk-1")
        );
        assert_eq!(server.remote_url, None);
        assert_eq!(server.bearer, None);
    }

    #[test]
    fn parses_bare_server_map_without_wrapper() {
        let parsed = parse_mcp_servers(
            r#"{ "everything": { "command": "npx", "args": [] } }"#,
        );
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].name, "everything");
        assert_eq!(parsed.servers[0].server_type, ServerType::Local);
    }

    #[test]
    fn url_entry_defaults_to_remote_streamable() {
        let parsed =
            parse_mcp_servers(r#"{ "mcpServers": { "atlas": { "url": "https://example.com/mcp" } } }"#);
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.servers[0].server_type, ServerType::RemoteStreamable);
        assert_eq!(
            parsed.servers[0].remote_url.as_deref(),
            Some("https://example.com/mcp")
        );
    }

    #[test]
    fn sse_url_suffix_maps_to_legacy_remote() {
        let parsed =
            parse_mcp_servers(r#"{ "mcpServers": { "atlas": { "url": "https://example.com/sse" } } }"#);
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.servers[0].server_type, ServerType::Remote);
    }

    #[test]
    fn strips_bearer_prefix_from_authorization_header() {
        let parsed = parse_mcp_servers(
            r#"{
                "mcpServers": {
                    "atlas": {
                        "url": "https://example.com/mcp",
                        "headers": { "Authorization": "Bearer tok-1" }
                    }
                }
            }"#,
        );
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.servers[0].bearer.as_deref(), Some("tok-1"));
    }

    #[test]
    fn non_bearer_authorization_is_invalid() {
        let parsed = parse_mcp_servers(
            r#"{
                "mcpServers": {
                    "atlas": {
                        "url": "https://example.com/mcp",
                        "headers": { "Authorization": "Basic abc" }
                    }
                }
            }"#,
        );
        assert!(parsed.servers.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].starts_with("atlas: "));
    }

    #[test]
    fn entry_without_command_or_url_is_reported() {
        let parsed = parse_mcp_servers(
            r#"{ "mcpServers": { "foo": {}, "bar": { "command": "npx" } } }"#,
        );
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].name, "bar");
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].starts_with("foo: "));
    }

    #[test]
    fn invalid_json_yields_error_only() {
        let parsed = parse_mcp_servers("not json at all");
        assert!(parsed.servers.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].to_lowercase().contains("json"));
    }

    #[test]
    fn non_string_env_value_is_reported() {
        let parsed = parse_mcp_servers(
            r#"{ "mcpServers": { "foo": { "command": "npx", "env": { "N": 1 } } } }"#,
        );
        assert!(parsed.servers.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].starts_with("foo: "));
    }

    #[test]
    fn blank_name_is_reported() {
        let parsed = parse_mcp_servers(r#"{ "mcpServers": { " ": { "command": "npx" } } }"#);
        assert!(parsed.servers.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].starts_with("name"));
    }
}
