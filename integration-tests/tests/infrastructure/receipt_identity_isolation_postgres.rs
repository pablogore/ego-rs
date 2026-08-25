//! **Guarantee:** the durable receipt identity — `(tenant_id, aggregate_type,
//! aggregate_id, operation_key)` — genuinely isolates on **each** of its four
//! fields independently. Two receipts that agree on three of the four and
//! differ on the remaining one never collide, and each keeps its own outcome.
//! Held against that: within one fixed scope, a genuine retry (same
//! fingerprint) replays instead of duplicating, and a different request
//! reusing the same identity (a different fingerprint) is refused rather than
//! silently overwriting what is there.
//!
//! **Layers traversed:** `PostgreSQLEventStore::confirm_receipt` /
//! `find_receipt` → real SQL → the two partial unique indexes migration 011
//! declares on `operation_receipts`.
//!
//! # Why this is a gap `schema_index_assertion.rs` does not close
//!
//! That file reads `pg_index`/`pg_attribute` and pins the *shape* the catalog
//! reports: which columns, in which order, unique, over which predicate. It
//! never files two receipts and checks what happens — its own module docs say
//! so. `conflict_from_postgres.rs` does file a real conflict, but only ever
//! under one tenant and one aggregate: `(tenant_id, operation_key)` is what it
//! loads, at the *reservation* table, which has no `aggregate_type` or
//! `aggregate_id` column at all. Neither file provokes a collision by varying
//! `aggregate_type` or `aggregate_id` while holding the rest of the identity
//! fixed, which is the one thing a shape assertion cannot show: an index can
//! report the right four columns and a bug elsewhere — a query that dropped a
//! predicate, an `ON CONFLICT` target narrower than the identity — could still
//! let two receipts for different aggregates suppress each other.
//!
//! # What is deliberately not re-proven here
//!
//! `each_aggregate_keeps_its_own_receipt_under_one_operation_key` in
//! `durable_entity_progress_postgres.rs` already shows two aggregates
//! (`tenant_organization` and `user`) holding separate receipts under one
//! operation key — but as **one logical operation** deliberately addressing
//! two aggregates, which is what a real dual-aggregate command does and what
//! that file's own docs describe as E1's precondition. The three scenarios
//! here are the opposite shape: **unrelated** operations that happen to share
//! three of the four identity fields by coincidence, never all four on
//! purpose. Held constant per scenario: the fingerprint, so any observed
//! difference is attributable to the identity field under test and not to a
//! side effect of also varying the request's content.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_domain::context::TenantId;
use ego_domain::operation::{
    AggregateOutcome, OperationFingerprint, OperationKey, OperationReceipt,
};
use ego_domain::persistence::{EventStore, PersistenceError};
use ego_integration_tests::isolated_database;
use ego_persistence::postgres::PostgreSQLEventStore;
use persistent_entity::testing::TestEvent;
use sqlx::PgPool;

/// The event type parameter `PostgreSQLEventStore` needs. Never actually
/// stored or loaded here — every scenario below only exercises the receipt
/// half of the port, which is keyed by an `aggregate_type` string parameter
/// rather than by this type. Reused from the shared testing helpers rather
/// than declaring a throwaway type, per the ladder: it already exists.
type Store = PostgreSQLEventStore<
    TestEvent,
    fn(
        &str,
        serde_json::Value,
        chrono::DateTime<chrono::Utc>,
    ) -> Result<TestEvent, PersistenceError>,
>;

/// Opens a store against `pool`. The deserializer is never called: nothing
/// here appends or loads an event.
async fn store(pool: PgPool) -> Store {
    let deserialize: fn(
        &str,
        serde_json::Value,
        chrono::DateTime<chrono::Utc>,
    ) -> Result<TestEvent, PersistenceError> = |_, _, _| {
        unreachable!("this suite only confirms and finds receipts; it never loads an event")
    };
    PostgreSQLEventStore::open(pool, deserialize)
        .await
        .expect("the store opens against a freshly migrated, empty database")
}

