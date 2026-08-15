//! Idempotency enforcement policy.
//!
//! [`IdempotencyEnforcementMode`] governs whether a missing client-supplied
//! `OperationKey` is rejected or, temporarily and explicitly, admitted.

use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;

use ego_domain::context::TenantId;
use ego_domain::operation::{
    OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence, OwnerId,
    ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
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
    ///
    /// Read by the builder's tests, which assert what `build()` wired, and by
    /// [`RuntimeInner::complete_idempotent_operation`](crate::runtime::RuntimeInner::complete_idempotent_operation),
    /// which is the production caller the `expect(dead_code)` this used to carry
    /// was waiting for. The attribute was `expect` rather than `allow` precisely
    /// so its removal would be forced rather than remembered.
    pub(crate) fn store(&self) -> &Arc<dyn OperationReservationStore> {
        &self.store
    }

    /// The identity this runtime instance reserves under.
    ///
    /// Read by the builder's tests, which assert that two runtimes never share
    /// an owner. See [`ReservationConfig::store`] for why the attribute is
    /// scoped this way.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "asserted by the builder's wiring tests")
    )]
    pub(crate) fn owner_id(&self) -> &OwnerId {
        &self.owner_id
    }

    /// The lease expiry a fresh reservation or takeover would establish,
    /// computed from the configured clock and nothing else — which is what
    /// makes expiry testable without wall time.
    /// The clock every expiry and retention decision reads.
    ///
    /// Exposed so the retention worker computes its cutoff from the same source
    /// the reservations were stamped by. A worker reading wall time while the
    /// store reads an injected clock would disagree with it under test and,
    /// worse, under clock skew.
    pub(crate) fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub(crate) fn lease_until(&self) -> chrono::DateTime<chrono::Utc> {
        self.clock.now() + self.lease_duration
    }
}

/// What the runtime decided a marked operation may do.
///
/// Two shapes, because a reservation can permit work or answer for work already
/// done, and those are not the same permission. Folding the stored response into
/// the permit would make "continue an operation that must not execute"
/// representable — the caller would hold something that says *proceed* while
/// meaning *do not*.
#[derive(Debug, Clone)]
pub enum ReservationDecision {
    /// The operation may run. The permit carries what completing it later
    /// requires.
    Proceed(ReservationPermit),
    /// The identical request already completed. Return this response; do not
    /// execute anything.
    Replay(StoredServiceResponse),
}

/// Proof that this runtime holds the reservation, and the fence it must present
/// to complete it.
#[derive(Debug, Clone)]
pub struct ReservationPermit {
    fence: OwnerFence,
}

impl ReservationPermit {
    /// The fence a later `complete` must present. Conditional on it, so a lease
    /// taken over in the meantime cannot have its result overwritten.
    ///
    /// The epilogue this was reserved for now exists: see
    /// [`RuntimeInner::complete_idempotent_operation`](crate::runtime::RuntimeInner::complete_idempotent_operation).
    pub(crate) fn fence(&self) -> &OwnerFence {
        &self.fence
    }
}

