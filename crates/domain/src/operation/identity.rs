//! `OperationIdentity` — the key and fingerprint of one operation, carried as
//! one value because half of them answers nothing.
//!
//! # Why this is a pair and not two fields
//!
//! The receipt gate needs both halves to decide anything.
//! [`OperationKey`] says *which operation* this is; [`OperationFingerprint`]
//! says *which request* it came from. With only the key, a retry cannot be told
//! apart from a different command reusing the same key — the gate's two
//! outcomes, replay and permanent conflict, are exactly the two the fingerprint
//! distinguishes. So a key without a fingerprint is not a partial identity; it
//! is an identity the gate must ignore entirely.
//!
//! Carried as two independent `Option` fields, "key present, fingerprint
//! missing" is representable, compiles, and reads at a glance like idempotency
//! is switched on. It is not: every consumer has to re-derive the pairing
//! defensively, and a caller that transfers one and forgets the other silently
//! disables the guarantee for that aggregate while looking like it enabled it.
//!
//! Carried as this type, that state does not exist — and it takes both halves of
//! the encapsulation to keep it that way. [`OperationIdentity::new`] requires
//! both values, **and** the fields are private so a struct literal cannot supply
//! them independently and route around it. Those are two separate properties
//! with two separate compile-fail fixtures
//! (`operation_identity_half_constructed.rs` and
//! `operation_identity_fields_public.rs`), because making the fields public
//! would leave the constructor's arity untouched and a fixture that only
//! checked the arity would never notice.
//!
//! Reading a half is fine, and supported: see [`OperationIdentity::key`] and
//! [`OperationIdentity::fingerprint`]. The guarantee is about what can be
//! **built**, not about what can be looked at.

use crate::operation::{OperationFingerprint, OperationKey};

/// The identity of one business operation: which operation, and which request
/// it came from.
///
/// Minted by the reservation that accepted the operation and carried unchanged
/// from there — never rebuilt, and never assembled from parts recovered
/// separately. See the module docs for why the two halves travel together.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperationIdentity {
    key: OperationKey,
    fingerprint: OperationFingerprint,
}

impl OperationIdentity {
    /// Pairs a key with the fingerprint of the request it was reserved under.
    ///
    /// Both are required. That is the whole point of the type: there is no way
    /// to construct an identity carrying only one half, so no consumer has to
    /// handle a state that would mean nothing.
    pub fn new(key: OperationKey, fingerprint: OperationFingerprint) -> Self {
        Self { key, fingerprint }
    }

    /// Which operation this is.
    pub fn key(&self) -> &OperationKey {
        &self.key
    }

    /// Which request this operation came from.
    ///
    /// **Read it; never recompute it.** A fingerprint derived a second time
    /// from a re-serialised request can differ from the first for reasons that
    /// have nothing to do with the request changing — map ordering, float
    /// formatting, a field that gained a default — and a legitimate retry would
    /// then be refused as a different request.
    pub fn fingerprint(&self) -> &OperationFingerprint {
        &self.fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> OperationKey {
        OperationKey::parse("op-identity-1").expect("a non-empty key parses")
    }

    fn fingerprint() -> OperationFingerprint {
        OperationFingerprint::new("f".repeat(64))
    }

    /// The accessors return exactly what was paired, with no normalisation on
    /// either side — this value is carried, not processed.
    #[test]
    fn an_identity_returns_both_halves_unchanged() {
        let identity = OperationIdentity::new(key(), fingerprint());

        assert_eq!(identity.key(), &key());
        assert_eq!(identity.fingerprint(), &fingerprint());
    }

    /// Equality covers both halves. A comparison that ignored the fingerprint
    /// would call a different request under the same key "the same operation",
    /// which is the precise confusion the fingerprint exists to prevent.
    #[test]
    fn two_identities_differing_only_in_fingerprint_are_not_equal() {
        let one = OperationIdentity::new(key(), fingerprint());
        let two = OperationIdentity::new(key(), OperationFingerprint::new("a".repeat(64)));

        assert_ne!(one, two);
    }

    /// …and the same for the key, so a shared fingerprint cannot make two
    /// distinct operations compare equal.
    #[test]
    fn two_identities_differing_only_in_key_are_not_equal() {
        let one = OperationIdentity::new(key(), fingerprint());
        let two = OperationIdentity::new(
            OperationKey::parse("op-identity-2").expect("a non-empty key parses"),
            fingerprint(),
        );

        assert_ne!(one, two);
    }
}
