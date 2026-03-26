//! # SandCastle
//!
//! Lightweight WASM-based sandbox runtime for AI agent code execution.
//!
//! SandCastle provides fast, secure sandboxes for running AI-generated JavaScript code.
//! It uses WebAssembly (Wasmtime) as the isolation layer and QuickJS as the JavaScript runtime.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use sandcastle::runtime::{Config, SandCastle};
//! use sandcastle::sandbox::ExecutionRequest;
//!
//! # async fn example() -> sandcastle::error::Result<()> {
//! let guest_module = std::fs::read("guest-js.wasm").unwrap();
//! let runtime = SandCastle::new(Config::new(guest_module))?;
//!
//! let result = runtime.execute(
//!     ExecutionRequest::new("return 1 + 1;")
//! ).await?;
//!
//! println!("Result: {:?}", result.output);
//! # Ok(())
//! # }
//! ```

pub mod capabilities;
pub mod capability;
pub mod error;
pub mod limits;
pub mod namespace;
pub mod pool;
pub mod registry;
pub mod runtime;
pub mod sandbox;
pub mod transcript;
pub mod types;

pub use capability::MethodSchema;
pub use error::{CapabilityError, Result, SandcastleError};
pub use runtime::{Config, SandCastle};
pub use sandbox::{ExecutionRequest, ExecutionResult, PersistentSandbox};

// Re-export the proc macro
pub use sandcastle_macros::capability;