/// Why a marked operation was refused before it ran.
///
/// The six cases stay distinguishable rather than collapsing into a message,
/// because they call for different responses and different operator action —
/// "retry shortly", "never retry" and "an operator must intervene" must not
/// require parsing prose to tell apart.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReservationRejection {
    /// This runtime already holds a valid lease for this key.
    ///
    /// Self-contention, not external. It still blocks: fencing proves ownership,
    /// not exclusion between two executions of the same owner (AD-3h).
    #[error("this operation is already in progress under this runtime")]
    SelfInProgress,
    /// Another owner holds a valid lease for this key.
    #[error("this operation is already in progress under another owner")]
    OtherInProgress,
    /// The same key arrived carrying a different request. Permanent: retrying
    /// the same thing will never succeed.
    #[error("this operation key is already bound to a different request")]
    FingerprintConflict,
    /// The reservation store itself could not answer.
    ///
    /// Deliberately not merged with the three above. Those are answers; this is
    /// the absence of one, and a caller that retried a conflict forever would be
    /// wrong where a caller retrying this one is right.
    #[error("the reservation store is unavailable")]
    StoreUnavailable,
    /// The operation already completed, but its stored response cannot be read
    /// back (AD-3k).
    ///
    /// Deliberately neither of its neighbours. Not `StoreUnavailable`: the store
    /// answered, correctly and promptly, and what it returned is unreadable. Not
    /// `FingerprintConflict`: the request is the very one that succeeded — only
    /// our ability to decode its answer changed.
    ///
    /// **Permanent for the caller, recoverable by an operator.** Retrying the
    /// identical request re-reads the identical bytes and fails identically, so
    /// a client retry loop is pointless. Purging that reservation, or deploying
    /// the build that can decode it, restores the operation — which is a
    /// different action from the one a fingerprint conflict calls for, and the
    /// caller cannot choose between them if the type does not say which.
    #[error("the operation completed, but its stored response could not be decoded")]
    StoredResponseIncompatible,
    /// The request's typed arguments could not be reduced to a canonical form,
    /// so no fingerprint exists to reserve under.
    ///
    /// The store is never reached in this case — this happens strictly before
    /// the reservation, which is why it cannot be `StoreUnavailable`, and why
    /// retrying is pointless where retrying that one is right. It is not
    /// `FingerprintConflict` either: a conflict means two fingerprints were
    /// computed and differed, and here none was computed at all.
    ///
    /// **Reachable only through a `Serialize` implementation that fails.** The
    /// derived implementations do not; a hand-written one that calls
    /// `serde::ser::Error::custom`, or a `f64` holding `NaN` under a
    /// self-describing format, does. That makes this a defect in an argument
    /// type rather than a condition of the request, and it is kept
    /// distinguishable so an operator is not sent looking for a store outage.
    ///
    /// Refusing here is the fail-closed answer: proceeding unreserved would run
    /// a marked operation with no idempotency at all, which is the one outcome
    /// the marker exists to prevent.
    #[error("the request's arguments could not be reduced to a canonical fingerprint")]
    RequestNotFingerprintable,
    /// No tenant scope had been resolved by the time the operation was about to
    /// be reserved, so there is no namespace to reserve it under.
    ///
    /// # Why this cannot be answered by defaulting to the absent scope
    ///
    /// "Not resolved" and "resolved to the systemwide scope" are different
    /// statements, and only the second one names a namespace. Treating the first
    /// as the second files the reservation in the shared, tenant-less partition,
    /// where two callers from two different tenants presenting the same key
    /// become one operation: the second is answered with the first's stored
    /// response, or refused as a conflict against a request it has nothing to do
    /// with. Both outcomes let one scope's result govern another, and the first
    /// discloses it.
    ///
    /// **The store is never reached.** Like
    /// [`RequestNotFingerprintable`](ReservationRejection::RequestNotFingerprintable),
    /// this is decided before the reservation, which is why it is neither
    /// `StoreUnavailable` — the store was never asked — nor
    /// `FingerprintConflict`, which requires two fingerprints that were both
    /// computed and then differed.
    ///
    /// **A deployment or authoring fault, not a client one.** The request may be
    /// perfectly well-formed; what is missing is that the operation is marked
    /// idempotent while nothing on its path resolved a tenant. No client retry
    /// changes that, so it is kept distinguishable from the refusals a caller can
    /// act on.
    #[error("no tenant scope was resolved before the reservation, so it has no namespace")]
    TenantUnresolved,
}

/// The digest of an operation's canonical semantic input (AD-3f).
///
/// # What this covers, and what it must not
///
/// `value` is the operation's already-deserialised typed arguments and nothing
/// else. It is the caller's job — the generated slot-3 code — to exclude the
/// context: `operation_key`, owner, lease, trace and correlation ids describe
/// *this attempt*, not *this request*, and folding any of them in would make
/// every retry look like a different request, which is the precise failure this
/// whole mechanism exists to avoid.
///
/// # Why the canonical form is built here rather than by `serde_json`
///
/// Serialising to a JSON string and hashing that would make the fingerprint
/// depend on `serde_json`'s map ordering, which is not a stable property of this
/// workspace: `serde_json`'s `preserve_order` feature is additive, so any crate
/// anywhere in the dependency graph enabling it silently swaps the `BTreeMap`
/// backing for an insertion-ordered one. Struct fields would still hash
/// deterministically, but a `HashMap` argument field would start hashing in
/// random iteration order — two identical retries producing two fingerprints,
/// each looking like a different request. That is a silent, remote-controlled
/// correctness change, so the ordering is established here instead of borrowed.
///
/// The encoding is tagged and length-prefixed for the same reason
/// `EffectFingerprint` is: without it, `["ab"]` and `["a", "b"]` reduce to the
/// same bytes, and two genuinely different requests would deduplicate against
/// each other.
pub fn operation_fingerprint<T: serde::Serialize>(
    value: &T,
) -> Result<OperationFingerprint, ReservationRejection> {
    use sha2::{Digest, Sha256};

    let canonical =
        serde_json::to_value(value).map_err(|_| ReservationRejection::RequestNotFingerprintable)?;

    let mut hasher = Sha256::new();
    absorb_canonical(&canonical, &mut hasher);
    let digest = hasher.finalize();

    Ok(OperationFingerprint::new(
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    ))
}

