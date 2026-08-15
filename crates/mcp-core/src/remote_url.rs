use url::Url;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RemoteUrlError {
    #[error("invalid url: {0}")]
    Invalid(String),
    #[error("url scheme must be https (or http to localhost)")]
    Scheme,
    #[error("refused host: {0}")]
    RefusedHost(String),
}

/// Allow https anywhere, and http only to localhost. Reject metadata/link-local hosts.
pub fn validate_remote_url(raw: &str) -> Result<Url, RemoteUrlError> {
    unimplemented!("validate_remote_url")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https() {
        let url = validate_remote_url("https://mcp.example.com/mcp").unwrap();
        assert_eq!(url.host_str(), Some("mcp.example.com"));
    }

    #[test]
    fn accepts_http_localhost() {
        validate_remote_url("http://127.0.0.1:3000/mcp").unwrap();
        validate_remote_url("http://localhost/sse").unwrap();
    }

    #[test]
    fn rejects_plain_http_to_public_host() {
        assert_eq!(
            validate_remote_url("http://example.com/mcp").unwrap_err(),
            RemoteUrlError::Scheme
        );
    }

    #[test]
    fn rejects_metadata_and_link_local() {
        assert!(matches!(
            validate_remote_url("https://169.254.169.254/latest"),
            Err(RemoteUrlError::RefusedHost(_))
        ));
        assert!(matches!(
            validate_remote_url("https://metadata.google.internal/"),
            Err(RemoteUrlError::RefusedHost(_))
        ));
    }
}
