//! Core domain types for the runtime-slice crate.
//!
//! Defines the fundamental building blocks of deterministic execution:
//! slice identity, governed inputs, execution context, and outcome.
//!
//! # Purpose
//! These types form the contract for governed execution. Every runtime
//! slice has a unique identity, a declared set of deterministic inputs,
//! and produces an observable outcome that can be validated for replay
//! equivalence.

use serde::{Deserialize, Serialize};

/// A unique identifier for a runtime execution slice.
///
/// Corresponds to a governed execution unit. Non-empty by construction —
/// the [`RuntimeSliceId::new`] constructor rejects empty or whitespace-only
/// values.
///
/// # Example
///
/// ```rust
/// use ego_runtime_slice::types::{RuntimeSliceId, RuntimeSliceError};
///
/// let id = RuntimeSliceId::new("order-processing-001").unwrap();
/// assert_eq!(id.as_str(), "order-processing-001");
///
/// let err = RuntimeSliceId::new("").unwrap_err();
/// assert_eq!(err, RuntimeSliceError::AmbiguousInput("runtime slice id is empty"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuntimeSliceId(String);

impl RuntimeSliceId {
    /// Constructor and accessor for `RuntimeSliceId`.
    /// Creates a new [`RuntimeSliceId`] from a string-like value.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSliceError::AmbiguousInput`] if the value is empty
    /// or only whitespace.
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeSliceError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RuntimeSliceError::AmbiguousInput("runtime slice id is empty"));
        }
        Ok(Self(value))
    }

    /// Returns the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A governed deterministic input for a runtime slice.
///
/// Key-value pairs that are explicitly declared as inputs to execution.
/// Both key and value are non-empty by construction.
///
/// # Example
///
/// ```rust
/// use ego_runtime_slice::types::DeterministicInput;
///
/// let input = DeterministicInput::new("user_id", "abc-123").unwrap();
/// assert_eq!(input.key, "user_id");
/// assert_eq!(input.value, "abc-123");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicInput {
    /// The input key (non-empty).
    pub key: String,
    /// The input value.
    pub value: String,
}

impl DeterministicInput {
    /// Constructor for `DeterministicInput` with key/value validation.
    /// Creates a new [`DeterministicInput`].
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSliceError::AmbiguousInput`] if the key is empty or whitespace.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Result<Self, RuntimeSliceError> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(RuntimeSliceError::AmbiguousInput("deterministic input key is empty"));
        }
        Ok(Self {
            key,
            value: value.into(),
        })
    }
}

/// The execution context for a unit of work.
///
/// Bundles the slice identity with its deterministic inputs. Requires
/// at least one input — empty input sets are rejected.
///
/// # Example
///
/// ```rust
/// use ego_runtime_slice::types::{ExecutionContext, RuntimeSliceId, DeterministicInput};
///
/// let slice_id = RuntimeSliceId::new("batch-42").unwrap();
/// let inputs = vec![DeterministicInput::new("count", "100").unwrap()];
/// let ctx = ExecutionContext::new(slice_id, inputs).unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// The runtime slice this execution belongs to.
    pub slice_id: RuntimeSliceId,
    /// The governed deterministic inputs for this execution.
    pub inputs: Vec<DeterministicInput>,
}

impl ExecutionContext {
    /// Constructor for `ExecutionContext` with non-empty input validation.
    /// Creates a new [`ExecutionContext`].
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSliceError::AmbiguousInput`] if `inputs` is empty.
    pub fn new(
        slice_id: RuntimeSliceId,
        inputs: Vec<DeterministicInput>,
    ) -> Result<Self, RuntimeSliceError> {
        if inputs.is_empty() {
            return Err(RuntimeSliceError::AmbiguousInput(
                "execution context has no governed inputs",
            ));
        }
        Ok(Self { slice_id, inputs })
    }
}

/// The observable outcome of a deterministic execution.
///
/// Produced after a unit of work completes successfully. Contains the
/// runtime slice ID and the observable semantics — the externally
/// visible effects of execution.
///
/// # Example
///
/// ```rust
/// use ego_runtime_slice::types::{ExecutionOutcome, RuntimeSliceId};
///
/// let slice_id = RuntimeSliceId::new("batch-42").unwrap();
/// let outcome = ExecutionOutcome::new(
///     slice_id,
///     vec!["processed 100 items".into(), "published events".into()],
/// ).unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    /// The runtime slice this outcome belongs to.
    pub slice_id: RuntimeSliceId,
    /// The observable semantics produced by execution.
    pub observable_semantics: Vec<String>,
}

impl ExecutionOutcome {
    /// Constructor for `ExecutionOutcome` with non-empty semantics validation.
    /// Creates a new [`ExecutionOutcome`].
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSliceError::AmbiguousOutcome`] if `observable_semantics` is empty.
    pub fn new(
        slice_id: RuntimeSliceId,
        observable_semantics: Vec<String>,
    ) -> Result<Self, RuntimeSliceError> {
        if observable_semantics.is_empty() {
            return Err(RuntimeSliceError::AmbiguousOutcome(
                "execution outcome has no observable semantics",
            ));
        }
        Ok(Self {
            slice_id,
            observable_semantics,
        })
    }
}

/// Fail-closed error variants for runtime slice operations.
///
/// All errors are explicit and descriptive. Ambiguous or undefined
/// states produce rejection — never silent continuation.
///
/// # Variants
///
/// | Variant | Meaning |
/// |---------|---------|
/// | `AmbiguousInput(msg)` | Input validation failed |
/// | `AmbiguousOutcome(msg)` | Outcome validation failed |
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeSliceError {
    /// Input was empty, whitespace-only, or otherwise ambiguous.
    #[error("ambiguous input: {0}")]
    AmbiguousInput(&'static str),

    /// Outcome had no observable semantics.
    #[error("ambiguous outcome: {0}")]
    AmbiguousOutcome(&'static str),
}