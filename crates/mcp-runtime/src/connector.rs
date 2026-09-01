use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use http::{HeaderName, HeaderValue};
use mcp_core::{
    empty_input_schema, parse_sse_message_events, validate_remote_url, AggregatorError,
    BackendConnector, McpBackend, RegistryError, ServerConfig, ServerType, SseClientTransport,
    Tool,
};
use mcp_platform::SecretStore;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::auth::{AuthError, AuthorizationManager, OAuthClientConfig};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpClientTransportConfig, StreamableHttpError,
    StreamableHttpPostResponse,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{serve_client, ClientHandler};
use serde_json::{json, Value};
use sse_stream::Sse;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::oauth::{pre_registered_client, KeychainCredentialStore};

/// Connects to local stdio, remote Streamable HTTP, or legacy HTTP+SSE servers.
/// stdio and Streamable HTTP ride on rmcp; the deprecated HTTP+SSE transport
/// (2024-11-05) keeps the hand-rolled JSON-RPC client below because rmcp no
/// longer ships a client for it.
#[derive(Clone, Default)]
pub struct McpConnector {
    /// Secret store for OAuth credential persistence. When absent, remote
    /// servers fall back to the static bearer from the secrets map.
    secrets: Option<Arc<dyn SecretStore>>,
}

impl McpConnector {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            secrets: Some(secrets),
        }
    }
}

/// Presents the aggregator's identity during the MCP handshake.
struct AggregatorClient;

impl ClientHandler for AggregatorClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("mcp-manager", env!("CARGO_PKG_VERSION")),
        )
    }
}

/// rmcp-backed backend for stdio and Streamable HTTP upstreams.
struct RmcpBackend {
    service: RunningService<RoleClient, AggregatorClient>,
}

#[async_trait]
impl McpBackend for RmcpBackend {
    async fn list_tools(&self) -> Result<Vec<Tool>, AggregatorError> {
        let tools = self
            .service
            .peer()
            .list_all_tools()
            .await
            .map_err(|err| AggregatorError::Backend(err.to_string()))?;
        Ok(tools
            .into_iter()
            .map(|tool| Tool {
                name: tool.name.to_string(),
                title: tool.title.clone(),
                description: tool.description.map(|description| description.to_string()),
                input_schema: Value::Object(tool.input_schema.as_ref().clone()),
                output_schema: tool
                    .output_schema
                    .as_ref()
                    .map(|schema| Value::Object(schema.as_ref().clone())),
                annotations: tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| serde_json::to_value(annotations).ok()),
                icons: tool
                    .icons
                    .as_ref()
                    .and_then(|icons| serde_json::to_value(icons).ok()),
                meta: tool.meta.as_ref().map(|meta| Value::Object(meta.0.clone())),
            })
            .collect())
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, AggregatorError> {
        // `CallToolRequestParams` is non-exhaustive, so build via Default.
        let mut params = CallToolRequestParams::default();
        params.name = name.to_owned().into();
        params.arguments = arguments.as_object().cloned();
        let result = self
            .service
            .peer()
            .call_tool(params)
            .await
            .map_err(|err| AggregatorError::Backend(err.to_string()))?;
        // Keep the raw wire shape (`content`, `isError`, ...) like the
        // hand-rolled client did; the aggregator forwards it verbatim.
        serde_json::to_value(&result).map_err(|err| AggregatorError::Backend(err.to_string()))
    }
}

async fn connect_rmcp_stdio(
    config: &ServerConfig,
    secrets: &HashMap<String, String>,
) -> Result<RmcpBackend, RegistryError> {
    let command = config.command.as_deref().ok_or_else(|| {
        RegistryError::InvalidConfig("command is required for local servers".into())
    })?;
    let mut child = Command::new(command);
    child.args(&config.args);
    for key in &config.env_keys {
        if let Some(value) = secrets.get(key) {
            child.env(key, value);
        }
    }
    let (transport, _stderr) = TokioChildProcess::builder(child)
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    let service = serve_client(AggregatorClient, transport)
        .await
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    Ok(RmcpBackend { service })
}

