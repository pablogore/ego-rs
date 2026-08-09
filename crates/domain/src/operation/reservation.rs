//! `OperationReservationStore` — the port through which one client-supplied
//! [`crate::operation::OperationKey`] is reserved under a fenced lease before
//! dispatch.
//!
//! This module defines the **port and its supporting types only**. Concrete
//! implementations (an in-memory test double, a durable Postgres-backed
//! store) live outside `ego-domain` — `ego-domain` depends on nothing
//! internal, so it never implements its own capability ports beyond a
//! trivial production default (compare [`crate::time::SystemClock`]).
//!
//! Every mutating call (`renew`, `complete`, `abandon`) MUST verify the full
//! triple `operation_id + owner_id + fencing_token` before mutating state —
//! storing a fencing token without comparing it on every call does not
//! satisfy the "Lease With Owner, Expiry, and Verified Fencing" requirement.
//! A caller whose lease has been taken over receives
//! [`ReservationError::StaleOwner`] and its call MUST NOT modify the
//! reservation.
//!
//! # Renewal is caller-driven, not automatic
//!
//! [`OperationReservationStore::renew`] exists as a capability for a caller
//! that needs to extend its own lease. No runtime component in this change
//! invokes it automatically: the chosen default is that lease length is
//! deployment configuration, and a long-running operation either completes
//! inside its configured lease or is legitimately taken over by a later
//! caller. Background/automatic renewal is a deliberately deferred
//! extension, not an oversight.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::context::TenantId;
use crate::operation::key::{OperationFingerprint, OperationKey};

/// The capability port for reserving, renewing, completing, abandoning, and
/// purging operation-scoped reservations.
///
/// See the module docs for the fencing-verification requirement every
/// mutating method MUST implement.
#[async_trait]
pub trait OperationReservationStore: Send + Sync {
    /// Attempts to reserve `req`'s operation, or observes its current state.
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError>;

    /// Extends an owned, still-valid lease to `until`.
    ///
    /// MUST verify the full `fence` triple before mutating; a stale fence
    /// returns [`ReservationError::StaleOwner`] and leaves the reservation
    /// unmodified.
    ///
    /// MUST also reject an already-expired lease the same way, even when the
    /// triple still matches. Once the lease bound has passed the holder is no
    /// longer the owner in any meaningful sense — another caller is entitled to
    /// take over at that instant — so permitting a renewal here would let a
    /// lapsed holder resurrect a dead lease and defeat a takeover that was
    /// already legitimate.
    async fn renew(&self, fence: &OwnerFence, until: DateTime<Utc>)
        -> Result<(), ReservationError>;

    /// Marks the reservation permanently completed with `response`, making it
    /// eligible for later replay until its retention TTL elapses.
    ///
    /// MUST verify the full `fence` triple before mutating; a stale fence
    /// returns [`ReservationError::StaleOwner`] and leaves the reservation
    /// unmodified.
    ///
    /// MUST also reject an already-expired lease the same way. A lapsed holder
    /// recording a completion would publish a result for an operation it no
    /// longer owns, which a subsequent replay would then serve as authoritative.
    async fn complete(
        &self,
        fence: &OwnerFence,
        response: StoredServiceResponse,
    ) -> Result<(), ReservationError>;

    /// Abandons the reservation, freeing its key for a future, unrelated
    /// operation.
    ///
    /// MUST verify the full `fence` triple before mutating; a stale fence
    /// returns [`ReservationError::StaleOwner`] and leaves the reservation
    /// unmodified.
    ///
    /// MUST also reject an already-expired lease the same way. A lapsed holder
    /// releasing the key would discard a reservation another caller is entitled
    /// to take over.
    async fn abandon(&self, fence: &OwnerFence) -> Result<(), ReservationError>;

    /// Purges completed reservations whose `completed_at` is older than
    /// `cutoff`, up to `batch` rows. Never purges an `InProgress`
    /// reservation, regardless of age — only lease expiry and takeover may
    /// resolve one of those. Returns the number of rows purged.
    async fn purge_completed_before(
        &self,
        cutoff: DateTime<Utc>,
        batch: usize,
    ) -> Result<u64, ReservationError>;