/// Feeds one value into the digest in a form that depends on its structure and
/// nothing else.
///
/// Object keys are sorted here rather than trusted to arrive sorted. Every arm
/// writes a distinct tag first, so a string can never reduce to the same bytes
/// as a one-element array holding it.
fn absorb_canonical(value: &serde_json::Value, hasher: &mut impl sha2::Digest) {
    fn absorb_bytes(bytes: &[u8], hasher: &mut impl sha2::Digest) {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }

    match value {
        serde_json::Value::Null => hasher.update([0u8]),
        serde_json::Value::Bool(b) => {
            hasher.update([1u8]);
            hasher.update([u8::from(*b)]);
        }
        // The rendered number, not the in-memory representation: `serde_json`
        // already normalises integers and floats to one textual form each, and
        // an integer and a float that happen to be numerically equal are
        // different typed values, so they are meant to differ here.
        serde_json::Value::Number(n) => {
            hasher.update([2u8]);
            absorb_bytes(n.to_string().as_bytes(), hasher);
        }
        serde_json::Value::String(s) => {
            hasher.update([3u8]);
            absorb_bytes(s.as_bytes(), hasher);
        }
        serde_json::Value::Array(items) => {
            hasher.update([4u8]);
            hasher.update((items.len() as u64).to_be_bytes());
            for item in items {
                absorb_canonical(item, hasher);
            }
        }
        serde_json::Value::Object(entries) => {
            hasher.update([5u8]);
            hasher.update((entries.len() as u64).to_be_bytes());
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort_unstable();
            for key in keys {
                absorb_bytes(key.as_bytes(), hasher);
                absorb_canonical(&entries[key], hasher);
            }
        }
    }
}

/// Encodes a completed operation's output for storage.
///
/// The writer half of the pair [`decode_stored_response`] reads. Both are
/// exported together on purpose: the format has exactly one owner (AD-3k), and
/// a caller that could only decode would have to hand-roll the envelope to
/// produce a stored response — which is the parallel serialisation the single
/// codec exists to prevent, and it would not fail at compile time.
///
/// B6.8's epilogue is its first production caller; today the replay tests are.
pub fn encode_stored_response<T: serde::Serialize>(
    value: &T,
) -> Result<StoredServiceResponse, StoredResponseError> {
    StoredResponseCodec::encode(value)
}

/// Decodes a replayed response into the operation's own output type.
///
/// Exists so generated code never interprets a codec failure (AD-3g). Every
/// [`StoredResponseError`] collapses to one rejection on purpose: the
/// distinction between "written under an unknown envelope" and "malformed"
/// tells an *operator* which action to take and is preserved in the codec's own
/// error, but it offers the *caller* no different course — both mean this
/// completed operation cannot answer, permanently, until somebody intervenes
/// (AD-3k).
pub fn decode_stored_response<T: serde::de::DeserializeOwned>(
    stored: &StoredServiceResponse,
) -> Result<T, ReservationRejection> {
    StoredResponseCodec::decode(stored)
        .map_err(|_| ReservationRejection::StoredResponseIncompatible)
}

/// The static metric names AD-10's `outcome` values fold into.
///
/// # Why the value is in the name
///
/// AD-10 specifies `idempotency.reservation.outcome` as one counter with an
/// `outcome` attribute. When this was written the port had **no attribute
/// parameter**, so that shape was not expressible, and every value in the
/// attribute is a bounded enum — six variants, fixed by [`ReservationOutcome`] —
/// so folding it into the name preserved the information without inventing an
/// attribute API.
///
/// The port now expresses attributes and kinds, so the folded names here are
/// migration debt rather than a limit: this function is replaced by the counter
/// carrying a real `outcome` attribute, in the slice that migrates the
/// already-emitted signals.
///
/// Exhaustive with no wildcard, deliberately. A seventh outcome added upstream must
/// break this match rather than be silently counted as whichever arm happened to be
/// last, which is the same reason the dispatch match below has none.
fn outcome_metric(outcome: &ReservationOutcome) -> &'static str {
    match outcome {
        ReservationOutcome::Fresh(_) => "idempotency.reservation.outcome.fresh",
        ReservationOutcome::TakenOver(_) => "idempotency.reservation.outcome.taken_over",
        ReservationOutcome::OwnedInProgress(_) => {
            "idempotency.reservation.outcome.owned_in_progress"
        }
        ReservationOutcome::OtherInProgress => "idempotency.reservation.outcome.other_in_progress",
        ReservationOutcome::Succeeded(_) => "idempotency.reservation.outcome.succeeded",
        ReservationOutcome::Conflict => "idempotency.reservation.outcome.conflict",
    }
}