/// Stages, confirms and commits one receipt in its own unit of work.
///
/// On `Err` the unit of work is dropped rather than committed — `sqlx`'s
/// transaction rolls back on drop, and nothing was staged for a conflict
/// anyway: `confirm_receipt`'s `INSERT ... ON CONFLICT DO NOTHING` affected no
/// rows on that path, so there is nothing to roll back but the read.
async fn confirm(store: &Store, receipt: &OperationReceipt) -> Result<(), PersistenceError> {
    let mut uow = store.begin().await.expect("a unit of work begins");
    let result = uow.confirm_receipt(receipt).await;
    if result.is_ok() {
        uow.commit().await.expect("the unit of work commits");
    }
    result
}

fn tenant(id: &str) -> TenantId {
    TenantId::new(id).expect("a valid tenant id")
}

fn key(raw: &str) -> OperationKey {
    OperationKey::parse(raw).expect("a valid operation key")
}

fn fp(raw: &str) -> OperationFingerprint {
    OperationFingerprint::new(raw)
}

fn receipt(
    aggregate_type: &str,
    aggregate_id: &str,
    tenant_id: Option<TenantId>,
    operation_key: &str,
    fingerprint: &str,
    outcome: AggregateOutcome,
) -> OperationReceipt {
    OperationReceipt::new(
        aggregate_type,
        aggregate_id,
        tenant_id,
        key(operation_key),
        fp(fingerprint),
        outcome,
    )
}

/// Every row currently filed under one operation key, across every scope —
/// the count the two partial unique indexes bound. A collision shows up here
/// as `1` where two independent receipts should have produced `2`; a leak
/// across scopes shows up as more rows than either scenario legitimately
/// filed.
async fn rows_for_key(pool: &PgPool, operation_key: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM operation_receipts WHERE operation_key = $1")
        .bind(operation_key)
        .fetch_one(pool)
        .await
        .expect("the count comes back")
}

/// The fingerprint stored for one exact identity, read independently of
/// `find_receipt` — the port's own read path — so the row-level assertions
/// below do not depend on the same code that wrote them to also grade them.
async fn stored_fingerprint(
    pool: &PgPool,
    tenant_id: Option<&str>,
    aggregate_type: &str,
    aggregate_id: &str,
    operation_key: &str,
) -> String {
    sqlx::query_scalar(
        "SELECT fingerprint FROM operation_receipts \
         WHERE tenant_id IS NOT DISTINCT FROM $1 AND aggregate_type = $2 \
           AND aggregate_id = $3 AND operation_key = $4",
    )
    .bind(tenant_id)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(operation_key)
    .fetch_one(pool)
    .await
    .expect("exactly one row exists for this identity")
}

// ---------------------------------------------------------------------------
// Scenario 17 — tenant isolation
// ---------------------------------------------------------------------------

