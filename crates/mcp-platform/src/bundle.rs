use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::{SecretStore, SecretStoreError};

/// Keychain account holding the single JSON bundle item. Legacy keys are
/// always `server:{id}:...`, so this name can never collide with them.
pub const DEFAULT_BUNDLE_ACCOUNT: &str = "secrets";

const BUNDLE_VERSION: u32 = 1;

#[derive(Serialize)]
struct BundleView<'a> {
    version: u32,
    entries: &'a HashMap<String, String>,
}

#[derive(Deserialize)]
struct Bundle {
    version: u32,
    entries: HashMap<String, String>,
}

/// Parses the stored envelope, rejecting unreadable or unknown-version bundles
/// so a stale or downgraded binary never overwrites state it cannot read.
fn parse_bundle(raw: &str) -> Result<HashMap<String, String>, SecretStoreError> {
    let bundle: Bundle = serde_json::from_str(raw)
        .map_err(|e| SecretStoreError::Operation(format!("secret bundle unreadable: {e}")))?;
    if bundle.version != BUNDLE_VERSION {
        return Err(SecretStoreError::Operation(format!(
            "secret bundle version {} unsupported (expected {BUNDLE_VERSION})",
            bundle.version
        )));
    }
    Ok(bundle.entries)
}

/// All secrets in ONE OS-store item (a JSON map) so a binary change costs at
/// most one keychain re-authorization prompt. The map is cached in memory and
/// rewritten on every mutation, so there must be exactly one instance per
/// process sharing one backend.
pub struct BundleSecretStore {
    inner: Arc<dyn SecretStore>,
    account: String,
    cache: Mutex<Option<HashMap<String, String>>>,
}

impl BundleSecretStore {
    pub fn new(inner: Arc<dyn SecretStore>) -> Self {
        Self::with_account(inner, DEFAULT_BUNDLE_ACCOUNT)
    }

    pub fn with_account(inner: Arc<dyn SecretStore>, account: impl Into<String>) -> Self {
        Self {
            inner,
            account: account.into(),
            cache: Mutex::new(None),
        }
    }

    /// Locks the cache, loading the backend item on first use, and runs `f`
    /// over the entries. `f` reports whether the map changed and must be
    /// written back before the lock is released. A failed load or flush stays
    /// unloaded so the next operation retries from the backend.
    fn with_entries(
        &self,
        f: impl FnOnce(&mut HashMap<String, String>) -> Result<bool, SecretStoreError>,
    ) -> Result<(), SecretStoreError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| SecretStoreError::Operation(e.to_string()))?;
        if cache.is_none() {
            *cache = Some(match self.inner.get(&self.account)? {
                Some(raw) => parse_bundle(&raw)?,
                None => HashMap::new(),
            });
        }
        let entries = cache.as_mut().expect("loaded above");
        if !f(entries)? {
            return Ok(());
        }
        if let Err(error) = self.flush(entries) {
            *cache = None; // the backend may hold newer state: reload next op
            return Err(error);
        }
        Ok(())
    }

    fn flush(&self, entries: &HashMap<String, String>) -> Result<(), SecretStoreError> {
        let json = serde_json::to_string(&BundleView {
            version: BUNDLE_VERSION,
            entries,
        })
        .map_err(|e| SecretStoreError::Operation(e.to_string()))?;
        self.inner.set(&self.account, &json)
    }
}

