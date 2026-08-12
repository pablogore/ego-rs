//! **Guarantee:** when a lease expires and a second owner takes it over, the
//! replaced owner can no longer change the reservation — and the durable record
//! ends up holding the taker's answer and only the taker's answer.
//!
//! **Layers traversed:** `OperationReservationStore` (the port) →
//! `PostgresOperationReservationStore` (the durable adapter) → real SQL against
//! a real PostgreSQL, including the migrations that create the table and the
//! conditional update that enforces fencing.
//!
//! **Why in-process cannot show this.** Every part of the property is a
//! database outcome:
//!
//! - *Expiry* is a comparison between a stored `lease_until` and the clock at
//!   the moment another owner asks — the `lease_until <= $N` guard. A test
//!   double decides that in Rust, where it cannot be wrong the way SQL can.
//! - *Takeover* is one statement that must both re-own the row and mint a
//!   strictly greater token, atomically. An in-memory store gets that for free
//!   by holding a lock; a real one has to get the SQL right.
//! - *Rejecting the replaced owner* is a `WHERE` clause matching the full
//!   `operation_id + owner_id + fencing_token` triple. If it matched on fewer
//!   columns, an in-process double would still behave correctly while the
//!   durable one silently let a dead owner overwrite live state.
//! - *What survives* can only be read back from the row after the fact.
//!
//! Issue #275 names this the highest-value invariant in the backlog, guarded by
//! nothing today. It is one scenario, asserted end to end: the takeover, the
//! refusal, the completion and the durable state are steps of a single
//! guarantee, not four separable cases.
//!
//! Run: `cargo test --manifest-path integration-tests/Cargo.toml`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use chrono::{Duration, Utc};
use ego_domain::operation::{
    OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
use ego_domain::time::Clock;
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

/// A clock the test moves by hand.
///
/// Not a convenience: lease expiry has to be reached *without waiting for it*.
/// Sleeping for a real lease would put a fixed timeout where a condition
/// belongs, and would make the suite's wall-clock budget depend on how long a
/// lease happens to be configured for.
///
/// The store still evaluates expiry in SQL against the value this hands it —
/// the clock decides *when* "now" is, never *what the database concludes*.
struct SettableClock(std::sync::Mutex<chrono::DateTime<Utc>>);

impl SettableClock {
    fn at(t: chrono::DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self(std::sync::Mutex::new(t)))
    }
    fn advance_to(&self, t: chrono::DateTime<Utc>) {
        *self.0.lock().expect("not poisoned") = t;
    }
}

impl Clock for SettableClock {
    fn now(&self) -> chrono::DateTime<Utc> {
        *self.0.lock().expect("not poisoned")
    }
}

fn key() -> OperationKey {
    OperationKey::parse("op-takeover-under-test").expect("a non-empty key parses")
}

fn fingerprint() -> OperationFingerprint {
    OperationFingerprint::new("f".repeat(64))
}

/// The identical request, from whichever owner is asking. Same key and same
/// fingerprint throughout: a takeover is the *same* operation changing hands,
/// and a differing fingerprint would make this a conflict test instead.
fn request(owner: &str, lease_until: chrono::DateTime<Utc>) -> ReserveRequest {
    ReserveRequest {
        tenant: None,
        operation_key: key(),
        fingerprint: fingerprint(),
        owner_id: OwnerId::new(owner),
        lease_until,
    }
}

fn fence_of(lease: &ego_domain::operation::Lease) -> OwnerFence {
    OwnerFence {
        operation_id: lease.operation_id.clone(),
        owner_id: lease.owner_id.clone(),
        fencing_token: lease.fencing_token,
    }
}

async fn migrated_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections");
    migrations::run(&pool)
        .await
        .expect("the real migrations apply — including the reservations table");
    pool
}