async fn connect_rmcp_http(
    url: &str,
    secrets: &HashMap<String, String>,
    store: Option<&Arc<dyn SecretStore>>,
    server_id: &str,
) -> Result<RmcpBackend, RegistryError> {
    validate_remote_url(url)?;
    // Refresh-aware OAuth credentials take precedence over a static bearer.
    if let Some(store) = store {
        if let Some(manager) = oauth_manager(url, store.clone(), server_id).await? {
            let transport = StreamableHttpClientTransport::with_client(
                RefreshingHttpClient::new(manager)?,
                StreamableHttpClientTransportConfig::with_uri(url.to_string()),
            );
            let service = serve_client(AggregatorClient, transport)
                .await
                .map_err(|err| RegistryError::Backend(err.to_string()))?;
            return Ok(RmcpBackend { service });
        }
    }
    let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
    if let Some(token) = bearer_from(secrets) {
        // rmcp applies the Bearer scheme itself via `bearer_auth`.
        transport_config = transport_config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::from_config(transport_config);
    let service = serve_client(AggregatorClient, transport)
        .await
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    Ok(RmcpBackend { service })
}

/// Build an `AuthorizationManager` over stored OAuth credentials. Returns
/// `None` when the server has none (static bearer applies instead).
async fn oauth_manager(
    url: &str,
    store: Arc<dyn SecretStore>,
    server_id: &str,
) -> Result<Option<Arc<AuthorizationManager>>, RegistryError> {
    let mut manager = AuthorizationManager::new(url.to_string())
        .await
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    manager.set_credential_store(KeychainCredentialStore::new(store.clone(), server_id));
    let initialized = manager
        .initialize_from_store()
        .await
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    // `initialize_from_store` restores only the client id (as a public
    // client). Re-arm a stored secret so confidential refreshes carry client
    // auth; the redirect URI is irrelevant on this path and mirrors rmcp's
    // own placeholder (the base URL). Id-only clients skip this: the restore
    // is already equivalent.
    if initialized {
        if let Ok(Some(client)) = pre_registered_client(store.as_ref(), server_id) {
            if let Some(secret) = client.client_secret {
                manager
                    .configure_client(
                        OAuthClientConfig::new(client.client_id, url.to_string())
                            .with_client_secret(secret),
                    )
                    .map_err(|err| RegistryError::Backend(err.to_string()))?;
            }
        }
    }
    Ok(initialized.then(|| Arc::new(manager)))
}

#[derive(Debug, thiserror::Error)]
enum AuthHttpError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// `StreamableHttpError` is non-exhaustive, so unknown future variants are
/// stringified rather than dropped.
fn map_http_error(
    error: StreamableHttpError<reqwest::Error>,
) -> StreamableHttpError<AuthHttpError> {
    match error {
        StreamableHttpError::Client(err) => StreamableHttpError::Client(err.into()),
        StreamableHttpError::Sse(err) => StreamableHttpError::Sse(err),
        StreamableHttpError::Io(err) => StreamableHttpError::Io(err),
        StreamableHttpError::UnexpectedEndOfStream => StreamableHttpError::UnexpectedEndOfStream,
        StreamableHttpError::UnexpectedServerResponse(message) => {
            StreamableHttpError::UnexpectedServerResponse(message)
        }
        StreamableHttpError::UnexpectedContentType(content_type) => {
            StreamableHttpError::UnexpectedContentType(content_type)
        }
        StreamableHttpError::ServerDoesNotSupportSse => {
            StreamableHttpError::ServerDoesNotSupportSse
        }
        StreamableHttpError::ServerDoesNotSupportDeleteSession => {
            StreamableHttpError::ServerDoesNotSupportDeleteSession
        }
        StreamableHttpError::TokioJoinError(err) => StreamableHttpError::TokioJoinError(err),
        StreamableHttpError::Deserialize(err) => StreamableHttpError::Deserialize(err),
        StreamableHttpError::TransportChannelClosed => StreamableHttpError::TransportChannelClosed,
        StreamableHttpError::MissingSessionIdInResponse => {
            StreamableHttpError::MissingSessionIdInResponse
        }
        StreamableHttpError::Auth(err) => StreamableHttpError::Auth(err),
        StreamableHttpError::AuthRequired(err) => StreamableHttpError::AuthRequired(err),
        StreamableHttpError::InsufficientScope(err) => StreamableHttpError::InsufficientScope(err),
        StreamableHttpError::ReservedHeaderConflict(name) => {
            StreamableHttpError::ReservedHeaderConflict(name)
        }
        StreamableHttpError::SessionExpired => StreamableHttpError::SessionExpired,
        other => StreamableHttpError::UnexpectedServerResponse(std::borrow::Cow::Owned(
            other.to_string(),
        )),
    }
}

fn is_auth_rejection<E>(error: &StreamableHttpError<E>) -> bool
where
    E: std::error::Error + Send + Sync + 'static,
{
    matches!(error, StreamableHttpError::AuthRequired(_))
        || matches!(
            error,
            StreamableHttpError::UnexpectedServerResponse(message) if message.contains("401")
        )
}

/// Streamable HTTP backend that pulls the bearer from the OAuth manager on
/// every request (refreshing near expiry) and retries once through a forced
/// refresh when the server rejects the token with a 401 challenge.
#[derive(Clone)]
struct RefreshingHttpClient {
    inner: reqwest::Client,
    manager: Arc<AuthorizationManager>,
}

impl RefreshingHttpClient {
    fn new(manager: Arc<AuthorizationManager>) -> Result<Self, RegistryError> {
        // Mirror rmcp's default backend: no pooled connections (avoid ACK
        // stalls) and no redirects (never replay auth headers).
        let inner = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| RegistryError::Backend(err.to_string()))?;
        Ok(Self { inner, manager })
    }

    async fn access_token(&self) -> Result<Option<String>, StreamableHttpError<AuthHttpError>> {
        self.manager
            .get_access_token()
            .await
            .map(Some)
            .map_err(|err| StreamableHttpError::Client(AuthHttpError::Auth(err)))
    }

    async fn force_refresh(&self) -> Result<(), StreamableHttpError<AuthHttpError>> {
        self.manager
            .refresh_token()
            .await
            .map(|_| ())
            .map_err(|err| StreamableHttpError::Client(AuthHttpError::Auth(err)))
    }
}

