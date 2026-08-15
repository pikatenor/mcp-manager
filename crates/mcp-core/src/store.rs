use std::path::Path;

use super::token::{TokenError, TokenService};

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
        unimplemented!("TokenService::open_sqlite")
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
