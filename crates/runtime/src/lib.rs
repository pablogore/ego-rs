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
//! | `interpreter` | `EffectInterpreter` trait for effect execution |
//! | `execution` | ExecutionId type |
//! | `lifecycle` | ExecutionState enum |
//! | `failure` | SendError and SpawnError types |
//! | `handle` | RuntimeHandle for scoped access |
//! | `scheduler` | Scheduling policies |
//! | `isolation` | Isolation strategies |
/// Runtime trait, execution identity, lifecycle, failure modes, and handle.
pub mod runtime;

/// Effect interpreter — interprets `Effect` values by executing
/// the described outcomes (replies, events, state mutations).
pub mod interpreter;

/// Read side projection runtime — scheduling, backpressure, and batch execution.
pub mod read_side;

/// External effect delivery subsystem — dedup, retry, executor registry
/// (CORE-019). Beside the dormant `EffectInterpreter`, not inside it.
pub mod effects;

pub use runtime::execution::ExecutionId;
pub use runtime::failure::{SendError, SendErrorKind, SpawnError, SpawnErrorKind};
pub use runtime::handle::RuntimeHandle;
pub use runtime::lifecycle::ExecutionState;
pub use runtime::runtime::Runtime;

pub use interpreter::{interpret_composed, EffectInterpreter, InterpretationError};