    /// Reports whether the backing store is reachable right now.
    ///
    /// Read-only and cheap. An implementation MUST NOT create, mutate, purge
    /// or observe any reservation: this runs on every readiness probe, and a
    /// health check that writes would make the act of asking change the thing
    /// being asked about. It answers exactly one question — can this store be
    /// talked to — and nothing about any particular operation.
    ///
    /// An error means "not reachable", whatever the cause. The variant is
    /// [`ReservationError::Backend`] in practice; the port does not narrow it,
    /// because an implementation's failure modes are its own.
    ///
    /// **No default implementation, deliberately.** A default returning `Ok`
    /// would make every store that forgot to write this one report itself
    /// reachable forever — a readiness probe that can only ever say yes is
    /// worse than none, because it is indistinguishable from a working one
    /// until the outage. The compiler refusing an incomplete `impl` is the
    /// cheapest place to catch that.
    async fn probe(&self) -> Result<(), ReservationError>;
}

/// The deterministic identity of one reservation: the `CanonicalTenant` it is
/// namespaced under (`None` for the systemwide scope) plus its
/// [`OperationKey`].
///
/// Two operations with the identical key under two different tenants (or one
/// real tenant and the systemwide scope) are distinct `OperationId`s — this
/// is what makes cross-tenant replay structurally impossible rather than a
/// discipline requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperationId {
    tenant: Option<TenantId>,
    operation_key: OperationKey,
}

impl OperationId {
    /// Builds the identity from its resolved tenant scope and operation key.
    pub fn new(tenant: Option<TenantId>, operation_key: OperationKey) -> Self {
        Self {
            tenant,
            operation_key,
        }
    }

    /// The resolved tenant scope, or `None` for the systemwide scope.
    pub fn tenant(&self) -> Option<&TenantId> {
        self.tenant.as_ref()
    }

    /// The operation key this identity was built from.
    pub fn operation_key(&self) -> &OperationKey {
        &self.operation_key
    }
}

/// Identifies the caller instance holding (or attempting to hold) a lease.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnerId(String);

impl OwnerId {
    /// Wraps a raw owner identity value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the owner id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OwnerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A monotonically increasing fencing token.
///
/// Every takeover mints a strictly greater token than the one it displaces
/// (spec: "Atomic takeover fences out the prior owner"), so a revived stale
/// owner's fence can never compare equal to the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FencingToken(u64);

impl FencingToken {
    /// The first token minted for a freshly created reservation.
    pub fn initial() -> Self {
        Self(1)
    }

    /// The strictly-greater token minted on takeover, or `None` when the
    /// sequence is exhausted.
    ///
    /// Deliberately checked rather than a bare `+ 1`. The type's promise is
    /// that a new token is *strictly greater* than the one it displaces, and a
    /// wrapping increment would break exactly that: a wrapped token could
    /// compare equal to a fence a prior owner still holds, un-fencing an owner
    /// the takeover was supposed to exclude. Returning `None` forces the caller
    /// to surface exhaustion instead of silently minting a token that no longer
    /// fences anything. In debug builds the unchecked form would panic instead;
    /// neither outcome belongs in a mechanism whose whole purpose is exclusion.
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    /// The token's raw numeric value.
    pub fn value(self) -> u64 {
        self.0
    }

    /// Rebuilds a token from a previously persisted raw value.
    ///
    /// A durable store reads tokens back as integers, so it needs a way in.
    /// It also lets a test position the sequence at its boundary without
    /// advancing it a prohibitive number of times.
    pub fn from_value(value: u64) -> Self {
        Self(value)
    }
}

/// An opaque, previously-computed handler response stored on completion so a
/// later replay with the identical key and fingerprint can return it without
/// re-executing the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredServiceResponse(Vec<u8>);

impl StoredServiceResponse {
    /// Wraps a precomputed response payload.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Returns the response payload as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A live lease over one reservation: who holds it, under which fencing
/// token, and until when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// The reservation this lease governs.
    pub operation_id: OperationId,
    /// The current holder.
    pub owner_id: OwnerId,
    /// The fencing token verifying this holder's right to mutate the
    /// reservation.
    pub fencing_token: FencingToken,
    /// The instant this lease expires and becomes takeover-eligible.
    pub lease_until: DateTime<Utc>,
}

/// The full verification triple every mutating call presents.
///
/// D6/spec requirement: a mutating call MUST verify `operation_id`,
/// `owner_id`, and `fencing_token` together — presenting only a subset
/// (e.g. just the fencing token, or just the operation id) does not satisfy
/// this requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerFence {
    /// The reservation being mutated.
    pub operation_id: OperationId,
    /// The caller claiming ownership.
    pub owner_id: OwnerId,
    /// The fencing token the caller was granted when it last (re)gained the
    /// lease.
    pub fencing_token: FencingToken,
}

