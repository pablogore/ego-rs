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
//! | `execution` | Execution context and state |
//! | `isolation` | Isolation strategies for actors |
//! | `scheduling` | Scheduling policies for actor execution |
//! | `error` | Runtime-specific error types |
//!
//! ## Design Context
//!
//! The system has no runtime abstraction. Previous design treated `ActorSystem` as the runtime entry point, coupling the platform API to actor concepts. This design resets to a runtime abstraction contract: the `Runtime` trait is the platform entry point, actor frameworks are optional backend implementations.
//!
//! The `ego-domain` crate already defines actor semantics (`Actor`, `ActorId`, `ActorLifecycleState`, `SupervisionStrategy`). Domain code consumes `impl Runtime` for execution. The Runtime trait is backend-agnostic — it does not reference actor types.
pub mod runtime;
pub mod execution;
pub mod isolation;
pub mod scheduling;
pub mod error;

pub use runtime::Runtime;
pub use execution::{ExecutionId, ExecutionState};
pub use isolation::Isolation;
pub use scheduling::SchedulingPolicy;
pub use error::RuntimeError;
