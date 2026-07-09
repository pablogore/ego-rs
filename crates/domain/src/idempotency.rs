//! [`IdempotencyKey`] uniquely identifies a single external effect dispatch attempt.
//!
//! Derived from the UoW identity and a per-effect sequence number:
//!
//! ```text
//! idempotency_key = f(uow_id, effect_index)
//! ```
//!
//! The receiving external system MUST use this key to detect and reject
//! duplicate dispatches. The EGO-RS system MUST include this key in every
//! external call.
//!
//! # Construction
//!
//! Use [`IdempotencyKey::new`] to create a new key. The key must not be empty.

use std::fmt;

use crate::context::id_type;

id_type!(
    IdempotencyKey,
    IdempotencyKeyError,
    "idempotency_key must not be empty"
);

impl IdempotencyKey {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_key_constructs() {
        let key = IdempotencyKey::new("uow-1:0").unwrap();
        assert_eq!(key.as_str(), "uow-1:0");
    }

    #[test]
    fn empty_key_rejected() {
        let err = IdempotencyKey::new("").unwrap_err();
        assert_eq!(err, IdempotencyKeyError);
    }

    #[test]
    fn whitespace_only_key_rejected() {
        let err = IdempotencyKey::new("   ").unwrap_err();
        assert_eq!(err, IdempotencyKeyError);
    }

    #[test]
    fn deserialize_valid_key() {
        let key: IdempotencyKey = serde_json::from_str("\"uow-1:0\"").unwrap();
        assert_eq!(key.as_str(), "uow-1:0");
    }

    #[test]
    fn deserialize_empty_key_rejected() {
        let result: Result<IdempotencyKey, _> = serde_json::from_str("\"\"");
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_whitespace_only_key_rejected() {
        let result: Result<IdempotencyKey, _> = serde_json::from_str("\"   \"");
        assert!(result.is_err());
    }
}
