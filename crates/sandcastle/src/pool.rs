use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Configuration for the warm pool.
pub struct WarmPoolConfig {
    /// Number of warm instances to maintain per script. 0 = disabled.
    pub pool_size: usize,
}

impl Default for WarmPoolConfig {
    fn default() -> Self {
        Self { pool_size: 0 } // disabled by default
    }
}

/// Metrics tracking concurrent and total executions through the pool.
pub struct PoolMetrics {
    /// Number of currently active (in-flight) executions.
    pub active_executions: AtomicUsize,
    /// Lifetime total number of executions started.
    pub total_executions: AtomicU64,
}

impl PoolMetrics {
    pub fn new() -> Self {
        Self {
            active_executions: AtomicUsize::new(0),
            total_executions: AtomicU64::new(0),
        }
    }

    /// Record the start of an execution. Returns a guard that decrements the
    /// active count when dropped.
    pub fn execution_started(&self) -> ExecutionGuard<'_> {
        self.active_executions.fetch_add(1, Ordering::Relaxed);
        self.total_executions.fetch_add(1, Ordering::Relaxed);
        ExecutionGuard { metrics: self }
    }

    /// Current number of active executions.
    pub fn active(&self) -> usize {
        self.active_executions.load(Ordering::Relaxed)
    }

    /// Lifetime total number of executions.
    pub fn total(&self) -> u64 {
        self.total_executions.load(Ordering::Relaxed)
    }
}

/// RAII guard that decrements the active execution count when dropped.
pub struct ExecutionGuard<'a> {
    metrics: &'a PoolMetrics,
}

impl Drop for ExecutionGuard<'_> {
    fn drop(&mut self) {
        self.metrics
            .active_executions
            .fetch_sub(1, Ordering::Relaxed);
    }
}

/// Warm pool for sandbox execution.
///
/// Since Wasmtime Stores are stateful and cannot be reset after execution, the
/// real optimization is in pre-compiling WASM modules (which is already handled
/// by the runtime's `Module::new()` call). This pool serves as a configuration
/// and metrics holder, with the structure ready for future pre-instantiation
/// support when Wasmtime adds Store snapshot/restore capabilities.
pub struct WarmPool {
    config: WarmPoolConfig,
    metrics: Arc<PoolMetrics>,
}

impl WarmPool {
    /// Create a new warm pool with the given configuration.
    pub fn new(config: WarmPoolConfig) -> Self {
        Self {
            config,
            metrics: Arc::new(PoolMetrics::new()),
        }
    }

    /// Get a reference to the pool metrics.
    pub fn metrics(&self) -> &PoolMetrics {
        &self.metrics
    }

    /// Get a reference to the pool configuration.
    pub fn config(&self) -> &WarmPoolConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_disabled() {
        let config = WarmPoolConfig::default();
        assert_eq!(config.pool_size, 0);
    }

    #[test]
    fn metrics_track_executions() {
        let pool = WarmPool::new(WarmPoolConfig::default());

        assert_eq!(pool.metrics().active(), 0);
        assert_eq!(pool.metrics().total(), 0);

        {
            let _guard = pool.metrics().execution_started();
            assert_eq!(pool.metrics().active(), 1);
            assert_eq!(pool.metrics().total(), 1);

            let _guard2 = pool.metrics().execution_started();
            assert_eq!(pool.metrics().active(), 2);
            assert_eq!(pool.metrics().total(), 2);
        }

        // Guards dropped, active should be back to 0.
        assert_eq!(pool.metrics().active(), 0);
        assert_eq!(pool.metrics().total(), 2);
    }

    #[test]
    fn config_accessible() {
        let pool = WarmPool::new(WarmPoolConfig { pool_size: 5 });
        assert_eq!(pool.config().pool_size, 5);
    }
}