impl SecretStore for BundleSecretStore {
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        self.with_entries(|entries| {
            entries.insert(key.to_string(), value.to_string());
            Ok(true)
        })
    }

    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        let mut found = None;
        self.with_entries(|entries| {
            found = entries.get(key).cloned();
            Ok(false)
        })?;
        Ok(found)
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        self.with_entries(|entries| Ok(entries.remove(key).is_some()))
    }

    fn set_many(&self, values: &HashMap<String, String>) -> Result<(), SecretStoreError> {
        if values.is_empty() {
            return Ok(());
        }
        self.with_entries(|entries| {
            for (key, value) in values {
                entries.insert(key.clone(), value.clone());
            }
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::MemorySecretStore;

    struct CountingStore {
        inner: MemorySecretStore,
        sets: AtomicUsize,
    }

    impl SecretStore for CountingStore {
        fn set(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
            self.sets.fetch_add(1, Ordering::SeqCst);
            self.inner.set(key, value)
        }

        fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
            self.inner.get(key)
        }

        fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
            self.inner.delete(key)
        }
    }

    #[test]
    fn round_trip_set_get_overwrite() {
        let store = BundleSecretStore::new(Arc::new(MemorySecretStore::new()));
        store.set("server:srv:bearer", "tok-1").unwrap();
        store.set("server:srv:env:API_TOKEN", "sk-1").unwrap();
        assert_eq!(
            store.get("server:srv:bearer").unwrap().as_deref(),
            Some("tok-1")
        );
        store.set("server:srv:bearer", "tok-2").unwrap();
        assert_eq!(
            store.get("server:srv:bearer").unwrap().as_deref(),
            Some("tok-2")
        );
        assert_eq!(
            store.get("server:srv:env:API_TOKEN").unwrap().as_deref(),
            Some("sk-1")
        );
    }

    #[test]
    fn missing_key_is_none_on_empty_backend() {
        let store = BundleSecretStore::new(Arc::new(MemorySecretStore::new()));
        assert_eq!(store.get("server:srv:bearer").unwrap(), None);
    }

    #[test]
    fn delete_missing_key_is_ok() {
        let store = BundleSecretStore::new(Arc::new(MemorySecretStore::new()));
        // Before the bundle item exists at all...
        store.delete("server:srv:bearer").unwrap();
        // ...and for an absent key inside an existing bundle.
        store.set("server:srv:bearer", "tok").unwrap();
        store.delete("server:srv:env:API_TOKEN").unwrap();
        assert_eq!(
            store.get("server:srv:bearer").unwrap().as_deref(),
            Some("tok")
        );
    }

    #[test]
    fn values_share_one_backend_item_as_versioned_json() {
        let memory = Arc::new(MemorySecretStore::new());
        let store = BundleSecretStore::new(memory.clone());
        store.set("server:srv:bearer", "tok-1").unwrap();
        store.set("server:srv:env:API_TOKEN", "sk-1").unwrap();

        let raw = memory
            .get(DEFAULT_BUNDLE_ACCOUNT)
            .unwrap()
            .expect("single bundle item exists");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["entries"]["server:srv:bearer"], "tok-1");
        assert_eq!(value["entries"]["server:srv:env:API_TOKEN"], "sk-1");
        // Exactly one backend item: no per-key entries leak into the store.
        assert_eq!(memory.get("server:srv:bearer").unwrap(), None);
    }

    #[test]
    fn writes_persist_for_a_second_store_over_the_same_backend() {
        let memory = Arc::new(MemorySecretStore::new());
        BundleSecretStore::new(memory.clone())
            .set("server:srv:bearer", "tok")
            .unwrap();
        let second = BundleSecretStore::new(memory);
        assert_eq!(
            second.get("server:srv:bearer").unwrap().as_deref(),
            Some("tok")
        );
    }

    #[test]
    fn set_many_flushes_once() {
        let counting = Arc::new(CountingStore {
            inner: MemorySecretStore::new(),
            sets: AtomicUsize::new(0),
        });
        let store = BundleSecretStore::new(counting.clone());
        let values = HashMap::from([
            ("server:a:bearer".to_string(), "tok".to_string()),
            ("server:a:env:K".to_string(), "v".to_string()),
        ]);
        store.set_many(&values).unwrap();

        assert_eq!(counting.sets.load(Ordering::SeqCst), 1);
        assert_eq!(store.get("server:a:bearer").unwrap().as_deref(), Some("tok"));
        assert_eq!(store.get("server:a:env:K").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn set_many_on_empty_entries_writes_nothing() {
        let counting = Arc::new(CountingStore {
            inner: MemorySecretStore::new(),
            sets: AtomicUsize::new(0),
        });
        BundleSecretStore::new(counting.clone())
            .set_many(&HashMap::new())
            .unwrap();
        assert_eq!(counting.sets.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unknown_version_fails_closed() {
        let memory = Arc::new(MemorySecretStore::new());
        let raw = r#"{"version":9,"entries":{"server:srv:bearer":"tok"}}"#;
        memory.set(DEFAULT_BUNDLE_ACCOUNT, raw).unwrap();
        let store = BundleSecretStore::new(memory.clone());

        assert!(store.get("server:srv:bearer").is_err());
        assert!(store.set("server:srv:bearer", "new").is_err());
        assert!(store.delete("server:srv:bearer").is_err());
        // A downgraded binary must not wipe a newer bundle.
        assert_eq!(
            memory.get(DEFAULT_BUNDLE_ACCOUNT).unwrap().as_deref(),
            Some(raw)
        );
    }

    #[test]
    fn corrupt_bundle_fails_closed() {
        let memory = Arc::new(MemorySecretStore::new());
        memory
            .set(DEFAULT_BUNDLE_ACCOUNT, "not a bundle")
            .unwrap();
        let store = BundleSecretStore::new(memory.clone());

        assert!(store.get("server:srv:bearer").is_err());
        assert!(store.set("server:srv:bearer", "new").is_err());
        assert!(store.delete("server:srv:bearer").is_err());
        assert_eq!(
            memory.get(DEFAULT_BUNDLE_ACCOUNT).unwrap().as_deref(),
            Some("not a bundle")
        );
    }

    #[test]
    fn delete_keeps_item_but_removes_entry() {
        let memory = Arc::new(MemorySecretStore::new());
        let store = BundleSecretStore::new(memory.clone());
        store.set("server:srv:bearer", "tok").unwrap();
        store.delete("server:srv:bearer").unwrap();

        assert_eq!(store.get("server:srv:bearer").unwrap(), None);
        let raw = memory
            .get(DEFAULT_BUNDLE_ACCOUNT)
            .unwrap()
            .expect("bundle item kept");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["entries"].as_object().unwrap().len(), 0);
    }
}
