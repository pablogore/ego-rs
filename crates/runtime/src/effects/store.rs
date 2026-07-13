//! Effect delivery state store and dedup store ports (CORE-019 Phase 1).
//!
//! Two public ports: [`EffectStateStore`] (pending → in-flight → succeeded |
//! retryable-failed | terminal-failed) and [`EffectDedupStore`] (scoped
//! idempotency dedup). [`InMemoryEffectStore`] implements both as one
//! composite (convenience only, design.md §3 caveat) for slice 1.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Instant;

use async_trait::async_trait;
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
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

/// Errors returned by [`EffectStateStore`] and [`EffectDedupStore`].
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
}

/// Runtime-owned metadata wrapper for one accepted effect attempt.
///
/// **Permanently private** (design.md §4) — never exported as public API.
/// Expected to grow fields (trace id, correlation id, timestamps) without a
/// semver-breaking change precisely because it stays crate-private.
#[derive(Debug, Clone)]
pub(crate) struct EffectEnvelope {
    /// The runtime-minted effect identifier.
    pub id: EffectId,
    /// The tenant established at acceptance time.
    // ponytail: unread within this slice — `EffectStateStore` only tracks
    // lifecycle state (id/attempt), while `tenant`/`description` travel to
    // the runner through the queue (Phase 4/5, out of scope here). Kept on
    // the envelope now because that's the accept-time unit design.md defines.
    #[allow(dead_code)]
    pub tenant: TenantId,
    /// The attempt number this envelope represents.
    pub attempt: u32,
    /// The frozen, handler-described effect.
    #[allow(dead_code)]
    pub description: ExternalEffectDescription,
}

/// Public port owning delivery-state bookkeeping for accepted effects.
///
/// Implementable only within `crates/runtime` for the foreseeable future:
/// [`accept`](EffectStateStore::accept) takes [`EffectEnvelope`] by value,
/// which is crate-private (design.md §4 consequence note). The
/// `private_interfaces` lint is silenced deliberately: the trait stays `pub`
/// (it's the stable seam other `crates/runtime` code depends on) while the
/// envelope type it takes never leaves the crate — that's the point, not a
/// leak.
#[async_trait]
#[allow(private_interfaces)]
pub trait EffectStateStore: Send + Sync {
    /// Records a newly-accepted effect as [`EffectState::Pending`].
    async fn accept(&self, env: EffectEnvelope) -> Result<(), EffectStoreError>;
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
        next_at: Instant,
    ) -> Result<(), EffectStoreError>;
    /// Transitions to [`EffectState::TerminalFailed`] with the given reason.
    async fn mark_terminal(
        &self,
        id: EffectId,
        reason: TerminalReason,
    ) -> Result<(), EffectStoreError>;
}

/// The result of a single-flight dedup reservation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupOutcome {
    /// No prior reservation existed for this scope; this attempt now owns it.
    Fresh,
    /// The same scope was already reserved with an identical fingerprint.
    Duplicate,
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
    /// Reserves `scope` for this attempt, keyed by a fingerprint of the
    /// effect's payload/destination.
    async fn reserve(
        &self,
        scope: &DedupScope,
        fingerprint: u64,
    ) -> Result<DedupOutcome, EffectStoreError>;
    /// Confirms the reservation as permanently delivered.
    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError>;
    /// Releases the reservation after a retryable failure, so a subsequent
    /// retry of the *same* effect is not mistaken for a duplicate.
    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError>;
}

#[derive(Debug, Clone)]
struct EffectRecord {
    state: EffectState,
    attempt: u32,
    next_at: Option<Instant>,
    terminal_reason: Option<TerminalReason>,
}

