use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::Connection;
use serde_json::Value;

use crate::capability::{Capability, MethodSchema};
use crate::error::CapabilityError;

/// A SQLite-backed persistent key-value store capability.
///
/// Data survives process restarts. The underlying SQLite database is
/// stored at the path provided during construction.
///
/// The API is identical to [`KvCapability`](super::KvCapability) so it
/// can be used as a drop-in replacement.
pub struct PersistentKvCapability {
    conn: Arc<Mutex<Connection>>,
    max_keys: usize,
    max_value_bytes: usize,
}

impl PersistentKvCapability {
    /// Create a new persistent KV backed by SQLite at the given path.
    pub fn new(path: impl AsRef<Path>, max_keys: usize, max_value_bytes: usize) -> Result<Self, String> {
        let conn = Connection::open(path.as_ref())
            .map_err(|e| format!("Failed to open SQLite database: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kv (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;",
        )
        .map_err(|e| format!("Failed to initialize KV table: {e}"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            max_keys,
            max_value_bytes,
        })
    }

    /// Create a persistent KV with default limits (1000 keys, 1MB max value).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        Self::new(path, 1000, 1024 * 1024)
    }

    /// Create an in-memory SQLite KV (useful for testing — not truly persistent).
    pub fn in_memory() -> Result<Self, String> {
        Self::new(":memory:", 1000, 1024 * 1024)
    }
}

#[async_trait]
impl Capability for PersistentKvCapability {
    fn name(&self) -> &str {
        "kv"
    }

    fn methods(&self) -> Vec<MethodSchema> {
        vec![
            MethodSchema::new("get", "Get a value by key",
                serde_json::json!({"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}),
                serde_json::json!({"description": "The stored value, or null"})),
            MethodSchema::new("set", "Set a key-value pair",
                serde_json::json!({"type": "object", "properties": {"key": {"type": "string"}, "value": {}}, "required": ["key", "value"]}),
                serde_json::json!({"type": "null"})),
            MethodSchema::new("delete", "Delete a key",
                serde_json::json!({"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}),
                serde_json::json!({"type": "boolean"})),
            MethodSchema::new("list", "List keys with optional prefix",
                serde_json::json!({"type": "object", "properties": {"prefix": {"type": "string"}}}),
                serde_json::json!({"type": "array", "items": {"type": "string"}})),
            MethodSchema::new("has", "Check if key exists",
                serde_json::json!({"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}),
                serde_json::json!({"type": "boolean"})),
        ]
    }

    async fn call(&self, method: &str, input: Value) -> Result<Value, CapabilityError> {
        let conn = self.conn.lock();

        match method {
            "get" => {
                let key = input.get("key").and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "get".into(),
                        message: "missing required parameter: key".into(),
                    })?;

                let result: Option<String> = conn
                    .query_row("SELECT value FROM kv WHERE key = ?1", [key], |row| row.get(0))
                    .ok();

                match result {
                    Some(json_str) => serde_json::from_str(&json_str).map_err(|e| {
                        CapabilityError::InvocationFailed {
                            capability: "kv".into(), method: "get".into(),
                            message: format!("corrupted value: {e}"),
                        }
                    }),
                    None => Ok(Value::Null),
                }
            }

            "set" => {
                let key = input.get("key").and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "set".into(),
                        message: "missing required parameter: key".into(),
                    })?;

                let value = input.get("value")
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "set".into(),
                        message: "missing required parameter: value".into(),
                    })?;

                let serialized = serde_json::to_string(value).unwrap_or_default();
                if serialized.len() > self.max_value_bytes {
                    return Err(CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "set".into(),
                        message: format!("value size {} exceeds limit of {} bytes", serialized.len(), self.max_value_bytes),
                    });
                }

                // Check key count for new keys
                let exists: bool = conn
                    .query_row("SELECT 1 FROM kv WHERE key = ?1", [key], |_| Ok(true))
                    .unwrap_or(false);

                if !exists {
                    let count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM kv", [], |row| row.get(0))
                        .unwrap_or(0);
                    let count = count as usize;
                    if count >= self.max_keys {
                        return Err(CapabilityError::InvocationFailed {
                            capability: "kv".into(), method: "set".into(),
                            message: format!("key count {} would exceed limit of {}", count + 1, self.max_keys),
                        });
                    }
                }

                conn.execute(
                    "INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)",
                    rusqlite::params![key, serialized],
                ).map_err(|e| CapabilityError::InvocationFailed {
                    capability: "kv".into(), method: "set".into(),
                    message: format!("write failed: {e}"),
                })?;

                Ok(Value::Null)
            }

            "delete" => {
                let key = input.get("key").and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "delete".into(),
                        message: "missing required parameter: key".into(),
                    })?;

