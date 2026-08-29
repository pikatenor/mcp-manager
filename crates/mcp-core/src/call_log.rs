//! Append-only log of tool invocations: metadata only, never payloads.
//!
//! Each call records which client invoked which server tool, when, and how
//! long it took. Arguments, results, and raw error strings are never stored,
//! so the log cannot leak secret values.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::store::StoreError;

/// One recorded tool invocation; `id` is the ordering key, newest largest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallEntry {
    pub id: i64,
    pub server: String,
    pub tool: String,
    pub client: String,
    pub ok: bool,
    pub error_kind: Option<String>,
    pub duration_ms: i64,
    pub called_at: i64,
}

/// SQLite-backed tool call log with count-capped retention.
pub struct CallLog {
    conn: Arc<Mutex<Connection>>,
    cap: usize,
}

impl CallLog {
    /// Retention cap: only the newest `DEFAULT_CAP` entries are kept.
    pub const DEFAULT_CAP: usize = 1000;

    /// Open (or create) the call log database at `path`.
    pub fn open_sqlite(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Database(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Database(e.to_string()))?;
        Self::with_cap(conn, Self::DEFAULT_CAP)
    }

    /// In-memory call log for tests and throwaway servers.
    pub fn memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(|e| StoreError::Database(e.to_string()))?;
        Self::with_cap(conn, Self::DEFAULT_CAP)
    }

    /// Wrap an existing connection with a custom retention cap.
    pub fn with_cap(conn: Connection, cap: usize) -> Result<Self, StoreError> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tool_calls (
                id INTEGER PRIMARY KEY,
                server TEXT NOT NULL,
                tool TEXT NOT NULL,
                client TEXT NOT NULL DEFAULT '',
                ok INTEGER NOT NULL,
                error_kind TEXT,
                duration_ms INTEGER NOT NULL,
                called_at INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            cap,
        })
    }

    /// Best-effort append; a failed write drops the entry rather than failing the request.
    pub fn record(
        &self,
        server: &str,
        tool: &str,
        client: &str,
        ok: bool,
        error_kind: Option<&str>,
        duration_ms: i64,
    ) {
        let called_at = now_unix_millis();
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO tool_calls (server, tool, client, ok, error_kind, duration_ms, called_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    server,
                    tool,
                    client,
                    ok,
                    error_kind,
                    duration_ms,
                    called_at,
                ],
            );
            // Count-capped retention: keep only the newest `cap` rows. The
            // NOT IN form survives id gaps left by clear().
            let _ = conn.execute(
                "DELETE FROM tool_calls WHERE id NOT IN (
                    SELECT id FROM tool_calls ORDER BY id DESC LIMIT ?1
                )",
                rusqlite::params![self.cap as i64],
            );
        }
    }

    /// Newest `limit` entries, most recent first.
    pub fn list_recent(&self, limit: usize) -> Result<Vec<ToolCallEntry>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Database(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, server, tool, client, ok, error_kind, duration_ms, called_at
                 FROM tool_calls ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| StoreError::Database(e.to_string()))?;
        let mapped = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(ToolCallEntry {
                    id: row.get(0)?,
                    server: row.get(1)?,
                    tool: row.get(2)?,
                    client: row.get(3)?,
                    ok: row.get::<_, i64>(4)? != 0,
                    error_kind: row.get(5)?,
                    duration_ms: row.get(6)?,
                    called_at: row.get(7)?,
                })
            })
            .map_err(|e| StoreError::Database(e.to_string()))?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| StoreError::Database(e.to_string()))
    }

    /// Best-effort removal of every entry.
    pub fn clear(&self) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute("DELETE FROM tool_calls", []);
        }
    }
}

fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_list_recent_returns_newest_first() {
        let log = CallLog::memory().unwrap();
        log.record("docs", "search", "cursor", true, None, 12);
        log.record(
            "github",
            "list_issues",
            "claude",
            false,
            Some("backend_error"),
            250,
        );
        let rows = log.list_recent(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].id > rows[1].id);
        assert_eq!(rows[0].server, "github");
        assert_eq!(rows[0].tool, "list_issues");
        assert_eq!(rows[0].client, "claude");
        assert!(!rows[0].ok);
        assert_eq!(rows[0].error_kind.as_deref(), Some("backend_error"));
        assert_eq!(rows[0].duration_ms, 250);
        assert!(rows[0].called_at > 0);
        assert_eq!(rows[1].server, "docs");
        assert_eq!(rows[1].tool, "search");
        assert_eq!(rows[1].client, "cursor");
        assert!(rows[1].ok);
        assert_eq!(rows[1].error_kind, None);
        assert_eq!(rows[1].duration_ms, 12);
    }

    #[test]
    fn list_recent_limit_bounds_rows() {
        let log = CallLog::memory().unwrap();
        for i in 0..5 {
            log.record("docs", &format!("t{i}"), "cursor", true, None, 1);
        }
        let rows = log.list_recent(3).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].tool, "t4");
        assert_eq!(rows[2].tool, "t2");
    }

    #[test]
    fn prune_keeps_newest_within_cap() {
        let conn = Connection::open_in_memory().unwrap();
        let log = CallLog::with_cap(conn, 3).unwrap();
        for i in 0..5 {
            log.record("docs", &format!("t{i}"), "cursor", true, None, 1);
        }
        let rows = log.list_recent(10).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].tool, "t4");
        assert_eq!(rows[2].tool, "t2");
    }

    #[test]
    fn clear_removes_all_rows() {
        let log = CallLog::memory().unwrap();
        log.record("docs", "search", "cursor", true, None, 1);
        log.clear();
        assert!(log.list_recent(10).unwrap().is_empty());
    }

    #[test]
    fn sqlite_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calls.db");
        {
            let log = CallLog::open_sqlite(&path).unwrap();
            log.record("docs", "search", "cursor", true, None, 5);
        }
        let log = CallLog::open_sqlite(&path).unwrap();
        let rows = log.list_recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].server, "docs");
        assert_eq!(rows[0].tool, "search");
        assert_eq!(rows[0].client, "cursor");
        assert_eq!(rows[0].duration_ms, 5);
    }
}
