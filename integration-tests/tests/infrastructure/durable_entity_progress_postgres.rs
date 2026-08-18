//! **Guarantee:** an entity's events *and* its confirmed receipt outlive the
//! runtime that wrote them.
//!
//! **Layers traversed:** `build_runtime_with` — the composition root `main`
//! calls — → `compose_entity_runtimes` → `EntityRuntimeBuilder::with_event_store`
//! → `PostgreSQLEventStore` → real SQL against a real PostgreSQL with real
//! migrations. The runtimes are read back from `BuiltRuntime::entities`, so what
//! is exercised is what the host composed rather than what this file could
//! compose for itself.
//!
//! # Why this had to come before the crash test
//!
//! E1 asks whether a dual-aggregate operation resumes after a process dies
//! without repeating what it already confirmed. That question was unanswerable:
//! **no `EntityRuntime` anywhere was given a durable event store.** Production
//! took `EntityRuntimeBuilder`'s in-memory default, and receipts live *in the
//! event store* — `find_receipt` and `confirm_receipt` are on that port, and the
//! in-memory implementation keeps streams and receipts in one in-process map.
//!
//! So a crash destroyed the events and the receipt together. A recovery test
//! written against that would have failed for the wrong reason — nothing durable
//! to recover — and a passing one would have proved only that a fixture kept
//! state in memory.
//!
//! # What this asserts, and what it deliberately does not
//!
//! It does not kill a process, take over a lease, or exercise the mid-operation
//! boundary. Those are E1's. Its claim is narrower and is E1's precondition:
//! **the progress E1 must recover survives the runtime disappearing entirely.**
//!
//! Both aggregates are covered, and that is not symmetry for its own sake: there
//! are two typed stores, so the likely defect is wiring one and forgetting the
//! other — exactly what happened with observability, where the SDK half reported
//! normally while the entity half was silent.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use chrono::Utc;
use ego_domain::operation::{OperationFingerprint, OperationIdentity, OperationKey};
use ego_integration_tests::{isolated_database, IsolatedDatabase};
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::persistent_entity::CommandResult;
use reference_app::domain::tenant_org::{
    OrganizationEnsured, TenantOrgCommand, TenantOrgState, TenantOrganizationEntity,
};
use reference_app::domain::user::{UserCommand, UserEntity, UserRegistered, UserState};
use reference_app::{AppConfig, EntityEventStores};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

const ORG_TYPE: &str = "tenant_organization";
const USER_TYPE: &str = "user";
const TENANT: &str = "tenant-a";
const OPERATION_KEY: &str = "op-durable-progress";

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// This test's own database, and a URL anyone can open their own pool against.
///
/// The database is cloned from the run's already-migrated template, so nothing
/// here starts a container or applies a migration. The guard must be held for the
/// test's life: dropping it releases the connection-budget permit.
async fn postgres() -> (IsolatedDatabase, String) {
    let db = isolated_database().await;
    let url = db.url().to_string();
    (db, url)
}

/// The identity both passes carry — the same operation, retried.
fn identity() -> OperationIdentity {
    OperationIdentity::new(
        OperationKey::parse(OPERATION_KEY).expect("a valid key"),
        OperationFingerprint::new("fp-stable"),
    )
}

