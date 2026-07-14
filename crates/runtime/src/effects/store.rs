//! Effect delivery state store and dedup store ports (CORE-019 Phase 1).
//!
//! Two public ports: [`EffectStateStore`] (pending → in-flight → succeeded |
//! retryable-failed | terminal-failed) and [`EffectDedupStore`] (scoped
//! idempotency dedup). [`InMemoryEffectStore`] implements both as one
//! composite (convenience only, design.md §3 caveat) for slice 1.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

/// Unique runtime-minted identifier for an accepted external effect.
///
/// Backed by a UUID v4, mirroring [`crate::runtime::execution::ExecutionId`].
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct EffectId(Uuid);

impl EffectId {
    /// Mints a new, globally-unique effect identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EffectId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A persistable, portable point in time (F-02).
///
/// Wraps [`chrono::DateTime<Utc>`] — the same convention `ego_domain`'s
/// [`Clock`](ego_domain::Clock) trait already returns — rather than
/// `std::time::Instant`. `Instant` is monotonic and process-local: it cannot
/// be serialized, persisted, or compared across a process restart, which is
/// exactly what a durable [`EffectStateStore`] needs for `next_at`,
/// `claim_due`, and `recover_in_flight`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    /// The current wall-clock instant.
    pub fn now() -> Self {
        Self(Utc::now())
    }

    /// Wraps an existing UTC timestamp (e.g. read back from a durable store).
    pub fn from_utc(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    /// Unwraps the underlying UTC timestamp.
    pub fn into_utc(self) -> DateTime<Utc> {
        self.0
    }
}

/// The lifecycle state of one accepted external effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectState {
    /// Accepted, not yet attempted.
    Pending,
    /// An attempt is currently being dispatched.
    InFlight,
    /// The effect was delivered successfully.
    Succeeded,
    /// The last attempt failed but may be retried.
    RetryableFailed,
    /// The effect will never be retried again.
    TerminalFailed,
}

/// Why an effect was marked terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalReason {
    /// No executor is registered for the effect's `effect_type`.
    ExecutorMissing,
    /// The effect conflicts with an existing dedup scope (different
    /// payload/destination for the same scoped key).
    InvalidEffect(String),
    /// Any other terminal reason (e.g. attempt cap exceeded).
    Other(String),
}

/// Errors returned by [`EffectStateStore`] and [`EffectDedupStore`] (F-03).
///
/// Beyond bookkeeping errors (`NotFound`/`InvalidTransition`/`Conflict`),
/// this taxonomy lets a durable backend express *transient* failures
/// (`TemporarilyUnavailable`) distinctly from *permanent* ones (`Backend`),
/// which AD-7's future delivery runner needs to classify a bookkeeping
/// failure as retryable vs terminal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectStoreError {
    /// No effect is recorded under this id.
    #[error("effect {0} not found")]
    NotFound(EffectId),
    /// The requested transition is not legal from the effect's current state.
    #[error("invalid transition for effect {id}: {from:?} -> {to:?}")]
    InvalidTransition {
        /// The effect that failed to transition.
        id: EffectId,
        /// Its actual current state.
        from: EffectState,
        /// The state the caller attempted to transition to.
        to: EffectState,
    },
    /// An optimistic-concurrency or dedup conflict (e.g. a concurrent writer
    /// won the race). Callers should treat this as retryable at the caller's
    /// discretion, not automatically terminal.
    #[error("conflict: {0}")]
    Conflict(String),
    /// The backend is reachable but momentarily unable to serve the request
    /// (connection pool exhausted, timeout, lock contention). Always
    /// retryable.
    #[error("backend temporarily unavailable: {0}")]
    TemporarilyUnavailable(String),
    /// A permanent backend failure (corruption, serialization failure,
    /// schema mismatch). Never automatically retryable.
    #[error("backend error: {0}")]
    Backend(String),
}

