use keyring::Entry;

use super::{SecretStore, SecretStoreError};

const DEFAULT_SERVICE: &str = "net.p1kachu.mcp-manager";

/// OS credential store (macOS Keychain, Linux Secret Service via `keyring`).
pub struct KeychainSecretStore {
    service: String,
}

impl KeychainSecretStore {
    pub fn new() -> Self {
        Self {
            service: DEFAULT_SERVICE.into(),
        }
    }

    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<Entry, SecretStoreError> {
        Entry::new(&self.service, key).map_err(|e| SecretStoreError::Unavailable(e.to_string()))
    }
}

impl Default for KeychainSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for KeychainSecretStore {
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| SecretStoreError::Operation(e.to_string()))
    }

    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(SecretStoreError::Operation(error.to_string())),
        }
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(SecretStoreError::Operation(error.to_string())),
        }
    }
}
