//! Persisted per-server tool lists so stopped servers can keep serving their
//! last-known `tools/list` across app restarts. Tool metadata only — never
//! secret values.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use super::aggregator::Tool;
use super::store::{db_err, json_from_sql, json_to_sql, StoreError};

pub struct ToolCacheStore {
    conn: Arc<Mutex<Connection>>,
}

fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl ToolCacheStore {
    pub fn open_sqlite(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(db_err)?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tool_cache (
                server_id TEXT PRIMARY KEY,
                tools TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(db_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// The server's last-known tool list, if any was ever persisted.
    pub fn get(&self, server_id: &str) -> Result<Option<Vec<Tool>>, StoreError> {
        self.conn
            .lock()
            .map_err(|e| StoreError::Database(e.to_string()))?
            .query_row(
                "SELECT tools FROM tool_cache WHERE server_id = ?1",
                [server_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_err)?
            .map(json_from_sql)
            .transpose()
    }

    pub fn set(&self, server_id: &str, tools: &[Tool]) -> Result<(), StoreError> {
        self.conn
            .lock()
            .map_err(|e| StoreError::Database(e.to_string()))?
            .execute(
                "INSERT INTO tool_cache (server_id, tools, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(server_id) DO UPDATE SET
                    tools = excluded.tools,
                    updated_at = excluded.updated_at",
                params![server_id, json_to_sql(tools)?, now_unix_millis()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete(&self, server_id: &str) -> Result<(), StoreError> {
        self.conn
            .lock()
            .map_err(|e| StoreError::Database(e.to_string()))?
            .execute("DELETE FROM tool_cache WHERE server_id = ?1", [server_id])
            .map_err(db_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> Tool {
        Tool {
            name: name.into(),
            title: Some(format!("{name} title")),
            description: Some(format!("{name} desc")),
            input_schema: json!({
                "type": "object",
                "properties": { "q": { "type": "string" } },
                "required": ["q"]
            }),
            output_schema: Some(json!({
                "type": "object",
                "properties": { "hits": { "type": "array" } },
                "required": ["hits"]
            })),
            annotations: Some(json!({ "readOnlyHint": true })),
            icons: Some(json!([{ "src": "https://example.com/i.png" }])),
            meta: Some(json!({ "tag": name })),
        }
    }

    #[test]
    fn set_then_get_round_trips_tool_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let store = ToolCacheStore::open_sqlite(&dir.path().join("tool-cache.db")).unwrap();
        store.set("srv-1", &[tool("echo")]).unwrap();

        let loaded = store.get("srv-1").unwrap().unwrap();
        assert_eq!(loaded, vec![tool("echo")]);
    }

    #[test]
    fn get_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = ToolCacheStore::open_sqlite(&dir.path().join("tool-cache.db")).unwrap();
        assert!(store.get("srv-1").unwrap().is_none());
    }

    #[test]
    fn set_replaces_previous_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = ToolCacheStore::open_sqlite(&dir.path().join("tool-cache.db")).unwrap();
        store.set("srv-1", &[tool("echo"), tool("delete")]).unwrap();
        store.set("srv-1", &[tool("echo")]).unwrap();

        let loaded = store.get("srv-1").unwrap().unwrap();
        assert_eq!(loaded, vec![tool("echo")]);
    }

    #[test]
    fn delete_removes_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = ToolCacheStore::open_sqlite(&dir.path().join("tool-cache.db")).unwrap();
        store.set("srv-1", &[tool("echo")]).unwrap();
        store.delete("srv-1").unwrap();
        assert!(store.get("srv-1").unwrap().is_none());
    }

    #[test]
    fn sqlite_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-cache.db");
        {
            let store = ToolCacheStore::open_sqlite(&path).unwrap();
            store.set("srv-1", &[tool("echo")]).unwrap();
        }
        let store = ToolCacheStore::open_sqlite(&path).unwrap();
        let loaded = store.get("srv-1").unwrap().unwrap();
        assert_eq!(loaded, vec![tool("echo")]);
    }
}
