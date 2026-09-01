use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;

/// Schema served for tools whose upstream omits `inputSchema`; the MCP spec
/// requires every tool to carry an object schema.
pub fn empty_input_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default = "empty_input_schema")]
    pub input_schema: Value,
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icons: Option<Value>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// An upstream tool under its public `{server}__{tool}` name. Serializes as
/// exactly an MCP Tool wire object, so the inbound `tools/list` can forward it
/// verbatim; `source_server` is internal-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregatedTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default = "empty_input_schema")]
    pub input_schema: Value,
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icons: Option<Value>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(skip_serializing, default)]
    pub source_server: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AggregatorError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool is private: {0}")]
    PrivateTool(String),
    #[error("backend error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait McpBackend: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError>;
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, AggregatorError>;
}

pub struct RegisteredServer {
    pub id: String,
    pub name: String,
    pub running: bool,
    pub tool_permissions: HashMap<String, bool>,
    pub backend: Arc<dyn McpBackend>,
}

pub struct Aggregator {
    servers: Vec<RegisteredServer>,
    tool_list_changes: broadcast::Sender<()>,
}

impl Default for Aggregator {
    fn default() -> Self {
        let (tool_list_changes, _) = broadcast::channel(16);
        Self {
            servers: Vec::new(),
            tool_list_changes,
        }
    }
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_server(&mut self, server: RegisteredServer) {
        self.servers.push(server);
    }

    pub fn upsert_server(&mut self, server: RegisteredServer) {
        if let Some(existing) = self.servers.iter_mut().find(|s| s.id == server.id) {
            *existing = server;
        } else {
            self.servers.push(server);
        }
        let _ = self.tool_list_changes.send(());
    }

    pub fn set_running(&mut self, id: &str, running: bool) {
        if let Some(server) = self.servers.iter_mut().find(|s| s.id == id) {
            if server.running == running {
                return;
            }
            server.running = running;
            let _ = self.tool_list_changes.send(());
        }
    }

    pub fn set_tool_permissions(&mut self, id: &str, tool_permissions: HashMap<String, bool>) {
        if let Some(server) = self.servers.iter_mut().find(|s| s.id == id) {
            if server.tool_permissions == tool_permissions {
                return;
            }
            server.tool_permissions = tool_permissions;
            let _ = self.tool_list_changes.send(());
        }
    }

    pub fn subscribe_tool_list_changes(&self) -> broadcast::Receiver<()> {
        self.tool_list_changes.subscribe()
    }

    pub async fn origin_tools(&self, id: &str) -> Result<Vec<Tool>, AggregatorError> {
        let Some(server) = self.servers.iter().find(|s| s.id == id && s.running) else {
            return Ok(Vec::new());
        };
        server.backend.list_tools().await
    }

    pub async fn list_tools(&self) -> Result<Vec<AggregatedTool>, AggregatorError> {
        let mut tools = Vec::new();
        for server in &self.servers {
            if !server.running {
                continue;
            }
            for tool in server.backend.list_tools().await? {
                if !crate::permissions::is_tool_public(&server.tool_permissions, &tool.name) {
                    continue;
                }
                tools.push(AggregatedTool {
                    name: crate::naming::prefix_tool_name(&server.name, &tool.name),
                    title: tool.title,
                    description: tool.description,
                    input_schema: tool.input_schema,
                    output_schema: tool.output_schema,
                    annotations: tool.annotations,
                    icons: tool.icons,
                    meta: tool.meta,
                    source_server: server.name.clone(),
                });
            }
        }
        Ok(tools)
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, AggregatorError> {
        let Some((server_name, tool_name)) = crate::naming::strip_server_prefix(name) else {
            return Err(AggregatorError::UnknownTool(name.to_string()));
        };
        let Some(server) = self
            .servers
            .iter()
            .find(|s| s.running && s.name == server_name)
        else {
            return Err(AggregatorError::UnknownTool(name.to_string()));
        };
        if !crate::permissions::is_tool_public(&server.tool_permissions, tool_name) {
            return Err(AggregatorError::PrivateTool(name.to_string()));
        }
        let available = server.backend.list_tools().await?;
        if !available.iter().any(|t| t.name == tool_name) {
            return Err(AggregatorError::UnknownTool(name.to_string()));
        }
        server.backend.call_tool(tool_name, arguments).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::Mutex;

    struct FakeBackend {
        tools: Vec<Tool>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl FakeBackend {
        fn new(tools: Vec<Tool>) -> Arc<Self> {
            Arc::new(Self {
                tools,
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl McpBackend for FakeBackend {
        async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError> {
            Ok(self.tools.clone())
        }

        async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, AggregatorError> {
            self.calls
                .lock()
                .await
                .push((name.to_string(), arguments.clone()));
            Ok(json!({ "ok": true, "tool": name }))
        }
    }

    fn tool(name: &str) -> Tool {
        Tool {
            name: name.into(),
            title: None,
            description: Some(format!("{name} desc")),
            input_schema: json!({
                "type": "object",
                "properties": { "q": { "type": "string" } },
                "required": ["q"]
            }),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }

    #[tokio::test]
    async fn list_tools_prefixes_and_skips_stopped_servers() {
        let github = FakeBackend::new(vec![tool("list_issues")]);
        let slack = FakeBackend::new(vec![tool("post_message")]);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "github".into(),
            running: true,
            tool_permissions: HashMap::new(),
            backend: github,
        });
        agg.add_server(RegisteredServer {
            id: "2".into(),
            name: "slack".into(),
            running: false,
            tool_permissions: HashMap::new(),
            backend: slack,
        });

        let tools = agg.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "github__list_issues");
        assert_eq!(tools[0].source_server, "github");
    }

    #[tokio::test]
    async fn list_tools_preserves_input_schema() {
        let backend = FakeBackend::new(vec![tool("search")]);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: true,
            tool_permissions: HashMap::new(),
            backend,
        });

        let tools = agg.list_tools().await.unwrap();
        assert_eq!(
            tools[0].input_schema,
            json!({
                "type": "object",
                "properties": { "q": { "type": "string" } },
                "required": ["q"]
            })
        );
    }

    fn metadata_tool(name: &str) -> Tool {
        Tool {
            title: Some("Fetch Page".into()),
            output_schema: Some(json!({
                "type": "object",
                "properties": { "html": { "type": "string" } },
                "required": ["html"]
            })),
            annotations: Some(json!({ "readOnlyHint": true })),
            icons: Some(json!([{ "src": "https://example.com/i.png", "mimeType": "image/png" }])),
            meta: Some(json!({ "upstream": "tag" })),
            ..tool(name)
        }
    }

    #[tokio::test]
    async fn list_tools_preserves_tool_metadata() {
        let backend = FakeBackend::new(vec![metadata_tool("fetch")]);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: true,
            tool_permissions: HashMap::new(),
            backend,
        });

        let tools = agg.list_tools().await.unwrap();
        assert_eq!(tools[0].title.as_deref(), Some("Fetch Page"));
        assert_eq!(
            tools[0].output_schema.as_ref().unwrap()["required"][0],
            "html"
        );
        assert_eq!(tools[0].annotations.as_ref().unwrap()["readOnlyHint"], true);
        assert_eq!(
            tools[0].icons.as_ref().unwrap()[0]["src"],
            "https://example.com/i.png"
        );
        assert_eq!(tools[0].meta.as_ref().unwrap()["upstream"], "tag");
    }

    #[tokio::test]
    async fn aggregated_tool_serializes_as_mcp_wire_tool() {
        let backend = FakeBackend::new(vec![metadata_tool("fetch")]);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: true,
            tool_permissions: HashMap::new(),
            backend,
        });