/// AD-10's `idempotency.lease.event`, for the two values a caller can observe.
///
/// `acquired` and `taken_over` are the two the runtime sees: a `Fresh` outcome means
/// this attempt now holds a lease it did not before, and `TakenOver` means it holds
/// one it displaced. AD-10's table originally listed two more — `renewed` and
/// `expired` — and **AD-10c withdraws both**, so this function is exhaustive against
/// the amended table rather than short of the original one.
///
/// They are withdrawn because neither exists as an independently observable runtime
/// event: renewal is never invoked, while expiry is discovered lazily inside
/// `reserve` and surfaces only through a subsequent takeover. The two differ — a
/// renewal never happens at all, whereas a lease genuinely does lapse but nothing
/// observes it doing so. Counting an expiry at takeover would name the reclaimed
/// ones after the whole population, and would double-count a single transition
/// alongside `taken_over`. AD-10c carries the full reasoning and the condition for
/// reopening it.
///
/// `None` for every other outcome: those observe no lease change at all.
fn lease_event_metric(outcome: &ReservationOutcome) -> Option<&'static str> {
    match outcome {
        ReservationOutcome::Fresh(_) => Some("idempotency.lease.event.acquired"),
        ReservationOutcome::TakenOver(_) => Some("idempotency.lease.event.taken_over"),
        ReservationOutcome::OwnedInProgress(_)
        | ReservationOutcome::OtherInProgress
        | ReservationOutcome::Succeeded(_)
        | ReservationOutcome::Conflict => None,
    }
}

impl ReservationConfig {
    /// Reserves one operation, and decides what dispatch may do with the answer.
    ///
    /// This is the only place a [`ReservationOutcome`] is interpreted. The macro
    /// places the call and converts the rejection; it never reads an outcome
    /// (AD-3g). Keeping the six-way mapping here means one implementation to
    /// test rather than one copy per generated operation.
    ///
    /// `tenant`, `key` and `fingerprint` arrive already definitive — the
    /// generated code canonicalises the typed arguments and computes the
    /// fingerprint before calling (AD-3f). Nothing is derived here.
    /// `observability` is passed rather than held on `ReservationConfig`, so this
    /// type stays about reservation policy instead of accumulating capabilities, and
    /// so the public `ReservationConfig::new` keeps its signature. There is one
    /// caller, and it already holds the registered instance.
    pub(crate) async fn reserve(
        &self,
        tenant: Option<TenantId>,
        key: OperationKey,
        fingerprint: OperationFingerprint,
        observability: Option<&Arc<dyn ego_domain::Observability>>,
    ) -> Result<ReservationDecision, ReservationRejection> {
        let outcome = self
            .store
            .reserve(ReserveRequest {
                tenant,
                operation_key: key,
                fingerprint,
                owner_id: self.owner_id.clone(),
                lease_until: self.lease_until(),
            })
            .await
            // A store that cannot answer is not a refusal to proceed on the
            // merits; it is the absence of an answer, and it stays its own case
            // so a caller can retry it where a conflict must never be retried.
            .map_err(|_| ReservationRejection::StoreUnavailable)?;

        // AD-10's reservation counters, emitted here because this is where the
        // outcome is known: `decide` below collapses `Fresh` and `TakenOver` into one
        // `Proceed`, so a caller reading the decision cannot tell them apart, and
        // both the outcome counter and the lease event need to.
        if let Some(obs) = observability {
            obs.counter(outcome_metric(&outcome), 1.0, &[]);
            if let Some(event) = lease_event_metric(&outcome) {
                obs.counter(event, 1.0, &[]);
            }
        }

        // Exhaustive on purpose: no wildcard. A seventh outcome added upstream
        // must break this match rather than fall into whichever arm happened to
        // be last.
        match outcome {
            // Nobody held it, or the previous holder's lease had expired and
            // this attempt took it over with a strictly greater token.
            ReservationOutcome::Fresh(lease) | ReservationOutcome::TakenOver(lease) => {
                Ok(ReservationDecision::Proceed(ReservationPermit {
                    fence: OwnerFence {
                        operation_id: lease.operation_id,
                        owner_id: lease.owner_id,
                        fencing_token: lease.fencing_token,
                    },
                }))
            }
            // Same owner, still-valid lease. Blocks anyway: fencing proves
            // ownership, not exclusion between two executions of the same owner,
            // and re-entering work that died midway is unsafe once it may
            // already have reached an external effect (AD-3h).
            ReservationOutcome::OwnedInProgress(_) => Err(ReservationRejection::SelfInProgress),
            ReservationOutcome::OtherInProgress => Err(ReservationRejection::OtherInProgress),
            // The identical request already completed. Replay, do not execute.
            ReservationOutcome::Succeeded(response) => Ok(ReservationDecision::Replay(response)),
            // Same key, different request. Permanent.
            ReservationOutcome::Conflict => Err(ReservationRejection::FingerprintConflict),
        }
    }
}