/// Two different tenants, same `aggregate_type` + `aggregate_id` +
/// `operation_key`: independent receipts, each keeping its own outcome. Then,
/// within one tenant's own scope: a genuine retry replays, and a fingerprint
/// mismatch conflicts without disturbing what is stored.
#[tokio::test]
async fn tenant_isolation_does_not_collide_and_the_same_scope_replays_or_conflicts() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let store = store(pool.clone()).await;

    const TYPE: &str = "order";
    const ID: &str = "shared-order-id";
    const OPERATION: &str = "op-tenant-isolation";
    const FINGERPRINT: &str = "fp-tenant-isolation";

    // Two unrelated operations that happen to agree on type, id and key, and
    // differ only in tenant. Different outcomes on purpose: a cross-scope
    // collision would surface here as the wrong tenant's outcome coming back,
    // not merely as a missing row.
    let a = receipt(
        TYPE,
        ID,
        Some(tenant("tenant-a")),
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::NoEvents,
    );
    let b = receipt(
        TYPE,
        ID,
        Some(tenant("tenant-b")),
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::events(4, 6).expect("a valid range"),
    );

    confirm(&store, &a)
        .await
        .expect("tenant A's receipt confirms");
    confirm(&store, &b).await.expect(
        "tenant B's receipt confirms independently — same type, id and key, \
                 different tenant, so this must not be read as a duplicate of A's",
    );

    assert_eq!(
        rows_for_key(&pool, OPERATION).await,
        2,
        "two distinct tenants must produce two distinct rows under one operation key"
    );

    let found_a = store
        .find_receipt(TYPE, ID, Some("tenant-a"), OPERATION)
        .await
        .expect("the lookup succeeds")
        .expect("tenant A's receipt is found");
    assert_eq!(*found_a.outcome(), AggregateOutcome::NoEvents);

    let found_b = store
        .find_receipt(TYPE, ID, Some("tenant-b"), OPERATION)
        .await
        .expect("the lookup succeeds")
        .expect("tenant B's receipt is found");
    assert_eq!(
        *found_b.outcome(),
        AggregateOutcome::events(4, 6).expect("a valid range"),
        "tenant B's own outcome, not tenant A's — a collision would return NoEvents here"
    );

    // --- negative control: same scope as A, same fingerprint --------------
    confirm(&store, &a)
        .await
        .expect("an identical retry within tenant A's own scope must replay, not fail");
    assert_eq!(
        rows_for_key(&pool, OPERATION).await,
        2,
        "a genuine replay must not add a row"
    );

    // --- negative control: same scope as A, different fingerprint ----------
    let conflicting = receipt(
        TYPE,
        ID,
        Some(tenant("tenant-a")),
        OPERATION,
        "fp-different-request",
        AggregateOutcome::NoEvents,
    );
    let err = confirm(&store, &conflicting)
        .await
        .expect_err("a different request reusing tenant A's identity must be refused");
    assert!(
        matches!(err, PersistenceError::Conflict { .. }),
        "must be reported as a conflict, not swallowed or mapped to something else: {err:?}"
    );
    assert_eq!(
        stored_fingerprint(&pool, Some("tenant-a"), TYPE, ID, OPERATION).await,
        FINGERPRINT,
        "the refused write must not have overwritten what tenant A already had stored — \
         this is the check that proves the isolation test above was not vacuous: if \
         scope isolation were broken by, say, an ON CONFLICT target narrower than the \
         real identity, this same-scope conflict would instead have silently replaced \
         the row, or the cross-tenant inserts above would have collided in the first \
         place"
    );
    assert_eq!(
        rows_for_key(&pool, OPERATION).await,
        2,
        "still exactly two rows"
    );

    db.close().await;
}

