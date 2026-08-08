//! `OperationKey` identifies one complete, client-supplied business operation
//! for end-to-end idempotent command processing.
//!
//! Deliberately unrelated to [`crate::idempotency::IdempotencyKey`], which
//! identifies a single external effect dispatch attempt derived from the
//! unit-of-work identity. `OperationKey` is the inverse: it is the
//! caller-supplied identity of one whole business operation, potentially
//! spanning multiple aggregates, and it is never derived or minted
//! server-side — a missing key is rejected, not invented.
//!
//! # Construction
//!
//! Use [`OperationKey::parse`]. It rejects empty or whitespace-only strings
//! and any string longer than [`MAX_LEN`] bytes.

use std::fmt;

/// Maximum length, in bytes, of a valid [`OperationKey`].
pub const MAX_LEN: usize = 255;

/// A client-supplied identity for one complete business operation.
///
/// See the module docs for how this differs from
/// [`crate::idempotency::IdempotencyKey`]. `OperationKey` has no `From`
/// conversion to or from `IdempotencyKey` — the two are deliberately
/// unrelated types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct OperationKey(String);

/// Error returned when a raw string is not a valid [`OperationKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKeyError {
    /// The supplied string was empty or whitespace-only.
    Empty,
    /// The supplied string exceeded [`MAX_LEN`] bytes.
    TooLong,
}

impl fmt::Display for OperationKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationKeyError::Empty => write!(f, "operation_key must not be empty"),
            OperationKeyError::TooLong => {
                write!(f, "operation_key must not exceed {MAX_LEN} bytes")
            }
        }
    }
}

impl std::error::Error for OperationKeyError {}

impl OperationKey {
    /// Parses and validates a raw string into an `OperationKey`.
    ///
    /// Rejects empty/whitespace-only strings and strings longer than
    /// [`MAX_LEN`] bytes. This is the sole constructor — the system never
    /// mints a key on the caller's behalf.
    pub fn parse(value: impl Into<String>) -> Result<Self, OperationKeyError> {
        let s = value.into();
        if s.trim().is_empty() {
            return Err(OperationKeyError::Empty);
        }
        if s.len() > MAX_LEN {
            return Err(OperationKeyError::TooLong);
        }
        Ok(Self(s))
    }

    /// Returns the key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for OperationKey {
    type Error = OperationKeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl fmt::Display for OperationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An opaque fingerprint of an operation's content.
///
/// Distinguishes a legitimate replay (same [`OperationKey`], same
/// fingerprint) from a permanent conflict (same key, different fingerprint —
/// see the `idempotent-command-processing` spec's "Fingerprint Determines
/// Replay vs. Conflict" requirement). `OperationFingerprint` carries no
/// opinion about how the fingerprint is computed from a request payload —
/// that is a caller concern; this type only guarantees stable equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OperationFingerprint(String);

impl OperationFingerprint {
    /// Wraps a precomputed fingerprint value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the fingerprint as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OperationFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_key_parses() {
        let key = OperationKey::parse("op-123").unwrap();
        assert_eq!(key.as_str(), "op-123");
    }

    #[test]
    fn empty_key_rejected() {
        let err = OperationKey::parse("").unwrap_err();
        assert_eq!(err, OperationKeyError::Empty);
    }

    #[test]
    fn whitespace_only_key_rejected() {
        let err = OperationKey::parse("   ").unwrap_err();
        assert_eq!(err, OperationKeyError::Empty);
    }

    #[test]
    fn bounded_length_key_accepted() {
        let value = "a".repeat(MAX_LEN);
        let key = OperationKey::parse(value.clone()).unwrap();
        assert_eq!(key.as_str(), value.as_str());
    }

    #[test]
    fn over_length_key_rejected() {
        let value = "a".repeat(MAX_LEN + 1);
        let err = OperationKey::parse(value).unwrap_err();
        assert_eq!(err, OperationKeyError::TooLong);
    }

    #[test]
    fn fingerprints_with_equal_value_are_equal() {
        assert_eq!(
            OperationFingerprint::new("f1"),
            OperationFingerprint::new("f1")
        );
    }

    #[test]
    fn fingerprints_with_different_value_are_not_equal() {
        assert_ne!(
            OperationFingerprint::new("f1"),
            OperationFingerprint::new("f2")
        );
    }
}
