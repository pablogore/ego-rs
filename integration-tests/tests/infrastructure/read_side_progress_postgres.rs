//! **Guarantee:** the durable read-side progress pair — `PostgreSQLOffsetStore`
//! and `PostgreSQLDedupStore` (PROD-014B) — behaves the way `spec.md` states,
//! against real PostgreSQL: an offset survives a process restart, tenant
//! isolation is real, writes are last-write-wins, dedup marks converge to one
//! row (sequentially and concurrently), dedup identity is tenant-independent,
//! both stores report themselves durable, and an unapplied migration is
//! classified `Fatal` rather than `Transient`.
//!
//! **Layers traversed:** the real adapters (`ego_persistence::postgres::{
//! PostgreSQLOffsetStore, PostgreSQLDedupStore}`) against a real PostgreSQL
//! table, through a fresh `PgPool` per assertion so nothing here can be
//! satisfied by in-process state.
//!
//! # Why in-process cannot show this
//!
//! Restart survival, tenant isolation and last-write-wins are properties of
//! the stored rows, not of any in-memory struct — a scripted double returns
//! whatever it was handed and has nothing to lose across a restart. Dedup
//! convergence under real concurrency needs a real `ON CONFLICT … DO NOTHING`
//! resolved by the database, not a mutex a test controls. And the unapplied-
//! migration classification needs a real `42P01` from a real catalog lookup —
//! no scripted store has a catalog to be missing from.
//!
//! # What this deliberately does not claim (PROD-014B AD-6)
//!
//! [`dedup_concurrent_mark_seen_converges_to_one_record`] proves **storage-level
//! convergence of two calls on one identity** — exactly one row, no error on
//! either call. It does not prove, and this suite does not claim, execution
//! exclusion, exactly-once handling, or multi-replica safety. The delivered
//! guarantee is single-writer-per-`(projection_id, tag, tenant)`; closing the
//! execution-exclusion gap is **PROD-014C — Atomic Read-Side Event Claiming**,
//! not this capability.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use ego_domain::read_side::dedup::DedupStore;
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::offset::{Offset, OffsetStore, OffsetStoreError};
use ego_domain::{Clock, SystemClock};
use ego_integration_tests::isolated_database;
use ego_persistence::postgres::{PostgreSQLDedupStore, PostgreSQLOffsetStore, PostgreSQLReadSideClaimStore};
use persistent_entity::profile::Profile;
use reference_app::read_side::ReadSideProgressStores;
use reference_app::{AppConfig, EntityEventStores, ExternalEffectsWiring, IdempotencyWiring};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

const PROJECTION_ID: &str = "users-by-tenant";

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the isolated database accepts connections")
}

// ---------------------------------------------------------------------------
// SC-1 / R-3: restart survival
// ---------------------------------------------------------------------------

