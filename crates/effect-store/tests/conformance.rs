//! In-process conformance suite (PROD-002 AD-13, design.md §3.6): runs the
//! shared harness from [`ego_effect_store::conformance`] against providers
//! that need no external resource — [`InMemoryEffectStore`] (Tier 1 only)
//! and, feature-gated, the embedded Stoolap provider (Tiers 1-2). The
//! Postgres tiers need a real container and live in
//! `crates/integration-tests/tests/effect_store_postgres_conformance.rs`
//! instead (`ego-rs-testing`: tests needing a real external resource must
//! live there, not inline in a production crate's own test binary).

use async_trait::async_trait;
use chrono::Duration;
use ego_effect_store::conformance::{
    accepted, fp, run_dedup_conformance, run_durable_conformance, run_multi_node_conformance,
    run_state_store_conformance, scope, DurableStoreFactory,
};
use ego_runtime::effects::store::{
    DedupOutcome, EffectDedupStore, EffectId, EffectStateStore, EffectStoreError,
    InMemoryEffectStore, Timestamp,
};

#[cfg(test)]
mod tier1_in_memory {
    use super::*;

    #[tokio::test]
    async fn in_memory_satisfies_state_store_conformance() {
        let store = InMemoryEffectStore::new();
        run_state_store_conformance(&store).await;
    }

    #[tokio::test]
    async fn in_memory_satisfies_dedup_conformance() {
        let store = InMemoryEffectStore::new();
        run_dedup_conformance(&store).await;
    }

    /// Tier 2 negative test (3.4, design §3.6): `InMemoryEffectStore`
    /// deliberately implements no `DurableStoreFactory` — a fresh in-memory
    /// instance shares no backing with a dropped one. This proves its honest
    /// non-durability is a *passing* assertion, not a silent omission (spec:
    /// "In-memory store loses undelivered effects on crash").
    #[tokio::test]
    async fn in_memory_effect_store_does_not_survive_drop_and_reconstruct() {
        let id = EffectId::new();
        {
            let store = InMemoryEffectStore::new();
            store.accept(accepted(id, "lost")).await.unwrap();
        }
        // A brand new instance shares no backing storage with the dropped one.
        let new_store = InMemoryEffectStore::new();
        let err = new_store.mark_in_flight(id).await.unwrap_err();
        assert!(
            matches!(err, EffectStoreError::NotFound(found) if found == id),
            "a fresh InMemoryEffectStore must have no memory of a prior instance's effects"
        );
    }
}

#[cfg(feature = "stoolap")]
mod tier1_stoolap {
    use super::*;
    use ego_effect_store::StoolapEffectStore;

    async fn fresh_store() -> StoolapEffectStore {
        let dir = tempfile::tempdir().expect("tempdir");
        StoolapEffectStore::open(dir.path())
            .await
            .expect("open StoolapEffectStore")
    }

    #[tokio::test]
    async fn stoolap_satisfies_state_store_conformance() {
        let store = fresh_store().await;
        run_state_store_conformance(&store).await;
    }

    #[tokio::test]
    async fn stoolap_satisfies_dedup_conformance() {
        let store = fresh_store().await;
        run_dedup_conformance(&store).await;
    }

    #[tokio::test]
    async fn stoolap_declares_durable_local_only_capabilities() {
        let store = fresh_store().await;

        let state_caps = EffectStateStore::capabilities(&store);
        assert!(state_caps.durable);
        assert!(state_caps.concurrent_local_safe);
        assert!(!state_caps.multi_node_safe);
        assert!(!state_caps.supports_leases);

        // G6: EffectDedupStore declares its own profile independently — must
        // not silently drift from EffectStateStore's (the two capability
        // literals are separate in StoolapEffectStore and could diverge on a
        // copy-paste edit without this assertion catching it).
        let dedup_caps = EffectDedupStore::capabilities(&store);
        assert_eq!(dedup_caps, state_caps);
    }

    /// 4.3: dedup crash-mid-reservation — the atomic upsert leaves no partial
    /// state, and `commit_success` flips `succeeded` in place, never deletes.
    #[tokio::test]
    async fn stoolap_dedup_reservation_is_atomic_and_commit_success_never_deletes() {
        let store = fresh_store().await;
        let s = scope("tenant-a", "atomic-uow:0");
        let owner = EffectId::new();

        // Reserve, succeed, then re-reserve: the row must still be there,
        // flipped in place, not removed.
        assert_eq!(
            store.reserve(&s, owner, fp("atomic")).await.unwrap(),
            DedupOutcome::Fresh
        );
        store.commit_success(&s).await.unwrap();
        assert_eq!(
            store.reserve(&s, owner, fp("atomic")).await.unwrap(),
            DedupOutcome::OwnedSucceeded
        );

        // A different submission for the same scope must see it settled too
        // — no partial/half-written state ever observable.
        assert_eq!(
            store
                .reserve(&s, EffectId::new(), fp("atomic"))
                .await
                .unwrap(),
            DedupOutcome::OtherSucceeded
        );
    }

