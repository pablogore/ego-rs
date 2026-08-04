//! Idempotency enforcement policy.
//!
//! [`IdempotencyEnforcementMode`] governs whether a missing client-supplied
//! `OperationKey` is rejected or, temporarily and explicitly, admitted.

/// Runtime-configured idempotency enforcement policy.
///
/// Mirrors [`crate::runtime::TenantEnforcementMode`]'s shape and posture
/// (`crates/service-sdk/src/runtime/tenant.rs`): a fixed-invariant enum with
/// a fail-closed default. Deliberately **not** `dyn`-dispatched — the
/// missing-key policy is a fixed invariant of this SDK, not a per-deployment
/// plugin a caller can substitute with an arbitrary strategy. Widening it to
/// a trait object would let an adopter quietly implement "admit anything",
/// which would defeat the point: the guarantee has to be verifiable from the
/// enum's two variants, not from an opaque implementation somebody else
/// supplies.
///
/// The escape hatch is exactly one bounded variant rather than free
/// per-endpoint configuration. [`IdempotencyEnforcementMode::Compatibility`]
/// exists for an explicit, temporary migration window, never as a silent
/// default — a per-endpoint switch would leave unguarded operations that look
/// guarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyEnforcementMode {
    /// Fail-closed (default). A missing `OperationKey` is rejected before
    /// dispatch — no aggregate is touched, and the system never mints a key
    /// on the caller's behalf. A server-minted key would be a function of the
    /// request as received, so a retry would produce a different one and
    /// deduplicate nothing.
    MandatoryKey,
    /// Bounded compatibility variant permitting a temporary transition
    /// period. A missing key is admitted only because this variant was
    /// explicitly configured. There is no undocumented default that permits
    /// it, so a deployment cannot end up unguarded by accident.
    Compatibility,
}

impl Default for IdempotencyEnforcementMode {
    /// The fail-closed [`IdempotencyEnforcementMode::MandatoryKey`] variant.
    /// Defaulting the other way would mean a caller who never thought about
    /// idempotency silently gets none.
    fn default() -> Self {
        Self::MandatoryKey
    }
}

#[cfg(test)]
mod tests {
    use super::IdempotencyEnforcementMode;

    #[test]
    fn default_mode_is_fail_closed_mandatory_key() {
        assert_eq!(
            IdempotencyEnforcementMode::default(),
            IdempotencyEnforcementMode::MandatoryKey
        );
    }

    #[test]
    fn compatibility_variant_is_distinct_from_the_default() {
        assert_ne!(
            IdempotencyEnforcementMode::Compatibility,
            IdempotencyEnforcementMode::default()
        );
    }
}
