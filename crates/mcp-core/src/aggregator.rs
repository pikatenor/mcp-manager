use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedTool {
    pub name: String,
    pub description: Option<String>,
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

#[derive(Default)]
pub struct Aggregator {
    servers: Vec<RegisteredServer>,
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_server(&mut self, server: RegisteredServer) {
        unimplemented!("Aggregator::add_server")
    }

    pub async fn list_tools(&self) -> Result<Vec<AggregatedTool>, AggregatorError> {
        unimplemented!("Aggregator::list_tools")
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, AggregatorError> {
        unimplemented!("Aggregator::call_tool")
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
            description: Some(format!("{name} desc")),
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
}
