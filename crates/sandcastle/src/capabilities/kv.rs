use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;

use crate::capability::{Capability, MethodSchema};
use crate::error::CapabilityError;

/// An in-memory key-value store capability backed by `DashMap`.
///
/// The underlying map is wrapped in `Arc` so it can be shared across
/// multiple sandbox executions.
pub struct KvCapability {
    store: Arc<DashMap<String, Value>>,
    max_keys: usize,
    max_value_bytes: usize,
}

impl KvCapability {
    /// Create a new KV capability with the given limits.
    pub fn new(max_keys: usize, max_value_bytes: usize) -> Self {
        Self {
            store: Arc::new(DashMap::new()),
            max_keys,
            max_value_bytes,
        }
    }

    /// Create a new KV capability that shares the given store.
    pub fn with_store(
        store: Arc<DashMap<String, Value>>,
        max_keys: usize,
        max_value_bytes: usize,
    ) -> Self {
        Self {
            store,
            max_keys,
            max_value_bytes,
        }
    }

    /// Returns a reference to the underlying store for sharing across instances.
    pub fn store(&self) -> &Arc<DashMap<String, Value>> {
        &self.store
    }
}

impl Default for KvCapability {
    fn default() -> Self {
        Self::new(1000, 1024 * 1024) // 1000 keys, 1 MB max value
    }
}

#[async_trait]
impl Capability for KvCapability {
    fn name(&self) -> &str {
        "kv"
    }

    fn methods(&self) -> Vec<MethodSchema> {
        vec![
            MethodSchema::new(
                "get",
                "Get a value by key",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "key": {"type": "string"}
                    },
                    "required": ["key"]
                }),
                serde_json::json!({
                    "description": "The stored value, or null if not found"
                }),
            ),
            MethodSchema::new(
                "set",
                "Set a key-value pair",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "key": {"type": "string"},
                        "value": {}
                    },
                    "required": ["key", "value"]
                }),
                serde_json::json!({"type": "null"}),
            ),
            MethodSchema::new(
                "delete",
                "Delete a key, returning whether it existed",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "key": {"type": "string"}
                    },
                    "required": ["key"]
                }),
                serde_json::json!({"type": "boolean"}),
            ),
            MethodSchema::new(
                "list",
                "List keys, optionally filtered by prefix",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "prefix": {"type": "string"}
                    }
                }),
                serde_json::json!({
                    "type": "array",
                    "items": {"type": "string"}
                }),
            ),
            MethodSchema::new(
                "has",
                "Check whether a key exists",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "key": {"type": "string"}
                    },
                    "required": ["key"]
                }),
                serde_json::json!({"type": "boolean"}),
            ),
        ]
    }

    async fn call(
        &self,
        method: &str,
        input: Value,
    ) -> Result<Value, CapabilityError> {
        match method {
            "get" => {
                let key = input
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(),
                        method: "get".into(),
                        message: "missing required parameter: key".into(),
                    })?;

                let value = self
                    .store
                    .get(key)
                    .map(|v| v.value().clone())
                    .unwrap_or(Value::Null);

                Ok(value)
            }

            "set" => {
                let key = input
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(),
                        method: "set".into(),
                        message: "missing required parameter: key".into(),
                    })?
                    .to_owned();

                let value = input
                    .get("value")
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(),
                        method: "set".into(),
                        message: "missing required parameter: value".into(),
                    })?
                    .clone();

                // Check value size
                let serialized = serde_json::to_vec(&value).unwrap_or_default();
                if serialized.len() > self.max_value_bytes {
                    return Err(CapabilityError::InvocationFailed {
                        capability: "kv".into(),
                        method: "set".into(),
                        message: format!(
                            "value size {} bytes exceeds limit of {} bytes",
                            serialized.len(),
                            self.max_value_bytes
                        ),
                    });
                }

                // Check key count (only if inserting a new key)
                if !self.store.contains_key(&key) && self.store.len() >= self.max_keys {
                    return Err(CapabilityError::InvocationFailed {
                        capability: "kv".into(),
                        method: "set".into(),
                        message: format!(
                            "key count {} would exceed limit of {}",
                            self.store.len() + 1,
                            self.max_keys
                        ),
                    });
                }

                self.store.insert(key, value);
                Ok(Value::Null)
            }

            "delete" => {
                let key = input
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(),
                        method: "delete".into(),
                        message: "missing required parameter: key".into(),
                    })?;

                let existed = self.store.remove(key).is_some();
                Ok(Value::Bool(existed))
            }

            "list" => {
                let prefix = input
                    .get("prefix")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let keys: Vec<Value> = self
                    .store
                    .iter()
                    .filter(|entry| entry.key().starts_with(prefix))
                    .map(|entry| Value::String(entry.key().clone()))
                    .collect();

                Ok(Value::Array(keys))
            }

            "has" => {
                let key = input
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CapabilityError::InvocationFailed {
                        capability: "kv".into(),
                        method: "has".into(),
                        message: "missing required parameter: key".into(),
                    })?;

                Ok(Value::Bool(self.store.contains_key(key)))
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
    async fn test_set_and_get() {
        let kv = KvCapability::default();
        let set_input = serde_json::json!({"key": "hello", "value": "world"});
        kv.call("set", set_input).await.unwrap();

        let get_input = serde_json::json!({"key": "hello"});
        let result = kv.call("get", get_input).await.unwrap();
        assert_eq!(result, serde_json::json!("world"));
    }

    #[tokio::test]
    async fn test_get_missing_key() {
        let kv = KvCapability::default();
        let result = kv.call("get", serde_json::json!({"key": "nope"})).await.unwrap();
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn test_delete() {
        let kv = KvCapability::default();
        kv.call("set", serde_json::json!({"key": "k", "value": 1})).await.unwrap();

        let result = kv.call("delete", serde_json::json!({"key": "k"})).await.unwrap();
        assert_eq!(result, Value::Bool(true));

        let result = kv.call("delete", serde_json::json!({"key": "k"})).await.unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_has() {
        let kv = KvCapability::default();
        kv.call("set", serde_json::json!({"key": "k", "value": 1})).await.unwrap();

        let result = kv.call("has", serde_json::json!({"key": "k"})).await.unwrap();
        assert_eq!(result, Value::Bool(true));

        let result = kv.call("has", serde_json::json!({"key": "nope"})).await.unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[tokio::test]
    async fn test_list_with_prefix() {
        let kv = KvCapability::default();
        kv.call("set", serde_json::json!({"key": "user:1", "value": "a"})).await.unwrap();
        kv.call("set", serde_json::json!({"key": "user:2", "value": "b"})).await.unwrap();
        kv.call("set", serde_json::json!({"key": "item:1", "value": "c"})).await.unwrap();

        let result = kv.call("list", serde_json::json!({"prefix": "user:"})).await.unwrap();
        let keys = result.as_array().unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn test_max_keys_exceeded() {
        let kv = KvCapability::new(2, 1024 * 1024);
        kv.call("set", serde_json::json!({"key": "a", "value": 1})).await.unwrap();
        kv.call("set", serde_json::json!({"key": "b", "value": 2})).await.unwrap();

        let result = kv.call("set", serde_json::json!({"key": "c", "value": 3})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_max_value_bytes_exceeded() {
        let kv = KvCapability::new(1000, 10); // 10 byte limit
        let big_value = "a".repeat(100);
        let result = kv.call("set", serde_json::json!({"key": "k", "value": big_value})).await;
        assert!(result.is_err());
    }
}
