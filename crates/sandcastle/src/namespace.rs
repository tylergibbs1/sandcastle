use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::Semaphore;

use crate::capability::CapabilityRegistry;
use crate::error::{Result, SandcastleError};
use crate::limits::Limits;
use crate::registry::{CompiledScript, ScriptRegistry};

/// Resource and concurrency limits for a dispatch namespace.
pub struct NamespaceLimits {
    /// Maximum number of scripts that can be registered in this namespace.
    pub max_scripts: usize,
    /// Maximum number of concurrent executions across all scripts in this namespace.
    pub max_concurrent_executions: usize,
    /// Default resource limits applied to scripts that do not specify their own.
    pub default_limits: Limits,
}

impl Default for NamespaceLimits {
    fn default() -> Self {
        Self {
            max_scripts: 1000,
            max_concurrent_executions: 100,
            default_limits: Limits::default(),
        }
    }
}

/// A named container of scripts with concurrency control.
///
/// Each namespace has its own script registry, default capabilities, and a
/// concurrency semaphore that limits how many scripts can execute simultaneously
/// within the namespace.
pub struct DispatchNamespace {
    name: String,
    registry: ScriptRegistry,
    limits: NamespaceLimits,
    concurrency: Arc<Semaphore>,
    default_capabilities: Arc<CapabilityRegistry>,
}

impl DispatchNamespace {
    /// Create a new dispatch namespace.
    pub fn new(
        name: impl Into<String>,
        limits: NamespaceLimits,
        capabilities: Arc<CapabilityRegistry>,
    ) -> Self {
        let concurrency = Arc::new(Semaphore::new(limits.max_concurrent_executions));
        let registry = ScriptRegistry::new(limits.max_scripts);

        Self {
            name: name.into(),
            registry,
            limits,
            concurrency,
            default_capabilities: capabilities,
        }
    }

    /// The name of this namespace.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Register a script using the namespace's default capabilities.
    ///
    /// If `limits` is `None`, the namespace's `default_limits` are used.
    pub fn register(
        &self,
        name: impl Into<String>,
        code: impl Into<String>,
        limits: Option<Limits>,
    ) -> Result<()> {
        let limits = limits.unwrap_or_else(|| self.limits.default_limits.clone());
        self.registry
            .register(name, code, self.default_capabilities.clone(), limits)
    }

    /// Register a script with explicit capabilities.
    ///
    /// If `limits` is `None`, the namespace's `default_limits` are used.
    pub fn register_with_capabilities(
        &self,
        name: impl Into<String>,
        code: impl Into<String>,
        capabilities: Arc<CapabilityRegistry>,
        limits: Option<Limits>,
    ) -> Result<()> {
        let limits = limits.unwrap_or_else(|| self.limits.default_limits.clone());
        self.registry.register(name, code, capabilities, limits)
    }

    /// Remove a script by name. Returns `true` if the script existed.
    pub fn remove(&self, name: &str) -> bool {
        self.registry.remove(name)
    }

    /// Retrieve a registered script by name.
    pub fn get_script(&self, name: &str) -> Option<Arc<CompiledScript>> {
        self.registry.get(name)
    }

    /// List the names of all registered scripts in this namespace.
    pub fn list_scripts(&self) -> Vec<String> {
        self.registry.list()
    }

    /// Get a reference to the namespace limits.
    pub fn limits(&self) -> &NamespaceLimits {
        &self.limits
    }

    /// Acquire a concurrency permit for executing a script in this namespace.
    ///
    /// Returns an error if the namespace has reached its maximum concurrent
    /// executions limit.
    pub fn acquire_permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.concurrency)
            .try_acquire_owned()
            .map_err(|_| {
                SandcastleError::ResourceLimit(format!(
                    "namespace `{}` concurrency limit reached: max {} concurrent executions",
                    self.name, self.limits.max_concurrent_executions
                ))
            })
    }
}

/// Manages multiple dispatch namespaces.
///
/// Provides creation, lookup, and deletion of namespaces, with a configurable
/// maximum number of namespaces.
pub struct NamespaceManager {
    namespaces: RwLock<HashMap<String, Arc<DispatchNamespace>>>,
    max_namespaces: usize,
}

impl NamespaceManager {
    /// Create a new namespace manager with the given maximum capacity.
    pub fn new(max_namespaces: usize) -> Self {
        Self {
            namespaces: RwLock::new(HashMap::new()),
            max_namespaces,
        }
    }

    /// Create a new namespace. Returns an error if the name already exists or
    /// the maximum number of namespaces has been reached.
    pub fn create(
        &self,
        name: impl Into<String>,
        limits: NamespaceLimits,
        capabilities: Arc<CapabilityRegistry>,
    ) -> Result<Arc<DispatchNamespace>> {
        let name = name.into();
        let mut namespaces = self.namespaces.write().unwrap();

        if namespaces.contains_key(&name) {
            return Err(SandcastleError::Config(format!(
                "namespace `{name}` already exists"
            )));
        }

        if namespaces.len() >= self.max_namespaces {
            return Err(SandcastleError::ResourceLimit(format!(
                "namespace manager full: maximum of {} namespaces reached",
                self.max_namespaces
            )));
        }

        let namespace = Arc::new(DispatchNamespace::new(name.clone(), limits, capabilities));
        namespaces.insert(name, namespace.clone());
        Ok(namespace)
    }

