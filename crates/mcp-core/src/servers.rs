use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::store::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerType {
    Local,
    Remote,
    RemoteStreamable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub server_type: ServerType,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env_keys: Vec<String>,
    pub remote_url: Option<String>,
    pub auto_start: bool,
    pub disabled: bool,
    pub tool_permissions: HashMap<String, bool>,
}

pub struct ServerStore {
    conn: Connection,
}

fn db_err(err: impl ToString) -> StoreError {
    StoreError::Database(err.to_string())
}

fn type_to_sql(server_type: ServerType) -> &'static str {
    match server_type {
        ServerType::Local => "local",
        ServerType::Remote => "remote",
        ServerType::RemoteStreamable => "remote-streamable",
    }
}

fn type_from_sql(value: &str) -> Result<ServerType, StoreError> {
    match value {
        "local" => Ok(ServerType::Local),
        "remote" => Ok(ServerType::Remote),
        "remote-streamable" => Ok(ServerType::RemoteStreamable),
        other => Err(StoreError::Database(format!(
            "unknown server type: {other}"
        ))),
    }
}

fn json_to_sql<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(db_err)
}

fn json_from_sql<T: for<'de> Deserialize<'de>>(raw: String) -> Result<T, StoreError> {
    serde_json::from_str(&raw).map_err(db_err)
}

fn row_to_config(row: &rusqlite::Row<'_>) -> Result<ServerConfig, StoreError> {
    Ok(ServerConfig {
        id: row.get(0).map_err(db_err)?,
        name: row.get(1).map_err(db_err)?,
        server_type: type_from_sql(&row.get::<_, String>(2).map_err(db_err)?)?,
        command: row.get(3).map_err(db_err)?,
        args: json_from_sql(row.get(4).map_err(db_err)?)?,
        env_keys: json_from_sql(row.get(5).map_err(db_err)?)?,
        remote_url: row.get(6).map_err(db_err)?,
        auto_start: row.get::<_, i64>(7).map_err(db_err)? != 0,
        disabled: row.get::<_, i64>(8).map_err(db_err)? != 0,
        tool_permissions: json_from_sql(row.get(9).map_err(db_err)?)?,
    })
}

