use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::token::{TokenError, TokenRecord, TokenService};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(String),
    #[error(transparent)]
    Token(#[from] TokenError),
}

impl TokenService {
    /// Open (or create) a SQLite file and load token records. Secrets stay hashed.
    pub fn open_sqlite(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Database(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Database(e.to_string()))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tokens (
                id TEXT PRIMARY KEY,
                client_name TEXT NOT NULL,
                token_hash TEXT NOT NULL UNIQUE,
                issued_at INTEGER NOT NULL,
                revoked_at INTEGER
            )",
            [],
        )
        .map_err(|e| StoreError::Database(e.to_string()))?;

        let records = {
            let mut stmt = conn
                .prepare("SELECT id, client_name, token_hash, issued_at, revoked_at FROM tokens")
                .map_err(|e| StoreError::Database(e.to_string()))?;
            let mapped = stmt
                .query_map([], |row| {
                    Ok(TokenRecord {
                        id: row.get(0)?,
                        client_name: row.get(1)?,
                        token_hash: row.get(2)?,
                        issued_at: row.get(3)?,
                        revoked_at: row.get(4)?,
                    })
                })
                .map_err(|e| StoreError::Database(e.to_string()))?;
            mapped
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| StoreError::Database(e.to_string()))?
        };

        Ok(TokenService {
            records,
            db: Some(Arc::new(Mutex::new(conn))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.db");
        let issued = {
            let mut svc = TokenService::open_sqlite(&path).unwrap();
            svc.issue("cursor")
        };
        let svc = TokenService::open_sqlite(&path).unwrap();
        let record = svc.validate(&issued.plaintext).unwrap();
        assert_eq!(record.client_name, "cursor");
        assert_eq!(record.id, issued.id);
    }

    #[test]
    fn sqlite_revoke_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.db");
        let issued = {
            let mut svc = TokenService::open_sqlite(&path).unwrap();
            let issued = svc.issue("cursor");
            svc.revoke(&issued.id).unwrap();
            issued
        };
        let svc = TokenService::open_sqlite(&path).unwrap();
        assert_eq!(
            svc.validate(&issued.plaintext),
            Err(crate::TokenError::Revoked)
        );
    }
}
