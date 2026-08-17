//! **Guarantee (E1):** after a process dies between the two halves of one
//! dual-aggregate operation, a retry resumes rather than repeats — the confirmed
//! half is not re-executed and the missing half runs exactly once.
//!
//! **Layers traversed:** `build_runtime_with` → `EntityEventStores::open` →
//! `PostgreSQLEventStore` and `PostgresOperationReservationStore` → real SQL
//! against a real PostgreSQL, across **two operating-system processes**.
//!
//! # This slice is RED on purpose
//!
//! E1.1 fixes the expectation; E1.2 implements the behaviour that satisfies it.
//! The assertion that fails today, and why, is stated at the end of the parent
//! test rather than weakened to pass. See `WHAT IS STILL RED` there.
//!
//! # Why two processes, and why `abort`
//!
//! A recoverable error is not a crash. Returning `Err` unwinds: destructors run,
//! pools close, and a lease could be abandoned on the way out — leaving a *tidy*
//! partial state, which is not the state a killed process leaves and not the one
//! recovery has to cope with. `panic!` has the same problem. `exit` is closer but
//! still runs atexit handlers and reports a code somebody chose.
//!
//! So the interruption is `std::process::abort()` in a **child process**: SIGABRT,
//! no unwinding, no destructors, no cleanup, no `abandon`. It has to be a child
//! because a test cannot abort itself and still make assertions, and the parent
//! checks for SIGABRT **specifically** — a test that accepted any non-zero exit
//! would also accept the child merely failing, which proves nothing about crash
//! recovery.
//!
//! # No container of its own
//!
//! It takes a database from `ego_integration_tests::isolated_database`, like every
//! other module here. The child is told that database's URL; the runner still owns
//! the one container, and still reclaims it even though the child dies by signal.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.

use std::os::unix::process::ExitStatusExt;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use ego_domain::operation::OwnerId;
use ego_integration_tests::isolated_database;
use ego_testkit::{TestClock, TestJwtBuilder};
use ego_transport::AppState;
use reference_app::{
    build_runtime_with, AppConfig, BuiltRuntime, EntityEventStores, IdempotencyWiring,
    CRASH_FAILPOINT_VAR,
};
use reference_app::ports::http::build_router;
use reference_app::{DEV_SIGNING_KEY, REFERENCE_APP_AUDIENCE};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;


/// Set by the parent to put the child into child mode, and to tell it which
/// database to use. Absent in an ordinary run, which is what makes the child test
/// a no-op when the suite runs normally.
const CHILD_DB_URL: &str = "EGO_IT_CHILD_DB_URL";

/// A token this app accepts.
///
/// The same six lines the sibling modules carry. Duplicated rather than shared
/// because the reference app's own `tests/support` belongs to another crate's test
/// binaries and is not public surface; all copies build on the same public
/// constants, so they cannot drift on the signing key or audience. Now that these
/// files are modules of one target a shared helper is finally possible, which is a
/// tidy-up for its own slice rather than a change to smuggle in here.
fn make_token(sub: &str, tenant_id: &str) -> String {
    TestJwtBuilder::new(DEV_SIGNING_KEY.to_vec())
        .subject(sub)
        .tenant_id(tenant_id)
        .claim("aud", Value::from(REFERENCE_APP_AUDIENCE))
        .build()
}

/// One operation, retried. Same key and same payload every time.
const KEY: &str = "op-e11-crash-recovery";
const TENANT: &str = "tenant-a";
const USER: &str = "user-1";

/// Owner A crashes; owner B recovers. Distinct because a takeover is one owner
/// displacing another, and a retry under the same owner would be a renewal.
const OWNER_A: &str = "replica-a-crashes";
const OWNER_B: &str = "replica-b-recovers";

/// Short, so the recovery phase can position a clock past it without waiting.
const LEASE: Duration = Duration::from_secs(30);