// rmcp's default raw-SSE event bound (its `DEFAULT_MAX_SSE_EVENT_SIZE`).
const MAX_SSE_EVENT_SIZE: usize = 16 * 1024 * 1024;

impl StreamableHttpClient for RefreshingHttpClient {
    type Error = AuthHttpError;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: rmcp::model::ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.post_message_with_max_sse_event_size(
            uri,
            message,
            session_id,
            _auth_header,
            custom_headers,
            MAX_SSE_EVENT_SIZE,
        )
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: rmcp::model::ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let token = self.access_token().await?;
        match self
            .inner
            .post_message_with_max_sse_event_size(
                uri.clone(),
                message.clone(),
                session_id.clone(),
                token,
                custom_headers.clone(),
                max_sse_event_size,
            )
            .await
        {
            Err(err) if is_auth_rejection(&err) => {
                self.force_refresh().await?;
                let token = self.access_token().await?;
                self.inner
                    .post_message_with_max_sse_event_size(
                        uri,
                        message,
                        session_id,
                        token,
                        custom_headers,
                        max_sse_event_size,
                    )
                    .await
                    .map_err(map_http_error)
            }
            other => other.map_err(map_http_error),
        }
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let token = self.access_token().await?;
        self.inner
            .delete_session(uri, session_id, token, custom_headers)
            .await
            .map_err(map_http_error)
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.get_stream_with_max_sse_event_size(
            uri,
            session_id,
            last_event_id,
            _auth_header,
            custom_headers,
            MAX_SSE_EVENT_SIZE,
        )
        .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let token = self.access_token().await?;
        self.inner
            .get_stream_with_max_sse_event_size(
                uri,
                session_id,
                last_event_id,
                token,
                custom_headers,
                max_sse_event_size,
            )
            .await
            .map_err(map_http_error)
    }
}

