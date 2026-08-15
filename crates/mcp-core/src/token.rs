use std::sync::{Arc, Mutex};

use rand::RngCore;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    pub(crate) records: Vec<TokenRecord>,
    pub(crate) db: Option<Arc<Mutex<Connection>>>,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn random_hex(nbytes: usize) -> String {
    let mut bytes = vec![0u8; nbytes];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

impl TokenService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(&mut self, client_name: &str) -> IssuedToken {
        let id = random_hex(16);
        let plaintext = format!("mcpm_{}", random_hex(24));
        let issued_at = now_unix();
        self.records.push(TokenRecord {
            id: id.clone(),
            client_name: client_name.to_string(),
            token_hash: hash_token(&plaintext),
            issued_at,
            revoked_at: None,
        });
        if let Some(db) = &self.db {
            let record = self.records.last().expect("just pushed");
            persist_insert(db, record);
        }
        IssuedToken {
            id,
            client_name: client_name.to_string(),
            plaintext,
            issued_at,
        }
    }

    pub fn validate(&self, plaintext: &str) -> Result<TokenRecord, TokenError> {
        let hash = hash_token(plaintext);
        let record = self
            .records
            .iter()
            .find(|r| r.token_hash == hash)
            .ok_or(TokenError::Invalid)?;
        if record.revoked_at.is_some() {
            return Err(TokenError::Revoked);
        }
        Ok(record.clone())
    }

    pub fn revoke(&mut self, id: &str) -> Result<(), TokenError> {
        let record = self
            .records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(TokenError::UnknownId)?;
        record.revoked_at = Some(now_unix());
        if let Some(db) = &self.db {
            persist_revoke(db, record);
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<TokenRecord> {
        self.records.clone()
    }
}

pub fn hash_token(plaintext: &str) -> String {
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

fn persist_insert(db: &Arc<Mutex<Connection>>, record: &TokenRecord) {
    if let Ok(conn) = db.lock() {
        let _ = conn.execute(
            "INSERT INTO tokens (id, client_name, token_hash, issued_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                record.id,
                record.client_name,
                record.token_hash,
                record.issued_at,
                record.revoked_at,
            ],
        );
    }
}

fn persist_revoke(db: &Arc<Mutex<Connection>>, record: &TokenRecord) {
    if let Ok(conn) = db.lock() {
        let _ = conn.execute(
            "UPDATE tokens SET revoked_at = ?1 WHERE id = ?2",
            rusqlite::params![record.revoked_at, record.id],
        );
    }
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
