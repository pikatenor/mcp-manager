const BEARER_PREFIX: &str = "Bearer ";

/// Accept `Bearer <token>` (any case) or a raw token string.
pub fn extract_bearer(header: Option<&str>) -> Option<String> {
    unimplemented!("extract_bearer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_scheme() {
        assert_eq!(
            extract_bearer(Some("Bearer mcpm_abc")),
            Some("mcpm_abc".into())
        );
    }

    #[test]
    fn parses_bearer_case_insensitive() {
        assert_eq!(
            extract_bearer(Some("bearer mcpm_abc")),
            Some("mcpm_abc".into())
        );
    }

    #[test]
    fn parses_raw_token() {
        assert_eq!(extract_bearer(Some("mcpm_abc")), Some("mcpm_abc".into()));
    }

    #[test]
    fn rejects_empty_and_missing() {
        assert_eq!(extract_bearer(None), None);
        assert_eq!(extract_bearer(Some("")), None);
        assert_eq!(extract_bearer(Some("Bearer ")), None);
    }
}