/// The input to [`OperationReservationStore::reserve`].
#[derive(Debug, Clone)]
pub struct ReserveRequest {
    /// The resolved `CanonicalTenant`, or `None` for the systemwide scope.
    /// MUST be the resolved tenant, never the raw client-supplied hint.
    pub tenant: Option<TenantId>,
    /// The client-supplied operation identity.
    pub operation_key: OperationKey,
    /// The fingerprint of this attempt's content, compared against any
    /// existing reservation under the same key.
    pub fingerprint: OperationFingerprint,
    /// The caller instance attempting to acquire or observe the lease.
    pub owner_id: OwnerId,
    /// The lease expiry this attempt would establish if it wins a fresh
    /// reservation or a takeover. Computed by the caller (`clock.now() +
    /// configured lease length`), not by the store.
    pub lease_until: DateTime<Utc>,
}

/// The result of a [`OperationReservationStore::reserve`] attempt.
///
/// Deliberately extends the shape of `EffectDedupStore`'s
/// (`crates/runtime/src/effects/store.rs`) `DedupOutcome` with `TakenOver`:
/// takeover must be independently observable, both for the AD-10 telemetry
/// counter and for the lease-expiry recovery scenario this capability exists
/// to serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationOutcome {
    /// No prior reservation existed; this attempt now owns it.
    Fresh(Lease),
    /// A prior reservation's lease had expired; this attempt atomically took
    /// it over with a strictly greater fencing token.
    TakenOver(Lease),
    /// This exact owner already holds this reservation's still-valid lease.
    ///
    /// **Observing the same owner is not permission to proceed.** Fencing
    /// proves *ownership*; it does not prove *exclusion between two executions
    /// of the same owner*. This outcome cannot tell a legitimate recovery apart
    /// from a concurrent retry, or from the earlier execution still running and
    /// merely slow — and re-entering an operation that died midway is unsafe
    /// once it may already have reached an external effect. The receipt written
    /// in the aggregate's unit of work protects work that was *confirmed*; it
    /// does not make half-finished work safe to repeat.
    ///
    /// Recovery happens by waiting instead: while the lease holds nobody
    /// re-executes, and once it expires `reserve` answers
    /// [`TakenOver`](ReservationOutcome::TakenOver) with a strictly greater
    /// fencing token, so the new execution is protected from the previous
    /// owner.
    ///
    /// The variant is kept distinct from
    /// [`OtherInProgress`](ReservationOutcome::OtherInProgress) because
    /// self-contention and external contention are worth telling apart for
    /// metrics, diagnostics, lease renewal, and any future explicit recovery —
    /// not because they dispatch differently. Both block.
    OwnedInProgress(Lease),
    /// A different owner holds this reservation's still-valid lease. The
    /// caller MUST NOT proceed and MUST NOT treat this as success.
    OtherInProgress,
    /// The reservation already completed with the identical fingerprint —
    /// safe to return the stored response without re-executing.
    Succeeded(StoredServiceResponse),
    /// The reservation exists under the same key but a *different*
    /// fingerprint — a permanent conflict, never a silent dedupe.
    Conflict,
}