/// Public DTO describing one accepted effect attempt (F-01).
///
/// This is the type [`EffectStateStore::accept`] takes — unlike the former
/// `EffectEnvelope`, every field here is public API (`EffectId`, `TenantId`,
/// `u32`, `ExternalEffectDescription`), so the trait is genuinely
/// implementable from any crate, not only from within `ego-runtime`.
#[derive(Debug, Clone)]
pub struct AcceptedEffect {
    /// The runtime-minted effect identifier.
    pub id: EffectId,
    /// The tenant established at acceptance time.
    pub tenant: TenantId,
    /// The attempt number this acceptance represents.
    pub attempt: u32,
    /// The frozen, handler-described effect.
    ///
    /// `Arc`-wrapped (fix 9, PR2 review) so retries/concurrent attempts
    /// clone the pointer, not the payload bytes — `tokio::spawn`'s `'static`
    /// bound used to force a full deep clone of every attempt's description.
    pub description: Arc<ExternalEffectDescription>,
}

/// Runtime-owned metadata wrapper around one [`AcceptedEffect`] (design.md §4).
///
/// **Permanently crate-private** — never exported as public API. Wraps the
/// public [`AcceptedEffect`] plus room to grow internal-only metadata (trace
/// context, `accepted_at`) later without a semver-breaking change. Used by
/// the future acceptor/queue/runner (PR2/PR3), not by [`EffectStateStore`]
/// anymore — that port only ever sees the public `AcceptedEffect`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // ponytail: unused until PR2/PR3 wire the queue/runner around it
pub(crate) struct EffectEnvelope {
    /// The publicly-shaped accepted effect this envelope carries.
    pub accepted: AcceptedEffect,
}

/// Everything needed to re-execute one accepted effect after a restart (F-02).
///
/// Returned by [`EffectStateStore::claim_due`] and used to reconstruct the
/// in-flight world after a crash — unlike the bare state bookkeeping the
/// original slice-1 store tracked, this retains `tenant` and `description`
/// so a future durable store can hand a real, re-dispatchable effect back to
/// the runner without any other source of truth.
#[derive(Debug, Clone)]
pub struct StoredEffect {
    /// The runtime-minted effect identifier.
    pub id: EffectId,
    /// The tenant established at acceptance time.
    pub tenant: TenantId,
    /// The frozen, handler-described effect.
    pub description: Arc<ExternalEffectDescription>,
    /// The next attempt number to use for re-dispatch.
    pub attempt: u32,
    /// The effect's current lifecycle state.
    pub state: EffectState,
    /// The earliest instant this effect may be (re-)dispatched.
    pub next_at: Timestamp,
}

/// Public port owning delivery-state bookkeeping for accepted effects.
///
/// Every method signature here is built from public types only
/// (`AcceptedEffect`, `EffectId`, `Timestamp`, `StoredEffect`,
/// `TerminalReason`, `EffectStoreError`) — implementable from any crate that
/// depends on `ego-runtime`, not only from within it (F-01).
#[async_trait]
pub trait EffectStateStore: Send + Sync {
    /// Records a newly-accepted effect as [`EffectState::Pending`].
    async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError>;
    /// Transitions to [`EffectState::InFlight`] before dispatching an attempt.
    async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError>;
    /// Transitions to [`EffectState::Succeeded`] after a successful attempt.
    async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError>;
    /// Transitions to [`EffectState::RetryableFailed`], recording the attempt
    /// number and the earliest instant the next attempt may run.
    async fn mark_retryable(
        &self,
        id: EffectId,
        attempt: u32,
        next_at: Timestamp,
    ) -> Result<(), EffectStoreError>;
    /// Transitions to [`EffectState::TerminalFailed`] with the given reason.
    async fn mark_terminal(
        &self,
        id: EffectId,
        reason: TerminalReason,
    ) -> Result<(), EffectStoreError>;
    /// Returns up to `limit` effects that are due for (re-)dispatch at `now`
    /// — i.e. `Pending` or `RetryableFailed` with `next_at <= now` — with
    /// enough data (`tenant` + `description`) to actually re-execute them
    /// (F-02). Does not itself transition state; a caller that intends to
    /// dispatch a claimed effect still calls `mark_in_flight`.
    async fn claim_due(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<StoredEffect>, EffectStoreError>;
    /// Recovers bookkeeping after a crash: any effect left `InFlight` (an
    /// attempt was dispatching when the process died) is returned to
    /// `Pending` so it becomes claimable again, and the store returns how
    /// many effects it recovered (F-02).
    async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError>;
}

/// A stable, cross-version, cross-build dedup fingerprint over an effect's
/// payload and destination (F-04).
///
/// Replaces the earlier `u64` computed via `std::collections::hash_map::
/// DefaultHasher` — `DefaultHasher` carries **no** stability guarantee across
/// Rust versions or even separate builds of the same binary, and 64 bits has
/// a meaningfully higher collision probability than is appropriate for a
/// store whose whole job is reliably detecting payload/destination
/// mismatches. Backed by SHA-256 (already a transitive workspace dependency
/// via other crates' `sha2 = "0.10"` use, e.g. `security-jwt`/
/// `security-apikey`) over a length-prefixed framing of each field, so
/// `payload=b"ab"` + `destination="cd"` never collides with `payload=b"a"` +
/// `destination="bcd"` the way naive concatenation would.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectFingerprint([u8; 32]);

impl EffectFingerprint {
    /// Computes the fingerprint of one effect's payload + destination.
    pub fn compute(payload: &[u8], destination: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update((payload.len() as u64).to_be_bytes());
        hasher.update(payload);
        let destination = destination.as_bytes();
        hasher.update((destination.len() as u64).to_be_bytes());
        hasher.update(destination);
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }
}

impl fmt::Debug for EffectFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EffectFingerprint({:x?})", &self.0[..4])
    }
}