/// Builds the reference app through the **productive** composition root.
///
/// Durable event stores, a durable reservation store, enforcement on, and the
/// owner this process reserves as. Nothing here assembles a runtime by hand: a
/// crash-recovery test that built its own wiring would be proving something about
/// the fixture.
async fn productive_app(
    url: &str,
    owner: &str,
    clock: Arc<dyn ego_domain::time::Clock>,
) -> BuiltRuntime {
    let pool = connect(url).await;
    let stores = EntityEventStores::open(pool.clone())
        .await
        .expect("the event stores open against the migrated database");
    let reservations = Arc::new(
        ego_persistence::postgres::reservation::PostgresOperationReservationStore::new(
            pool,
            clock.clone(),
        ),
    );

    build_runtime_with(
        &AppConfig::default(),
        stores,
        IdempotencyWiring::Enforced {
            store: reservations,
            owner_id: OwnerId::new(owner),
            lease_duration: LEASE,
            clock,
        },
        None,
    )
    .expect("the reference app builds")
}

async fn connect(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the database accepts connections")
}

/// The one request, identical on every attempt.
fn post() -> Request<Body> {
    let token = make_token(USER, TENANT);
    Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", KEY)
        .body(Body::from(
            serde_json::json!({
                "user_id": USER,
                "email": "user@example.com",
                "tenant_id": TENANT,
                "org_name": "Acme",
            })
            .to_string(),
        ))
        .unwrap()
}

/// Drives one attempt through the real HTTP surface.
async fn attempt(built: BuiltRuntime) -> StatusCode {
    let router = build_router(
        AppState::new(built.app.resolver(), built.authn.clone()),
        built.read_side.query.clone(),
    );
    router
        .oneshot(post())
        .await
        .expect("the router answers")
        .status()
}

// --- durable observations ---------------------------------------------------

async fn event_count(pool: &PgPool, aggregate_type: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE aggregate_type = $1")
        .bind(aggregate_type)
        .fetch_one(pool)
        .await
        .expect("the count comes back")
}

async fn receipt_count(pool: &PgPool, aggregate_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM operation_receipts \
         WHERE aggregate_type = $1 AND operation_key = $2",
    )
    .bind(aggregate_type)
    .bind(KEY)
    .fetch_one(pool)
    .await
    .expect("the count comes back")
}

/// The reservation's owner and fencing token, or `None` if no row exists.
///
/// The token is what distinguishes a takeover from a fresh acquisition: a fresh
/// reservation starts at 1, and displacing an existing one advances it. So
/// `(owner_b, token > 1)` is a takeover, and `(owner_b, 1)` would mean the row had
/// been removed and re-created — recovery by forgetting, not by resuming.
async fn reservation(pool: &PgPool) -> Option<(String, i64)> {
    sqlx::query_as("SELECT owner_id, fencing_token FROM operation_reservations WHERE operation_key = $1")
        .bind(KEY)
        .fetch_optional(pool)
        .await
        .expect("the reservation reads back")
}

// --- the child --------------------------------------------------------------

/// The crashing half, run only as a child process.
///
/// A no-op in an ordinary suite run: without the parent's environment there is no
/// database to use, and aborting here would take the whole suite down with it.
/// That guard is load-bearing, not defensive.
#[tokio::test]
async fn child_crashes_after_the_org_receipt_is_confirmed() {
    let Ok(url) = std::env::var(CHILD_DB_URL) else {
        // Not a child. Nothing to do.
        return;
    };

    // Real time here: this process is about to die, and a positioned clock would
    // only describe a lease nobody outlives.
    let built = productive_app(&url, OWNER_A, Arc::new(ego_domain::time::SystemClock)).await;

    // The failpoint is installed by the composition root because the parent set
    // `CRASH_FAILPOINT_VAR`. This call does not return.
    let _ = attempt(built).await;

    panic!(
        "the failpoint did not fire: this child was supposed to abort between the \
         organization's confirmed receipt and the user command, and instead ran to \
         completion. Either {CRASH_FAILPOINT_VAR} did not reach the composition \
         root, or the boundary moved."
    );
}

// --- the parent -------------------------------------------------------------

