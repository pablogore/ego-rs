//! Three-tier conformance suite (PROD-002 AD-13, design.md §3.6).
//!
//! - **Tier 1** ([`run_state_store_conformance`] / [`run_dedup_conformance`]):
//!   everything provable against ONE live instance, no restart boundary.
//! - **Tier 2** ([`run_durable_conformance`], design §3.6): a durable
//!   provider's real close→reopen behavior, via the test-only
//!   [`DurableStoreFactory`] trait. Never added to the production ports in
//!   `crates/runtime/src/effects/store.rs`.
//! - **Tier 3** ([`run_multi_node_conformance`]): cross-claimer exclusivity,
//!   reusing [`DurableStoreFactory`] by calling `open()` twice without
//!   dropping either result (concurrent, not sequential). Capability-gated —
//!   a no-op unless the factory's store declares `multi_node_safe: true`.
//!
//! Public so both this crate's own `tests/conformance.rs` (in-memory,
//! Stoolap) and the top-level `integration-tests/` workspace (Postgres,
//! which needs a real container and therefore cannot live in this crate —
//! see `ego-rs-testing`) can run the identical harness against every
//! provider.

use async_trait::async_trait;
use chrono::Duration;
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
use ego_runtime::effects::store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId,
    EffectState, EffectStateStore, EffectStoreError, TerminalReason, Timestamp,
};
use std::sync::Arc;

fn description(seed: &str) -> ExternalEffectDescription {
    ExternalEffectDescription {
        idempotency_key: IdempotencyKey::new(format!("uow-{seed}:0")).unwrap(),
        effect_type: "invoice.created".to_string(),
        payload: seed.as_bytes().to_vec(),
        destination: "https://example.com".to_string(),
    }
}

pub fn accepted(id: EffectId, seed: &str) -> AcceptedEffect {
    AcceptedEffect {
        id,
        tenant: TenantId::new("tenant-a").unwrap(),
        attempt: 0,
        description: Arc::new(description(seed)),
    }
}

pub fn scope(tenant: &str, key: &str) -> DedupScope {
    DedupScope {
        tenant: TenantId::new(tenant).unwrap(),
        effect_type: "invoice.created".to_string(),
        key: IdempotencyKey::new(key).unwrap(),
    }
}

pub fn fp(seed: &str) -> EffectFingerprint {
    EffectFingerprint::compute(seed.as_bytes(), "https://example.com")
}