/// Builds the app through the **production** entry point, over a fresh pool.
///
/// A new pool per pass, on purpose: sharing one would leave the second pass
/// holding connections the first opened, and the claim is about what is in the
/// database rather than what a live handle still remembers.
async fn build(url: &str) -> reference_app::ObservedEntityRuntimes {
    let pool = connect(url).await;
    let stores = EntityEventStores::open(pool)
        .await
        .expect("the stores open against a migrated database");

    // Through `build_runtime_with` — the composition root `main` calls — and the
    // runtimes are read back from what it composed.
    //
    // Not `compose_entity_runtimes` directly, though that is where the stores
    // reach the aggregates. Calling it here would test the wiring subcomponent
    // and leave the host untested: a `build_runtime_with` that ignored the
    // stores it was handed and composed in-memory ones would keep every
    // assertion below green, because the test would have supplied the durable
    // stores itself.
    //
    // Reading them from `BuiltRuntime::entities` rather than resolving through
    // DI, because only `UserEntity` is registered — `RegisterUserImpl` holds the
    // organization runtime by hand — so resolution would reach one aggregate and
    // not the other, and this claim is about both.
    reference_app::build_runtime_with(
        &AppConfig::default(),
        stores,
        // E0's claim is about the event stores; the reservation posture is E1's.
        reference_app::IdempotencyWiring::Compatibility,
        None,
    )
    .expect("the reference app builds")
    .entities
}

/// A committed write, whichever shape the runtime reports it as.
///
/// `UserEntity` describes a welcome-email effect and this app wires no
/// acceptor, so a successful registration comes back as
/// `EffectsAcceptanceFailed` — a real, committed write with a post-commit
/// warning attached, never a command failure. Treating it as anything else
/// would make this test assert the absence of an effect subsystem rather than
/// the presence of durable events.
fn committed<E: std::fmt::Debug, S: std::fmt::Debug>(result: &CommandResult<E, S>) -> bool {
    matches!(
        result,
        CommandResult::Events { .. } | CommandResult::EffectsAcceptanceFailed { .. }
    )
}

/// How many events one aggregate has, straight from the table.
async fn event_count(pool: &PgPool, aggregate_type: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE aggregate_type = $1")
        .bind(aggregate_type)
        .fetch_one(pool)
        .await
        .expect("the count comes back")
}

/// How many confirmed receipts one aggregate has under this operation key.
async fn receipt_count(pool: &PgPool, aggregate_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM operation_receipts \
         WHERE aggregate_type = $1 AND operation_key = $2",
    )
    .bind(aggregate_type)
    .bind(OPERATION_KEY)
    .fetch_one(pool)
    .await
    .expect("the count comes back")
}

/// Runs the organization step once, through a runtime built from `url`.
async fn ensure_org(url: &str) -> CommandResult<OrganizationEnsured, TenantOrgState> {
    let runtimes = build(url).await;
    let entity_ref = runtimes
        .org
        .entity_ref::<TenantOrgCommand, TenantOrgState>(
            ORG_TYPE,
            TENANT,
            Arc::new(TenantOrganizationEntity),
        )
        .expect("an entity ref is obtainable");
    entity_ref
        .send_command(
            TenantOrgCommand::Ensure {
                org_id: TENANT.to_string(),
                name: "Acme".to_string(),
            },
            CommandContext::new(ORG_TYPE.to_string()).carrying(Some(identity())),
        )
        .await
        .expect("the organization command succeeds")
    // `runtimes` is dropped here: both entity runtimes and both stores go with
    // it. Everything the next pass sees came out of Postgres.
}

/// The organization's progress survives the runtime that wrote it.
#[tokio::test]
async fn an_organization_receipt_outlives_the_runtime_that_confirmed_it() {
    let (db, url) = postgres().await;
    let pool = connect(&url).await;

    let first = ensure_org(&url).await;
    assert!(
        committed(&first),
        "the first pass executes and commits: {first:?}"
    );
    assert_eq!(
        (
            event_count(&pool, ORG_TYPE).await,
            receipt_count(&pool, ORG_TYPE).await
        ),
        (1, 1),
        "one durable event and one confirmed receipt, both in the database"
    );

    // Everything that wrote the first pass is gone — runtime, entity runtimes,
    // stores, pool. This is a new one over the same database.
    let second = ensure_org(&url).await;
    assert!(
        matches!(second, CommandResult::Replayed { .. }),
        "a new runtime must find the confirmed receipt and replay rather than \
         re-execute — re-executing is what an in-memory store forced: {second:?}"
    );
    assert_eq!(
        (
            event_count(&pool, ORG_TYPE).await,
            receipt_count(&pool, ORG_TYPE).await
        ),
        (1, 1),
        "still exactly one event and one receipt: the replay wrote nothing"
    );

    // Released here: the semaphore counts live databases, so a guard left
    // to the container teardown would make that count a fiction.
    db.close().await;
}

