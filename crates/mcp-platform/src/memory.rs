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
        unimplemented!("MemorySecretStore::set")
    }

    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        unimplemented!("MemorySecretStore::get")
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        unimplemented!("MemorySecretStore::delete")
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
}