/// The result of a single-flight dedup reservation attempt (F-02/F-04, PR2
/// round 4 redesign; further split in PR2 round 5, F-02).
///
/// A reservation now records not just a fingerprint but *ownership*
/// (`EffectId`) and *status* (has the owner reached `Succeeded`) — see
/// [`EffectDedupStore::reserve`]'s new `effect_id` parameter. Before the
/// round 4 redesign, `Fresh`/`Duplicate`/`Conflict` alone could not
/// distinguish "a different submission already succeeded" from "this exact
/// effect's own reservation is still in-flight, recovering from a crash" —
/// both looked like a plain `Duplicate`. Round 4 fixed that for the
/// SAME-owner case (`OwnedInProgress`/`OwnedSucceeded`), but still collapsed
/// every DIFFERENT-owner case into one flat `Duplicate`, treated by the
/// runner exactly like `OwnedSucceeded` — silently marking a fresh
/// submission `Succeeded` without ever executing, even when the actual
/// owner was still `OwnedInProgress` (not yet delivered). If that real owner
/// later failed terminally and released its reservation, the only recorded
/// outcome for the idempotency key was this false `Succeeded` — silent data
/// loss (F-02, round 5 BLOCKER). `Duplicate` is now split the same way
/// `Fresh`'s same-owner sibling already was: `OtherInProgress` (a different
/// owner holds the reservation, not yet succeeded — the caller must NOT
/// execute or mark succeeded yet) and `OtherSucceeded` (a different owner's
/// reservation is already recorded `Succeeded` — safe to mark succeeded
/// without re-executing, same as `OwnedSucceeded`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupOutcome {
    /// No prior reservation existed for this scope; this attempt now owns it.
    Fresh,
    /// This exact `EffectId` already owns this scope's reservation, and it
    /// has not yet been recorded [`Succeeded`](EffectState::Succeeded) —
    /// legitimately the SAME effect recovering from a crash or retrying.
    /// The caller MUST proceed to (re-)execute, never short-circuit to
    /// success here.
    OwnedInProgress,
    /// This exact `EffectId` already owns this scope's reservation, and it
    /// has already been recorded [`Succeeded`](EffectState::Succeeded) (via
    /// [`EffectDedupStore::commit_success`]) — genuinely already delivered.
    /// Safe to short-circuit to success without re-executing.
    OwnedSucceeded,
    /// A *different* `EffectId` holds this scope's reservation, and it has
    /// not yet been recorded `Succeeded` — the actual outcome for this
    /// idempotency key isn't known yet (F-02, round 5). The caller MUST NOT
    /// execute or mark succeeded now; it must leave this effect to be
    /// re-evaluated later (see `runner.rs`'s `OtherInProgress` handling).
    OtherInProgress,
    /// A *different* `EffectId`'s reservation for this scope is already
    /// recorded `Succeeded` — a real duplicate submission from elsewhere
    /// that genuinely already delivered. Safe to short-circuit to success
    /// without re-executing (this effect never owned the reservation it
    /// collided with, so there is nothing to release).
    OtherSucceeded,
    /// The same scope was reserved with a *different* fingerprint — the
    /// caller MUST treat this as `InvalidEffect` (terminal), never silently
    /// deduplicated.
    Conflict,
}