    /// Retrieve a namespace by name.
    pub fn get(&self, name: &str) -> Option<Arc<DispatchNamespace>> {
        let namespaces = self.namespaces.read().unwrap();
        namespaces.get(name).cloned()
    }

    /// Delete a namespace by name. Returns `true` if the namespace existed.
    pub fn delete(&self, name: &str) -> bool {
        let mut namespaces = self.namespaces.write().unwrap();
        namespaces.remove(name).is_some()
    }

    /// List the names of all namespaces.
    pub fn list(&self) -> Vec<String> {
        let namespaces = self.namespaces.read().unwrap();
        namespaces.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_caps() -> Arc<CapabilityRegistry> {
        Arc::new(CapabilityRegistry::new())
    }

    #[test]
    fn namespace_register_and_get_script() {
        let ns = DispatchNamespace::new("test", NamespaceLimits::default(), test_caps());

        ns.register("hello", "return 1;", None).unwrap();

        let script = ns.get_script("hello").unwrap();
        assert_eq!(script.code, "return 1;");
    }

    #[test]
    fn namespace_uses_default_limits() {
        let mut limits = NamespaceLimits::default();
        limits.default_limits.memory_mb = 64;

        let ns = DispatchNamespace::new("test", limits, test_caps());
        ns.register("s", "code", None).unwrap();

        let script = ns.get_script("s").unwrap();
        assert_eq!(script.limits.memory_mb, 64);
    }

    #[test]
    fn namespace_custom_limits_override_default() {
        let ns = DispatchNamespace::new("test", NamespaceLimits::default(), test_caps());

        let custom = Limits {
            memory_mb: 128,
            ..Limits::default()
        };
        ns.register("s", "code", Some(custom)).unwrap();

        let script = ns.get_script("s").unwrap();
        assert_eq!(script.limits.memory_mb, 128);
    }

    #[test]
    fn namespace_max_scripts_enforced() {
        let limits = NamespaceLimits {
            max_scripts: 1,
            ..Default::default()
        };
        let ns = DispatchNamespace::new("test", limits, test_caps());

        ns.register("a", "code", None).unwrap();
        let result = ns.register("b", "code", None);
        assert!(result.is_err());
    }

    #[test]
    fn namespace_remove_and_list() {
        let ns = DispatchNamespace::new("test", NamespaceLimits::default(), test_caps());

        ns.register("x", "code", None).unwrap();
        ns.register("y", "code", None).unwrap();

        assert!(ns.remove("x"));
        assert!(!ns.remove("x"));

        let scripts = ns.list_scripts();
        assert_eq!(scripts.len(), 1);
        assert!(scripts.contains(&"y".to_string()));
    }

    #[test]
    fn namespace_acquire_permit() {
        let limits = NamespaceLimits {
            max_concurrent_executions: 1,
            ..Default::default()
        };
        let ns = DispatchNamespace::new("test", limits, test_caps());

        let permit = ns.acquire_permit();
        assert!(permit.is_ok());

        // Second permit should fail (try_acquire).
        let permit2 = ns.acquire_permit();
        assert!(permit2.is_err());

        // Drop first permit, now a new one should succeed.
        drop(permit);
        let permit3 = ns.acquire_permit();
        assert!(permit3.is_ok());
    }

    #[test]
    fn namespace_name() {
        let ns = DispatchNamespace::new("my-ns", NamespaceLimits::default(), test_caps());
        assert_eq!(ns.name(), "my-ns");
    }

    #[test]
    fn manager_create_and_get() {
        let manager = NamespaceManager::new(10);

        let ns = manager
            .create("prod", NamespaceLimits::default(), test_caps())
            .unwrap();
        assert_eq!(ns.name(), "prod");

        let retrieved = manager.get("prod").unwrap();
        assert_eq!(retrieved.name(), "prod");
    }

    #[test]
    fn manager_duplicate_name_errors() {
        let manager = NamespaceManager::new(10);
        manager
            .create("ns", NamespaceLimits::default(), test_caps())
            .unwrap();

        let result = manager.create("ns", NamespaceLimits::default(), test_caps());
        assert!(result.is_err());
    }

    #[test]
    fn manager_max_namespaces_enforced() {
        let manager = NamespaceManager::new(1);
        manager
            .create("a", NamespaceLimits::default(), test_caps())
            .unwrap();

        let result = manager.create("b", NamespaceLimits::default(), test_caps());
        assert!(result.is_err());
    }

    #[test]
    fn manager_delete_and_list() {
        let manager = NamespaceManager::new(10);
        manager
            .create("x", NamespaceLimits::default(), test_caps())
            .unwrap();
        manager
            .create("y", NamespaceLimits::default(), test_caps())
            .unwrap();

        assert_eq!(manager.list().len(), 2);
        assert!(manager.delete("x"));
        assert!(!manager.delete("x"));
        assert_eq!(manager.list().len(), 1);
        assert!(manager.get("x").is_none());
    }
}
