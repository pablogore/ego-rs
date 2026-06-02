//! # ego-runtime
//!
//! The runtime layer of ego-rs — a hexagonal, actor-oriented, deterministic
//! backend framework.
//!
//! ## Layer responsibility
//!
//! The runtime layer provides the platform abstraction for executing
//! actors. It defines the `Runtime` trait, which is implemented by
//! different backend runtimes (e.g., Tokio, Goakt).
//!
//! ## Architecture
//!
//! ```text
//! transport → application → domain → runtime
//! infrastructure → application → domain → runtime
//! runtime → (nothing internal)
//! ```
//!
//! ## Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `runtime` | `Runtime` trait for platform abstraction |
//! | `execution` | ExecutionId type |
//! | `lifecycle` | ExecutionState enum |
//! | `failure` | SendError and SpawnError types |
//! | `handle` | RuntimeHandle for scoped access |
//! | `scheduler` | Scheduling policies |
//! | `isolation` | Isolation strategies |
pub mod runtime;

pub use runtime::execution::ExecutionId;
pub use runtime::failure::{SendError, SendErrorKind, SpawnError, SpawnErrorKind};
pub use runtime::handle::RuntimeHandle;
pub use runtime::lifecycle::ExecutionState;
pub use runtime::runtime::Runtime;