/// The six-way mapping from a store outcome to a dispatch decision.
///
/// These cover the mapping in isolation. They do **not** close the unit: a
/// correct `match` that nothing reaches governs nothing, and the slot the macro
/// emits is still empty. The behavioural pass — `#[idempotent]` through to an
/// observable consequence — is what closes it.
#[cfg(test)]
mod reserve_mapping_tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use ego_domain::operation::{FencingToken, Lease, OperationId, ReservationError};
    use ego_domain::time::Clock;
    use std::time::Duration;

    struct FrozenClock;
    impl Clock for FrozenClock {
        fn now(&self) -> DateTime<Utc> {
            Utc.timestamp_opt(1_000, 0).single().expect("valid instant")
        }
    }

    /// Answers `reserve` with whatever the test scripted, and records the
    /// request it was handed.
    struct ScriptedStore {
        answer: Mutex<Option<Result<ReservationOutcome, ReservationError>>>,
        seen: Mutex<Option<ReserveRequest>>,
    }

    impl ScriptedStore {
        fn new(answer: Result<ReservationOutcome, ReservationError>) -> Arc<Self> {
            Arc::new(Self {
                answer: Mutex::new(Some(answer)),
                seen: Mutex::new(None),
            })
        }
    }

    #[async_trait::async_trait]
    impl OperationReservationStore for ScriptedStore {
        async fn reserve(
            &self,
            req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            *self.seen.lock().expect("not poisoned") = Some(req);
            self.answer
                .lock()
                .expect("not poisoned")
                .take()
                .expect("each scripted store answers exactly one reserve")
        }
        async fn renew(&self, _f: &OwnerFence, _u: DateTime<Utc>) -> Result<(), ReservationError> {
            panic!("these tests only reserve");
        }
        async fn complete(
            &self,
            _f: &OwnerFence,
            _r: StoredServiceResponse,
        ) -> Result<(), ReservationError> {
            panic!("these tests only reserve");
        }
        async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
            panic!("these tests only reserve");
        }
        async fn purge_completed_before(
            &self,
            _c: DateTime<Utc>,
            _b: usize,
        ) -> Result<u64, ReservationError> {
            panic!("these tests only reserve");
        }
        async fn probe(&self) -> Result<(), ReservationError> {
            Ok(())
        }
    }

    /// Drives one `reserve` against a scripted outcome and returns what was counted.
    async fn metrics_for(outcome: ReservationOutcome) -> Vec<(String, f64)> {
        let store = ScriptedStore::new(Ok(outcome));
        let config = ReservationConfig::new(
            store,
            Arc::new(FrozenClock),
            OwnerId::new("owner-under-test"),
            Duration::from_secs(30),
        )
        .expect("a valid config");
        let obs = Arc::new(crate::test_support::RecordingObservability::new());
        let as_port: Arc<dyn ego_domain::Observability> = obs.clone();
        let _ = config
            .reserve(None, key(), OperationFingerprint::new("fp"), Some(&as_port))
            .await;
        obs.metrics()
    }

    /// Each of the six outcomes counts under its own static name, and the two that
    /// change lease ownership also count a lease event.
    ///
    /// The table is written out rather than derived from `outcome_metric`, which
    /// would be circular: a mutation renaming a metric would rename the expectation
    /// with it. Every name here is a literal a dashboard would be configured with.
    #[tokio::test]
    async fn each_outcome_counts_under_its_own_static_name() {
        let cases: Vec<(ReservationOutcome, Vec<&str>)> = vec![
            (
                ReservationOutcome::Fresh(lease_with(0)),
                vec![
                    "idempotency.reservation.outcome.fresh",
                    "idempotency.lease.event.acquired",
                ],
            ),
            (
                ReservationOutcome::TakenOver(lease_with(1)),
                vec![
                    "idempotency.reservation.outcome.taken_over",
                    "idempotency.lease.event.taken_over",
                ],
            ),
            (
                ReservationOutcome::OwnedInProgress(lease_with(0)),
                vec!["idempotency.reservation.outcome.owned_in_progress"],
            ),
            (
                ReservationOutcome::OtherInProgress,
                vec!["idempotency.reservation.outcome.other_in_progress"],
            ),
            (
                ReservationOutcome::Succeeded(StoredServiceResponse::new(b"x".to_vec())),
                vec!["idempotency.reservation.outcome.succeeded"],
            ),
            (
                ReservationOutcome::Conflict,
                vec!["idempotency.reservation.outcome.conflict"],
            ),
        ];

        for (outcome, expected) in cases {
            let label = format!("{outcome:?}");
            let recorded = metrics_for(outcome).await;
            let names: Vec<String> = recorded.iter().map(|(n, _)| n.clone()).collect();
            assert_eq!(
                names,
                expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                "wrong counters for {label}"
            );
            for (name, value) in &recorded {
                assert_eq!(
                    *value, 1.0,
                    "{name} is a counter increment, so each emission is exactly one"
                );
            }
        }
    }

    /// The outcomes that observe no lease change emit no lease event.
    ///
    /// Stated as its own test because the table above would still pass if a lease
    /// event were emitted for, say, `Conflict` *and* the expectation updated to match.
    /// This pins the property rather than the table: only ownership changes count as
    /// lease events.
    #[tokio::test]
    async fn only_an_ownership_change_counts_as_a_lease_event() {
        for outcome in [
            ReservationOutcome::OwnedInProgress(lease_with(0)),
            ReservationOutcome::OtherInProgress,
            ReservationOutcome::Succeeded(StoredServiceResponse::new(b"x".to_vec())),
            ReservationOutcome::Conflict,
        ] {
            let label = format!("{outcome:?}");
            let recorded = metrics_for(outcome).await;
            assert!(
                !recorded
                    .iter()
                    .any(|(name, _)| name.starts_with("idempotency.lease.event")),
                "{label} changes no lease, so it must count no lease event: {recorded:?}"
            );
        }
    }

    /// A runtime with no observability reserves identically and counts nothing.
    ///
    /// The negative control: metrics must not become a precondition for dispatch, and
    /// the uninstrumented configuration is the one most deployments run today — there
    /// is no OTLP metrics exporter in this workspace at all.
    #[tokio::test]
    async fn an_uninstrumented_reservation_still_proceeds() {
        let store = ScriptedStore::new(Ok(ReservationOutcome::Fresh(lease_with(0))));
        let config = ReservationConfig::new(
            store,
            Arc::new(FrozenClock),
            OwnerId::new("owner-under-test"),
            Duration::from_secs(30),
        )
        .expect("a valid config");

        let decision = config
            .reserve(None, key(), OperationFingerprint::new("fp"), None)
            .await;
        assert!(
            matches!(decision, Ok(ReservationDecision::Proceed(_))),
            "got {decision:?}"
        );
    }

    fn key() -> OperationKey {
        OperationKey::parse("op-1").expect("a non-empty key parses")
    }

    /// `bumps` takeovers past the initial token, so a test can name the token
    /// it expects the permit to carry.
    fn lease_with(bumps: usize) -> Lease {
        let mut token = FencingToken::initial();
        for _ in 0..bumps {
            token = token
                .next()
                .expect("the sequence is not exhausted in tests");
        }
        Lease {
            operation_id: OperationId::new(None, key()),
            owner_id: OwnerId::new("owner-under-test"),
            fencing_token: token,
            lease_until: Utc.timestamp_opt(2_000, 0).single().expect("valid"),
        }
    }

    fn token_after(bumps: usize) -> FencingToken {
        let mut token = FencingToken::initial();
        for _ in 0..bumps {
            token = token
                .next()
                .expect("the sequence is not exhausted in tests");
        }
        token
    }

    async fn decide(
        answer: Result<ReservationOutcome, ReservationError>,
    ) -> (
        Result<ReservationDecision, ReservationRejection>,
        Arc<ScriptedStore>,
    ) {
        let store = ScriptedStore::new(answer);
        let config = ReservationConfig::new(
            store.clone(),
            Arc::new(FrozenClock),
            OwnerId::new("owner-under-test"),
            Duration::from_secs(30),
        )
        .expect("a positive lease");
        let decision = config
            .reserve(None, key(), OperationFingerprint::new("fp"), None)
            .await;
        (decision, store)
    }

    #[tokio::test]
    async fn a_fresh_reservation_permits_the_operation() {
        let (decision, _) = decide(Ok(ReservationOutcome::Fresh(lease_with(0)))).await;
        match decision {
            Ok(ReservationDecision::Proceed(permit)) => {
                assert_eq!(permit.fence().fencing_token, token_after(0))
            }
            other => panic!("expected Proceed, got {other:?}"),
        }
    }

    /// A takeover permits too, and the permit must carry the *new* token — the
    /// one the store minted, not the one the previous owner held. Completing
    /// under a stale token is exactly what fencing exists to refuse.
    #[tokio::test]
    async fn a_takeover_permits_and_carries_the_new_token() {
        let (decision, _) = decide(Ok(ReservationOutcome::TakenOver(lease_with(3)))).await;
        match decision {
            Ok(ReservationDecision::Proceed(permit)) => assert_eq!(
                permit.fence().fencing_token,
                token_after(3),
                "the permit must carry the token the takeover granted"
            ),
            other => panic!("expected Proceed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_completed_reservation_replays_without_permitting() {
        let stored = StoredServiceResponse::new(b"the recorded answer".to_vec());
        let (decision, _) = decide(Ok(ReservationOutcome::Succeeded(stored))).await;
        match decision {
            Ok(ReservationDecision::Replay(response)) => {
                assert_eq!(response.as_bytes(), b"the recorded answer")
            }
            other => panic!("expected Replay, got {other:?}"),
        }
    }

    /// Self-contention blocks. Fencing proves ownership, not exclusion between
    /// two executions of the same owner (AD-3h).
    #[tokio::test]
    async fn our_own_in_progress_lease_blocks() {
        let (decision, _) = decide(Ok(ReservationOutcome::OwnedInProgress(lease_with(2)))).await;
        assert_eq!(decision.unwrap_err(), ReservationRejection::SelfInProgress);
    }

    #[tokio::test]
    async fn another_owners_lease_blocks_distinguishably() {
        let (decision, _) = decide(Ok(ReservationOutcome::OtherInProgress)).await;
        assert_eq!(decision.unwrap_err(), ReservationRejection::OtherInProgress);
    }

    #[tokio::test]
    async fn a_different_request_under_the_same_key_is_a_permanent_conflict() {
        let (decision, _) = decide(Ok(ReservationOutcome::Conflict)).await;
        assert_eq!(
            decision.unwrap_err(),
            ReservationRejection::FingerprintConflict
        );
    }

    /// A store that cannot answer is its own case: retrying it is right, where
    /// retrying a conflict never is.
    #[tokio::test]
    async fn a_store_failure_is_not_a_reservation_decision() {
        let (decision, _) = decide(Err(ReservationError::Backend("down".to_string()))).await;
        assert_eq!(
            decision.unwrap_err(),
            ReservationRejection::StoreUnavailable
        );
    }

    /// The request is passed through, not re-derived: AD-3f puts canonicalisation
    /// and fingerprinting in the generated code, and the lease comes from the
    /// configured clock.
    #[tokio::test]
    async fn the_request_carries_what_the_caller_supplied() {
        let (_, store) = decide(Ok(ReservationOutcome::Fresh(lease_with(0)))).await;
        let seen = store
            .seen
            .lock()
            .expect("not poisoned")
            .clone()
            .expect("one reserve");
        assert_eq!(seen.operation_key, key());
        assert_eq!(seen.fingerprint, OperationFingerprint::new("fp"));
        assert_eq!(seen.owner_id, OwnerId::new("owner-under-test"));
        assert_eq!(
            seen.lease_until,
            Utc.timestamp_opt(1_030, 0).single().expect("valid"),
            "lease_until must be the configured clock plus the configured lease"
        );
    }
}

/// The single codec for a stored operation response.
///
/// Both directions live here on purpose. The reader is the replay path and the
/// writer is the slot-3 epilogue; defining either alone would fix the format
/// from the side with less information, and a mismatch would not fail at
/// compile time — it would fail on the first real retry in production,
/// answering a completed operation with nonsense (AD-3k).
pub struct StoredResponseCodec;

/// The envelope tag written beside every payload.
///
/// A bare payload cannot be told apart from a payload of a different shape:
/// `serde_json` will happily decode `{"id":"x"}` into more than one type, and
/// into `null` for an `Option`. The tag makes a version change a detected
/// failure instead of a silently wrong answer.
const STORED_RESPONSE_ENVELOPE: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope<T> {
    /// Bumped when the encoding changes in a way a previous reader cannot
    /// handle. Never reused.
    v: u32,
    payload: T,
}

/// Why a stored response could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoredResponseError {
    /// The bytes were written under an envelope this build does not understand.
    ///
    /// The case a deployment straddling two versions produces, and the reason
    /// the tag exists at all.
    #[error("the stored response was written under envelope version {found}, expected {expected}")]
    UnknownEnvelope {
        /// The version the stored bytes carry.
        found: u32,
        /// The version this build writes and reads.
        expected: u32,
    },
    /// The bytes did not parse, or did not fit the expected shape.
    #[error("the stored response could not be decoded")]
    Malformed,
}