/// The same, for `User` — the second typed store.
///
/// Not symmetry for its own sake. Two stores means two chances to wire one and
/// forget the other, and a test covering only the organization would pass with
/// `User` still writing to memory.
#[tokio::test]
async fn a_user_receipt_outlives_the_runtime_that_confirmed_it() {
    let (db, url) = postgres().await;
    let pool = connect(&url).await;

    let register = |url: String| async move {
        let runtimes = build(&url).await;
        let entity_ref = runtimes
            .user
            .entity_ref::<UserCommand, UserState>(USER_TYPE, "u1", Arc::new(UserEntity))
            .expect("an entity ref is obtainable");
        let result: CommandResult<UserRegistered, UserState> = entity_ref
            .send_command(
                UserCommand::Register {
                    user_id: "u1".to_string(),
                    email: "u1@example.test".to_string(),
                    tenant_id: TENANT.to_string(),
                },
                CommandContext::new(USER_TYPE.to_string()).carrying(Some(identity())),
            )
            .await
            .expect("the registration succeeds");
        result
    };

    let first = register(url.clone()).await;
    assert!(
        committed(&first),
        "the first pass executes and commits: {first:?}"
    );

    let second = register(url.clone()).await;
    assert!(
        matches!(second, CommandResult::Replayed { .. }),
        "a new runtime must replay from the durable receipt: {second:?}"
    );
    assert_eq!(
        (
            event_count(&pool, USER_TYPE).await,
            receipt_count(&pool, USER_TYPE).await
        ),
        (1, 1),
        "exactly one UserRegistered and one confirmed receipt survive both passes"
    );

    // Released here: the semaphore counts live databases, so a guard left
    // to the container teardown would make that count a fiction.
    db.close().await;
}

/// The two aggregates keep separate receipts under one operation key.
///
/// This is what E1 will lean on: after a crash between the two steps, the
/// organization must be found confirmed while the user is not. If both shared a
/// receipt — or if one store shadowed the other — that distinction would not
/// exist, and recovery would have nothing to decide from.
#[tokio::test]
async fn each_aggregate_keeps_its_own_receipt_under_one_operation_key() {
    let (db, url) = postgres().await;
    let pool = connect(&url).await;

    let _ = ensure_org(&url).await;

    assert_eq!(
        receipt_count(&pool, ORG_TYPE).await,
        1,
        "the organization step confirmed its own receipt"
    );
    assert_eq!(
        receipt_count(&pool, USER_TYPE).await,
        0,
        "and the user step, which never ran, has none — the partial state E1 \
         recovers from is representable"
    );

    // Now run the user step under the *same* key, which is what recovery does.
    //
    // The assertion above shows the partial state is representable; on its own it
    // would also hold if the two aggregates could not both hold a receipt for one
    // operation. Recovery needs both properties: distinguishable while partial,
    // and coexisting once complete.
    let runtimes = build(&url).await;
    let entity_ref = runtimes
        .user
        .entity_ref::<UserCommand, UserState>(USER_TYPE, "u1", Arc::new(UserEntity))
        .expect("an entity ref is obtainable");
    let result: CommandResult<UserRegistered, UserState> = entity_ref
        .send_command(
            UserCommand::Register {
                user_id: "u1".to_string(),
                email: "u1@example.test".to_string(),
                tenant_id: TENANT.to_string(),
            },
            CommandContext::new(USER_TYPE.to_string()).carrying(Some(identity())),
        )
        .await
        .expect("the registration succeeds");
    assert!(
        committed(&result),
        "the user step runs: its own receipt is absent, whatever the organization\'s          says: {result:?}"
    );

    assert_eq!(
        (
            receipt_count(&pool, ORG_TYPE).await,
            receipt_count(&pool, USER_TYPE).await
        ),
        (1, 1),
        "both aggregates now hold their own confirmed receipt under one operation \
         key — neither overwrote nor blocked the other"
    );

    // Released here: the semaphore counts live databases, so a guard left
    // to the container teardown would make that count a fiction.
    db.close().await;
}

