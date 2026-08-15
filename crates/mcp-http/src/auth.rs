/// Accept `Bearer <token>` (any case) or a raw token string.
pub fn extract_bearer(header: Option<&str>) -> Option<String> {
    let value = header?.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    if let Some((scheme, rest)) = value.split_once(char::is_whitespace) {
        if scheme.eq_ignore_ascii_case("Bearer") {
            let token = rest.trim();
            return if token.is_empty() {
                None
            } else {
                Some(token.to_string())
            };
        }
    }
    Some(value.to_string())
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
