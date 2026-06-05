use std::fmt;

/// Uniquely identifies a single external effect dispatch attempt.
///
/// Derived from the UoW identity and a per-effect sequence number:
///
/// ```text
/// idempotency_key = f(uow_id, effect_index)
/// ```
///
/// The receiving external system MUST use this key to detect and reject
/// duplicate dispatches. The EGO-RS system MUST include this key in every
/// external call.
///
/// # Construction
///
/// Use [`IdempotencyKey::new`] to create a new key. The key must not be empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Create a new `IdempotencyKey`.
    ///
    /// Returns `Err(IdempotencyKeyError)` if the value is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, IdempotencyKeyError> {
        let s = value.into();
        if s.trim().is_empty() {
            Err(IdempotencyKeyError)
        } else {
            Ok(Self(s))
        }
    }

    /// View the key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the key and return the inner string.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Error returned when attempting to create an empty [`IdempotencyKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdempotencyKeyError;

impl fmt::Display for IdempotencyKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "idempotency_key must not be empty")
    }
}

impl std::error::Error for IdempotencyKeyError {}
