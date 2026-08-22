//! `PostgresEffectStore` claim/lease/reclaim/retention behavior against a
//! real PostgreSQL — assertions specific to what *this* backend's SQL
//! actually enforces, not to the `EffectStateStore`/`EffectDedupStore`
//! contract every backend must satisfy identically (that shared contract is
//! Tier 1, `crates/effect-store/tests/conformance.rs`, unmoved).
//!
//! **Guarantee:** claim exclusivity (a live claim is never re-stamped),
//! expired-lease reclaim (`claim_due`/`recover_in_flight` scoped to expired
//! leases only), epoch-fenced writes (a superseded worker's write is
//! rejected, never silently applied), atomic dedup reservation (no partial
//! state across a crash), the AD-9 retention batch bound, and the G10
//! clock-injection guarantee that a lease decision is computed from the
//! store's *injected* clock, never wall-clock time.
//!
//! **Layers traversed:** `PostgresEffectStore` → real SQL, real
//! transactions, real row-level guards → PostgreSQL.
//!
//! **Why in-process cannot show this.** Every one of these is a property of
//! what the real database enforces under a real conditional `UPDATE`, a real
//! primary-key upsert, or real row atomicity — none of which a scripted
//! double can misrepresent in a way this suite would catch.
//!
//! Relocated twice: first out of
//! `crates/effect-store/src/postgres/mod.rs`'s `#[cfg(test)]` module
//! (`ego-rs-testing`: a test needing a real external resource must live
//! outside a production crate), then — PROD-002 G11 — out of the old
//! per-crate `crates/integration-tests` (one `testcontainers` container per
//! test file, its own ad-hoc per-test schema) into this suite's shared
//! container and per-test isolated **database**. Each test's database is
//! already exclusive to it, so unlike the pre-G11 version there is nothing
//! left for a schema name to disambiguate — a fixed name is used instead of
//! a `uuid`-suffixed one.
//!
//! The tests that simulate lease expiry do not reach into the store's
//! private connection pool to hand-edit `claim_expires_at` (that field isn't
//! visible outside `ego-effect-store`) — instead they construct a store with
//! a short real lease and sleep past it, exactly like the
//! `PostgresDurableStoreFactory` pattern in
//! `effect_store_postgres_conformance.rs`. That is black-box through the
//! public API: it proves the same thing (an expired lease's row becomes
//! reclaimable) by letting a real lease actually expire.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ego_domain::{Clock, IdempotencyKey, SystemClock, TenantId};
use ego_effect_store::conformance::accepted;
use ego_effect_store::PostgresEffectStore;
use ego_integration_tests::{isolated_database, IsolatedDatabase};
use ego_runtime::effects::store::{
    DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId, EffectState,
    EffectStateStore, EffectStoreError, Timestamp,
};

/// Fixed rather than `uuid`-suffixed (see module docs): each test already
/// owns an exclusive database from the harness, so nothing else can ever
/// share it.
const SCHEMA: &str = "effect_store";

/// A fixed-time clock for deterministic testing — same idiom as
/// `security-jwt`/`security-apikey`'s local test-only `FixedClock` doubles.
struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// A default, non-expiring-for-the-test-duration lease, matching what the
/// original in-crate tests used before the short-lease rewrites below.
const NORMAL_LEASE: Duration = Duration::seconds(30);
/// Short enough that sleeping `3 * SHORT_LEASE_MS` deterministically expires
/// it, without needing to reach into the store's private connection pool.
const SHORT_LEASE_MS: i64 = 50;

/// A store against this test's isolated database, with `SystemClock`.
async fn store(db: &IsolatedDatabase, lease: Duration) -> PostgresEffectStore {
    PostgresEffectStore::connect(db.url(), SCHEMA, lease, Arc::new(SystemClock))
        .await
        .expect("connect PostgresEffectStore")
}

/// A store against this test's isolated database, with an injected clock.
async fn store_with_clock(
    db: &IsolatedDatabase,
    lease: Duration,
    clock: Arc<dyn Clock>,
) -> PostgresEffectStore {
    PostgresEffectStore::connect(db.url(), SCHEMA, lease, clock)
        .await
        .expect("connect PostgresEffectStore with an injected clock")
}

/// 5.3 RED: a second `claim_due` call must never re-stamp a row already
/// carrying a live claim (G1 fix).
#[tokio::test]
async fn claim_due_never_re_stamps_a_row_already_carrying_a_live_claim() {
    let db = isolated_database().await;
    let s = store(&db, NORMAL_LEASE).await;

    let id = EffectId::new();
    s.accept(accepted(id, "g1")).await.unwrap();

    let first = s.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        first.iter().any(|e| e.id == id),
        "first claim_due must claim the fresh row"
    );

    let second = s.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        !second.iter().any(|e| e.id == id),
        "a second claim_due call must not re-stamp a row a live claim already covers"
    );

    db.close().await;
}