                let deleted = conn
                    .execute("DELETE FROM kv WHERE key = ?1", [key])
                    .map(|n| n > 0)
                    .unwrap_or(false);

                Ok(Value::Bool(deleted))
            }

            "list" => {
                let prefix = input.get("prefix").and_then(|v| v.as_str()).unwrap_or("");

                let mut stmt = conn
                    .prepare("SELECT key FROM kv WHERE key LIKE ?1")
                    .map_err(|e| CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "list".into(),
                        message: format!("query failed: {e}"),
                    })?;

                let pattern = format!("{}%", prefix.replace('%', "\\%").replace('_', "\\_"));
                let keys: Vec<Value> = stmt
                    .query_map([&pattern], |row| {
                        let key: String = row.get(0)?;
                        Ok(Value::String(key))
                    })
                    .map_err(|e| CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "list".into(),
                        message: format!("query failed: {e}"),
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                Ok(Value::Array(keys))
            }

            "has" => {
                let key = input.get("key").and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(), method: "has".into(),
                        message: "missing required parameter: key".into(),
                    })?;

                let exists: bool = conn
                    .query_row("SELECT 1 FROM kv WHERE key = ?1", [key], |_| Ok(true))
                    .unwrap_or(false);

                Ok(Value::Bool(exists))
            }

            _ => Err(CapabilityError::NotFound {
                capability: "kv".into(),
                method: method.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_persistent_set_get() {
        let kv = PersistentKvCapability::in_memory().unwrap();
        kv.call("set", serde_json::json!({"key": "hello", "value": "world"})).await.unwrap();
        let result = kv.call("get", serde_json::json!({"key": "hello"})).await.unwrap();
        assert_eq!(result, serde_json::json!("world"));
    }

    #[tokio::test]
    async fn test_persistent_list_delete_has() {
        let kv = PersistentKvCapability::in_memory().unwrap();
        kv.call("set", serde_json::json!({"key": "a:1", "value": 1})).await.unwrap();
        kv.call("set", serde_json::json!({"key": "a:2", "value": 2})).await.unwrap();
        kv.call("set", serde_json::json!({"key": "b:1", "value": 3})).await.unwrap();

        let list = kv.call("list", serde_json::json!({"prefix": "a:"})).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);

        let has = kv.call("has", serde_json::json!({"key": "a:1"})).await.unwrap();
        assert_eq!(has, Value::Bool(true));

        let deleted = kv.call("delete", serde_json::json!({"key": "a:1"})).await.unwrap();
        assert_eq!(deleted, Value::Bool(true));

        let has_after = kv.call("has", serde_json::json!({"key": "a:1"})).await.unwrap();
        assert_eq!(has_after, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_persistent_survives_reopen() {
        let dir = std::env::temp_dir().join("sandcastle_test_kv.db");
        let _ = std::fs::remove_file(&dir);

        // Write
        {
            let kv = PersistentKvCapability::open(&dir).unwrap();
            kv.call("set", serde_json::json!({"key": "persistent", "value": "data"})).await.unwrap();
        }

        // Reopen and read
        {
            let kv = PersistentKvCapability::open(&dir).unwrap();
            let result = kv.call("get", serde_json::json!({"key": "persistent"})).await.unwrap();
            assert_eq!(result, serde_json::json!("data"));
        }

        let _ = std::fs::remove_file(&dir);
    }
}
