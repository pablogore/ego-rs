//! # ego-runtime-slice
//!
//! Deterministic execution primitives: types, executor, projection, and
//! validation support for governed runtime slices.
//!
//! ## Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `types` | Core domain types — slice identity, inputs, context, outcome |
//! | `executor` | Deterministic execution engine — `Executor`, `UnitOfWork` |
//! | `projection` | Immutable read-only projections of execution outcomes |
//! | `validation` | Equivalence and replay-safety validators |

/// Deterministic execution domain types.
pub mod types;

/// Deterministic execution engine and state machine.
pub mod executor;

/// Immutable read-only projections of execution outcomes.
pub mod projection;

/// Equivalence and replay-safety validators.
pub mod validation;

#[cfg(test)]
mod tests;

/// Core domain types for runtime slice identity and execution.
pub use types::{
    DeterministicInput, ExecutionContext, ExecutionOutcome, RuntimeSliceError, RuntimeSliceId,
};

/// Execution engine types — `Executor`, `LifecycleState`, `UnitOfWork`.
pub use executor::{Executor, LifecycleState, UnitOfWork};

/// Immutable projection of an execution outcome.
pub use projection::Projection;

/// Equivalence validation functions.
pub use validation::{validate_deterministic_equivalence, validate_replay_equivalence};