/// Scopes a dedup reservation to `(tenant, effect_type, key)`, per the
/// tenant-isolation requirement (spec: "Tenant Isolation").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupScope {
    /// The tenant that produced the effect.
    pub tenant: TenantId,
    /// The effect's `effect_type`.
    pub effect_type: String,
    /// The effect's idempotency key.
    pub key: IdempotencyKey,
}

/// Public port owning single-flight idempotency reservations.
#[async_trait]
pub trait EffectDedupStore: Send + Sync {
    /// Reserves `scope` for `effect_id`'s attempt, keyed by a fingerprint of
    /// the effect's payload/destination (F-02, PR2 round 4: `effect_id` is
    /// new — reservations now track ownership, not just occupancy, so the
    /// runner can tell its own in-progress/recovering attempt apart from a
    /// genuinely different duplicate submission).
    async fn reserve(
        &self,
        scope: &DedupScope,
        effect_id: EffectId,
        fingerprint: EffectFingerprint,
    ) -> Result<DedupOutcome, EffectStoreError>;
    /// Confirms the reservation as permanently delivered. Marks the scope's
    /// owner `Succeeded` (F-02, PR2 round 4) rather than removing the
    /// reservation outright — a later crash-recovery re-attempt by the SAME
    /// effect must still find `OwnedSucceeded`, not `Fresh`.
    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError>;
    /// Releases the reservation after a retryable failure, so a subsequent
    /// retry of the *same* effect is not mistaken for a duplicate.
    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError>;
}

#[derive(Debug, Clone)]
struct EffectRecord {
    tenant: TenantId,
    description: Arc<ExternalEffectDescription>,
    state: EffectState,
    attempt: u32,
    next_at: Option<Timestamp>,
    terminal_reason: Option<TerminalReason>,
}

/// The slice-1 in-memory composite implementing **both** public ports.
///
/// Convenience only (design.md §3 caveat) — a future durable implementation
/// is expected to satisfy each port independently. Loses all pending/
/// in-flight effects on process crash (spec: "In-memory store loses
/// undelivered effects on crash").
/// One scope's dedup reservation: who owns it, what fingerprint it was
/// reserved under, and whether that owner has reached `Succeeded` (F-02,
/// PR2 round 4). `commit_success` flips `succeeded` in place rather than
/// removing the entry — a later crash-recovery re-attempt by the same
/// `effect_id` must still see `OwnedSucceeded`, and a genuinely different
/// future submission under the same scope must still be told it's settled
/// (`Duplicate`), not `Fresh`. Only `release` (a genuinely terminal, non-
/// success outcome) clears the scope entirely.
#[derive(Debug, Clone)]
struct ReservationRecord {
    effect_id: EffectId,
    fingerprint: EffectFingerprint,
    succeeded: bool,
}

#[derive(Default)]
pub struct InMemoryEffectStore {
    states: Mutex<HashMap<EffectId, EffectRecord>>,
    dedup: Mutex<HashMap<DedupScope, ReservationRecord>>,
}

impl InMemoryEffectStore {
    /// Creates a fresh, empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }

    fn transition<'a>(
        states: &'a mut HashMap<EffectId, EffectRecord>,
        id: EffectId,
        allowed_from: &[EffectState],
        to: EffectState,
    ) -> Result<&'a mut EffectRecord, EffectStoreError> {
        let record = states.get_mut(&id).ok_or(EffectStoreError::NotFound(id))?;
        if !allowed_from.contains(&record.state) {
            return Err(EffectStoreError::InvalidTransition {
                id,
                from: record.state,
                to,
            });
        }
        record.state = to;
        Ok(record)
    }
}

#[async_trait]
impl EffectStateStore for InMemoryEffectStore {
    async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
        self.states.lock().unwrap().insert(
            effect.id,
            EffectRecord {
                tenant: effect.tenant,
                description: effect.description,
                state: EffectState::Pending,
                attempt: effect.attempt,
                next_at: None,
                terminal_reason: None,
            },
        );
        Ok(())
    }

    async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
        let mut states = self.states.lock().unwrap();
        Self::transition(
            &mut states,
            id,
            &[EffectState::Pending, EffectState::RetryableFailed],
            EffectState::InFlight,
        )?;
        Ok(())
    }

    async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
        let mut states = self.states.lock().unwrap();
        Self::transition(&mut states, id, &[EffectState::InFlight], EffectState::Succeeded)?;
        Ok(())
    }

    async fn mark_retryable(
        &self,
        id: EffectId,
        attempt: u32,
        next_at: Timestamp,
    ) -> Result<(), EffectStoreError> {
        let mut states = self.states.lock().unwrap();
        let record = Self::transition(
            &mut states,
            id,
            &[EffectState::InFlight],
            EffectState::RetryableFailed,
        )?;
        record.attempt = attempt;
        record.next_at = Some(next_at);
        Ok(())
    }

    async fn mark_terminal(
        &self,
        id: EffectId,
        reason: TerminalReason,
    ) -> Result<(), EffectStoreError> {
        let mut states = self.states.lock().unwrap();
        let record = Self::transition(
            &mut states,
            id,
            &[EffectState::InFlight, EffectState::RetryableFailed],
            EffectState::TerminalFailed,
        )?;
        record.terminal_reason = Some(reason);
        Ok(())
    }

    async fn claim_due(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<StoredEffect>, EffectStoreError> {
        let states = self.states.lock().unwrap();
        let due = states
            .iter()
            .filter(|(_, record)| {
                matches!(record.state, EffectState::Pending | EffectState::RetryableFailed)
            })
            .filter(|(_, record)| record.next_at.is_none_or(|next_at| next_at <= now))
            .take(limit)
            .map(|(id, record)| StoredEffect {
                id: *id,
                tenant: record.tenant.clone(),
                description: record.description.clone(),
                attempt: record.attempt,
                state: record.state,
                next_at: record.next_at.unwrap_or(now),
            })
            .collect();
        Ok(due)
    }

    async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
        let mut states = self.states.lock().unwrap();
        let mut recovered = 0u64;
        for record in states.values_mut() {
            if record.state == EffectState::InFlight {
                record.state = EffectState::Pending;
                record.next_at = Some(now);
                recovered += 1;
            }
        }
        Ok(recovered)
    }
}