/// The other half of tenant isolation: the tenant-scoped partition and the
/// systemwide (`tenant_id IS NULL`) partition are the two complementary
/// halves migration 011 declares, and they must isolate from each other the
/// same way two concrete tenants do — not just in the in-memory store, which
/// keys on `Option<TenantId>` where `None == None` by construction and cannot
/// exercise `NULLS NOT DISTINCT`-style semantics against a real partial index.
#[tokio::test]
async fn tenant_isolation_covers_the_systemwide_partition_against_a_scoped_tenant() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let store = store(pool.clone()).await;

    const TYPE: &str = "invoice";
    const ID: &str = "shared-invoice-id";
    const OPERATION: &str = "op-systemwide-vs-scoped";
    const FINGERPRINT: &str = "fp-systemwide";

    let scoped = receipt(
        TYPE,
        ID,
        Some(tenant("tenant-a")),
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::NoEvents,
    );
    let systemwide = receipt(
        TYPE,
        ID,
        None,
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::events(1, 1).expect("a valid range"),
    );

    confirm(&store, &scoped)
        .await
        .expect("the tenant-scoped receipt confirms");
    confirm(&store, &systemwide).await.expect(
        "the systemwide receipt confirms independently — a comparison using `=` \
         instead of `IS NOT DISTINCT FROM` would make this collide or vanish",
    );

    assert_eq!(rows_for_key(&pool, OPERATION).await, 2);
    assert_eq!(
        *store
            .find_receipt(TYPE, ID, Some("tenant-a"), OPERATION)
            .await
            .expect("the lookup succeeds")
            .expect("the scoped receipt is found")
            .outcome(),
        AggregateOutcome::NoEvents
    );
    assert_eq!(
        *store
            .find_receipt(TYPE, ID, None, OPERATION)
            .await
            .expect("the lookup succeeds")
            .expect("the systemwide receipt is found")
            .outcome(),
        AggregateOutcome::events(1, 1).expect("a valid range"),
        "the systemwide scope's own outcome, not the tenant-scoped one's"
    );

    // --- negative control: within the systemwide scope itself --------------
    confirm(&store, &systemwide)
        .await
        .expect("a genuine retry within the systemwide scope replays");
    assert_eq!(rows_for_key(&pool, OPERATION).await, 2, "still no new row");

    let conflicting = receipt(
        TYPE,
        ID,
        None,
        OPERATION,
        "fp-different-request",
        AggregateOutcome::NoEvents,
    );
    let err = confirm(&store, &conflicting)
        .await
        .expect_err("a different request under the same systemwide identity is refused");
    assert!(matches!(err, PersistenceError::Conflict { .. }));
    assert_eq!(
        stored_fingerprint(&pool, None, TYPE, ID, OPERATION).await,
        FINGERPRINT,
        "the systemwide row is untouched by the refused write"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// Scenario 18 — aggregate_type isolation
// ---------------------------------------------------------------------------

/// Two different `aggregate_type` values, same tenant + `aggregate_id` +
/// `operation_key`: independent receipts. Then the same negative control
/// within one aggregate_type's own scope.
#[tokio::test]
async fn aggregate_type_isolation_does_not_collide_and_the_same_scope_replays_or_conflicts() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let store = store(pool.clone()).await;

    const TENANT: &str = "tenant-shared";
    const ID: &str = "shared-instance-id";
    const OPERATION: &str = "op-aggregate-type-isolation";
    const FINGERPRINT: &str = "fp-aggregate-type-isolation";

    // Unrelated operations against two entirely different kinds of aggregate
    // that happen to have been assigned the same bare id and reuse the same
    // operation key by coincidence — the case this identity has to rule out.
    let order = receipt(
        "order",
        ID,
        Some(tenant(TENANT)),
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::NoEvents,
    );
    let invoice = receipt(
        "invoice",
        ID,
        Some(tenant(TENANT)),
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::events(2, 5).expect("a valid range"),
    );

    confirm(&store, &order)
        .await
        .expect("the order receipt confirms");
    confirm(&store, &invoice).await.expect(
        "the invoice receipt confirms independently — same tenant, id and key, a \
         different aggregate_type, so this must not be read as a duplicate of order's",
    );

    assert_eq!(rows_for_key(&pool, OPERATION).await, 2);

    assert_eq!(
        *store
            .find_receipt("order", ID, Some(TENANT), OPERATION)
            .await
            .expect("the lookup succeeds")
            .expect("the order receipt is found")
            .outcome(),
        AggregateOutcome::NoEvents
    );
    assert_eq!(
        *store
            .find_receipt("invoice", ID, Some(TENANT), OPERATION)
            .await
            .expect("the lookup succeeds")
            .expect("the invoice receipt is found")
            .outcome(),
        AggregateOutcome::events(2, 5).expect("a valid range"),
        "the invoice's own outcome, not the order's — a collision would return NoEvents"
    );

    // --- negative control: same scope as `order`, same fingerprint --------
    confirm(&store, &order)
        .await
        .expect("a genuine retry within `order`'s own scope replays");
    assert_eq!(rows_for_key(&pool, OPERATION).await, 2, "still no new row");

    // --- negative control: same scope as `order`, different fingerprint ----
    let conflicting = receipt(
        "order",
        ID,
        Some(tenant(TENANT)),
        OPERATION,
        "fp-different-request",
        AggregateOutcome::NoEvents,
    );
    let err = confirm(&store, &conflicting)
        .await
        .expect_err("a different request reusing `order`'s identity is refused");
    assert!(matches!(err, PersistenceError::Conflict { .. }));
    assert_eq!(
        stored_fingerprint(&pool, Some(TENANT), "order", ID, OPERATION).await,
        FINGERPRINT,
        "the refused write must not have overwritten `order`'s stored fingerprint — the \
         check that proves the cross-type isolation above was not vacuous"
    );
    assert_eq!(
        rows_for_key(&pool, OPERATION).await,
        2,
        "still exactly two rows"
    );

    db.close().await;
}