/// 5.4 RED: `claim_due` picks up expired-lease `in_flight` rows alongside
/// due `pending`/`retryable_failed`, without transitioning `state` (AD-4).
#[tokio::test]
async fn claim_due_picks_up_expired_lease_in_flight_rows_without_transitioning_state() {
    let db = isolated_database().await;
    let s = store(&db, Duration::milliseconds(SHORT_LEASE_MS)).await;

    let id = EffectId::new();
    s.accept(accepted(id, "stale")).await.unwrap();
    s.mark_in_flight(id).await.unwrap();

    // Let the short real lease actually expire.
    tokio::time::sleep(std::time::Duration::from_millis(SHORT_LEASE_MS as u64 * 3)).await;

    let claimed = s.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        claimed
            .iter()
            .any(|e| e.id == id && e.state == EffectState::InFlight),
        "an expired-lease in_flight row must be claimable, with its state reported as InFlight"
    );

    db.close().await;
}

/// PROD-002 G10: `mark_in_flight` must compute `claim_expires_at` from the
/// store's *injected* `Clock`, not real wall-clock time — proven here
/// deterministically (no `sleep`) by pinning the clock to an instant already
/// far enough in the past that `now + lease` is still in the past relative to
/// real wall-clock time. If `mark_in_flight` used `Utc::now()` directly (the
/// pre-fix behavior), `claim_expires_at` would be close to real "now" and
/// this row would NOT yet be reclaimable.
#[tokio::test]
async fn mark_in_flight_computes_claim_expires_at_from_the_injected_clock_not_wall_clock() {
    let db = isolated_database().await;
    let pinned_past = Utc::now() - Duration::hours(1);
    let lease = Duration::minutes(1);
    let s = store_with_clock(&db, lease, Arc::new(FixedClock(pinned_past))).await;

    let id = EffectId::new();
    s.accept(accepted(id, "pinned-clock")).await.unwrap();
    s.mark_in_flight(id).await.unwrap();

    // No sleep: claim_expires_at = pinned_past + 1 minute, ~59 minutes before
    // real wall-clock "now" — already expired without waiting at all.
    let claimed = s.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        claimed
            .iter()
            .any(|e| e.id == id && e.state == EffectState::InFlight),
        "a claim computed from a clock pinned in the past must already be reclaimable, \
         proving mark_in_flight used the injected clock rather than real wall-clock time"
    );

    db.close().await;
}

/// 5.6 RED: a superseded worker's conditional UPDATE affects 0 rows ->
/// Conflict; a live worker's transition applies.
#[tokio::test]
async fn superseded_worker_write_is_conflict_live_worker_succeeds() {
    let db = isolated_database().await;
    let lease = Duration::milliseconds(SHORT_LEASE_MS);

    let worker_a = store(&db, lease).await;
    let worker_b = store(&db, lease).await;

    let id = EffectId::new();
    worker_a.accept(accepted(id, "reclaim")).await.unwrap();
    worker_a.mark_in_flight(id).await.unwrap();

    // A's short lease expires; B reclaims via claim_due (state stays in_flight).
    tokio::time::sleep(std::time::Duration::from_millis(SHORT_LEASE_MS as u64 * 3)).await;
    let reclaimed = worker_b.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(reclaimed.iter().any(|e| e.id == id));

    // A's stale write now fails: its ownership was superseded.
    let err = worker_a.mark_succeeded(id).await.unwrap_err();
    assert!(
        matches!(err, EffectStoreError::Conflict(_)),
        "a superseded worker's write must be Conflict, got {err:?}"
    );

    // B's write succeeds: it now legitimately owns the claim.
    worker_b.mark_succeeded(id).await.unwrap();

    db.close().await;
}

/// 5.8 RED: `recover_in_flight` is scoped to expired-lease rows only — it
/// must never reset a live peer's in-flight row.
#[tokio::test]
async fn recover_in_flight_never_resets_a_live_peers_in_flight_row() {
    let db = isolated_database().await;

    // Same schema (and, post-G11, the same isolated database), two different
    // lease durations: one that will genuinely expire before we check, one
    // that stays valid throughout.
    let short_lived = store(&db, Duration::milliseconds(SHORT_LEASE_MS)).await;
    let long_lived = store(&db, NORMAL_LEASE).await;

    let expired_id = EffectId::new();
    short_lived
        .accept(accepted(expired_id, "expired"))
        .await
        .unwrap();
    short_lived.mark_in_flight(expired_id).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(SHORT_LEASE_MS as u64 * 3)).await;

    let live_id = EffectId::new();
    long_lived.accept(accepted(live_id, "live")).await.unwrap();
    long_lived.mark_in_flight(live_id).await.unwrap();

    let recovered = long_lived
        .recover_in_flight(Timestamp::now())
        .await
        .unwrap();
    assert_eq!(recovered, 1, "only the expired-lease row must be recovered");

    let claimable = long_lived.claim_due(Timestamp::now(), 100).await.unwrap();
    assert!(
        claimable.iter().any(|e| e.id == expired_id),
        "the expired row must become claimable again"
    );
    assert!(
        !claimable.iter().any(|e| e.id == live_id),
        "a live peer's still-valid claim must survive recover_in_flight untouched"
    );

    db.close().await;
}