#[async_trait]
impl EffectDedupStore for InMemoryEffectStore {
    async fn reserve(
        &self,
        scope: &DedupScope,
        effect_id: EffectId,
        fingerprint: EffectFingerprint,
    ) -> Result<DedupOutcome, EffectStoreError> {
        let mut dedup = self.dedup.lock().unwrap();
        match dedup.get(scope) {
            None => {
                dedup.insert(
                    scope.clone(),
                    ReservationRecord {
                        effect_id,
                        fingerprint,
                        succeeded: false,
                    },
                );
                Ok(DedupOutcome::Fresh)
            }
            // Different fingerprint under the same scope is always a
            // conflict, regardless of who owns the existing reservation.
            Some(existing) if existing.fingerprint != fingerprint => Ok(DedupOutcome::Conflict),
            Some(existing) if existing.effect_id == effect_id => {
                if existing.succeeded {
                    Ok(DedupOutcome::OwnedSucceeded)
                } else {
                    Ok(DedupOutcome::OwnedInProgress)
                }
            }
            // F-02 (round 5): a different owner's status now matters too —
            // not yet succeeded (`OtherInProgress`) vs. genuinely already
            // delivered (`OtherSucceeded`).
            Some(existing) => {
                if existing.succeeded {
                    Ok(DedupOutcome::OtherSucceeded)
                } else {
                    Ok(DedupOutcome::OtherInProgress)
                }
            }
        }
    }

    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        if let Some(record) = self.dedup.lock().unwrap().get_mut(scope) {
            record.succeeded = true;
        }
        Ok(())
    }

    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        self.dedup.lock().unwrap().remove(scope);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};

    fn sample_description() -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
            effect_type: "invoice.created".to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com".to_string(),
        }
    }

    fn accepted_effect(id: EffectId) -> AcceptedEffect {
        AcceptedEffect {
            id,
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt: 0,
            description: Arc::new(sample_description()),
        }
    }

    fn scope(tenant: &str, key: &str) -> DedupScope {
        DedupScope {
            tenant: TenantId::new(tenant).unwrap(),
            effect_type: "invoice.created".to_string(),
            key: IdempotencyKey::new(key).unwrap(),
        }
    }

    fn fp(seed: &str) -> EffectFingerprint {
        EffectFingerprint::compute(seed.as_bytes(), "https://example.com")
    }

    #[tokio::test]
    async fn accepted_effect_starts_pending_then_moves_in_flight() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id)).await.unwrap();

        store.mark_in_flight(id).await.unwrap();

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::InFlight,
                to: EffectState::InFlight,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn in_flight_effect_can_succeed() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id)).await.unwrap();
        store.mark_in_flight(id).await.unwrap();

        store.mark_succeeded(id).await.unwrap();

        let err = store.mark_succeeded(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::Succeeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn in_flight_effect_can_be_marked_retryable_then_redispatched() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id)).await.unwrap();
        store.mark_in_flight(id).await.unwrap();

        store.mark_retryable(id, 1, Timestamp::now()).await.unwrap();
        // Retry loop: the runner re-dispatches, returning it to in-flight.
        store.mark_in_flight(id).await.unwrap();
        store.mark_succeeded(id).await.unwrap();
    }

    #[tokio::test]
    async fn retryable_effect_can_become_terminal() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id)).await.unwrap();
        store.mark_in_flight(id).await.unwrap();
        store.mark_retryable(id, 3, Timestamp::now()).await.unwrap();

        store
            .mark_terminal(id, TerminalReason::Other("attempt cap exceeded".into()))
            .await
            .unwrap();

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::TerminalFailed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn marking_unknown_effect_id_fails_not_found() {
        let store = InMemoryEffectStore::new();
        let unknown = EffectId::new();

        let err = store.mark_in_flight(unknown).await.unwrap_err();

        assert!(matches!(err, EffectStoreError::NotFound(id) if id == unknown));
    }

    #[tokio::test]
    async fn first_reservation_for_a_scope_is_fresh() {
        let store = InMemoryEffectStore::new();
        let outcome = store
            .reserve(&scope("tenant-a", "uow-1:0"), EffectId::new(), fp("a"))
            .await
            .unwrap();
        assert_eq!(outcome, DedupOutcome::Fresh);
    }

    // --- F-02 (PR2 round 4): dedup identity/status ---

    #[tokio::test]
    async fn repeated_reservation_by_the_same_effect_before_success_is_owned_in_progress() {
        // Crash-recovery re-attempt of the SAME effect, before it ever
        // reached `Succeeded` — must proceed to execute, not be mistaken for
        // a different submission's duplicate.
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        let id = EffectId::new();
        store.reserve(&s, id, fp("a")).await.unwrap();

        let outcome = store.reserve(&s, id, fp("a")).await.unwrap();

        assert_eq!(outcome, DedupOutcome::OwnedInProgress);
    }

    #[tokio::test]
    async fn repeated_reservation_by_a_different_effect_before_success_is_other_in_progress() {
        // F-02 (PR2 round 5): a genuinely different submission racing the
        // same scope while the original owner hasn't yet succeeded must be
        // told the OTHER owner is merely in progress, not a flat
        // `Duplicate` — the caller must not treat this as "already
        // delivered" (that used to cause silent data loss: see
        // `runner.rs`'s `OtherInProgress` handling).
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();

        let outcome = store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();

        assert_eq!(outcome, DedupOutcome::OtherInProgress);
    }

    #[tokio::test]
    async fn reservation_with_different_fingerprint_same_scope_is_conflict() {
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();

        let outcome = store.reserve(&s, EffectId::new(), fp("b")).await.unwrap();

        assert_eq!(outcome, DedupOutcome::Conflict);
    }

    #[tokio::test]
    async fn released_scope_can_be_reserved_fresh_again() {
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();

        store.release(&s).await.unwrap();
        let outcome = store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();

        assert_eq!(outcome, DedupOutcome::Fresh);
    }

    #[tokio::test]
    async fn commit_success_then_same_effect_reserve_is_owned_succeeded() {
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        let id = EffectId::new();
        store.reserve(&s, id, fp("a")).await.unwrap();

        store.commit_success(&s).await.unwrap();
        let outcome = store.reserve(&s, id, fp("a")).await.unwrap();

        assert_eq!(outcome, DedupOutcome::OwnedSucceeded);
    }

    #[tokio::test]
    async fn commit_success_then_different_effect_reserve_is_other_succeeded() {
        // F-02 (PR2 round 5): a genuinely different future submission under
        // the same scope, once the actual owner has resolved to Succeeded,
        // must be told so precisely (`OtherSucceeded`) — safe to mark
        // succeeded without re-executing, unlike `OtherInProgress`.
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        let id = EffectId::new();
        store.reserve(&s, id, fp("a")).await.unwrap();
        store.commit_success(&s).await.unwrap();

        let outcome = store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();

        assert_eq!(outcome, DedupOutcome::OtherSucceeded);
    }

    // --- F-04: stable, portable EffectFingerprint ---

    #[test]
    fn fingerprint_is_stable_for_identical_inputs() {
        let a = EffectFingerprint::compute(b"payload", "https://example.com");
        let b = EffectFingerprint::compute(b"payload", "https://example.com");
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_differs_for_different_payloads_or_destinations() {
        let base = EffectFingerprint::compute(b"payload", "https://example.com");
        let different_payload = EffectFingerprint::compute(b"other", "https://example.com");
        let different_destination = EffectFingerprint::compute(b"payload", "https://other.com");
        assert_ne!(base, different_payload);
        assert_ne!(base, different_destination);
    }

    #[test]
    fn fingerprint_length_prefixing_avoids_concatenation_ambiguity() {
        // Naive concatenation would make payload=b"ab"+destination="cd" collide
        // with payload=b"a"+destination="bcd" (both concatenate to "abcd").
        let a = EffectFingerprint::compute(b"ab", "cd");
        let b = EffectFingerprint::compute(b"a", "bcd");
        assert_ne!(a, b);
    }

    // --- F-02: claim_due / recover_in_flight ---

    #[tokio::test]
    async fn claim_due_returns_pending_effect_with_tenant_and_description_retained() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id)).await.unwrap();

        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();

        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, id);
        assert_eq!(claimed[0].tenant, TenantId::new("tenant-a").unwrap());
        assert_eq!(*claimed[0].description, sample_description());
        assert_eq!(claimed[0].state, EffectState::Pending);
    }

    #[tokio::test]
    async fn claim_due_hands_back_the_same_arc_allocation_not_a_deep_clone() {
        // Fix 9 (PR2 review): `AcceptedEffect`/`StoredEffect` wrap
        // `description` in `Arc` precisely so a round-trip through the store
        // clones a pointer, never the payload bytes.
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        let effect = accepted_effect(id);
        let original_ptr = Arc::as_ptr(&effect.description);
        store.accept(effect).await.unwrap();

        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();

        assert_eq!(Arc::as_ptr(&claimed[0].description), original_ptr);
    }

    #[tokio::test]
    async fn claim_due_excludes_retryable_effect_whose_next_at_is_in_the_future() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id)).await.unwrap();
        store.mark_in_flight(id).await.unwrap();
        let far_future = Timestamp::from_utc(Utc::now() + chrono::Duration::hours(1));
        store.mark_retryable(id, 1, far_future).await.unwrap();

        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();

        assert!(claimed.is_empty());
    }

    #[tokio::test]
    async fn claim_due_includes_retryable_effect_once_next_at_has_passed() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id)).await.unwrap();
        store.mark_in_flight(id).await.unwrap();
        let just_passed = Timestamp::from_utc(Utc::now() - chrono::Duration::seconds(1));
        store.mark_retryable(id, 1, just_passed).await.unwrap();

        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();

        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].attempt, 1);
        assert_eq!(claimed[0].state, EffectState::RetryableFailed);
    }

    #[tokio::test]
    async fn claim_due_respects_limit() {
        let store = InMemoryEffectStore::new();
        for _ in 0..3 {
            store
                .accept(accepted_effect(EffectId::new()))
                .await
                .unwrap();
        }

        let claimed = store.claim_due(Timestamp::now(), 2).await.unwrap();

        assert_eq!(claimed.len(), 2);
    }

    #[tokio::test]
    async fn claim_due_excludes_in_flight_and_terminal_effects() {
        let store = InMemoryEffectStore::new();
        let in_flight_id = EffectId::new();
        store
            .accept(accepted_effect(in_flight_id))
            .await
            .unwrap();
        store.mark_in_flight(in_flight_id).await.unwrap();

        let terminal_id = EffectId::new();
        store.accept(accepted_effect(terminal_id)).await.unwrap();
        store.mark_in_flight(terminal_id).await.unwrap();
        store
            .mark_terminal(terminal_id, TerminalReason::ExecutorMissing)
            .await
            .unwrap();

        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();

        assert!(claimed.is_empty());
    }

    #[tokio::test]
    async fn recover_in_flight_returns_in_flight_effects_to_pending_and_counts_them() {
        let store = InMemoryEffectStore::new();
        let recovered_id = EffectId::new();
        store.accept(accepted_effect(recovered_id)).await.unwrap();
        store.mark_in_flight(recovered_id).await.unwrap();

        let untouched_id = EffectId::new();
        store.accept(accepted_effect(untouched_id)).await.unwrap();

        let recovered = store.recover_in_flight(Timestamp::now()).await.unwrap();

        assert_eq!(recovered, 1);
        // The recovered effect is claimable again (back to Pending).
        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(claimed.len(), 2);
        assert!(claimed.iter().any(|e| e.id == recovered_id));
        assert!(claimed.iter().any(|e| e.id == untouched_id));
    }

    #[tokio::test]
    async fn recover_in_flight_is_zero_when_nothing_is_in_flight() {
        let store = InMemoryEffectStore::new();
        store
            .accept(accepted_effect(EffectId::new()))
            .await
            .unwrap();

        let recovered = store.recover_in_flight(Timestamp::now()).await.unwrap();

        assert_eq!(recovered, 0);
    }

    // --- F-03: error taxonomy ---

    #[test]
    fn error_taxonomy_distinguishes_transient_from_permanent_backend_failures() {
        let transient = EffectStoreError::TemporarilyUnavailable("connection pool exhausted".into());
        let permanent = EffectStoreError::Backend("corrupt record".into());
        let conflict = EffectStoreError::Conflict("optimistic lock lost".into());

        // A caller classifying retryability sees the three kinds as distinct.
        assert!(matches!(transient, EffectStoreError::TemporarilyUnavailable(_)));
        assert!(matches!(permanent, EffectStoreError::Backend(_)));
        assert!(matches!(conflict, EffectStoreError::Conflict(_)));
        assert_ne!(transient, permanent);

        // Each variant carries a human-readable message via `thiserror`.
        assert_eq!(
            transient.to_string(),
            "backend temporarily unavailable: connection pool exhausted"
        );
        assert_eq!(permanent.to_string(), "backend error: corrupt record");
        assert_eq!(conflict.to_string(), "conflict: optimistic lock lost");
    }
}
