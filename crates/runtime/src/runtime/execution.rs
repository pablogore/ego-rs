//! Execution identity.
//!
//! Provides `ExecutionId`, a unique, location-transparent identifier for
//! each execution spawned by a `Runtime`. Backed by a UUID v4 for global
//! uniqueness.

use std::fmt;

use uuid::Uuid;

/// The unique identifier for an execution.
///
/// # Uniqueness
/// Generated as a random UUID v4. Collision probability is negligible
/// for practical purposes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ExecutionId(Uuid);

impl ExecutionId {
    /// Constructor and accessor for `ExecutionId`.

    /// Creates a new `ExecutionId` with a randomly generated v4 UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ExecutionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Deref for ExecutionId {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