/// 5.10 RED: dedup `reserve`/`commit_success`/`release` — atomic upsert,
/// in-place `succeeded` flip, crash-mid-reservation leaves no partial state
/// (AD-8).
#[tokio::test]
async fn dedup_reservation_is_atomic_and_commit_success_never_deletes() {
    let db = isolated_database().await;
    let s = store(&db, NORMAL_LEASE).await;

    let scope = DedupScope {
        tenant: TenantId::new("tenant-a").unwrap(),
        effect_type: "invoice.created".to_string(),
        key: IdempotencyKey::new("atomic-uow:0").unwrap(),
    };
    let owner = EffectId::new();
    let fp = EffectFingerprint::compute(b"atomic", "https://example.com");

    assert_eq!(
        s.reserve(&scope, owner, fp).await.unwrap(),
        DedupOutcome::Fresh
    );
    s.commit_success(&scope).await.unwrap();
    assert_eq!(
        s.reserve(&scope, owner, fp).await.unwrap(),
        DedupOutcome::OwnedSucceeded,
        "commit_success must flip succeeded in place, never delete the row"
    );
    assert_eq!(
        s.reserve(&scope, EffectId::new(), fp).await.unwrap(),
        DedupOutcome::OtherSucceeded
    );

    db.close().await;
}

/// `run_retention`'s `now`/`ttl` are evaluated against the client's own
/// clock, then compared against `settled_at` values the *server* stamped
/// via its own `now()`. A zero-`ttl` test asserting "everything just
/// settled is eligible" is otherwise vulnerable to ordinary
/// test-runner/DB clock skew — a few seconds of margin absorbs any
/// reasonable skew without weakening what the test actually proves (the
/// batch limit, not TTL-boundary precision).
fn retention_cutoff_with_skew_margin() -> Timestamp {
    Timestamp::from_utc(Utc::now() + Duration::seconds(5))
}

/// AD-9 batch bound: retention must delete exactly `batch` rows in one
/// call, never all eligible rows.
#[tokio::test]
async fn run_retention_respects_the_batch_limit() {
    let db = isolated_database().await;
    let s = store(&db, NORMAL_LEASE).await;

    const ELIGIBLE: usize = 5;
    const BATCH: i64 = 2;
    for i in 0..ELIGIBLE {
        let id = EffectId::new();
        s.accept(accepted(id, &format!("batch-{i}"))).await.unwrap();
        s.mark_in_flight(id).await.unwrap();
        s.mark_succeeded(id).await.unwrap();
    }

    let deleted = s
        .run_retention(
            retention_cutoff_with_skew_margin(),
            Duration::seconds(0),
            BATCH,
        )
        .await
        .unwrap();
    assert_eq!(
        deleted, BATCH as u64,
        "run_retention must respect the batch limit"
    );

    db.close().await;
}

/// Correction-round fix 1 (TOCTOU): a live, non-terminal reservation
/// sharing a settled scope key must survive retention.
#[tokio::test]
async fn run_retention_does_not_delete_a_live_non_terminal_dedup_reservation_sharing_a_scope_key() {
    let db = isolated_database().await;
    let s = store(&db, NORMAL_LEASE).await;

    let scope = DedupScope {
        tenant: TenantId::new("tenant-a").unwrap(),
        effect_type: "invoice.created".to_string(),
        key: IdempotencyKey::new("toctou-uow:0").unwrap(),
    };
    let fp = EffectFingerprint::compute(b"payload", "https://example.com");

    let old_owner = EffectId::new();
    assert_eq!(
        s.reserve(&scope, old_owner, fp).await.unwrap(),
        DedupOutcome::Fresh
    );
    s.commit_success(&scope).await.unwrap();

    s.release(&scope).await.unwrap();
    let new_owner = EffectId::new();
    assert_eq!(
        s.reserve(&scope, new_owner, fp).await.unwrap(),
        DedupOutcome::Fresh
    );

    s.run_retention(
        retention_cutoff_with_skew_margin(),
        Duration::seconds(0),
        100,
    )
    .await
    .unwrap();

    assert_eq!(
        s.reserve(&scope, new_owner, fp).await.unwrap(),
        DedupOutcome::OwnedInProgress,
        "the fresh reservation sharing the settled scope key must survive retention"
    );

    db.close().await;
}

/// `capabilities()` — both ports independently declare the durable,
/// multi-node-safe profile.
#[tokio::test]
async fn postgres_declares_durable_multi_node_safe_capabilities() {
    let db = isolated_database().await;
    let s = store(&db, NORMAL_LEASE).await;

    let state_caps = EffectStateStore::capabilities(&s);
    assert!(state_caps.durable);
    assert!(state_caps.concurrent_local_safe);
    assert!(state_caps.multi_node_safe);
    assert!(state_caps.supports_leases);

    let dedup_caps = EffectDedupStore::capabilities(&s);
    assert_eq!(dedup_caps, state_caps);

    db.close().await;
}
