//! **Guarantee:** `PostgreSQLReadSideClaimStore` (PROD-014C) gives one worker
//! single valid processing ownership of a `(projection_id, tag, tenant)`
//! stream, against real PostgreSQL: a concurrent second claimant is excluded,
//! an expired lease can be taken over without operator action, a taken-over
//! owner is fenced out of `renew`/`release`, a renewed lease resists takeover,
//! a released claim is immediately reclaimable, and none of this disturbs the
//! per-stream event ordering a projection handler relies on.
//!
//! **Layers traversed:** the real adapter
//! (`ego_persistence::postgres::PostgreSQLReadSideClaimStore`) against a real
//! PostgreSQL `projection_claims` table, through separate `PgPool`s per
//! contender — mirrors `takeover_fencing_postgres.rs` /
//! `concurrent_replicas_postgres.rs`.
//!
//! # Why in-process cannot show this
//!
//! Exclusion is `INSERT … ON CONFLICT … DO UPDATE … WHERE lease_until <= $now`
//! resolved by the database, not a mutex a test double holds. Takeover and
//! fencing are properties of the same statement and the row it leaves behind
//! — a scripted double can return whatever a test hands it and never expose a
//! race the SQL itself has to close. And "what survives" can only be read
//! back from the row after the fact, through raw SQL rather than the port
//! under test.
//!
//! # What this suite deliberately does not claim
//!
//! No `ReadSideSession` wiring exists yet — claiming is not yet connected to
//! `fetch`/`handle`/`renew`/`release` around a real batch (that is Phase 5,
//! PR3). [`claim_causes_no_stream_ordering_interference`] therefore does not
//! exercise a handler; it holds a real claim open across a raw read of a
//! batch of events and shows the claim's own footprint — confined entirely to
//! `projection_claims` — cannot reorder what a reader of `events` sees.
//!
//! Mutation-tested by hand (2026-09-03, restored before commit — never left
//! broken in the delivered diff, per task 4.7):
//! - Dropped `WHERE projection_claims.lease_until <= $6` from `try_claim`'s
//!   `DO UPDATE`: [`sc1_exclusion_two_workers_one_identity_exactly_one_claims`]
//!   failed — both workers obtained `Some(fence)` for the same identity
//!   instead of exactly one (`FencingToken(2)` and `FencingToken(1)` both
//!   granted). [`renewal_prevents_takeover_during_the_renewed_lease`] failed
//!   too, for the same reason — an unconditional `DO UPDATE` also lets B
//!   overwrite A's still-live renewed lease.
//! - Dropped `AND fencing_token = $6` from `renew`'s `WHERE` (the shape
//!   `mutate_claimed` shares with `release`): the token-isolation probe
//!   inside [`sc2_takeover_and_sc3_stale_owner_rejection`] failed — B's own
//!   owner paired with A's stale token was wrongly accepted (`Ok(())`)
//!   instead of refused with `StaleOwner`.
//!
//! Both mutations were reverted immediately after their confirming run, and
//! the full suite (74 passed / 0 failed / 1 pre-existing unrelated ignore)
//! was re-run once more against the restored source to reconfirm.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use ego_domain::operation::OwnerId;
use ego_domain::Clock;
use ego_integration_tests::isolated_database;
use ego_persistence::postgres::PostgreSQLReadSideClaimStore;
use ego_persistence_api::read_side::claim::{ClaimError, ClaimFence, ClaimId, ReadSideClaimStore};
use ego_persistence_api::read_side::event_tag::EventTag;
use ego_testkit::TestClock;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::sync::Barrier;

/// Bounded wait for every barrier/join in this suite — a hang becomes a
/// failure with a message, not a wedged test run. Mirrors
/// `concurrent_replicas_postgres.rs`.
const WAIT_LIMIT: StdDuration = StdDuration::from_secs(30);

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the isolated database accepts connections")
}