/// Tier 1 — everything provable against ONE live [`EffectStateStore`]
/// instance, without crossing a restart boundary (design §3.6).
pub async fn run_state_store_conformance(store: &impl EffectStateStore) {
    // --- accept: pending -> in_flight, invalid transitions rejected ---
    let id = EffectId::new();
    store.accept(accepted(id, "a")).await.unwrap();

    store.mark_in_flight(id).await.unwrap();
    let err = store.mark_in_flight(id).await.unwrap_err();
    assert!(
        matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::InFlight,
                to: EffectState::InFlight,
                ..
            }
        ),
        "double mark_in_flight must be InvalidTransition, got {err:?}"
    );

    // --- accept is idempotent for a replayed identical acceptance ---
    store.accept(accepted(id, "a")).await.unwrap();
    let err = store.mark_in_flight(id).await.unwrap_err();
    assert!(
        matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::InFlight,
                ..
            }
        ),
        "replayed accept must not disturb current state"
    );

    // --- accept with different content under the same id is a Conflict ---
    let mut different = accepted(id, "a");
    different.tenant = TenantId::new("tenant-b").unwrap();
    let err = store.accept(different).await.unwrap_err();
    assert!(
        matches!(err, EffectStoreError::Conflict(_)),
        "same id, different content must be Conflict"
    );

    // --- succeed, then reject a second mark_succeeded ---
    store.mark_succeeded(id).await.unwrap();
    let err = store.mark_succeeded(id).await.unwrap_err();
    assert!(matches!(
        err,
        EffectStoreError::InvalidTransition {
            from: EffectState::Succeeded,
            ..
        }
    ));

    // --- retry bookkeeping: mark_retryable resumes attempt, never resets ---
    let retry_id = EffectId::new();
    store.accept(accepted(retry_id, "r")).await.unwrap();
    store.mark_in_flight(retry_id).await.unwrap();
    store
        .mark_retryable(
            retry_id,
            1,
            Timestamp::from_utc(chrono::Utc::now() - Duration::seconds(1)),
        )
        .await
        .unwrap();
    let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
    let claimed = due
        .iter()
        .find(|e| e.id == retry_id)
        .expect("retryable effect due");
    assert_eq!(
        claimed.attempt, 1,
        "claim_due must surface the persisted attempt number"
    );

    store.mark_in_flight(retry_id).await.unwrap();
    store
        .mark_retryable(
            retry_id,
            2,
            Timestamp::from_utc(chrono::Utc::now() - Duration::seconds(1)),
        )
        .await
        .unwrap();
    let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
    let claimed = due
        .iter()
        .find(|e| e.id == retry_id)
        .expect("retryable effect due again");
    assert_eq!(
        claimed.attempt, 2,
        "attempt must advance, never reset, across retries"
    );

    // --- retryable can become terminal ---
    store.mark_in_flight(retry_id).await.unwrap();
    store
        .mark_terminal(
            retry_id,
            TerminalReason::Other("attempt cap exceeded".into()),
        )
        .await
        .unwrap();
    let err = store.mark_in_flight(retry_id).await.unwrap_err();
    assert!(matches!(
        err,
        EffectStoreError::InvalidTransition {
            from: EffectState::TerminalFailed,
            ..
        }
    ));

    // --- unknown id is NotFound ---
    let unknown = EffectId::new();
    let err = store.mark_in_flight(unknown).await.unwrap_err();
    assert!(matches!(err, EffectStoreError::NotFound(found) if found == unknown));

    // --- claim_due: due/future/limit/exclusion semantics ---
    let due_id = EffectId::new();
    store.accept(accepted(due_id, "due")).await.unwrap();
    let future_id = EffectId::new();
    store.accept(accepted(future_id, "future")).await.unwrap();
    store.mark_in_flight(future_id).await.unwrap();
    let far_future = Timestamp::from_utc(chrono::Utc::now() + Duration::hours(1));
    store
        .mark_retryable(future_id, 1, far_future)
        .await
        .unwrap();

    let claimed = store.claim_due(Timestamp::now(), 100).await.unwrap();
    assert!(
        claimed.iter().any(|e| e.id == due_id),
        "a pending effect with no next_at must be claimable"
    );
    assert!(
        !claimed.iter().any(|e| e.id == future_id),
        "a retryable effect whose next_at is in the future must be excluded"
    );
    assert!(
        !claimed.iter().any(|e| e.id == id),
        "a succeeded effect must never be claimable"
    );

    let limited = store.claim_due(Timestamp::now(), 1).await.unwrap();
    assert_eq!(limited.len(), 1, "claim_due must respect its limit");

    // --- recover_in_flight: only InFlight effects are recovered, and counted ---
    let in_flight_id = EffectId::new();
    store.accept(accepted(in_flight_id, "if")).await.unwrap();
    store.mark_in_flight(in_flight_id).await.unwrap();

    let recovered = store.recover_in_flight(Timestamp::now()).await.unwrap();
    assert!(
        recovered >= 1,
        "recover_in_flight must count the recovered in-flight effect"
    );
    let claimed_after_recovery = store.claim_due(Timestamp::now(), 100).await.unwrap();
    assert!(
        claimed_after_recovery.iter().any(|e| e.id == in_flight_id),
        "a recovered effect must become claimable again"
    );
}

/// Tier 1 — everything provable against ONE live [`EffectDedupStore`]
/// instance: all six [`DedupOutcome`] variants, fingerprint-mismatch
/// conflict, tenant isolation.
pub async fn run_dedup_conformance(store: &impl EffectDedupStore) {
    let s = scope("tenant-a", "uow-1:0");
    let owner = EffectId::new();

    // Fresh
    let outcome = store.reserve(&s, owner, fp("a")).await.unwrap();
    assert_eq!(outcome, DedupOutcome::Fresh);

    // OwnedInProgress — same owner, not yet succeeded
    let outcome = store.reserve(&s, owner, fp("a")).await.unwrap();
    assert_eq!(outcome, DedupOutcome::OwnedInProgress);

    // OtherInProgress — different owner, not yet succeeded
    let other = EffectId::new();
    let outcome = store.reserve(&s, other, fp("a")).await.unwrap();
    assert_eq!(outcome, DedupOutcome::OtherInProgress);

    // Conflict — different fingerprint, same scope
    let outcome = store.reserve(&s, EffectId::new(), fp("b")).await.unwrap();
    assert_eq!(outcome, DedupOutcome::Conflict);

    // OwnedSucceeded — the owner commits, then re-reserves
    store.commit_success(&s).await.unwrap();
    let outcome = store.reserve(&s, owner, fp("a")).await.unwrap();
    assert_eq!(outcome, DedupOutcome::OwnedSucceeded);

    // OtherSucceeded — a genuinely different submission after settlement
    let outcome = store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();
    assert_eq!(outcome, DedupOutcome::OtherSucceeded);

    // release frees the scope for a fresh reservation
    store.release(&s).await.unwrap();
    let outcome = store.reserve(&s, EffectId::new(), fp("a")).await.unwrap();
    assert_eq!(outcome, DedupOutcome::Fresh);

    // Tenant isolation: identical effect_type + key, different tenant, never collides
    let s_b = scope("tenant-b", "uow-1:0");
    let outcome = store.reserve(&s_b, EffectId::new(), fp("a")).await.unwrap();
    assert_eq!(
        outcome,
        DedupOutcome::Fresh,
        "a different tenant's identical scope key must not collide"
    );
}