impl StoredResponseCodec {
    /// Encodes an operation's output for storage.
    pub(crate) fn encode<T: serde::Serialize>(
        value: &T,
    ) -> Result<StoredServiceResponse, StoredResponseError> {
        let envelope = Envelope {
            v: STORED_RESPONSE_ENVELOPE,
            payload: value,
        };
        serde_json::to_vec(&envelope)
            .map(StoredServiceResponse::new)
            .map_err(|_| StoredResponseError::Malformed)
    }

    /// Decodes a stored response back into the operation's output.
    ///
    /// The version is checked **before** the payload, so a shape mismatch under
    /// a known envelope and an entirely unknown envelope stay distinguishable —
    /// the first is a bug, the second is a deployment straddling two versions,
    /// and they call for different action.
    pub(crate) fn decode<T: serde::de::DeserializeOwned>(
        stored: &StoredServiceResponse,
    ) -> Result<T, StoredResponseError> {
        let tagged: serde_json::Value = serde_json::from_slice(stored.as_bytes())
            .map_err(|_| StoredResponseError::Malformed)?;

        let found = tagged
            .get("v")
            .and_then(serde_json::Value::as_u64)
            .ok_or(StoredResponseError::Malformed)?;
        let found = u32::try_from(found).map_err(|_| StoredResponseError::Malformed)?;
        if found != STORED_RESPONSE_ENVELOPE {
            return Err(StoredResponseError::UnknownEnvelope {
                found,
                expected: STORED_RESPONSE_ENVELOPE,
            });
        }

        let payload = tagged
            .get("payload")
            .ok_or(StoredResponseError::Malformed)?
            .clone();
        serde_json::from_value(payload).map_err(|_| StoredResponseError::Malformed)
    }
}