// ---------------------------------------------------------------------------
// Scenario 19 — aggregate_id isolation
// ---------------------------------------------------------------------------

/// Two different `aggregate_id` values, same tenant + `aggregate_type` +
/// `operation_key`: independent receipts. Then the same negative control
/// within one aggregate_id's own scope.
#[tokio::test]
async fn aggregate_id_isolation_does_not_collide_and_the_same_scope_replays_or_conflicts() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let store = store(pool.clone()).await;

    const TENANT: &str = "tenant-shared";
    const TYPE: &str = "order";
    const OPERATION: &str = "op-aggregate-id-isolation";
    const FINGERPRINT: &str = "fp-aggregate-id-isolation";

    // Two unrelated orders that happen to reuse the same operation key by
    // coincidence — a client-supplied idempotency key has no obligation to be
    // globally unique across every aggregate it is ever presented against.
    let first = receipt(
        TYPE,
        "order-1",
        Some(tenant(TENANT)),
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::NoEvents,
    );
    let second = receipt(
        TYPE,
        "order-2",
        Some(tenant(TENANT)),
        OPERATION,
        FINGERPRINT,
        AggregateOutcome::events(9, 9).expect("a valid range"),
    );

    confirm(&store, &first)
        .await
        .expect("order-1's receipt confirms");
    confirm(&store, &second).await.expect(
        "order-2's receipt confirms independently — same tenant, type and key, a \
         different aggregate_id, so this must not be read as a duplicate of order-1's",
    );

    assert_eq!(rows_for_key(&pool, OPERATION).await, 2);

    assert_eq!(
        *store
            .find_receipt(TYPE, "order-1", Some(TENANT), OPERATION)
            .await
            .expect("the lookup succeeds")
            .expect("order-1's receipt is found")
            .outcome(),
        AggregateOutcome::NoEvents
    );
    assert_eq!(
        *store
            .find_receipt(TYPE, "order-2", Some(TENANT), OPERATION)
            .await
            .expect("the lookup succeeds")
            .expect("order-2's receipt is found")
            .outcome(),
        AggregateOutcome::events(9, 9).expect("a valid range"),
        "order-2's own outcome, not order-1's — a collision would return NoEvents"
    );

    // --- negative control: same scope as order-1, same fingerprint --------
    confirm(&store, &first)
        .await
        .expect("a genuine retry within order-1's own scope replays");
    assert_eq!(rows_for_key(&pool, OPERATION).await, 2, "still no new row");

    // --- negative control: same scope as order-1, different fingerprint ----
    let conflicting = receipt(
        TYPE,
        "order-1",
        Some(tenant(TENANT)),
        OPERATION,
        "fp-different-request",
        AggregateOutcome::NoEvents,
    );
    let err = confirm(&store, &conflicting)
        .await
        .expect_err("a different request reusing order-1's identity is refused");
    assert!(matches!(err, PersistenceError::Conflict { .. }));
    assert_eq!(
        stored_fingerprint(&pool, Some(TENANT), TYPE, "order-1", OPERATION).await,
        FINGERPRINT,
        "the refused write must not have overwritten order-1's stored fingerprint — the \
         check that proves the cross-aggregate isolation above was not vacuous"
    );
    assert_eq!(
        rows_for_key(&pool, OPERATION).await,
        2,
        "still exactly two rows"
    );

    db.close().await;
}