// ---------------------------------------------------------------------------
// E0a: the envelope timestamp survives the round trip
// ---------------------------------------------------------------------------

/// The instant an event happened survives the **whole** round trip.
///
/// Written through the real port, not with hand-rolled SQL. An earlier version
/// inserted rows directly, which proved only that `load` hands `created_at` to
/// the factories — a regression putting `NOW()` back on the *write* side would
/// have survived it. The path this covers is:
///
/// ```text
/// event.occurred_at() -> append -> events.created_at -> load -> deserialize(.., occurred_at)
/// ```
///
/// The instant is far in the past and not a round number, so a synthesised
/// `now()` — or a plausible substitute like the row's insertion moment — differs
/// rather than coincidentally matching. Truncated to microseconds, which is
/// PostgreSQL's real `TIMESTAMPTZ` resolution: asserting nanoseconds would fail
/// a correct implementation.
///
/// Both factories, because there are two and a test covering one is blind to the
/// other dropping its timestamp — measured, that exact mutation survived until
/// this covered both.
#[tokio::test]
async fn the_instant_an_event_happened_survives_append_and_load() {
    use chrono::{TimeZone, Timelike};
    use ego_domain::event::DomainEvent;
    use ego_domain::persistence::StoredEvent;

    let (db, url) = postgres().await;

    let happened_at = Utc
        .with_ymd_and_hms(2021, 3, 14, 15, 9, 26)
        .unwrap()
        .with_nanosecond(535_897_000)
        .expect("a microsecond-precision instant");

    // --- write, through the real store -------------------------------------
    {
        let stores = EntityEventStores::open(connect(&url).await)
            .await
            .expect("the stores open");

        let org = OrganizationEnsured::from_stored(
            serde_json::json!({ "org_id": TENANT, "name": "Acme" }),
            happened_at,
        )
        .expect("the organization event rebuilds");
        assert_eq!(
            *org.occurred_at(),
            happened_at,
            "the fixture itself must carry the instant, or the test proves nothing"
        );
        stores
            .org
            .append(ORG_TYPE, TENANT, None, 0, vec![StoredEvent::new(org)])
            .await
            .expect("the organization event appends");

        let user = UserRegistered::from_stored(
            serde_json::json!({
                "user_id": "u1",
                "email": "u1@example.test",
                "tenant_id": TENANT,
            }),
            happened_at,
        )
        .expect("the user event rebuilds");
        stores
            .user
            .append(USER_TYPE, "u1", None, 0, vec![StoredEvent::new(user)])
            .await
            .expect("the user event appends");
        // Both stores and their pool are dropped here.
    }

    // --- read, through a completely new store ------------------------------
    let stores = EntityEventStores::open(connect(&url).await)
        .await
        .expect("the stores open again");

    let org = stores
        .org
        .load(ORG_TYPE, TENANT, None)
        .await
        .expect("the organization stream loads");
    assert_eq!(
        *org[0].event.occurred_at(),
        happened_at,
        "the organization event must report when it happened, end to end"
    );

    let user = stores
        .user
        .load(USER_TYPE, "u1", None)
        .await
        .expect("the user stream loads");
    assert_eq!(
        *user[0].event.occurred_at(),
        happened_at,
        "and so must the user event — two factories, two chances to drop it"
    );

    // Released here: the semaphore counts live databases, so a guard left
    // to the container teardown would make that count a fiction.
    db.close().await;
}