/// The stored-response codec: one owner for both directions (AD-3k).
#[cfg(test)]
mod stored_response_codec_tests {
    use super::*;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Answer {
        id: String,
        count: u32,
    }

    /// The property the writer and the reader exist to share.
    #[test]
    fn a_response_survives_the_round_trip() {
        let original = Answer {
            id: "user-1".to_string(),
            count: 3,
        };
        let stored = StoredResponseCodec::encode(&original).expect("encoding succeeds");
        let decoded: Answer = StoredResponseCodec::decode(&stored).expect("decoding succeeds");
        assert_eq!(decoded, original);
    }

    /// The case a deployment straddling two versions produces — and the one
    /// nobody finds by reasoning, because both sides look correct in isolation.
    #[test]
    fn a_response_from_another_envelope_is_refused_by_version_not_by_shape() {
        let foreign = StoredServiceResponse::new(
            serde_json::to_vec(&serde_json::json!({
                "v": 99,
                "payload": { "id": "user-1", "count": 3 }
            }))
            .expect("valid json"),
        );

        let refused = StoredResponseCodec::decode::<Answer>(&foreign);
        assert_eq!(
            refused.unwrap_err(),
            StoredResponseError::UnknownEnvelope {
                found: 99,
                expected: STORED_RESPONSE_ENVELOPE,
            },
            "the payload here is perfectly decodable — only the envelope differs, \
             and reporting that as Malformed would send an operator looking for \
             corruption instead of for a version skew"
        );
    }

