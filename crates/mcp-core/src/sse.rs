use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SseTransportError {
    #[error("missing endpoint event")]
    MissingEndpoint,
    #[error("http error: {0}")]
    Http(String),
}

/// Parse the first `endpoint` SSE event from a stream chunk.
pub fn parse_sse_endpoint_event(chunk: &str) -> Result<String, SseTransportError> {
    unimplemented!("parse_sse_endpoint_event")
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
        unimplemented!("SseClientTransport::connect")
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