    /// 4.4: provider-owned TTL retention (AD-9) — batched delete of settled
    /// rows, never touching pending/in-flight effects.
    #[tokio::test]
    async fn stoolap_retention_deletes_only_settled_rows_past_ttl() {
        let store = fresh_store().await;

        let settled_id = EffectId::new();
        store.accept(accepted(settled_id, "settled")).await.unwrap();
        store.mark_in_flight(settled_id).await.unwrap();
        store.mark_succeeded(settled_id).await.unwrap();

        let pending_id = EffectId::new();
        store.accept(accepted(pending_id, "pending")).await.unwrap();

        // TTL of zero: the settled row is immediately eligible; the pending
        // row must never be touched regardless of TTL.
        let deleted = store
            .run_retention(Timestamp::now(), Duration::seconds(0), 100)
            .await
            .unwrap();
        assert_eq!(deleted, 1, "only the settled row must be deleted");

        let err = store.mark_in_flight(settled_id).await.unwrap_err();
        assert!(matches!(err, EffectStoreError::NotFound(_)));

        // The pending effect must be untouched.
        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert!(claimed.iter().any(|e| e.id == pending_id));
    }

    /// 4.4 / AD-9 bounded-batch guarantee (fix 6, reliability review): more
    /// eligible settled rows exist than `batch` — `run_retention` must delete
    /// exactly `batch` rows in one call, never all of them.
    #[tokio::test]
    async fn stoolap_retention_respects_the_batch_limit() {
        let store = fresh_store().await;

        const ELIGIBLE: usize = 5;
        const BATCH: i64 = 2;

        for i in 0..ELIGIBLE {
            let id = EffectId::new();
            store
                .accept(accepted(id, &format!("batch-{i}")))
                .await
                .unwrap();
            store.mark_in_flight(id).await.unwrap();
            store.mark_succeeded(id).await.unwrap();
        }

        let deleted = store
            .run_retention(Timestamp::now(), Duration::seconds(0), BATCH)
            .await
            .unwrap();
        assert_eq!(
            deleted, BATCH as u64,
            "run_retention must delete exactly `batch` rows, not all eligible rows"
        );
    }

    /// PROD-002 G12: the `RetentionMaintenance` capability wiring calls
    /// through to the SAME `run_retention` SQL — not a reimplementation —
    /// so a settled row purged via the trait is indistinguishable from one
    /// purged by calling `run_retention` directly.
    #[tokio::test]
    async fn retention_maintenance_purge_before_calls_through_to_run_retention() {
        use ego_runtime::effects::store::RetentionMaintenance;

        let store = fresh_store().await;

        let settled_id = EffectId::new();
        store.accept(accepted(settled_id, "settled")).await.unwrap();
        store.mark_in_flight(settled_id).await.unwrap();
        store.mark_succeeded(settled_id).await.unwrap();

        let pending_id = EffectId::new();
        store.accept(accepted(pending_id, "pending")).await.unwrap();

        let deleted = RetentionMaintenance::purge_before(&store, Timestamp::now(), 100)
            .await
            .unwrap();
        assert_eq!(deleted, 1, "only the settled row must be deleted");

        let err = store.mark_in_flight(settled_id).await.unwrap_err();
        assert!(matches!(err, EffectStoreError::NotFound(_)));
        let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert!(claimed.iter().any(|e| e.id == pending_id));
    }

    /// PROD-002 G12: neither provider tracks the oldest-settled-row query
    /// yet, so `oldest_terminal` must fall through to the trait default
    /// rather than the provider silently claiming an empty backlog.
    #[tokio::test]
    async fn retention_maintenance_oldest_terminal_defaults_to_none() {
        use ego_runtime::effects::store::RetentionMaintenance;

        let store = fresh_store().await;
        assert_eq!(
            RetentionMaintenance::oldest_terminal(&store).await,
            Ok(None)
        );
    }
}

#[cfg(feature = "stoolap")]
mod tier2_stoolap_durable {
    use super::*;
    use ego_effect_store::StoolapEffectStore;

    /// 4.6: `StoolapDurableStoreFactory` — owns a fixed `tempfile::TempDir`;
    /// `open()` reopens a `StoolapEffectStore` at that same path, so a
    /// dropped store's data is genuinely reloaded by the next `open()`.
    struct StoolapDurableStoreFactory {
        dir: tempfile::TempDir,
    }

    impl StoolapDurableStoreFactory {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().expect("tempdir"),
            }
        }
    }

    #[async_trait]
    impl DurableStoreFactory for StoolapDurableStoreFactory {
        type Store = StoolapEffectStore;

        async fn open(&self) -> Self::Store {
            StoolapEffectStore::open(self.dir.path())
                .await
                .expect("reopen StoolapEffectStore at the same path")
        }
    }

    #[tokio::test]
    async fn stoolap_satisfies_durable_conformance() {
        let factory = StoolapDurableStoreFactory::new();
        run_durable_conformance(&factory).await;
    }

    /// Stoolap declares `multi_node_safe: false` — Tier 3 must be a
    /// documented no-op against it, never exercised for real (design §3.6).
    #[tokio::test]
    async fn stoolap_multi_node_conformance_is_a_documented_no_op() {
        let factory = StoolapDurableStoreFactory::new();
        run_multi_node_conformance(&factory).await;
    }
}
