//! `PostgresEffectStore` claim/lease/reclaim/retention behavior against a
//! real Postgres testcontainer. Relocated here from
//! `crates/effect-store/src/postgres/mod.rs`'s `#[cfg(test)] mod tests`
//! (`ego-rs-testing`: a test needing a real external resource must live in
//! this crate, not inline in a production crate).
//!
//! The three tests that simulate lease expiry no longer reach into the
//! store's private connection pool to hand-edit `claim_expires_at` (that
//! field isn't visible outside `ego-effect-store`) — instead they construct
//! a store with a short real lease and sleep past it, exactly like the
//! `PostgresDurableStoreFactory` pattern already established in
//! `effect_store_postgres_conformance.rs`. That is black-box through the
//! public API, not a step down from the original white-box check: it proves
//! the same thing (an expired lease's row becomes reclaimable) by letting a
//! real lease actually expire rather than editing the column directly.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ego_domain::{Clock, IdempotencyKey, SystemClock, TenantId};
use ego_effect_store::conformance::accepted;
use ego_effect_store::PostgresEffectStore;
use ego_runtime::effects::store::{
    DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId, EffectState,
    EffectStateStore, EffectStoreError, Timestamp,
};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const POSTGRES_IMAGE_TAG: &str = "14-alpine";

async fn start_postgres() -> (String, ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_tag(POSTGRES_IMAGE_TAG)
        .start()
        .await
        .expect(
            "the Postgres testcontainer must start; if Docker is not running \
             this test cannot run and must fail loudly, not be skipped",
        );

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("the container must publish its mapped Postgres port");
    let host = container
        .get_host()
        .await
        .expect("the container must report a reachable host address")
        .to_string();
    let host = if host == "localhost" {
        "127.0.0.1".to_string()
    } else {
        host
    };

    (
        format!("postgres://postgres:postgres@{host}:{port}/postgres"),
        container,
    )
}

/// A fresh store against its own schema, backed by a fresh container. The
/// container guard must stay alive for as long as the store is used.
async fn fresh_store(lease: Duration) -> (PostgresEffectStore, ContainerAsync<Postgres>) {
    let (url, container) = start_postgres().await;
    let schema = format!("effect_unit_{}", uuid::Uuid::new_v4().simple());
    let store = PostgresEffectStore::connect(&url, &schema, lease, Arc::new(SystemClock))
        .await
        .expect("connect PostgresEffectStore");
    (store, container)
}

/// A fixed-time clock for deterministic testing — same idiom as
/// `security-jwt`/`security-apikey`'s local test-only `FixedClock` doubles.
struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// A default, non-expiring-for-the-test-duration lease, matching what the
/// original in-crate tests used before this file's short-lease rewrites.
const NORMAL_LEASE: Duration = Duration::seconds(30);
/// Short enough that sleeping `3 * SHORT_LEASE_MS` deterministically expires
/// it, without needing to reach into the store's private connection pool.
const SHORT_LEASE_MS: i64 = 50;

/// 5.3 RED: a second `claim_due` call must never re-stamp a row already
/// carrying a live claim (G1 fix).
#[tokio::test]
async fn claim_due_never_re_stamps_a_row_already_carrying_a_live_claim() {
    let (store, _container) = fresh_store(NORMAL_LEASE).await;

    let id = EffectId::new();
    store.accept(accepted(id, "g1")).await.unwrap();

    let first = store.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        first.iter().any(|e| e.id == id),
        "first claim_due must claim the fresh row"
    );

    let second = store.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        !second.iter().any(|e| e.id == id),
        "a second claim_due call must not re-stamp a row a live claim already covers"
    );
}

/// 5.4 RED: `claim_due` picks up expired-lease `in_flight` rows alongside
/// due `pending`/`retryable_failed`, without transitioning `state` (AD-4).
#[tokio::test]
async fn claim_due_picks_up_expired_lease_in_flight_rows_without_transitioning_state() {
    let (store, _container) = fresh_store(Duration::milliseconds(SHORT_LEASE_MS)).await;

    let id = EffectId::new();
    store.accept(accepted(id, "stale")).await.unwrap();
    store.mark_in_flight(id).await.unwrap();

    // Let the short real lease actually expire.
    tokio::time::sleep(std::time::Duration::from_millis(SHORT_LEASE_MS as u64 * 3)).await;

    let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        claimed
            .iter()
            .any(|e| e.id == id && e.state == EffectState::InFlight),
        "an expired-lease in_flight row must be claimable, with its state reported as InFlight"
    );
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
    let (url, _container) = start_postgres().await;
    let schema = format!("effect_unit_{}", uuid::Uuid::new_v4().simple());
    let pinned_past = Utc::now() - Duration::hours(1);
    let lease = Duration::minutes(1);
    let store =
        PostgresEffectStore::connect(&url, &schema, lease, Arc::new(FixedClock(pinned_past)))
            .await
            .expect("connect PostgresEffectStore with a pinned-past clock");

    let id = EffectId::new();
    store.accept(accepted(id, "pinned-clock")).await.unwrap();
    store.mark_in_flight(id).await.unwrap();

    // No sleep: claim_expires_at = pinned_past + 1 minute, ~59 minutes before
    // real wall-clock "now" — already expired without waiting at all.
    let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();
    assert!(
        claimed
            .iter()
            .any(|e| e.id == id && e.state == EffectState::InFlight),
        "a claim computed from a clock pinned in the past must already be reclaimable, \
         proving mark_in_flight used the injected clock rather than real wall-clock time"
    );
}