// ---- legacy HTTP+SSE (2024-11-05) backend ----
//
// The transport muxes POSTs and responses on a GET event stream, so it cannot
// ride on rmcp's transports; it shares the endpoint-discovery client in
// mcp-core and talks plain JSON-RPC POSTs to the discovered endpoint.

fn bearer_from(secrets: &HashMap<String, String>) -> Option<String> {
    secrets
        .get("BEARER_TOKEN")
        .cloned()
        .filter(|value| !value.is_empty())
}

struct HttpSession {
    client: reqwest::Client,
    url: String,
    bearer: Option<String>,
    session_id: Option<String>,
    next_id: i64,
}

struct JsonRpcBackend {
    session: Mutex<HttpSession>,
}

/// Streamable-HTTP-style POST contract, kept for the legacy endpoint: accept
/// both response formats and, once the server assigned one, echo the session.
fn post_jsonrpc(session: &HttpSession, message: &Value) -> reqwest::RequestBuilder {
    let mut request = session
        .client
        .post(session.url.as_str())
        .json(message)
        .header("Accept", "application/json, text/event-stream");
    if let Some(token) = &session.bearer {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    if let Some(session_id) = &session.session_id {
        request = request.header("Mcp-Session-Id", session_id);
    }
    request
}

async fn notify(
    session: &mut HttpSession,
    method: &str,
    params: Value,
) -> Result<(), RegistryError> {
    let message = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });
    let request = post_jsonrpc(session, &message);
    let _ = request.send().await;
    Ok(())
}

/// Read the JSON-RPC reply for `id` out of an SSE-framed POST response.
/// Servers may keep the stream open, so return as soon as it arrives.
async fn read_sse_response(
    mut response: reqwest::Response,
    id: i64,
) -> Result<Value, RegistryError> {
    let mut buffer = String::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| RegistryError::Backend(err.to_string()))?
    {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        let Some(split) = buffer.rfind("\n\n") else {
            continue;
        };
        let complete = buffer[..split + 2].to_string();
        buffer.drain(..split + 2);
        for payload in parse_sse_message_events(&complete) {
            if let Ok(value) = serde_json::from_str::<Value>(&payload) {
                if value.get("id").and_then(Value::as_i64) == Some(id) {
                    return Ok(value);
                }
            }
        }
    }
    Err(RegistryError::Backend(
        "stream closed before a JSON-RPC response arrived".into(),
    ))
}

async fn rpc(
    session: &mut HttpSession,
    method: &str,
    params: Value,
) -> Result<Value, RegistryError> {
    session.next_id += 1;
    let id = session.next_id;
    let message = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    let http_response = post_jsonrpc(session, &message)
        .send()
        .await
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    let status = http_response.status();
    if !status.is_success() {
        let body = http_response.text().await.unwrap_or_default();
        return Err(RegistryError::Backend(format!("{status}: {body}")));
    }
    if method == "initialize" {
        if let Some(assigned) = http_response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            session.session_id = Some(assigned.to_string());
        }
    }
    let content_type = http_response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let response = if content_type.starts_with("text/event-stream") {
        read_sse_response(http_response, id).await?
    } else {
        let body = http_response
            .text()
            .await
            .map_err(|err| RegistryError::Backend(err.to_string()))?;
        serde_json::from_str(&body).map_err(|err| RegistryError::Backend(err.to_string()))?
    };
    if let Some(error) = response.get("error") {
        return Err(RegistryError::Backend(error.to_string()));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
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
                title: tool
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                input_schema: tool
                    .get("inputSchema")
                    .filter(|schema| schema.is_object())
                    .cloned()
                    .unwrap_or_else(empty_input_schema),
                output_schema: tool
                    .get("outputSchema")
                    .filter(|schema| schema.is_object())
                    .cloned(),
                annotations: tool
                    .get("annotations")
                    .filter(|schema| schema.is_object())
                    .cloned(),
                icons: tool.get("icons").filter(|icons| icons.is_array()).cloned(),
                meta: tool.get("_meta").filter(|meta| meta.is_object()).cloned(),
            })
        })
        .collect()
}

