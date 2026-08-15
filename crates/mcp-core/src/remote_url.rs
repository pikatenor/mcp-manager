use std::net::IpAddr;

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
    let url = Url::parse(raw).map_err(|e| RemoteUrlError::Invalid(e.to_string()))?;
    let host = url
        .host_str()
        .ok_or_else(|| RemoteUrlError::Invalid("missing host".into()))?;

    if is_refused_host(host) {
        return Err(RemoteUrlError::RefusedHost(host.to_string()));
    }

    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback_host(host) => Ok(url),
        "http" => Err(RemoteUrlError::Scheme),
        _ => Err(RemoteUrlError::Scheme),
    }
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn is_refused_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host == "metadata.google.internal" {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_unicast_link_local(),
        Err(_) => false,
    }
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