        let tools = agg.list_tools().await.unwrap();
        let value = serde_json::to_value(&tools[0]).unwrap();
        assert_eq!(
            value,
            json!({
                "name": "docs__fetch",
                "title": "Fetch Page",
                "description": "fetch desc",
                "inputSchema": {
                    "type": "object",
                    "properties": { "q": { "type": "string" } },
                    "required": ["q"]
                },
                "outputSchema": {
                    "type": "object",
                    "properties": { "html": { "type": "string" } },
                    "required": ["html"]
                },
                "annotations": { "readOnlyHint": true },
                "icons": [{ "src": "https://example.com/i.png", "mimeType": "image/png" }],
                "_meta": { "upstream": "tag" }
            })
        );
    }

    #[test]
    fn tool_deserializes_without_optional_fields() {
        let parsed: Tool =
            serde_json::from_value(json!({ "name": "bare", "inputSchema": { "type": "object" } }))
                .unwrap();
        assert_eq!(parsed.name, "bare");
        assert_eq!(parsed.title, None);
        assert_eq!(parsed.description, None);
        assert_eq!(parsed.output_schema, None);
        assert_eq!(parsed.annotations, None);
        assert_eq!(parsed.icons, None);
        assert_eq!(parsed.meta, None);
    }

    #[tokio::test]
    async fn list_tools_omits_private_tools() {
        let backend = FakeBackend::new(vec![tool("search"), tool("delete")]);
        let mut perms = HashMap::new();
        perms.insert("delete".into(), false);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: true,
            tool_permissions: perms,
            backend,
        });

        let tools = agg.list_tools().await.unwrap();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["docs__search"]);
    }

    #[tokio::test]
    async fn call_tool_routes_to_origin_server() {
        let github = FakeBackend::new(vec![tool("list_issues")]);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "github".into(),
            running: true,
            tool_permissions: HashMap::new(),
            backend: github.clone(),
        });

        let result = agg
            .call_tool("github__list_issues", json!({ "repo": "mcp-manager" }))
            .await
            .unwrap();
        assert_eq!(result["tool"], "list_issues");
        let calls = github.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "list_issues");
    }

    #[tokio::test]
    async fn call_tool_rejects_private_and_unknown() {
        let backend = FakeBackend::new(vec![tool("search")]);
        let mut perms = HashMap::new();
        perms.insert("search".into(), false);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: true,
            tool_permissions: perms,
            backend,
        });

        assert_eq!(
            agg.call_tool("docs__search", json!({})).await.unwrap_err(),
            AggregatorError::PrivateTool("docs__search".into())
        );
        assert_eq!(
            agg.call_tool("docs__missing", json!({})).await.unwrap_err(),
            AggregatorError::UnknownTool("docs__missing".into())
        );
    }

    #[tokio::test]
    async fn running_state_changes_notify_tool_list_subscribers() {
        let backend = FakeBackend::new(vec![tool("search")]);
        let mut agg = Aggregator::new();
        agg.add_server(RegisteredServer {
            id: "1".into(),
            name: "docs".into(),
            running: false,
            tool_permissions: HashMap::new(),
            backend,
        });
        let mut changes = agg.subscribe_tool_list_changes();

        agg.set_running("1", true);
        changes.recv().await.unwrap();

        agg.set_running("1", false);
        changes.recv().await.unwrap();
    }
}