/// The slice-1 in-memory composite implementing **both** public ports.
///
/// Convenience only (design.md §3 caveat) — a future durable implementation
/// is expected to satisfy each port independently. Loses all pending/
/// in-flight effects on process crash (spec: "In-memory store loses
/// undelivered effects on crash").
#[derive(Default)]
pub struct InMemoryEffectStore {
    states: Mutex<HashMap<EffectId, EffectRecord>>,
    // ponytail: a single fingerprint map is enough for slice-1's semantics —
    // `reserve` after `release` re-opens the scope, `commit_success` doesn't
    // need a separate "committed" flag because nothing calls `release` after
    // a successful delivery. Revisit only if a durable store needs to tell
    // "reserved" and "committed" apart.
    dedup: Mutex<HashMap<DedupScope, u64>>,
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
#[allow(private_interfaces)]
impl EffectStateStore for InMemoryEffectStore {
    async fn accept(&self, env: EffectEnvelope) -> Result<(), EffectStoreError> {
        self.states.lock().unwrap().insert(
            env.id,
            EffectRecord {
                state: EffectState::Pending,
                attempt: env.attempt,
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
        next_at: Instant,
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
}

#[async_trait]
impl EffectDedupStore for InMemoryEffectStore {
    async fn reserve(
        &self,
        scope: &DedupScope,
        fingerprint: u64,
    ) -> Result<DedupOutcome, EffectStoreError> {
        let mut dedup = self.dedup.lock().unwrap();
        match dedup.get(scope) {
            None => {
                dedup.insert(scope.clone(), fingerprint);
                Ok(DedupOutcome::Fresh)
            }
            Some(existing) if *existing == fingerprint => Ok(DedupOutcome::Duplicate),
            Some(_) => Ok(DedupOutcome::Conflict),
        }
    }

    async fn commit_success(&self, _scope: &DedupScope) -> Result<(), EffectStoreError> {
        // The fingerprint entry recorded at `reserve` time already blocks
        // re-reservation; no separate committed marker is needed (see the
        // ponytail note on `InMemoryEffectStore::dedup`).
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
    use std::time::Instant;

    fn sample_description() -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
            effect_type: "invoice.created".to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com".to_string(),
        }
    }

    fn envelope(id: EffectId) -> EffectEnvelope {
        EffectEnvelope {
            id,
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt: 0,
            description: sample_description(),
        }
    }

    fn scope(tenant: &str, key: &str) -> DedupScope {
        DedupScope {
            tenant: TenantId::new(tenant).unwrap(),
            effect_type: "invoice.created".to_string(),
            key: IdempotencyKey::new(key).unwrap(),
        }
    }

    #[tokio::test]
    async fn accepted_effect_starts_pending_then_moves_in_flight() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(envelope(id)).await.unwrap();

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
        store.accept(envelope(id)).await.unwrap();
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
        store.accept(envelope(id)).await.unwrap();
        store.mark_in_flight(id).await.unwrap();

        store.mark_retryable(id, 1, Instant::now()).await.unwrap();
        // Retry loop: the runner re-dispatches, returning it to in-flight.
        store.mark_in_flight(id).await.unwrap();
        store.mark_succeeded(id).await.unwrap();
    }

    #[tokio::test]
    async fn retryable_effect_can_become_terminal() {
        let store = InMemoryEffectStore::new();
        let id = EffectId::new();
        store.accept(envelope(id)).await.unwrap();
        store.mark_in_flight(id).await.unwrap();
        store.mark_retryable(id, 3, Instant::now()).await.unwrap();

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
            .reserve(&scope("tenant-a", "uow-1:0"), 42)
            .await
            .unwrap();
        assert_eq!(outcome, DedupOutcome::Fresh);
    }

    #[tokio::test]
    async fn repeated_reservation_with_same_fingerprint_is_duplicate() {
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        store.reserve(&s, 42).await.unwrap();

        let outcome = store.reserve(&s, 42).await.unwrap();

        assert_eq!(outcome, DedupOutcome::Duplicate);
    }

    #[tokio::test]
    async fn reservation_with_different_fingerprint_same_scope_is_conflict() {
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        store.reserve(&s, 42).await.unwrap();

        let outcome = store.reserve(&s, 99).await.unwrap();

        assert_eq!(outcome, DedupOutcome::Conflict);
    }

    #[tokio::test]
    async fn released_scope_can_be_reserved_fresh_again() {
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        store.reserve(&s, 42).await.unwrap();

        store.release(&s).await.unwrap();
        let outcome = store.reserve(&s, 42).await.unwrap();

        assert_eq!(outcome, DedupOutcome::Fresh);
    }

    #[tokio::test]
    async fn commit_success_keeps_scope_reserved() {
        let store = InMemoryEffectStore::new();
        let s = scope("tenant-a", "uow-1:0");
        store.reserve(&s, 42).await.unwrap();

        store.commit_success(&s).await.unwrap();
        let outcome = store.reserve(&s, 42).await.unwrap();

        assert_eq!(outcome, DedupOutcome::Duplicate);
    }
}
