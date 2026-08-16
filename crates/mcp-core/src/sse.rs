use std::collections::HashMap;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SseTransportError {
    #[error("missing endpoint event")]
    MissingEndpoint,
    #[error("http error: {0}")]
    Http(String),
}

/// Parse the first `endpoint` SSE event from a stream chunk.
pub fn parse_sse_endpoint_event(chunk: &str) -> Result<String, SseTransportError> {
    for block in chunk.split("\n\n") {
        let mut event: Option<&str> = None;
        let mut data = Vec::new();
        for line in block.lines() {
            if line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("event:") {
                event = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push(value.trim_start());
            }
        }
        if event == Some("endpoint") {
            let endpoint = data.join("\n");
            if !endpoint.is_empty() {
                return Ok(endpoint);
            }
        }
    }
    Err(SseTransportError::MissingEndpoint)
}

/// Parse JSON-RPC payloads (`message` events) from complete SSE blocks.
/// Blocks without an `event:` field default to `message` per the SSE spec.
pub fn parse_sse_message_events(chunk: &str) -> Vec<String> {
    chunk
        .split("\n\n")
        .filter_map(|block| {
            let mut event: Option<&str> = None;
            let mut data = Vec::new();
            for line in block.lines() {
                if line.starts_with(':') {
                    continue;
                }
                if let Some(value) = line.strip_prefix("event:") {
                    event = Some(value.trim());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.trim_start());
                }
            }
            let is_message = event.is_none() || event == Some("message");
            if is_message && !data.is_empty() {
                Some(data.join("\n"))
            } else {
                None
            }
        })
        .collect()
}

/// Legacy HTTP+SSE client (`2024-11-05`): GET event stream, POST to endpoint URL.
pub struct SseClientTransport {
    pub endpoint_url: String,
}

impl SseClientTransport {
    pub async fn connect(
        sse_url: &str,
        headers: HashMap<String, String>,
    ) -> Result<Self, SseTransportError> {
        let url = Url::parse(sse_url).map_err(|e| SseTransportError::Http(e.to_string()))?;
        let host = url
            .host_str()
            .ok_or_else(|| SseTransportError::Http("missing host".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| SseTransportError::Http("missing port".into()))?;
        let mut path = url.path().to_string();
        if path.is_empty() {
            path = "/".into();
        }
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }

        let mut stream = TcpStream::connect((host, port))
            .await
            .map_err(|e| SseTransportError::Http(e.to_string()))?;
        let mut request =
            format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\n");
        for (name, value) in &headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("Connection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| SseTransportError::Http(e.to_string()))?;

        let mut buf = Vec::new();
        stream
            .read_to_end(&mut buf)
            .await
            .map_err(|e| SseTransportError::Http(e.to_string()))?;
        let text = String::from_utf8_lossy(&buf);
        let body = text
            .split("\r\n\r\n")
            .nth(1)
            .ok_or_else(|| SseTransportError::Http("missing response body".into()))?;
        let status = text.lines().next().unwrap_or_default();
        if !status.contains("200") {
            return Err(SseTransportError::Http(status.to_string()));
        }

        let endpoint = parse_sse_endpoint_event(body)?;
        let endpoint_url = url
            .join(&endpoint)
            .map_err(|e| SseTransportError::Http(e.to_string()))?
            .to_string();
        Ok(Self { endpoint_url })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn parses_endpoint_event() {
        let chunk = "event: endpoint\ndata: /messages?session=abc\n\n";
        assert_eq!(
            parse_sse_endpoint_event(chunk).unwrap(),
            "/messages?session=abc"
        );
    }

    #[test]
    fn parses_endpoint_event_with_prelude() {
        let chunk = ": ping\n\nevent: endpoint\ndata: http://127.0.0.1:9/mcp/messages\n\n";
        assert_eq!(
            parse_sse_endpoint_event(chunk).unwrap(),
            "http://127.0.0.1:9/mcp/messages"
        );
    }

    #[test]
    fn parses_message_event_payloads() {
        let chunk = "event: message\ndata: {\"jsonrpc\":\"2.0\"}\n\nevent: ping\ndata: {}\n\n";
        assert_eq!(
            parse_sse_message_events(chunk),
            vec!["{\"jsonrpc\":\"2.0\"}".to_string()]
        );
    }

    #[test]
    fn data_without_event_field_is_a_message() {
        assert_eq!(
            parse_sse_message_events("data: {\"id\":1}\n\n"),
            vec!["{\"id\":1}".to_string()]
        );
    }

    #[test]
    fn multi_line_data_is_joined() {
        let chunk = "event: message\ndata: {\"a\":\ndata: 1}\n\n";
        assert_eq!(
            parse_sse_message_events(chunk),
            vec!["{\"a\":\n1}".to_string()]
        );
    }

    #[test]
    fn comments_and_empty_blocks_are_ignored() {
        let chunk = ": keep-alive\n\nevent: message\ndata: {}\n\n\n";
        assert_eq!(parse_sse_message_events(chunk), vec!["{}".to_string()]);
    }

    #[test]
    fn missing_endpoint_is_an_error() {
        assert_eq!(
            parse_sse_endpoint_event("event: message\ndata: {}\n\n").unwrap_err(),
            SseTransportError::MissingEndpoint
        );
    }

    #[tokio::test]
    async fn connect_reads_endpoint_event() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let body = "event: endpoint\ndata: /messages\n\n";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(resp.as_bytes()).await;
        });

        let transport = SseClientTransport::connect(&format!("http://{addr}/sse"), HashMap::new())
            .await
            .unwrap();
        assert!(
            transport.endpoint_url.ends_with("/messages"),
            "{}",
            transport.endpoint_url
        );
    }
}
