//! Runtime layer: platform abstraction for executing actors.
//!
//! Defines the `Runtime` trait and supporting types — execution identity,
//! lifecycle states, error types, and a scoped handle for self-access.
//!
//! ## Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `runtime` | `Runtime` trait and `TokioRuntime` implementation |
//! | `execution` | `ExecutionId` — unique identity for each execution |
//! | `lifecycle` | `ExecutionState` — Active, Draining, Terminated, Failed |
//! | `failure` | `SendError`, `SpawnError` — typed failure modes |
//! | `handle` | `RuntimeHandle` — closure-based scoped execution access |
//! | `scheduler` | Scheduling policies (reserved) |
//! | `isolation` | Isolation strategies (reserved) |

#[allow(clippy::module_inception)]
/// Backend-neutral runtime trait and Tokio-backed implementation.
pub mod runtime;

/// Execution identity — unique `ExecutionId` backed by UUID.
pub mod execution;

/// Execution lifecycle states — Active, Draining, Terminated, Failed.
pub mod lifecycle;

/// Typed failure modes for send and spawn operations.
pub mod failure;

/// Closure-based scoped handle for execution self-access.
pub mod handle;

/// Scheduling policies (reserved for future use).
pub mod scheduler;

/// Isolation strategies (reserved for future use).
pub mod isolation;