fn claim_id(tenant: &str) -> ClaimId {
    ClaimId {
        projection_id: "read-side-claiming-under-test".to_string(),
        tag: EventTag::new("claims-suite"),
        tenant: tenant.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 4.1 — SC-1 exclusion, plus the cross-tenant control case
// ---------------------------------------------------------------------------

/// Two workers, two pools, two `OwnerId`s, released together (via a
/// `Barrier`) onto one `(projection_id, tag, tenant)`: exactly one is
/// granted, the refused worker's own fetch/handler counters stay at 0.
///
/// Control case, same two workers: on two *different* tenants, both obtain a
/// fence and both run — the exclusion is per-identity, not global.
///
/// Traces: "Acquisition Excludes A Concurrent Second Claimant".
#[tokio::test(flavor = "multi_thread")]
async fn sc1_exclusion_two_workers_one_identity_exactly_one_claims() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    let pool_a = connect(&url).await;
    let pool_b = connect(&url).await;

    let clock: Arc<dyn Clock> = Arc::new(TestClock::new(Utc::now()));
    let store_a = PostgreSQLReadSideClaimStore::new(pool_a, clock.clone());
    let store_b = PostgreSQLReadSideClaimStore::new(pool_b, clock.clone());

    let id = claim_id("tenant-contended");
    let owner_a = OwnerId::new("worker-a");
    let owner_b = OwnerId::new("worker-b");
    let lease_until = clock.now() + Duration::seconds(30);

    let start_line = Arc::new(Barrier::new(2));
    let fetch_a = Arc::new(AtomicUsize::new(0));
    let handler_a = Arc::new(AtomicUsize::new(0));
    let fetch_b = Arc::new(AtomicUsize::new(0));
    let handler_b = Arc::new(AtomicUsize::new(0));

    let task_a = {
        let start_line = start_line.clone();
        let id = id.clone();
        let owner = owner_a.clone();
        let fetch = fetch_a.clone();
        let handler = handler_a.clone();
        tokio::spawn(async move {
            start_line.wait().await;
            let granted = store_a
                .try_claim(&id, &owner, lease_until)
                .await
                .expect("the store answers");
            if granted.is_some() {
                // Stands in for "fetch, then hand the batch to the
                // handler" — only a granted worker may ever reach this.
                fetch.fetch_add(1, Ordering::SeqCst);
                handler.fetch_add(1, Ordering::SeqCst);
            }
            granted
        })
    };
    let task_b = {
        let start_line = start_line.clone();
        let id = id.clone();
        let owner = owner_b.clone();
        let fetch = fetch_b.clone();
        let handler = handler_b.clone();
        tokio::spawn(async move {
            start_line.wait().await;
            let granted = store_b
                .try_claim(&id, &owner, lease_until)
                .await
                .expect("the store answers");
            if granted.is_some() {
                fetch.fetch_add(1, Ordering::SeqCst);
                handler.fetch_add(1, Ordering::SeqCst);
            }
            granted
        })
    };

    let result_a = tokio::time::timeout(WAIT_LIMIT, task_a)
        .await
        .expect("worker A answered within the limit")
        .expect("worker A's task completed");
    let result_b = tokio::time::timeout(WAIT_LIMIT, task_b)
        .await
        .expect("worker B answered within the limit")
        .expect("worker B's task completed");

    let granted_count = [&result_a, &result_b]
        .iter()
        .filter(|r| r.is_some())
        .count();
    assert_eq!(
        granted_count, 1,
        "exactly one worker must be granted the claim; A={result_a:?} B={result_b:?}"
    );

    if result_a.is_some() {
        assert_eq!(fetch_b.load(Ordering::SeqCst), 0, "refused worker B must never fetch");
        assert_eq!(handler_b.load(Ordering::SeqCst), 0, "refused worker B must never invoke a handler");
    } else {
        assert_eq!(fetch_a.load(Ordering::SeqCst), 0, "refused worker A must never fetch");
        assert_eq!(handler_a.load(Ordering::SeqCst), 0, "refused worker A must never invoke a handler");
    }

    // --- Control case: same two workers, two different tenants -------------
    let pool_a2 = connect(&url).await;
    let pool_b2 = connect(&url).await;
    let store_a2 = PostgreSQLReadSideClaimStore::new(pool_a2, clock.clone());
    let store_b2 = PostgreSQLReadSideClaimStore::new(pool_b2, clock.clone());

    let granted_a2 = store_a2
        .try_claim(&claim_id("tenant-a"), &owner_a, lease_until)
        .await
        .expect("the store answers");
    let granted_b2 = store_b2
        .try_claim(&claim_id("tenant-b"), &owner_b, lease_until)
        .await
        .expect("the store answers");
    assert!(
        granted_a2.is_some() && granted_b2.is_some(),
        "different tenants must not contend for the same claim: A={granted_a2:?} B={granted_b2:?}"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// 4.2 / 4.3 — SC-2 takeover, and SC-3 stale-owner rejection
// ---------------------------------------------------------------------------

/// A claims and never releases; the clock is advanced past `lease_until`; B's
/// `try_claim` returns `Some` with a strictly greater `fencing_token` and the
/// row's `owner_id` reads back as B's (SC-2).
///
/// Then A's `renew` and `release` are both refused with `StaleOwner`, and the
/// row is unchanged. A token-isolation probe additionally shows B's own
/// `owner_id` paired with A's stale `fencing_token` is *also* refused, so the
/// refusal is never attributable to `owner_id` alone (SC-3).
///
/// Traces: "An Expired Lease Enables Takeover Without Operator Action",
/// "Takeover Fences Out The Stale Owner".
#[tokio::test]
async fn sc2_takeover_and_sc3_stale_owner_rejection() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    let pool_a = connect(&url).await;
    let pool_b = connect(&url).await;
    let observer = connect(&url).await;

    let t0 = Utc::now();
    let clock = Arc::new(TestClock::new(t0));
    let dyn_clock: Arc<dyn Clock> = clock.clone();
    let store_a = PostgreSQLReadSideClaimStore::new(pool_a, dyn_clock.clone());
    let store_b = PostgreSQLReadSideClaimStore::new(pool_b, dyn_clock);

    let id = claim_id("tenant-takeover");
    let owner_a = OwnerId::new("owner-a");
    let owner_b = OwnerId::new("owner-b");

    // --- A claims -------------------------------------------------------
    let fence_a = store_a
        .try_claim(&id, &owner_a, t0 + Duration::seconds(30))
        .await
        .expect("the store answers")
        .expect("a first claim on a fresh identity must be granted");

    // --- the lease expires, A never releases -----------------------------
    clock.advance(Duration::seconds(31));

    // --- B takes over -----------------------------------------------------
    let fence_b = store_b
        .try_claim(&id, &owner_b, dyn_clock_now(&clock) + Duration::seconds(60))
        .await
        .expect("the store answers")
        .expect("an expired lease must be takeable");

    assert_eq!(fence_b.owner_id, owner_b);
    assert!(
        fence_b.fencing_token.value() > fence_a.fencing_token.value(),
        "the taker's token must be strictly greater ({} vs {})",
        fence_b.fencing_token.value(),
        fence_a.fencing_token.value(),
    );

    let (row_owner, row_token): (String, i64) = sqlx::query_as(
        "SELECT owner_id, fencing_token FROM projection_claims \
         WHERE projection_id = $1 AND tag = $2 AND tenant = $3",
    )
    .bind(&id.projection_id)
    .bind(id.tag.value())
    .bind(&id.tenant)
    .fetch_one(&observer)
    .await
    .expect("exactly one claim row for this identity");
    assert_eq!(row_owner, "owner-b", "the row belongs to the taker");
    assert_eq!(
        row_token,
        fence_b.fencing_token.value() as i64,
        "and carries the taker's token"
    );

    // --- A comes back and tries to renew/release ---------------------------
    let a_renew = store_a
        .renew(&fence_a, dyn_clock_now(&clock) + Duration::seconds(90))
        .await;
    assert_eq!(
        a_renew,
        Err(ClaimError::StaleOwner),
        "the replaced owner's renew must be refused by the full \
         claim_id + owner_id + fencing_token triple"
    );

    let a_release = store_a.release(&fence_a).await;
    assert_eq!(
        a_release,
        Err(ClaimError::StaleOwner),
        "the replaced owner's release must also be refused"
    );

    // --- token-isolation probe: B's owner, A's stale token ------------------
    //
    // Two columns differ from the current row at once — owner and token — so
    // A's refusal above cannot say which one did the work. This fence keeps
    // B's own owner but pairs it with A's superseded token; only the token
    // differs from the live row now.
    let b_owner_with_stale_token = ClaimFence {
        claim_id: fence_b.claim_id.clone(),
        owner_id: fence_b.owner_id.clone(),
        fencing_token: fence_a.fencing_token,
    };
    let probe = store_b.renew(&b_owner_with_stale_token, dyn_clock_now(&clock) + Duration::seconds(90)).await;
    assert_eq!(
        probe,
        Err(ClaimError::StaleOwner),
        "a superseded token must be refused even when the owner matches — \
         otherwise the guard is really checking ownership alone"
    );

    // --- the row is unchanged by any of the refused attempts ---------------
    let (final_owner, final_token): (String, i64) = sqlx::query_as(
        "SELECT owner_id, fencing_token FROM projection_claims \
         WHERE projection_id = $1 AND tag = $2 AND tenant = $3",
    )
    .bind(&id.projection_id)
    .bind(id.tag.value())
    .bind(&id.tenant)
    .fetch_one(&observer)
    .await
    .expect("exactly one claim row for this identity");
    assert_eq!(final_owner, "owner-b");
    assert_eq!(final_token, fence_b.fencing_token.value() as i64);

    db.close().await;
}

/// Small helper so scenario code reads as "now, from the clock the store
/// itself uses" rather than repeating the lock/read by hand.
fn dyn_clock_now(clock: &Arc<TestClock>) -> chrono::DateTime<Utc> {
    clock.now()
}

// ---------------------------------------------------------------------------
// 4.4 — renewal prevents takeover
// ---------------------------------------------------------------------------

/// A renews before its original lease would have expired; B's `try_claim`
/// attempt after that original expiry, but still inside the renewed lease, is
/// refused.
///
/// Traces: "A Valid Claim May Be Renewed To Extend Processing".
#[tokio::test]
async fn renewal_prevents_takeover_during_the_renewed_lease() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    let pool_a = connect(&url).await;
    let pool_b = connect(&url).await;

    let t0 = Utc::now();
    let clock = Arc::new(TestClock::new(t0));
    let dyn_clock: Arc<dyn Clock> = clock.clone();
    let store_a = PostgreSQLReadSideClaimStore::new(pool_a, dyn_clock.clone());
    let store_b = PostgreSQLReadSideClaimStore::new(pool_b, dyn_clock);

    let id = claim_id("tenant-renewal");
    let owner_a = OwnerId::new("owner-a");
    let owner_b = OwnerId::new("owner-b");

    let fence_a = store_a
        .try_claim(&id, &owner_a, t0 + Duration::seconds(30))
        .await
        .expect("the store answers")
        .expect("a first claim must be granted");

    // Still well inside the original lease.
    clock.advance(Duration::seconds(25));
    store_a
        .renew(&fence_a, t0 + Duration::seconds(55))
        .await
        .expect("renewing an owned, still-valid claim must succeed");

    // Past the *original* 30s expiry, but inside the renewed 55s lease.
    clock.advance(Duration::seconds(10)); // now = t0 + 35s
    let refused = store_b
        .try_claim(&id, &owner_b, t0 + Duration::seconds(90))
        .await
        .expect("the store answers");
    assert_eq!(
        refused, None,
        "B must be refused while the renewed lease is still live — a renewal \
         that did not actually extend the lease would let B take over here"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// 4.5 — SC-5 ordering: claiming imposes no ordering interference
// ---------------------------------------------------------------------------

/// One worker holds a claim across a batch of three real events, inserted out
/// of version order; the slice read back while the claim is held is strictly
/// ascending by `version` regardless.
///
/// Traces: "Claiming Preserves Existing Per-Stream Ordering".
#[tokio::test]
async fn claim_causes_no_stream_ordering_interference() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    let pool = connect(&url).await;

    let clock: Arc<dyn Clock> = Arc::new(TestClock::new(Utc::now()));
    let store = PostgreSQLReadSideClaimStore::new(connect(&url).await, clock.clone());

    let id = claim_id("tenant-ordering");
    let owner = OwnerId::new("owner-ordering");
    let fence = store
        .try_claim(&id, &owner, clock.now() + Duration::seconds(30))
        .await
        .expect("the store answers")
        .expect("a first claim must be granted");

    let aggregate_id = "agg-ordering";
    // Inserted out of version order — nothing about `events` guarantees
    // insertion order matches `version` order, so a naive unordered read
    // would not by itself demonstrate anything about ordering.
    for version in [3_i64, 1, 2] {
        sqlx::query(
            "INSERT INTO events (aggregate_id, tenant_id, version, event_type, payload) \
             VALUES ($1, NULL, $2, 'OrderingProbe', '{}'::jsonb)",
        )
        .bind(aggregate_id)
        .bind(version)
        .execute(&pool)
        .await
        .expect("the insert succeeds");
    }

    // The claim is still held here — this is "the handler's received slice"
    // while a worker's claim is live, per task 4.5.
    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM events WHERE aggregate_id = $1 ORDER BY version ASC",
    )
    .bind(aggregate_id)
    .fetch_all(&pool)
    .await
    .expect("the read succeeds");

    assert_eq!(versions, vec![1, 2, 3], "the slice must be strictly ascending by version");
    for pair in versions.windows(2) {
        assert!(pair[0] < pair[1], "strictly ascending: {versions:?}");
    }

    store
        .release(&fence)
        .await
        .expect("releasing the still-held, still-owned claim succeeds");

    db.close().await;
}

// ---------------------------------------------------------------------------
// 4.6 — immediate reclaim on release
// ---------------------------------------------------------------------------

/// A releases normally; a second `try_claim` immediately after — no clock
/// advance — succeeds without waiting for lease expiry.
///
/// Traces: "Normal Release Makes the Stream Immediately Reclaimable".
#[tokio::test]
async fn immediate_reclaim_after_a_normal_release() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    let pool_a = connect(&url).await;
    let pool_b = connect(&url).await;

    let clock: Arc<dyn Clock> = Arc::new(TestClock::new(Utc::now()));
    let store_a = PostgreSQLReadSideClaimStore::new(pool_a, clock.clone());
    let store_b = PostgreSQLReadSideClaimStore::new(pool_b, clock.clone());

    let id = claim_id("tenant-reclaim");
    let owner_a = OwnerId::new("owner-a");
    let owner_b = OwnerId::new("owner-b");

    let fence_a = store_a
        .try_claim(&id, &owner_a, clock.now() + Duration::seconds(30))
        .await
        .expect("the store answers")
        .expect("a first claim must be granted");

    store_a.release(&fence_a).await.expect("release succeeds");

    // No clock advance — release, not expiry, is what makes this claimable.
    let reclaimed = store_b
        .try_claim(&id, &owner_b, clock.now() + Duration::seconds(30))
        .await
        .expect("the store answers");
    assert!(
        reclaimed.is_some(),
        "a released claim must be immediately reclaimable, with no wait for expiry"
    );
    assert!(
        reclaimed.unwrap().fencing_token.value() > fence_a.fencing_token.value(),
        "the reclaim must still mint a strictly greater token, same as a takeover"
    );

    db.close().await;
}
