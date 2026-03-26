use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::capability::CapabilityRegistry;
use crate::error::{Result, SandcastleError};
use crate::limits::Limits;

/// A pre-registered script with its associated capabilities and resource limits.
pub struct CompiledScript {
    /// The unique name of this script.
    pub name: String,
    /// The JavaScript source code.
    pub code: String,
    /// Host capabilities available to this script during execution.
    pub capabilities: Arc<CapabilityRegistry>,
    /// Resource limits applied when executing this script.
    pub limits: Limits,
}

/// A thread-safe registry of named, pre-registered scripts.
///
/// Scripts can be registered ahead of time and later retrieved by name for
/// execution. The registry enforces a configurable maximum number of scripts.
pub struct ScriptRegistry {
    scripts: RwLock<HashMap<String, Arc<CompiledScript>>>,
    max_scripts: usize,
}

impl ScriptRegistry {
    /// Create a new script registry with the given maximum capacity.
    pub fn new(max_scripts: usize) -> Self {
        Self {
            scripts: RwLock::new(HashMap::new()),
            max_scripts,
        }
    }

    /// Register a named script with the given code, capabilities, and limits.
    ///
    /// Returns an error if the registry has reached its maximum capacity.
    pub fn register(
        &self,
        name: impl Into<String>,
        code: impl Into<String>,
        capabilities: Arc<CapabilityRegistry>,
        limits: Limits,
    ) -> Result<()> {
        let name = name.into();
        let code = code.into();

        let mut scripts = self.scripts.write().unwrap();

        // Allow replacing an existing script without counting against the limit.
        if !scripts.contains_key(&name) && scripts.len() >= self.max_scripts {
            return Err(SandcastleError::ResourceLimit(format!(
                "script registry full: maximum of {} scripts reached",
                self.max_scripts
            )));
        }

        let compiled = Arc::new(CompiledScript {
            name: name.clone(),
            code,
            capabilities,
            limits,
        });

        scripts.insert(name, compiled);
        Ok(())
    }

    /// Retrieve a previously registered script by name.
    pub fn get(&self, name: &str) -> Option<Arc<CompiledScript>> {
        let scripts = self.scripts.read().unwrap();
        scripts.get(name).cloned()
    }

    /// Remove a script by name. Returns `true` if the script existed.
    pub fn remove(&self, name: &str) -> bool {
        let mut scripts = self.scripts.write().unwrap();
        scripts.remove(name).is_some()
    }

    /// List the names of all registered scripts.
    pub fn list(&self) -> Vec<String> {
        let scripts = self.scripts.read().unwrap();
        scripts.keys().cloned().collect()
    }

    /// Return the number of currently registered scripts.
    pub fn len(&self) -> usize {
        let scripts = self.scripts.read().unwrap();
        scripts.len()
    }

    /// Return `true` if the registry contains no scripts.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_retrieve() {
        let registry = ScriptRegistry::new(10);
        let caps = Arc::new(CapabilityRegistry::new());

        registry
            .register("hello", "return 1;", caps.clone(), Limits::default())
            .unwrap();

        let script = registry.get("hello").unwrap();
        assert_eq!(script.name, "hello");
        assert_eq!(script.code, "return 1;");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn max_scripts_enforced() {
        let registry = ScriptRegistry::new(1);
        let caps = Arc::new(CapabilityRegistry::new());

        registry
            .register("a", "code", caps.clone(), Limits::default())
            .unwrap();

        let result = registry.register("b", "code", caps.clone(), Limits::default());
        assert!(result.is_err());
    }

    #[test]
    fn replace_existing_does_not_count_against_limit() {
        let registry = ScriptRegistry::new(1);
        let caps = Arc::new(CapabilityRegistry::new());

        registry
            .register("a", "v1", caps.clone(), Limits::default())
            .unwrap();

        // Replacing "a" should succeed even though we are at capacity.
        registry
            .register("a", "v2", caps.clone(), Limits::default())
            .unwrap();

        let script = registry.get("a").unwrap();
        assert_eq!(script.code, "v2");
    }

    #[test]
    fn remove_and_list() {
        let registry = ScriptRegistry::new(10);
        let caps = Arc::new(CapabilityRegistry::new());

        registry
            .register("x", "code", caps.clone(), Limits::default())
            .unwrap();
        registry
            .register("y", "code", caps.clone(), Limits::default())
            .unwrap();

        assert_eq!(registry.len(), 2);
        assert!(registry.remove("x"));
        assert!(!registry.remove("x")); // already removed
        assert_eq!(registry.len(), 1);

        let names = registry.list();
        assert_eq!(names, vec!["y"]);
    }

    #[test]
    fn get_missing_returns_none() {
        let registry = ScriptRegistry::new(10);
        assert!(registry.get("nonexistent").is_none());
    }
}