impl ServerStore {
    pub fn open_sqlite(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(db_err)?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS servers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                server_type TEXT NOT NULL,
                command TEXT,
                args TEXT NOT NULL,
                env_keys TEXT NOT NULL,
                remote_url TEXT,
                auto_start INTEGER NOT NULL,
                disabled INTEGER NOT NULL,
                tool_permissions TEXT NOT NULL
            )",
            [],
        )
        .map_err(db_err)?;
        Ok(Self { conn })
    }

    pub fn add(&mut self, config: ServerConfig) -> Result<ServerConfig, StoreError> {
        self.conn
            .execute(
                "INSERT INTO servers (
                    id, name, server_type, command, args, env_keys,
                    remote_url, auto_start, disabled, tool_permissions
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    config.id,
                    config.name,
                    type_to_sql(config.server_type),
                    config.command,
                    json_to_sql(&config.args)?,
                    json_to_sql(&config.env_keys)?,
                    config.remote_url,
                    config.auto_start as i64,
                    config.disabled as i64,
                    json_to_sql(&config.tool_permissions)?,
                ],
            )
            .map_err(db_err)?;
        Ok(config)
    }

    pub fn list(&self) -> Result<Vec<ServerConfig>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, server_type, command, args, env_keys,
                        remote_url, auto_start, disabled, tool_permissions
                 FROM servers
                 ORDER BY name, id",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| Ok(row_to_config(row)))
            .map_err(db_err)?;
        let mut configs = Vec::new();
        for row in rows {
            configs.push(row.map_err(db_err)??);
        }
        Ok(configs)
    }

    pub fn get(&self, id: &str) -> Result<Option<ServerConfig>, StoreError> {
        self.conn
            .query_row(
                "SELECT id, name, server_type, command, args, env_keys,
                        remote_url, auto_start, disabled, tool_permissions
                 FROM servers WHERE id = ?1",
                [id],
                |row| Ok(row_to_config(row)),
            )
            .optional()
            .map_err(db_err)?
            .transpose()
    }

    pub fn update(&mut self, config: ServerConfig) -> Result<ServerConfig, StoreError> {
        let changed = self
            .conn
            .execute(
                "UPDATE servers SET
                    name = ?2,
                    server_type = ?3,
                    command = ?4,
                    args = ?5,
                    env_keys = ?6,
                    remote_url = ?7,
                    auto_start = ?8,
                    disabled = ?9,
                    tool_permissions = ?10
                 WHERE id = ?1",
                params![
                    config.id,
                    config.name,
                    type_to_sql(config.server_type),
                    config.command,
                    json_to_sql(&config.args)?,
                    json_to_sql(&config.env_keys)?,
                    config.remote_url,
                    config.auto_start as i64,
                    config.disabled as i64,
                    json_to_sql(&config.tool_permissions)?,
                ],
            )
            .map_err(db_err)?;
        if changed == 0 {
            return Err(StoreError::Database(format!(
                "unknown server id: {}",
                config.id
            )));
        }
        Ok(config)
    }

    pub fn delete(&mut self, id: &str) -> Result<bool, StoreError> {
        let changed = self
            .conn
            .execute("DELETE FROM servers WHERE id = ?1", [id])
            .map_err(db_err)?;
        Ok(changed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn local_config(name: &str) -> ServerConfig {
        ServerConfig {
            id: "srv-1".into(),
            name: name.into(),
            server_type: ServerType::Local,
            command: Some("npx".into()),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-everything".into(),
            ],
            env_keys: vec!["API_TOKEN".into()],
            remote_url: None,
            auto_start: true,
            disabled: false,
            tool_permissions: HashMap::from([("delete".into(), false)]),
        }
    }

    #[test]
    fn sqlite_round_trip_without_secret_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let mut store = ServerStore::open_sqlite(&path).unwrap();
            store.add(local_config("everything")).unwrap();
        }
        let store = ServerStore::open_sqlite(&path).unwrap();
        let loaded = store.get("srv-1").unwrap().unwrap();
        assert_eq!(loaded.name, "everything");
        assert_eq!(loaded.server_type, ServerType::Local);
        assert_eq!(loaded.env_keys, vec!["API_TOKEN"]);
        assert_eq!(loaded.tool_permissions.get("delete"), Some(&false));
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|_| String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned());
        assert!(
            !raw.contains("sk-secret"),
            "sqlite must not store secret values"
        );
    }

    #[test]
    fn stores_remote_sse_and_streamable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut store = ServerStore::open_sqlite(&path).unwrap();
        store
            .add(ServerConfig {
                id: "sse".into(),
                name: "legacy".into(),
                server_type: ServerType::Remote,
                command: None,
                args: vec![],
                env_keys: vec![],
                remote_url: Some("https://example.com/sse".into()),
                auto_start: false,
                disabled: false,
                tool_permissions: HashMap::new(),
            })
            .unwrap();
        store
            .add(ServerConfig {
                id: "http".into(),
                name: "modern".into(),
                server_type: ServerType::RemoteStreamable,
                command: None,
                args: vec![],
                env_keys: vec![],
                remote_url: Some("https://example.com/mcp".into()),
                auto_start: false,
                disabled: false,
                tool_permissions: HashMap::new(),
            })
            .unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            store.get("sse").unwrap().unwrap().server_type,
            ServerType::Remote
        );
        assert_eq!(
            store.get("http").unwrap().unwrap().server_type,
            ServerType::RemoteStreamable
        );
    }

    #[test]
    fn update_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut store = ServerStore::open_sqlite(&path).unwrap();
        let mut config = local_config("everything");
        store.add(config.clone()).unwrap();
        config.disabled = true;
        config.auto_start = false;
        config.tool_permissions.insert("search".into(), true);
        store.update(config).unwrap();
        let loaded = store.get("srv-1").unwrap().unwrap();
        assert!(loaded.disabled);
        assert!(!loaded.auto_start);
        assert_eq!(loaded.tool_permissions.get("search"), Some(&true));
        assert!(store.delete("srv-1").unwrap());
        assert!(store.get("srv-1").unwrap().is_none());
    }
}