/// 5.6 RED: a superseded worker's conditional UPDATE affects 0 rows ->
/// Conflict; a live worker's transition applies.
#[tokio::test]
async fn superseded_worker_write_is_conflict_live_worker_succeeds() {
    let (database_url, _container) = start_postgres().await;
    let schema = format!("effect_unit_{}", uuid::Uuid::new_v4().simple());
    let lease = Duration::milliseconds(SHORT_LEASE_MS);

    let worker_a =
        PostgresEffectStore::connect(&database_url, &schema, lease, Arc::new(SystemClock))
            .await
            .expect("connect worker_a");
    let worker_b =
        PostgresEffectStore::connect(&database_url, &schema, lease, Arc::new(SystemClock))
            .await
            .expect("connect worker_b");

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
}

/// 5.8 RED: `recover_in_flight` is scoped to expired-lease rows only — it
/// must never reset a live peer's in-flight row.
#[tokio::test]
async fn recover_in_flight_never_resets_a_live_peers_in_flight_row() {
    let (database_url, _container) = start_postgres().await;
    let schema = format!("effect_unit_{}", uuid::Uuid::new_v4().simple());

    // Same schema, two different lease durations: one that will genuinely
    // expire before we check, one that stays valid throughout.
    let short_lived = PostgresEffectStore::connect(
        &database_url,
        &schema,
        Duration::milliseconds(SHORT_LEASE_MS),
        Arc::new(SystemClock),
    )
    .await
    .expect("connect short-lease PostgresEffectStore");
    let long_lived =
        PostgresEffectStore::connect(&database_url, &schema, NORMAL_LEASE, Arc::new(SystemClock))
            .await
            .expect("connect long-lease PostgresEffectStore");

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
}

/// 5.10 RED: dedup `reserve`/`commit_success`/`release` — atomic upsert,
/// in-place `succeeded` flip, crash-mid-reservation leaves no partial state
/// (AD-8).
#[tokio::test]
async fn dedup_reservation_is_atomic_and_commit_success_never_deletes() {
    let (store, _container) = fresh_store(NORMAL_LEASE).await;

    let s = DedupScope {
        tenant: TenantId::new("tenant-a").unwrap(),
        effect_type: "invoice.created".to_string(),
        key: IdempotencyKey::new("atomic-uow:0").unwrap(),
    };
    let owner = EffectId::new();
    let fp = EffectFingerprint::compute(b"atomic", "https://example.com");

    assert_eq!(
        store.reserve(&s, owner, fp).await.unwrap(),
        DedupOutcome::Fresh
    );
    store.commit_success(&s).await.unwrap();
    assert_eq!(
        store.reserve(&s, owner, fp).await.unwrap(),
        DedupOutcome::OwnedSucceeded,
        "commit_success must flip succeeded in place, never delete the row"
    );
    assert_eq!(
        store.reserve(&s, EffectId::new(), fp).await.unwrap(),
        DedupOutcome::OtherSucceeded
    );
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
    let (store, _container) = fresh_store(NORMAL_LEASE).await;

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
}

/// Correction-round fix 1 (TOCTOU): a live, non-terminal reservation
/// sharing a settled scope key must survive retention.
#[tokio::test]
async fn run_retention_does_not_delete_a_live_non_terminal_dedup_reservation_sharing_a_scope_key() {
    let (store, _container) = fresh_store(NORMAL_LEASE).await;

    let s = DedupScope {
        tenant: TenantId::new("tenant-a").unwrap(),
        effect_type: "invoice.created".to_string(),
        key: IdempotencyKey::new("toctou-uow:0").unwrap(),
    };
    let fp = EffectFingerprint::compute(b"payload", "https://example.com");

    let old_owner = EffectId::new();
    assert_eq!(
        store.reserve(&s, old_owner, fp).await.unwrap(),
        DedupOutcome::Fresh
    );
    store.commit_success(&s).await.unwrap();

    store.release(&s).await.unwrap();
    let new_owner = EffectId::new();
    assert_eq!(
        store.reserve(&s, new_owner, fp).await.unwrap(),
        DedupOutcome::Fresh
    );

    store
        .run_retention(
            retention_cutoff_with_skew_margin(),
            Duration::seconds(0),
            100,
        )
        .await
        .unwrap();

    assert_eq!(
        store.reserve(&s, new_owner, fp).await.unwrap(),
        DedupOutcome::OwnedInProgress,
        "the fresh reservation sharing the settled scope key must survive retention"
    );
}

/// `capabilities()` — both ports independently declare the durable,
/// multi-node-safe profile.
#[tokio::test]
async fn postgres_declares_durable_multi_node_safe_capabilities() {
    let (store, _container) = fresh_store(NORMAL_LEASE).await;

    let state_caps = EffectStateStore::capabilities(&store);
    assert!(state_caps.durable);
    assert!(state_caps.concurrent_local_safe);
    assert!(state_caps.multi_node_safe);
    assert!(state_caps.supports_leases);

    let dedup_caps = EffectDedupStore::capabilities(&store);
    assert_eq!(dedup_caps, state_caps);
}