/// A crash between the two aggregates is recovered by takeover, not repeated.
#[tokio::test]
async fn a_crash_between_the_aggregates_is_recovered_by_takeover() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    // --- phase 1: a real crash, in a real other process ---------------------
    let child = std::process::Command::new(
        std::env::current_exe().expect("this test binary has a path"),
    )
    .args([
        "--exact",
        "infrastructure::dual_aggregate_crash_recovery_postgres::\
         child_crashes_after_the_org_receipt_is_confirmed",
        "--nocapture",
        "--test-threads=1",
    ])
    .env(CHILD_DB_URL, db.url())
    .env(CRASH_FAILPOINT_VAR, "1")
    .output()
    .expect("the child process starts");

    // SIGABRT specifically, not merely non-zero.
    //
    // A test that accepted any failure would also accept the child panicking, or
    // failing an assertion, or not finding its database — none of which is a
    // crash, and all of which unwind. Signal 6 is the only outcome that means
    // "this process stopped existing mid-operation".
    assert_eq!(
        child.status.signal(),
        Some(6),
        "the child must die by SIGABRT, so that nothing unwound and no lease was \
         abandoned on the way out. Got status {:?}\n--- child stdout ---\n{}\n\
         --- child stderr ---\n{}",
        child.status,
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr),
    );

    // --- the partial durable state the crash left ---------------------------
    //
    // Asserted before any recovery is attempted. If this is wrong, the failpoint
    // is in the wrong place and everything after it would be measuring the wrong
    // scenario.
    assert_eq!(
        (
            event_count(&pool, "tenant_organization").await,
            receipt_count(&pool, "tenant_organization").await,
        ),
        (1, 1),
        "the organization half is durable: exactly one event and one confirmed \
         receipt survived the crash"
    );
    assert_eq!(
        (
            event_count(&pool, "user").await,
            receipt_count(&pool, "user").await,
        ),
        (0, 0),
        "and the user half does not exist: the process died before its command was \
         sent, which is the boundary this whole test exists to cross"
    );

    let held = reservation(&pool)
        .await
        .expect("the crash left its reservation behind — nothing abandoned it");
    assert_eq!(
        held.0, OWNER_A,
        "the reservation still belongs to the owner that died: an abort runs no \
         destructors, so nothing released it cleanly"
    );

    // --- phase 2: a new process's worth of state, and a later clock ---------
    //
    // A new runtime, a new pool, a different owner, and a clock positioned past
    // the lease — the shape a replica retrying after its peer died actually has.
    // The clock is positioned rather than waited for: a sleep standing in for
    // expiry is a condition nobody stated.
    let recovery_clock = Arc::new(TestClock::new(
        Utc::now() + chrono::Duration::from_std(LEASE).expect("a small duration") + chrono::Duration::seconds(1),
    ));
    let recovered = productive_app(db.url(), OWNER_B, recovery_clock).await;
    let status = attempt(recovered).await;

    // --- takeover, not a fresh acquisition ---------------------------------
    let after = reservation(&pool)
        .await
        .expect("the recovery attempt left a reservation");
    assert_eq!(
        after.0, OWNER_B,
        "the recovering replica now holds the reservation"
    );
    assert!(
        after.1 > held.1,
        "the fencing token must have advanced past the dead owner's: that is what \
         makes this a takeover rather than a fresh acquisition of a key nobody \
         held. Was {} before, {} after",
        held.1,
        after.1
    );

    // --- WHAT E1 PROMISES ---------------------------------------------------
    //
    // Recovery is resumption, not repetition: the confirmed half is not re-run,
    // and the missing half runs exactly once.
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the retry completes the operation — 201, the same status a first successful \
         registration returns, because a recovered operation is not a different \
         outcome"
    );
    assert_eq!(
        event_count(&pool, "tenant_organization").await,
        1,
        "the organization was NOT re-executed: its receipt was already confirmed, \
         so a second event would mean the receipt was ignored"
    );
    assert_eq!(
        event_count(&pool, "user").await,
        1,
        "the user half ran, exactly once"
    );
    assert_eq!(
        (
            receipt_count(&pool, "tenant_organization").await,
            receipt_count(&pool, "user").await,
        ),
        (1, 1),
        "both halves now hold their own confirmed receipt under the one key"
    );

    // --- a third identical attempt changes nothing --------------------------
    let third_clock = Arc::new(TestClock::new(
        Utc::now() + chrono::Duration::from_std(LEASE).expect("a small duration") * 2,
    ));
    let again = productive_app(db.url(), OWNER_B, third_clock).await;
    let third = attempt(again).await;
    assert_eq!(
        third,
        StatusCode::CREATED,
        "a further retry still succeeds, with the same status"
    );
    assert_eq!(
        (
            event_count(&pool, "tenant_organization").await,
            event_count(&pool, "user").await,
        ),
        (1, 1),
        "and adds nothing: a completed operation replayed is observably identical"
    );

    pool.close().await;
    db.close().await;
}
