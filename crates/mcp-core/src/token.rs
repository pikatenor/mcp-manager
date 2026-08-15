use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedToken {
    pub id: String,
    pub client_name: String,
    pub plaintext: String,
    pub issued_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRecord {
    pub id: String,
    pub client_name: String,
    pub token_hash: String,
    pub issued_at: i64,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("invalid token")]
    Invalid,
    #[error("revoked token")]
    Revoked,
    #[error("unknown token id")]
    UnknownId,
}

#[derive(Default)]
pub struct TokenService {
    _private: (),
}

impl TokenService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(&mut self, client_name: &str) -> IssuedToken {
        unimplemented!("TokenService::issue")
    }

    pub fn validate(&self, plaintext: &str) -> Result<TokenRecord, TokenError> {
        unimplemented!("TokenService::validate")
    }

    pub fn revoke(&mut self, id: &str) -> Result<(), TokenError> {
        unimplemented!("TokenService::revoke")
    }

    pub fn list(&self) -> Vec<TokenRecord> {
        unimplemented!("TokenService::list")
    }
}

pub fn hash_token(plaintext: &str) -> String {
    unimplemented!("hash_token")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_returns_mcpm_prefixed_secret_once() {
        let mut svc = TokenService::new();
        let issued = svc.issue("cursor");
        assert_eq!(issued.client_name, "cursor");
        assert!(issued.plaintext.starts_with("mcpm_"));
        assert!(issued.plaintext.len() > 12);
        assert!(!issued.id.is_empty());
    }

    #[test]
    fn issue_creates_unique_secrets() {
        let mut svc = TokenService::new();
        let a = svc.issue("cursor");
        let b = svc.issue("claude");
        assert_ne!(a.plaintext, b.plaintext);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn validate_accepts_issued_token() {
        let mut svc = TokenService::new();
        let issued = svc.issue("cursor");
        let record = svc.validate(&issued.plaintext).unwrap();
        assert_eq!(record.id, issued.id);
        assert_eq!(record.client_name, "cursor");
        assert_eq!(record.token_hash, hash_token(&issued.plaintext));
        assert!(record.revoked_at.is_none());
    }

    #[test]
    fn validate_rejects_unknown_secret() {
        let svc = TokenService::new();
        assert_eq!(
            svc.validate("mcpm_not-a-real-token"),
            Err(TokenError::Invalid)
        );
    }

    #[test]
    fn revoke_then_validate_fails() {
        let mut svc = TokenService::new();
        let issued = svc.issue("cursor");
        svc.revoke(&issued.id).unwrap();
        assert_eq!(svc.validate(&issued.plaintext), Err(TokenError::Revoked));
    }

    #[test]
    fn revoke_unknown_id_errors() {
        let mut svc = TokenService::new();
        assert_eq!(svc.revoke("missing"), Err(TokenError::UnknownId));
    }

    #[test]
    fn list_omits_plaintext() {
        let mut svc = TokenService::new();
        let issued = svc.issue("cursor");
        let listed = svc.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, issued.id);
        let encoded = serde_json::to_string(&listed[0]).unwrap();
        assert!(!encoded.contains(&issued.plaintext));
    }

    #[test]
    fn hash_token_is_sha256_hex() {
        let digest = hash_token("mcpm_secret");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(digest, hash_token("mcpm_secret"));
        assert_ne!(digest, hash_token("mcpm_other"));
    }
}