    /// Under a known envelope, a payload of the wrong shape is a bug, not a
    /// version skew. They stay distinguishable because they call for different
    /// action.
    #[test]
    fn a_wrong_shape_under_a_known_envelope_is_malformed_not_a_version_skew() {
        let mismatched = StoredServiceResponse::new(
            serde_json::to_vec(&serde_json::json!({
                "v": STORED_RESPONSE_ENVELOPE,
                "payload": { "unexpected": true }
            }))
            .expect("valid json"),
        );
        assert_eq!(
            StoredResponseCodec::decode::<Answer>(&mismatched).unwrap_err(),
            StoredResponseError::Malformed
        );
    }

    /// A bare payload with no envelope at all — what a writer that skipped the
    /// tag would produce. It must be refused rather than guessed at.
    #[test]
    fn an_untagged_payload_is_refused() {
        let untagged = StoredServiceResponse::new(
            serde_json::to_vec(&serde_json::json!({ "id": "user-1", "count": 3 }))
                .expect("valid json"),
        );
        assert_eq!(
            StoredResponseCodec::decode::<Answer>(&untagged).unwrap_err(),
            StoredResponseError::Malformed,
            "without a tag there is nothing to distinguish this from a payload of \
             a different shape, which is the whole reason the envelope exists"
        );
    }

    /// Bytes that are not JSON at all.
    #[test]
    fn garbage_bytes_are_refused() {
        let garbage = StoredServiceResponse::new(b"not json at all".to_vec());
        assert_eq!(
            StoredResponseCodec::decode::<Answer>(&garbage).unwrap_err(),
            StoredResponseError::Malformed
        );
    }
}
