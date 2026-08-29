use std::collections::HashMap;
use std::sync::Mutex;

use super::{SecretStore, SecretStoreError};

/// In-memory store for tests and as a fallback when no OS keychain is present.
#[derive(Default)]
pub struct MemorySecretStore {
    inner: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        self.inner
            .lock()
            .map_err(|e| SecretStoreError::Operation(e.to_string()))?
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        Ok(self
            .inner
            .lock()
            .map_err(|e| SecretStoreError::Operation(e.to_string()))?
            .get(key)
            .cloned())
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        self.inner
            .lock()
            .map_err(|e| SecretStoreError::Operation(e.to_string()))?
            .remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{server_bearer_key, server_env_key};

    #[test]
    fn round_trip_and_overwrite() {
        let store = MemorySecretStore::new();
        let key = server_env_key("srv1", "API_TOKEN");
        store.set(&key, "secret-a").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("secret-a"));
        store.set(&key, "secret-b").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("secret-b"));
    }

    #[test]
    fn missing_key_is_none() {
        let store = MemorySecretStore::new();
        assert_eq!(store.get("nope").unwrap(), None);
    }

    #[test]
    fn delete_removes_value() {
        let store = MemorySecretStore::new();
        let key = server_bearer_key("srv1");
        store.set(&key, "tok").unwrap();
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap();
    }

    #[test]
    fn set_many_default_writes_all_keys() {
        let store = MemorySecretStore::new();
        let entries = HashMap::from([
            (server_bearer_key("srv1"), "tok".to_string()),
            (server_env_key("srv1", "API_TOKEN"), "sk".to_string()),
        ]);
        store.set_many(&entries).unwrap();
        assert_eq!(
            store.get(&server_bearer_key("srv1")).unwrap().as_deref(),
            Some("tok")
        );
        assert_eq!(
            store
                .get(&server_env_key("srv1", "API_TOKEN"))
                .unwrap()
                .as_deref(),
            Some("sk")
        );
    }
}
