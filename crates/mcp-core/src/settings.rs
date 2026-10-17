//! App-level feature flags, persisted as key/value rows so new settings need
//! no schema migration.

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use rusqlite::{params, Connection, OptionalExtension};

use super::store::StoreError;

/// Feature flags served to the inbound endpoint. Defaults preserve the
/// pre-settings behavior: stopped servers are absent and never auto-started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppSettings {
    /// `tools/list` also serves stopped servers' cached tool lists.
    pub list_stopped_from_cache: bool,
    /// `tools/call` starts the target server when it is not running.
    pub on_demand_start: bool,
}

/// Read-mostly settings handle; the HTTP layer snapshots it per request.
pub type SharedSettings = Arc<RwLock<AppSettings>>;

/// Snapshot the shared settings, tolerating a poisoned lock: flags only gate
/// optional behavior, so serving the last value beats failing the request.
pub fn read_settings(shared: &SharedSettings) -> AppSettings {
    *shared
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const KEY_LIST_STOPPED_FROM_CACHE: &str = "list-stopped-from-cache";
const KEY_ON_DEMAND_START: &str = "on-demand-start";

pub struct SettingsStore {
    conn: Arc<Mutex<Connection>>,
}

fn db_err(err: impl ToString) -> StoreError {
    StoreError::Database(err.to_string())
}

impl SettingsStore {
    pub fn open_sqlite(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(db_err)?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )
        .map_err(db_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Missing rows fall back to per-field defaults; unknown keys are ignored.
    pub fn load(&self) -> Result<AppSettings, StoreError> {
        Ok(AppSettings {
            list_stopped_from_cache: self.load_flag(KEY_LIST_STOPPED_FROM_CACHE)?,
            on_demand_start: self.load_flag(KEY_ON_DEMAND_START)?,
        })
    }

    /// One flag row; an absent or unparsable value keeps the field default.
    fn load_flag(&self, key: &str) -> Result<bool, StoreError> {
        let value = self
            .conn
            .lock()
            .map_err(|e| StoreError::Database(e.to_string()))?
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(db_err)?;
        Ok(value.as_deref() == Some("true"))
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::Database(e.to_string()))?;
        for (key, value) in [
            (
                KEY_LIST_STOPPED_FROM_CACHE,
                settings.list_stopped_from_cache,
            ),
            (KEY_ON_DEMAND_START, settings.on_demand_start),
        ] {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, if value { "true" } else { "false" }],
            )
            .map_err(db_err)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_db_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open_sqlite(&dir.path().join("settings.db")).unwrap();
        assert_eq!(store.load().unwrap(), AppSettings::default());
        assert!(!store.load().unwrap().list_stopped_from_cache);
        assert!(!store.load().unwrap().on_demand_start);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open_sqlite(&dir.path().join("settings.db")).unwrap();
        store
            .save(&AppSettings {
                list_stopped_from_cache: true,
                on_demand_start: true,
            })
            .unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.list_stopped_from_cache);
        assert!(loaded.on_demand_start);

        store
            .save(&AppSettings {
                list_stopped_from_cache: false,
                on_demand_start: true,
            })
            .unwrap();
        let loaded = store.load().unwrap();
        assert!(!loaded.list_stopped_from_cache);
        assert!(loaded.on_demand_start);
    }

    #[test]
    fn values_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.db");
        {
            let store = SettingsStore::open_sqlite(&path).unwrap();
            store
                .save(&AppSettings {
                    list_stopped_from_cache: true,
                    on_demand_start: false,
                })
                .unwrap();
        }
        let store = SettingsStore::open_sqlite(&path).unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.list_stopped_from_cache);
        assert!(!loaded.on_demand_start);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open_sqlite(&dir.path().join("settings.db")).unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO settings (key, value) VALUES ('future-feature', 'true')",
                [],
            )
            .unwrap();
        assert_eq!(store.load().unwrap(), AppSettings::default());
    }
}