#[tokio::test]
async fn a_taken_over_lease_locks_out_its_previous_owner_and_keeps_only_the_takers_answer() {
    let container = Postgres::default()
        .start()
        .await
        .expect("a PostgreSQL container starts");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.expect("a host"),
        container
            .get_host_port_ipv4(5432)
            .await
            .expect("the mapped port"),
    );
    let pool = migrated_pool(&url).await;

    let t0 = Utc::now();
    let clock = SettableClock::at(t0);
    let store = PostgresOperationReservationStore::new(pool.clone(), clock.clone());

    // --- A reserves ---------------------------------------------------------
    let a_lease = match store
        .reserve(request("owner-a", t0 + Duration::seconds(30)))
        .await
        .expect("the store answers")
    {
        ReservationOutcome::Fresh(lease) => lease,
        other => panic!("a first reservation must be Fresh, got {other:?}"),
    };
    let a_fence = fence_of(&a_lease);

    // --- the lease expires --------------------------------------------------
    //
    // Moved past `lease_until`, not slept through. What matters is that the
    // *database* now evaluates the row as expired.
    clock.advance_to(t0 + Duration::seconds(31));

    // --- B takes over -------------------------------------------------------
    let b_lease = match store
        .reserve(request("owner-b", t0 + Duration::seconds(61)))
        .await
        .expect("the store answers")
    {
        ReservationOutcome::TakenOver(lease) => lease,
        other => panic!(
            "an expired lease must be takeable — an operation whose owner died \
             would otherwise be unrecoverable. Got {other:?}"
        ),
    };
    let b_fence = fence_of(&b_lease);

    assert_eq!(b_lease.owner_id, OwnerId::new("owner-b"));
    assert!(
        b_lease.fencing_token.value() > a_lease.fencing_token.value(),
        "the taker's token must be strictly greater ({} vs {}) — it is what makes \
         the replaced owner's writes identifiable as stale",
        b_lease.fencing_token.value(),
        a_lease.fencing_token.value(),
    );

    // --- A comes back and tries to complete ---------------------------------
    //
    // The case the whole mechanism exists for: a process that stalled long
    // enough to lose its lease, then woke up and finished its work.
    let a_late = store
        .complete(
            &a_fence,
            StoredServiceResponse::new(b"answer-from-A".to_vec()),
        )
        .await;

    assert_eq!(
        a_late,
        Err(ReservationError::StaleOwner),
        "the replaced owner must be refused, by the full \
         operation_id + owner_id + fencing_token triple"
    );

    // --- and the token alone is load-bearing --------------------------------
    //
    // A's fence differs from the row in *two* columns — owner and token — so
    // its refusal above cannot say which one did the work. Measured: with
    // `fencing_token` removed from the guard, `owner_id` alone still rejected A
    // and the scenario passed unchanged. The probe below is what closed that.
    //
    // `owner_id`'s own independent role is deliberately NOT isolated here. Doing
    // so would need a third fence — A's owner with B's token — and at that point
    // the test is enumerating the columns of a `WHERE` clause rather than
    // demonstrating a guarantee. That belongs in the fast suite; see README.
    //
    // So the same reservation is probed once more with the **taker's owner and
    // the replaced token**. Only the token differs now, which is the column a
    // fencing guard exists for.
    let b_owner_with_stale_token = OwnerFence {
        operation_id: b_lease.operation_id.clone(),
        owner_id: b_lease.owner_id.clone(),
        fencing_token: a_lease.fencing_token,
    };
    assert_eq!(
        store
            .complete(
                &b_owner_with_stale_token,
                StoredServiceResponse::new(b"answer-under-a-stale-token".to_vec()),
            )
            .await,
        Err(ReservationError::StaleOwner),
        "a superseded token must be refused even when the owner matches — \
         otherwise the guard is really checking ownership, and a retrying owner \
         could overwrite the result of the attempt that replaced it"
    );

    // --- B completes --------------------------------------------------------
    store
        .complete(
            &b_fence,
            StoredServiceResponse::new(b"answer-from-B".to_vec()),
        )
        .await
        .expect("the current owner completes");

    // --- what the database actually holds -----------------------------------
    //
    // Read back through SQL rather than through the port: the port is what is
    // under test, so asking it to confirm its own work would let a store that
    // refused A and then wrote nothing pass. The row is the evidence.
    let (owner, token, response): (String, i64, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT owner_id, fencing_token, response \
         FROM operation_reservations WHERE operation_key = $1",
    )
    .bind(key().as_str())
    .fetch_one(&pool)
    .await
    .expect("exactly one reservation row for this key");

    assert_eq!(owner, "owner-b", "the row belongs to the taker");
    assert_eq!(
        token,
        b_lease.fencing_token.value() as i64,
        "and carries the taker's token, unchanged by A's refused attempt"
    );
    assert_eq!(
        response.as_deref(),
        Some(b"answer-from-B".as_ref()),
        "only the taker's answer is durable. A's refused completion must have \
         written nothing — a refusal that still overwrote the response would \
         serve a dead owner's result to every later replay"
    );
}