async fn initialize(session: &mut HttpSession) -> Result<(), RegistryError> {
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
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|err| RegistryError::Backend(err.to_string()))?;
    let mut session = HttpSession {
        client,
        url: url.to_string(),
        bearer: bearer_from(secrets),
        session_id: None,
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

#[async_trait]
impl BackendConnector for McpConnector {
    async fn connect(
        &self,
        config: &ServerConfig,
        secrets: &HashMap<String, String>,
    ) -> Result<std::sync::Arc<dyn McpBackend>, RegistryError> {
        let require_url = || {
            config
                .remote_url
                .as_deref()
                .ok_or_else(|| RegistryError::InvalidConfig("remote_url is required".into()))
        };
        let backend: std::sync::Arc<dyn McpBackend> = match config.server_type {
            ServerType::Local => std::sync::Arc::new(connect_rmcp_stdio(config, secrets).await?),
            ServerType::RemoteStreamable => std::sync::Arc::new(
                connect_rmcp_http(require_url()?, secrets, self.secrets.as_ref(), &config.id)
                    .await?,
            ),
            ServerType::Remote => std::sync::Arc::new(connect_sse(require_url()?, secrets).await?),
        };
        Ok(backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_from_result_keeps_input_schema() {
        let result = json!({
            "tools": [{
                "name": "echo",
                "description": "echo",
                "inputSchema": {
                    "type": "object",
                    "properties": { "q": { "type": "string" } },
                    "required": ["q"]
                }
            }]
        });

        let tools = tools_from_result(result);

        assert_eq!(tools[0].input_schema["type"], "object");
        assert_eq!(tools[0].input_schema["properties"]["q"]["type"], "string");
        assert_eq!(tools[0].input_schema["required"][0], "q");
    }

    #[test]
    fn tools_from_result_defaults_missing_or_malformed_schema() {
        let result = json!({
            "tools": [
                { "name": "no_schema", "description": "bare" },
                { "name": "bad_schema", "inputSchema": "not an object" }
            ]
        });

        let tools = tools_from_result(result);

        let empty = json!({ "type": "object", "properties": {} });
        assert_eq!(tools[0].input_schema, empty);
        assert_eq!(tools[1].input_schema, empty);
    }

    #[test]
    fn tools_from_result_keeps_optional_metadata() {
        let result = json!({
            "tools": [{
                "name": "fetch",
                "title": "Fetch Page",
                "description": "fetch desc",
                "inputSchema": { "type": "object", "properties": {} },
                "outputSchema": {
                    "type": "object",
                    "properties": { "html": { "type": "string" } }
                },
                "annotations": { "readOnlyHint": true },
                "icons": [{ "src": "https://example.com/i.png", "mimeType": "image/png" }],
                "_meta": { "upstream": "tag" }
            }]
        });

        let tools = tools_from_result(result);

        assert_eq!(tools[0].title.as_deref(), Some("Fetch Page"));
        assert_eq!(
            tools[0].output_schema.as_ref().unwrap()["properties"]["html"]["type"],
            "string"
        );
        assert_eq!(tools[0].annotations.as_ref().unwrap()["readOnlyHint"], true);
        assert_eq!(tools[0].icons.as_ref().unwrap()[0]["mimeType"], "image/png");
        assert_eq!(tools[0].meta.as_ref().unwrap()["upstream"], "tag");
    }

    #[test]
    fn tools_from_result_defaults_malformed_metadata() {
        let result = json!({
            "tools": [{
                "name": "weird",
                "title": 42,
                "outputSchema": "not an object",
                "annotations": [],
                "icons": "nope",
                "_meta": "also not"
            }]
        });

        let tools = tools_from_result(result);

        assert_eq!(tools[0].title, None);
        assert_eq!(tools[0].output_schema, None);
        assert_eq!(tools[0].annotations, None);
        assert_eq!(tools[0].icons, None);
        assert_eq!(tools[0].meta, None);
    }
}
