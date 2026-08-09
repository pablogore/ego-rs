//! Idempotency enforcement policy.
//!
//! [`IdempotencyEnforcementMode`] governs whether a missing client-supplied
//! `OperationKey` is rejected or, temporarily and explicitly, admitted.

use std::sync::Arc;
use std::time::Duration;

use ego_domain::operation::{OperationReservationStore, OwnerId};
use ego_domain::time::Clock;

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

/// Everything the runtime needs to reserve an operation, as one value.
///
/// # Why these four travel together
///
/// They are not four independent settings. A store with no clock cannot compute
/// a `lease_until`; an owner with no store means nothing; a lease length without
/// a clock is unusable. Kept as separate optional fields they would admit
/// sixteen combinations, thirteen of them incoherent, and every use site would
/// have to check for the ones that are not.
///
/// The optionality lives **outside** this struct — a runtime holds
/// `Option<ReservationConfig>`, so exactly two states are representable:
/// reservations disabled, or a complete and valid configuration. There are
/// deliberately no `Option` fields inside.
#[derive(Clone)]
// Read through the accessors below, which the tests exercise and which the
// reservation method lands on in the next slice. The annotation is scoped to
// this struct and must be removed then — an `expect` that outlives its reason
// stops being a note and becomes a claim nobody rechecks.
pub struct ReservationConfig {
    store: Arc<dyn OperationReservationStore>,
    clock: Arc<dyn Clock>,
    owner_id: OwnerId,
    lease_duration: Duration,
}

/// Why a [`ReservationConfig`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReservationConfigError {
    /// The lease length was zero.
    ///
    /// A zero lease expires the instant it is taken, so every attempt would see
    /// the previous one as expired and take it over — the reservation would
    /// exclude nobody while appearing to work.
    #[error("the reservation lease duration must be greater than zero")]
    ZeroLease,
}

// The accessors are exercised by the builder's tests; nothing in a release
// build reads them until the reservation method lands in the next slice. Scoped
// to this impl and to non-test builds, and it must be removed then — an
// `expect` that outlives its reason stops being a note and becomes a claim
// nobody rechecks.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "consumed by the reservation method, next slice")
)]
impl ReservationConfig {
    /// Builds a configuration, or refuses to.
    ///
    /// Validating here rather than in `build()` means there is one place a
    /// zero lease can be rejected, and no way for a later caller to assemble an
    /// unvalidated one.
    ///
    /// # Operational contract
    ///
    /// `lease_duration` must exceed the longest a legitimate execution can
    /// take. When a lease expires another owner may take the reservation over
    /// **while the original is still running** — until renewal exists, a lease
    /// shorter than a real operation permits overlap, which is a correctness
    /// problem rather than a tuning preference.
    pub fn new(
        store: Arc<dyn OperationReservationStore>,
        clock: Arc<dyn Clock>,
        owner_id: OwnerId,
        lease_duration: Duration,
    ) -> Result<Self, ReservationConfigError> {
        if lease_duration.is_zero() {
            return Err(ReservationConfigError::ZeroLease);
        }
        Ok(Self {
            store,
            clock,
            owner_id,
            lease_duration,
        })
    }

    /// The durable reservation store.
    pub(crate) fn store(&self) -> &Arc<dyn OperationReservationStore> {
        &self.store
    }

    /// The identity this runtime instance reserves under.
    pub(crate) fn owner_id(&self) -> &OwnerId {
        &self.owner_id
    }

    /// The lease expiry a fresh reservation or takeover would establish,
    /// computed from the configured clock and nothing else — which is what
    /// makes expiry testable without wall time.
    pub(crate) fn lease_until(&self) -> chrono::DateTime<chrono::Utc> {
        self.clock.now() + self.lease_duration
    }
}
