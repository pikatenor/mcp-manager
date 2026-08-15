use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use mcp_core::{
    validate_remote_url, AggregatorError, BackendConnector, McpBackend, RegistryError,
    ServerConfig, ServerType, SseClientTransport, Tool,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Connects to local stdio, remote Streamable HTTP, or legacy HTTP+SSE servers.
#[derive(Debug, Default, Clone, Copy)]
pub struct McpConnector;

struct JsonRpcBackend {
    session: Mutex<Session>,
}

enum Session {
    Http {
        client: reqwest::Client,
        url: String,
        bearer: Option<String>,
        next_id: i64,
    },
    Stdio {
        _child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
        next_id: i64,
    },
}

fn bearer_from(secrets: &HashMap<String, String>) -> Option<String> {
    secrets
        .get("BEARER_TOKEN")
        .cloned()
        .filter(|value| !value.is_empty())
}

fn http_client() -> Result<reqwest::Client, RegistryError> {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|err| RegistryError::Backend(err.to_string()))
}

async fn initialize(session: &mut Session) -> Result<(), RegistryError> {
    rpc(
        session,
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "mcp-manager", "version": env!("CARGO_PKG_VERSION") }
        }),
    )
    .await?;
    notify(session, "notifications/initialized", json!({})).await
}

async fn notify(session: &mut Session, method: &str, params: Value) -> Result<(), RegistryError> {
    let message = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });
    match session {
        Session::Http {
            client,
            url,
            bearer,
            ..
        } => {
            let mut request = client.post(url.as_str()).json(&message);
            if let Some(token) = bearer.as_ref() {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            let _ = request.send().await;
            Ok(())
        }
        Session::Stdio { stdin, .. } => {
            let mut line = serde_json::to_vec(&message)
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            line.push(b'\n');
            stdin
                .write_all(&line)
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            stdin
                .flush()
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            Ok(())
        }
    }
}

async fn rpc(session: &mut Session, method: &str, params: Value) -> Result<Value, RegistryError> {
    let id = match session {
        Session::Http { next_id, .. } | Session::Stdio { next_id, .. } => {
            *next_id += 1;
            *next_id
        }
    };
    let message = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    let response: Value = match session {
        Session::Http {
            client,
            url,
            bearer,
            ..
        } => {
            let mut request = client.post(url.as_str()).json(&message);
            if let Some(token) = bearer.as_ref() {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            let response = request
                .send()
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            if !status.is_success() {
                return Err(RegistryError::Backend(format!("{status}: {body}")));
            }
            serde_json::from_str(&body).map_err(|err| RegistryError::Backend(err.to_string()))?
        }
        Session::Stdio {
            stdin, stdout, ..
        } => {
            let mut line =
                serde_json::to_vec(&message).map_err(|err| RegistryError::Backend(err.to_string()))?;
            line.push(b'\n');
            stdin
                .write_all(&line)
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            stdin
                .flush()
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            let mut reply = String::new();
            stdout
                .read_line(&mut reply)
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            serde_json::from_str(reply.trim())
                .map_err(|err| RegistryError::Backend(err.to_string()))?
        }
    };
    if let Some(error) = response.get("error") {
        return Err(RegistryError::Backend(error.to_string()));
    }
    Ok(response
        .get("result")
        .cloned()
        .unwrap_or(Value::Null))
}

fn tools_from_result(result: Value) -> Vec<Tool> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            Some(Tool {
                name: tool.get("name")?.as_str()?.to_string(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

#[async_trait]
impl McpBackend for JsonRpcBackend {
    async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError> {
        let mut session = self.session.lock().await;
        let result = rpc(&mut session, "tools/list", json!({}))
            .await
            .map_err(|err| AggregatorError::Backend(err.to_string()))?;
        Ok(tools_from_result(result))
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, AggregatorError> {
        let mut session = self.session.lock().await;
        rpc(
            &mut session,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
        .map_err(|err| AggregatorError::Backend(err.to_string()))
    }
}

async fn connect_http(
    url: &str,
    secrets: &HashMap<String, String>,
) -> Result<JsonRpcBackend, RegistryError> {
    validate_remote_url(url)?;
    let client = http_client()?;
    let mut session = Session::Http {
        client,
        url: url.to_string(),
        bearer: bearer_from(secrets),
        next_id: 0,
    };
    initialize(&mut session).await?;
    Ok(JsonRpcBackend {
        session: Mutex::new(session),
    })
}

async fn connect_sse(
    sse_url: &str,
    secrets: &HashMap<String, String>,
) -> Result<JsonRpcBackend, RegistryError> {
    validate_remote_url(sse_url)?;
    let mut headers = HashMap::new();
    if let Some(token) = bearer_from(secrets) {
        headers.insert("Authorization".into(), format!("Bearer {token}"));
    }
    let transport = SseClientTransport::connect(sse_url, headers)
        .await
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    connect_http(&transport.endpoint_url, secrets).await
}

async fn connect_stdio(
    config: &ServerConfig,
    secrets: &HashMap<String, String>,
) -> Result<JsonRpcBackend, RegistryError> {
    let command = config.command.as_deref().ok_or_else(|| {
        RegistryError::InvalidConfig("command is required for local servers".into())
    })?;
    let mut child = Command::new(command);
    child
        .args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    for key in &config.env_keys {
        if let Some(value) = secrets.get(key) {
            child.env(key, value);
        }
    }
    let mut child = child
        .spawn()
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| RegistryError::Backend("missing stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RegistryError::Backend("missing stdout".into()))?;
    let mut session = Session::Stdio {
        _child: child,
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 0,
    };
    initialize(&mut session).await?;
    Ok(JsonRpcBackend {
        session: Mutex::new(session),
    })
}

#[async_trait]
impl BackendConnector for McpConnector {
    async fn connect(
        &self,
        config: &ServerConfig,
        secrets: &HashMap<String, String>,
    ) -> Result<Arc<dyn McpBackend>, RegistryError> {
        let backend = match config.server_type {
            ServerType::Local => connect_stdio(config, secrets).await?,
            ServerType::RemoteStreamable => {
                let url = config.remote_url.as_deref().ok_or_else(|| {
                    RegistryError::InvalidConfig("remote_url is required".into())
                })?;
                connect_http(url, secrets).await?
            }
            ServerType::Remote => {
                let url = config.remote_url.as_deref().ok_or_else(|| {
                    RegistryError::InvalidConfig("remote_url is required".into())
                })?;
                connect_sse(url, secrets).await?
            }
        };
        Ok(Arc::new(backend))
    }
}