/// Test-only factory (design §3.6). **Never** added to the production ports
/// in `crates/runtime/src/effects/store.rs`. Each durable provider supplies a
/// factory pinned to a fixed backing location at construction, so `open()` is
/// genuinely a reopen, not a fresh store.
#[async_trait]
pub trait DurableStoreFactory {
    /// The concrete store this factory opens — must satisfy both ports.
    type Store: EffectStateStore + EffectDedupStore;

    /// Opens a store bound to this factory's fixed backing location. Calling
    /// this more than once models close→reopen against the SAME storage.
    async fn open(&self) -> Self::Store;
}

/// Tier 2 — durable-provider conformance (design §3.6, AD-13). Asserts that
/// state genuinely survives a drop/reopen against the SAME backing storage:
/// an accepted effect survives; an effect left `InFlight` at drop becomes
/// redispatch-eligible after reopen; a scoped dedup reservation survives.
pub async fn run_durable_conformance<F>(factory: &F)
where
    F: DurableStoreFactory,
{
    // --- an accepted, settled effect survives drop/reopen ---
    let settled_id = EffectId::new();
    {
        let store = factory.open().await;
        store.accept(accepted(settled_id, "settled")).await.unwrap();
        store.mark_in_flight(settled_id).await.unwrap();
        store.mark_succeeded(settled_id).await.unwrap();
    }
    {
        let store = factory.open().await;
        // A settled effect is terminal, so it must be absent from claim_due,
        // but a second mark_in_flight must still resolve against ITS actual
        // (persisted) state, not NotFound — i.e. the row itself survived.
        let err = store.mark_in_flight(settled_id).await.unwrap_err();
        assert!(
            matches!(
                err,
                EffectStoreError::InvalidTransition {
                    from: EffectState::Succeeded,
                    ..
                }
            ),
            "a settled effect's Succeeded state must survive reopen, got {err:?}"
        );
    }

    // --- an effect left InFlight at drop becomes redispatch-eligible ---
    let in_flight_id = EffectId::new();
    {
        let store = factory.open().await;
        store
            .accept(accepted(in_flight_id, "inflight"))
            .await
            .unwrap();
        store.mark_in_flight(in_flight_id).await.unwrap();
        // Dropped here without ever marking succeeded/terminal — simulates a
        // process death mid-delivery.
    }
    {
        let store = factory.open().await;
        store.recover_in_flight(Timestamp::now()).await.unwrap();
        let claimed = store.claim_due(Timestamp::now(), 100).await.unwrap();
        assert!(
            claimed.iter().any(|e| e.id == in_flight_id),
            "an in-flight-at-drop effect must become claimable after reopen"
        );
    }

    // --- a scoped dedup reservation survives reopen ---
    let dedup_scope = scope("tenant-a", "durable-uow:0");
    let dedup_owner = EffectId::new();
    {
        let store = factory.open().await;
        let outcome = store
            .reserve(&dedup_scope, dedup_owner, fp("durable"))
            .await
            .unwrap();
        assert_eq!(outcome, DedupOutcome::Fresh);
    }
    {
        let store = factory.open().await;
        let outcome = store
            .reserve(&dedup_scope, dedup_owner, fp("durable"))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            DedupOutcome::OwnedInProgress,
            "a same-owner reservation must survive reopen as OwnedInProgress, never Fresh"
        );
    }
}

/// Tier 3 — multi-node conformance (design §3.6). Reuses
/// [`DurableStoreFactory`] by calling `open()` **twice** without dropping
/// either result (concurrent, not sequential — models two live nodes).
/// Capability-gated: a no-op unless the factory's store declares
/// `multi_node_safe: true`, so Stoolap (which declares `multi_node_safe:
/// false`) is never asked to prove a guarantee it does not offer.
pub async fn run_multi_node_conformance<F>(factory: &F)
where
    F: DurableStoreFactory,
{
    let node_a = factory.open().await;
    let node_b = factory.open().await;

    if !EffectStateStore::capabilities(&node_a).multi_node_safe {
        return;
    }

    let id = EffectId::new();
    node_a.accept(accepted(id, "multi-node")).await.unwrap();

    let claimed_a = node_a.claim_due(Timestamp::now(), 10).await.unwrap();
    let claimed_b = node_b.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        !(claimed_a.iter().any(|e| e.id == id) && claimed_b.iter().any(|e| e.id == id)),
        "two claimers must never both observe the same effect as claimable \
         while a valid claim is held"
    );
}
