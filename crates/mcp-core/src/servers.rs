use std::collections::HashMap;
use std::path::Path;

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

#[derive(Default)]
pub struct ServerStore {
    _private: (),
}

impl ServerStore {
    pub fn open_sqlite(path: &Path) -> Result<Self, StoreError> {
        unimplemented!("ServerStore::open_sqlite")
    }

    pub fn add(&mut self, config: ServerConfig) -> Result<ServerConfig, StoreError> {
        unimplemented!("ServerStore::add")
    }

    pub fn list(&self) -> Result<Vec<ServerConfig>, StoreError> {
        unimplemented!("ServerStore::list")
    }

    pub fn get(&self, id: &str) -> Result<Option<ServerConfig>, StoreError> {
        unimplemented!("ServerStore::get")
    }

    pub fn update(&mut self, config: ServerConfig) -> Result<ServerConfig, StoreError> {
        unimplemented!("ServerStore::update")
    }

    pub fn delete(&mut self, id: &str) -> Result<bool, StoreError> {
        unimplemented!("ServerStore::delete")
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
            args: vec!["-y".into(), "@modelcontextprotocol/server-everything".into()],
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
        let raw = fs::read_to_string(&path).unwrap_or_else(|_| {
            String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned()
        });
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