/// Errors returned by [`OperationReservationStore`] operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReservationError {
    /// The presented `OwnerFence` no longer matches the reservation's current
    /// owner/fencing token — typically because the lease was taken over.
    /// The reservation is guaranteed unmodified by this call.
    #[error("stale owner: the presented fence no longer matches the current reservation")]
    StaleOwner,
    /// The fencing sequence for this reservation is exhausted, so no strictly
    /// greater token can be minted and takeover cannot proceed safely.
    ///
    /// Unreachable in practice — it takes `u64::MAX` takeovers of a single
    /// reservation — but represented explicitly because the alternative is
    /// minting a token that silently stops fencing.
    #[error("fencing token sequence exhausted for this reservation")]
    FencingExhausted,
    /// An underlying storage failure, opaque to the port.
    #[error("reservation backend error: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::context::TenantId;
    use crate::operation::key::{OperationFingerprint, OperationKey};

    fn ts(hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, hour, 0, 0).unwrap()
    }

    #[test]
    fn fencing_token_takeover_is_strictly_greater_than_the_original() {
        let f1 = FencingToken::initial();
        let f2 = f1.next().expect("a fresh sequence has room to advance");
        assert!(f2 > f1, "takeover token must be strictly greater");
        assert_ne!(f1, f2);
    }

    #[test]
    fn fencing_token_reports_exhaustion_instead_of_wrapping() {
        let last = FencingToken::from_value(u64::MAX);

        assert_eq!(
            None,
            last.next(),
            "the final token must report exhaustion, never wrap: a wrapped \
             token could compare equal to a fence a prior owner still holds, \
             un-fencing the owner a takeover exists to exclude"
        );
    }

    #[test]
    fn fencing_token_advance_is_strictly_monotonic_across_repeated_takeovers() {
        let mut token = FencingToken::initial();
        for _ in 0..1_000 {
            let advanced = token.next().expect("the sequence has room to advance");
            assert!(
                advanced > token,
                "every advance must be strictly greater than its predecessor"
            );
            token = advanced;
        }
    }

    #[test]
    fn operation_id_is_scoped_by_tenant_and_key() {
        let key = OperationKey::parse("op-1").unwrap();
        let tenant_a = OperationId::new(Some(TenantId::new("tenant-a").unwrap()), key.clone());
        let tenant_b = OperationId::new(Some(TenantId::new("tenant-b").unwrap()), key.clone());
        let systemwide = OperationId::new(None, key);

        assert_ne!(tenant_a, tenant_b, "distinct tenants must not collide");
        assert_ne!(
            tenant_a, systemwide,
            "a real tenant must not collide with the systemwide scope"
        );
    }

    #[test]
    fn owner_fence_carries_the_full_verification_triple() {
        let key = OperationKey::parse("op-2").unwrap();
        let operation_id = OperationId::new(None, key);
        let fence = OwnerFence {
            operation_id: operation_id.clone(),
            owner_id: OwnerId::new("owner-1"),
            fencing_token: FencingToken::initial(),
        };
        assert_eq!(fence.operation_id, operation_id);
        assert_eq!(fence.owner_id, OwnerId::new("owner-1"));
        assert_eq!(fence.fencing_token, FencingToken::initial());
    }

    #[test]
    fn reservation_outcome_variants_are_constructible_and_comparable() {
        let key = OperationKey::parse("op-3").unwrap();
        let lease = Lease {
            operation_id: OperationId::new(None, key),
            owner_id: OwnerId::new("owner-1"),
            fencing_token: FencingToken::initial(),
            lease_until: ts(1),
        };

        assert_eq!(
            ReservationOutcome::Fresh(lease.clone()),
            ReservationOutcome::Fresh(lease.clone())
        );
        assert_ne!(
            ReservationOutcome::Fresh(lease),
            ReservationOutcome::Conflict
        );
        assert_eq!(
            ReservationOutcome::OtherInProgress,
            ReservationOutcome::OtherInProgress
        );
    }

    #[test]
    fn stored_response_equality_is_by_content() {
        assert_eq!(
            StoredServiceResponse::new(b"ok".to_vec()),
            StoredServiceResponse::new(b"ok".to_vec())
        );
        assert_ne!(
            StoredServiceResponse::new(b"ok".to_vec()),
            StoredServiceResponse::new(b"no".to_vec())
        );
    }

    #[test]
    fn reservation_error_stale_owner_is_distinguishable_from_backend_errors() {
        assert_eq!(ReservationError::StaleOwner, ReservationError::StaleOwner);
        assert_ne!(
            ReservationError::StaleOwner,
            ReservationError::Backend("boom".into())
        );
    }

    #[test]
    fn reserve_request_carries_fingerprint_and_lease_bound() {
        let req = ReserveRequest {
            tenant: None,
            operation_key: OperationKey::parse("op-4").unwrap(),
            fingerprint: OperationFingerprint::new("fp-1"),
            owner_id: OwnerId::new("owner-1"),
            lease_until: ts(2),
        };
        assert_eq!(req.fingerprint, OperationFingerprint::new("fp-1"));
        assert_eq!(req.lease_until, ts(2));
    }
}
