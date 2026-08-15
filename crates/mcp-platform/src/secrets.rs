/// OS credential store. Implementations must not log values.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError>;
    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError>;
    fn delete(&self, key: &str) -> Result<(), SecretStoreError>;
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
