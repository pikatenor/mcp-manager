use std::collections::HashMap;

/// OS credential store. Implementations must not log values.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError>;
    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError>;
    fn delete(&self, key: &str) -> Result<(), SecretStoreError>;

    /// Batch write. Stores where a write is expensive override this.
    fn set_many(&self, entries: &HashMap<String, String>) -> Result<(), SecretStoreError> {
        for (key, value) in entries {
            self.set(key, value)?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    #[error("secret store unavailable: {0}")]
    Unavailable(String),
    #[error("secret operation failed: {0}")]
    Operation(String),
}

/// Key helpers so callers do not invent ad-hoc key strings.
pub fn server_env_key(server_id: &str, env_name: &str) -> String {
    format!("server:{server_id}:env:{env_name}")
}

pub fn server_bearer_key(server_id: &str) -> String {
    format!("server:{server_id}:bearer")
}

pub fn server_oauth_key(server_id: &str, field: &str) -> String {
    format!("server:{server_id}:oauth:{field}")
}

/// Pre-registered OAuth client identity configured by the user, not minted by
/// a registration flow.
pub fn server_oauth_client_id_key(server_id: &str) -> String {
    server_oauth_key(server_id, "client_id")
}

pub fn server_oauth_client_secret_key(server_id: &str) -> String {
    server_oauth_key(server_id, "client_secret")
}