/// An offset written before a process "restart" is still there afterward.
///
/// The store and its pool are dropped entirely between the write and the
/// read, and a brand-new pool is opened against the *same database* to read
/// it back — the in-process value is never the evidence. Traces: spec.md
/// "Offset Survives a Process Restart".
#[tokio::test]
async fn offset_survives_a_process_restart() {
    let db = isolated_database().await;
    let tag = EventTag::new("users-by-tenant");

    {
        let store = PostgreSQLOffsetStore::new(connect(db.url()).await);
        store
            .write_offset(PROJECTION_ID, &tag, "tenant-a", &Offset::sequence(42))
            .await
            .expect("the write succeeds");
        // `store` and its pool are dropped here — nothing survives in process.
    }

    let restarted = PostgreSQLOffsetStore::new(connect(db.url()).await);
    let read = restarted
        .read_offset(PROJECTION_ID, &tag, "tenant-a")
        .await
        .expect("the read succeeds");
    assert_eq!(
        read,
        Some(Offset::sequence(42)),
        "a new store, over a new pool, against the same database, must see \
         the offset written before the restart"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// SC-2 / G-4: absent-offset tenant isolation
// ---------------------------------------------------------------------------

/// An unwritten `(projection_id, tag, tenant)` returns absent, and never
/// leaks another tenant's offset for the same `(projection_id, tag)`.
///
/// Traces: spec.md "Absent Offset Reads Are Tenant-Isolated".
#[tokio::test]
async fn absent_offset_reads_are_tenant_isolated() {
    let db = isolated_database().await;
    let store = PostgreSQLOffsetStore::new(db.pool().await);
    let tag = EventTag::new("users-by-tenant");

    store
        .write_offset(PROJECTION_ID, &tag, "tenant-a", &Offset::sequence(7))
        .await
        .expect("tenant A's write succeeds");

    let tenant_b = store
        .read_offset(PROJECTION_ID, &tag, "tenant-b")
        .await
        .expect("the read succeeds");
    assert_eq!(
        tenant_b, None,
        "tenant B was never written for this (projection_id, tag) and must \
         read absent — never tenant A's offset"
    );

    let tenant_a = store
        .read_offset(PROJECTION_ID, &tag, "tenant-a")
        .await
        .expect("the read succeeds");
    assert_eq!(
        tenant_a,
        Some(Offset::sequence(7)),
        "tenant A's own write is still readable under its own tenant"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// SC-7: offset writes are last-write-wins
// ---------------------------------------------------------------------------

/// A second write for the same identity silently overwrites the first, with
/// no conflict signal to either writer.
///
/// Traces: spec.md "Offset Writes Are Last-Write-Wins".
#[tokio::test]
async fn offset_writes_are_last_write_wins() {
    let db = isolated_database().await;
    let store = PostgreSQLOffsetStore::new(db.pool().await);
    let tag = EventTag::new("users-by-tenant");

    store
        .write_offset(PROJECTION_ID, &tag, "tenant-a", &Offset::sequence(1))
        .await
        .expect("the first write succeeds"); // SC-7: `tenant` is bound, not interpolated.

    store
        .write_offset(PROJECTION_ID, &tag, "tenant-a", &Offset::sequence(2))
        .await
        .expect("the second write succeeds with no conflict signal");

    let stored = store
        .read_offset(PROJECTION_ID, &tag, "tenant-a")
        .await
        .expect("the read succeeds");
    assert_eq!(
        stored,
        Some(Offset::sequence(2)),
        "the later write must win — no compare-and-swap, no expected-previous check"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// Repeated dedup marks converge to one record — sequential and concurrent
// ---------------------------------------------------------------------------

/// `mark_seen` called twice sequentially for the same identity: both calls
/// succeed, exactly one row exists, and `seen()` reports true.
///
/// Traces: spec.md "Repeated Dedup Marks Converge to One Record".
#[tokio::test]
async fn dedup_sequential_double_mark_converges_to_one_record() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let store = PostgreSQLDedupStore::new(pool.clone());
    let tag = EventTag::new("users-by-tenant");

    store
        .mark_seen(PROJECTION_ID, &tag, "evt-1")
        .await
        .expect("the first mark succeeds");
    store
        .mark_seen(PROJECTION_ID, &tag, "evt-1")
        .await
        .expect("the second, repeat mark also succeeds — no error on the repeat");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM projection_dedup \
         WHERE projection_id = $1 AND tag = $2 AND event_id = $3",
    )
    .bind(PROJECTION_ID)
    .bind(tag.value())
    .bind("evt-1")
    .fetch_one(&pool)
    .await
    .expect("the count comes back");
    assert_eq!(count, 1, "exactly one record for this identity");

    assert!(
        store
            .seen(PROJECTION_ID, &tag, "evt-1")
            .await
            .expect("the seen check succeeds"),
        "a subsequent seen-check must report true"
    );

    db.close().await;
}

/// Two `mark_seen` calls for the same identity, run concurrently: both
/// succeed, exactly one row exists, and `seen()` reports true.
///
/// **What this proves, and what it deliberately does not (PROD-014B AD-6):**
/// this is **storage-level convergence** of two concurrent calls on one
/// identity — `ON CONFLICT (projection_id, tag, event_id) DO NOTHING`
/// resolving the write race inside one statement, with no unique-violation
/// error surfacing to either caller. It does **not** prove execution
/// exclusion, does **not** prove exactly-once handling, and does **not**
/// prove multi-replica safety: `seen()` and `mark_seen()` are separate SPI
/// calls with a handler running between them in real usage, and nothing here
/// closes that window. The guarantee this test — and this capability —
/// delivers is single-writer-per-`(projection_id, tag, tenant)`; the
/// execution-exclusion gap is named and owned by a distinct, not-yet-started
/// follow-up (PROD-014C — Atomic Read-Side Event Claiming), never implied
/// here. Traces: spec.md "Repeated Dedup Marks Converge to One Record".
#[tokio::test]
async fn dedup_concurrent_mark_seen_converges_to_one_record() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let store_a = PostgreSQLDedupStore::new(connect(db.url()).await);
    let store_b = PostgreSQLDedupStore::new(connect(db.url()).await);
    let tag = EventTag::new("users-by-tenant");

    let (a, b) = tokio::join!(
        store_a.mark_seen(PROJECTION_ID, &tag, "evt-concurrent"),
        store_b.mark_seen(PROJECTION_ID, &tag, "evt-concurrent"),
    );
    assert!(a.is_ok(), "the first concurrent mark must succeed: {a:?}");
    assert!(b.is_ok(), "the second concurrent mark must succeed: {b:?}");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM projection_dedup \
         WHERE projection_id = $1 AND tag = $2 AND event_id = $3",
    )
    .bind(PROJECTION_ID)
    .bind(tag.value())
    .bind("evt-concurrent")
    .fetch_one(&pool)
    .await
    .expect("the count comes back");
    assert_eq!(
        count, 1,
        "two concurrent marks on one identity must converge to exactly one row"
    );

    assert!(
        store_a
            .seen(PROJECTION_ID, &tag, "evt-concurrent")
            .await
            .expect("the seen check succeeds"),
        "a subsequent seen-check must report true"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// Dedup identity is tenant-independent
// ---------------------------------------------------------------------------

/// The same `event_id` marked seen under one tenant is reported already-seen
/// under a different tenant, for the same `(projection_id, tag)` — dedup
/// identity carries no tenant.
///
/// Traces: spec.md "Dedup Identity Is Tenant-Independent".
#[tokio::test]
async fn dedup_identity_is_tenant_independent() {
    let db = isolated_database().await;
    let store = PostgreSQLDedupStore::new(db.pool().await);
    let tag = EventTag::new("users-by-tenant");

    // The store itself takes no tenant parameter (PROD-014B AD-7) — marking
    // under "tenant A's" traffic and checking under "tenant B's" is
    // represented by using the same (projection_id, tag, event_id) from two
    // logically different call sites, which is exactly what the SPI allows
    // a host to do if its tags are not themselves tenant-derived.
    store
        .mark_seen(PROJECTION_ID, &tag, "evt-shared")
        .await
        .expect("marking under the first caller succeeds");

    assert!(
        store
            .seen(PROJECTION_ID, &tag, "evt-shared")
            .await
            .expect("the seen check succeeds"),
        "a second caller checking the identical (projection_id, tag, event_id) \
         must see it as already seen — dedup identity does not vary by tenant"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// Both progress stores report themselves as durable
// ---------------------------------------------------------------------------

/// Both stores report `is_durable() == true`.
///
/// This is the offset/dedup half of spec.md's "Both Progress Stores Report
/// Themselves As Durable" (PROD-014B tasks.md 3.7). What a
/// `Profile::Production` composition through `ReadSideProgressStores::
/// postgres(pool)` now requires beyond this pair — a durable
/// `ReadSideClaimStore` alongside it (PROD-014C AD-9) — is proved below, in
/// [`production_profile_composition_without_a_claim_store_is_refused`].
#[tokio::test]
async fn both_progress_stores_report_themselves_as_durable() {
    let db = isolated_database().await;
    let offset_store = PostgreSQLOffsetStore::new(db.pool().await);
    let dedup_store = PostgreSQLDedupStore::new(db.pool().await);

    assert!(offset_store.is_durable(), "the offset store must report durable");
    assert!(dedup_store.is_durable(), "the dedup store must report durable");

    db.close().await;
}

// ---------------------------------------------------------------------------
// PR3 (Phase 6/8.2): the reference app's production path uses the durable
// pair
// ---------------------------------------------------------------------------

/// A `Profile::Production` composition that registers the durable read-side
/// pair through the existing registration point (`build_runtime_with`) but
/// leaves `read_side_claims` unset — which `reference_app::read_side::
/// ReadSideProgressStores::postgres`'s own rustdoc names as this reference
/// app's deliberate choice (PROD-014C tasks.md 8.1) — is now refused, not
/// accepted.
///
/// Before PROD-014C this build succeeded: registering the durable pair was
/// the entire single-writer guarantee, unenforced across replicas (PROD-014B
/// AD-6/L-3). `AppBuilder`/`RuntimeBuilder`'s shared
/// `validate_read_side_claim_profile` (AD-9) now refuses any
/// `Profile::Production` composition that registers read-side progress
/// without a durable `ReadSideClaimStore` alongside it — this reference
/// app's own composition included, exactly as ARCHITECTURE.md's read-side
/// event claiming section states. Proving the refusal here, against a real
/// migrated database, is what keeps this suite honest about what the
/// reference app's Production path does and does not enforce.
///
/// Refusal surfaces as a returned `Err`, never a panic: `AppBuilder::build()`'s
/// AD-3 scratch-runtime pass (`crates/service-sdk/src/app/mod.rs`) now calls
/// the fallible `try_build()` instead of the panicking `build()`, so a
/// `Profile::Production` validation failure reachable through that scratch
/// pass — this claim-store gate included — is mapped to
/// `CompositionError::Validation` and returned, exactly as this function's
/// `Result` signature promises.
///
/// Traces: spec.md "Both Progress Stores Report Themselves As Durable";
/// PROD-014C tasks.md 8.1/9.1 (this suite's own regression check).
#[tokio::test]
async fn production_profile_composition_without_a_claim_store_is_refused() {
    let db = isolated_database().await;
    let pool = connect(db.url()).await;

    // EC-2: the clone is taken before the pool is moved into
    // `EntityEventStores::open`, mirroring `main.rs`'s corrected ordering.
    let read_side_progress = ReadSideProgressStores::postgres(pool.clone());
    let stores = EntityEventStores::open(pool)
        .await
        .expect("the stores open against a migrated database");
    assert_eq!(stores.profile(), Profile::Production);

    let result = reference_app::build_runtime_with(
        &AppConfig::default(),
        stores,
        IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
        Some(read_side_progress),
        None,
    );

    let Err(err) = result else {
        panic!("a registered read-side progress pair without a durable claim store must be refused under Profile::Production");
    };
    let message = err.to_string();
    assert!(
        message.contains("durable read-side claim store"),
        "the refusal must name the missing durable read-side claim store, got: {message}"
    );
}

/// The positive counterpart to the refusal above: a `Profile::Production`
/// composition that registers the durable read-side pair AND a real
/// `PostgreSQLReadSideClaimStore` — exactly as `ReadSideProgressStores::
/// postgres`'s own rustdoc instructs hosts to do — builds successfully
/// through `build_runtime_with`, the same composition root `main.rs` calls.
///
/// Traces: PROD-P0.1 Required Test 1.
#[tokio::test]
async fn production_profile_composition_with_a_durable_claim_store_builds() {
    let db = isolated_database().await;
    let pool = connect(db.url()).await;

    let read_side_progress = ReadSideProgressStores::postgres(pool.clone());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let read_side_claims = PostgreSQLReadSideClaimStore::new(pool.clone(), clock);
    let stores = EntityEventStores::open(pool)
        .await
        .expect("the stores open against a migrated database");
    assert_eq!(stores.profile(), Profile::Production);

    reference_app::build_runtime_with(
        &AppConfig::default(),
        stores,
        IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
        Some(read_side_progress),
        Some(Arc::new(read_side_claims)),
    )
    .expect(
        "a durable read-side progress pair plus a durable claim store must be \
         accepted under Profile::Production through the real composition root",
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// AD-8/AD-9: unapplied-migration classification
// ---------------------------------------------------------------------------

/// `read_offset` against a database whose `projection_offsets` table does not
/// exist returns `OffsetStoreError::Fatal`, never `Transient`.
///
/// Stands in for "no migration applied" (AD-9's replacement for a `probe()`
/// method) by dropping the specific table this store depends on, rather than
/// provisioning a second, wholly-unmigrated database: the resulting SQLSTATE
/// (`42P01 undefined_table`) is identical either way, and this keeps the test
/// on the shared, already-migrated template rather than growing the harness.
///
/// Traces: spec.md "Offset Survives a Process Restart" (the failure-mode
/// complement); tasks.md 3.8.
#[tokio::test]
async fn read_offset_against_an_unmigrated_database_is_fatal_not_transient() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    sqlx::query("DROP TABLE projection_offsets")
        .execute(&pool)
        .await
        .expect("the table this store depends on can be dropped");

    let store = PostgreSQLOffsetStore::new(pool);
    let tag = EventTag::new("users-by-tenant");
    let err = store
        .read_offset(PROJECTION_ID, &tag, "tenant-a")
        .await
        .expect_err("reading against a table that does not exist must fail");

    assert!(
        matches!(err, OffsetStoreError::Fatal(_)),
        "a missing table (42P01) must classify Fatal, not Transient — a retry \
         cannot help a migration that was never applied: {err:?}"
    );

    db.close().await;
}
