//! Reproduction tests for SEC-001 (issue #466) — cross-tenant actor routing.
//!
//! `EntityRuntime::entity_ref(entity_type, entity_id, handler)`
//! (`crates/persistent-entity/src/runtime.rs:248-264`) takes **no tenant
//! parameter**. When `single_tenant_mode` is `false`, the tenant used to
//! build the routing `EntityTriple` is the runtime's own fixed
//! `config.tenant_id`, set once at construction via
//! `EntityRuntimeBuilder::single_tenant(false).tenant_id("...")`
//! (`crates/persistent-entity/src/builder.rs:131-139`).
//!
//! `EntityRegistry` (`crates/persistent-entity/src/registry.rs`) — the
//! routing authority that decides "does a live actor already exist for this
//! triple" — keys every live entry purely by
//! `EntityTriple::aggregate_id()`, i.e. `"{entity_type}-{entity_id}"`
//! (`crates/persistent-entity/src/scheduler.rs:30`). The tenant component of
//! the triple is dropped from that key entirely.
//!
//! `EntityRuntimeBuilder::with_registry(Arc<EntityRegistry>)`
//! (`crates/persistent-entity/src/builder.rs:141`) is public, so nothing
//! stops a host from constructing two `EntityRuntime`s with different fixed
//! `tenant_id`s that share one `EntityRegistry`. Because the registry's key
//! ignores tenant, that shared registry cannot tell those two runtimes'
//! identical `(entity_type, entity_id)` requests apart — the cross-tenant
//! path this file reproduces.
//!
//! Every test here uses one shared `InMemoryEventStore` for both runtimes,
//! so the *store* — which correctly partitions streams by
//! `(aggregate_type, aggregate_id, tenant)` (`persistence.rs` ~485-563) — is
//! never the reason isolation holds or breaks. Only the registry is varied
//! (shared vs. per-runtime) between the "_shared" tests and their "_control"
//! counterpart.

use std::sync::Arc;
use std::time::Duration;

use ego_domain::persistence::{EventStore, PersistenceError};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistence::InMemoryEventStore;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::registry::EntityRegistry;
use persistent_entity::runtime::EntityRuntime;
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{TestCommand, TestEvent, TestState};

const ENTITY_TYPE: &str = "order";
const ENTITY_ID: &str = "42";

fn handler(
) -> Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>> {
    Arc::new(TestEntity::new())
}

fn ctx() -> CommandContext {
    CommandContext::new(ENTITY_TYPE.to_string())
}

/// Builds a runtime pinned to `tenant_id`, sharing `store` and (optionally) `registry`.
fn runtime_for(
    tenant_id: &str,
    store: Arc<InMemoryEventStore<TestEvent>>,
    registry: Option<Arc<EntityRegistry>>,
) -> EntityRuntime<TestEvent> {
    let mut builder = EntityRuntimeBuilder::<TestEvent>::new()
        .single_tenant(false)
        .tenant_id(tenant_id)
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(store);
    if let Some(registry) = registry {
        builder = builder.with_registry(registry);
    }
    builder.build()
}

async fn get_value(runtime: &EntityRuntime<TestEvent>, entity_id: &str) -> u64 {
    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>(ENTITY_TYPE, entity_id, handler())
        .expect("entity_ref must succeed");
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::GetState, ctx()).await;
    match result.expect("GetState must succeed") {
        CommandResult::NoEvents { state } => state.value,
        CommandResult::Events { new_state, .. } => new_state.value,
        other => panic!("expected NoEvents or Events, got {other:?}"),
    }
}

async fn increment(runtime: &EntityRuntime<TestEvent>, entity_id: &str, amount: u64) -> u64 {
    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>(ENTITY_TYPE, entity_id, handler())
        .expect("entity_ref must succeed");
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(amount), ctx())
        .await;
    match result.expect("Increment must succeed") {
        CommandResult::Events { new_state, .. } => new_state.value,
        CommandResult::NoEvents { state } => state.value,
        other => panic!("expected Events or NoEvents, got {other:?}"),
    }
}

/// Number of events in one tenant's stream; a stream that was never written counts as empty.
async fn stream_len(store: &InMemoryEventStore<TestEvent>, tenant: &str) -> usize {
    match store.load(ENTITY_TYPE, ENTITY_ID, Some(tenant)).await {
        Ok(events) => events.len(),
        Err(PersistenceError::NotFound { .. }) => 0,
        Err(other) => panic!("loading {tenant}'s stream failed: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// A_shared: two fixed-tenant runtimes sharing one registry (and one store) —
// the vulnerable configuration.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn shared_registry_keeps_same_id_entities_of_two_tenants_isolated() {
    let store = Arc::new(InMemoryEventStore::<TestEvent>::new());
    let registry = Arc::new(EntityRegistry::new());

    let runtime_a = runtime_for("tenant-a", store.clone(), Some(registry.clone()));
    let runtime_b = runtime_for("tenant-b", store.clone(), Some(registry));

    // 1. tenant-a mutates entity 42.
    let value_after_a = increment(&runtime_a, ENTITY_ID, 5).await;
    assert_eq!(
        value_after_a, 5,
        "tenant-a's own increment must be reflected in tenant-a's own read"
    );

    // 2. tenant-b reads the "same" entity id through its own runtime.
    let value_seen_by_b = get_value(&runtime_b, ENTITY_ID).await;
    assert_eq!(
        value_seen_by_b, 0,
        "SEC-001: tenant-b's entity_ref(\"{ENTITY_TYPE}\", \"{ENTITY_ID}\") must reach its \
         own fresh entity (value=0), got {value_seen_by_b}: the shared EntityRegistry routed \
         it onto tenant-a's live actor"
    );

    // 3. tenant-b mutates its own entity.
    let value_after_b = increment(&runtime_b, ENTITY_ID, 100).await;
    assert_eq!(
        value_after_b, 100,
        "SEC-001: tenant-b's increment must apply to tenant-b's own entity (100), \
         got {value_after_b}"
    );

    // 4. tenant-a's own read must be unaffected by tenant-b's write.
    let value_seen_by_a_after = get_value(&runtime_a, ENTITY_ID).await;
    assert_eq!(
        value_seen_by_a_after, 5,
        "SEC-001: tenant-a's state changed from 5 to {value_seen_by_a_after} because of a \
         command tenant-b issued — the shared registry routed tenant-b's write onto \
         tenant-a's actor instance"
    );

    // 5. the store itself must still hold one stream per tenant.
    let stream_a = stream_len(&store, "tenant-a").await;
    let stream_b = stream_len(&store, "tenant-b").await;
    assert_eq!(
        stream_a, 1,
        "SEC-001: tenant-a's stream must hold only tenant-a's own event, found {} events — \
         a shared live actor persists under whichever tenant reached the mailbox first, \
         corrupting the per-tenant stream split the store itself correctly implements",
        stream_a
    );
    assert_eq!(
        stream_b, 1,
        "tenant-b's own stream must hold exactly tenant-b's own event, found {} events",
        stream_b
    );
}

// ---------------------------------------------------------------------------
// A_control: identical setup, but each runtime keeps its own (default)
// registry — isolation is expected to hold here today.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn control_separate_registries_keep_tenants_isolated() {
    let store = Arc::new(InMemoryEventStore::<TestEvent>::new());

    let runtime_a = runtime_for("tenant-a", store.clone(), None);
    let runtime_b = runtime_for("tenant-b", store.clone(), None);

    let value_after_a = increment(&runtime_a, ENTITY_ID, 5).await;
    assert_eq!(value_after_a, 5, "tenant-a's own increment must apply");

    let value_seen_by_b = get_value(&runtime_b, ENTITY_ID).await;
    assert_eq!(
        value_seen_by_b, 0,
        "control: with separate registries, tenant-b must see a fresh (value=0) entity, \
         not tenant-a's state"
    );

    let value_after_b = increment(&runtime_b, ENTITY_ID, 100).await;
    assert_eq!(
        value_after_b, 100,
        "control: tenant-b's own increment must apply to tenant-b's own fresh entity"
    );

    let value_seen_by_a_after = get_value(&runtime_a, ENTITY_ID).await;
    assert_eq!(
        value_seen_by_a_after, 5,
        "control: tenant-a's state must be unaffected by tenant-b's write"
    );

    let stream_a = stream_len(&store, "tenant-a").await;
    let stream_b = stream_len(&store, "tenant-b").await;
    assert_eq!(
        stream_a, 1,
        "control: tenant-a's stream must hold exactly tenant-a's own event"
    );
    assert_eq!(
        stream_b, 1,
        "control: tenant-b's stream must hold exactly tenant-b's own event"
    );
}

// ---------------------------------------------------------------------------
// B_passivation_shared: the shared-registry collision must also be exercised
// through deactivate_if_mine / mark_passivated — i.e. it is not merely a
// first-activation race, it also corrupts recovery after passivation.
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn shared_registry_passivation_recovers_each_tenants_own_state() {
    let store = Arc::new(InMemoryEventStore::<TestEvent>::new());
    let registry = Arc::new(EntityRegistry::new());

    // Very short passivation timeout so the deterministic clock advance below
    // drives the actor through deactivate_if_mine / mark_passivated.
    let short_timeout = Duration::from_millis(10);
    let runtime_a = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            .single_tenant(false)
            .tenant_id("tenant-a")
            .passivation_timeout(short_timeout)
            .snapshot_strategy(Arc::new(NoSnapshot))
            .with_event_store(store.clone())
            .with_registry(registry.clone())
            .build(),
    );
    let runtime_b = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            .single_tenant(false)
            .tenant_id("tenant-b")
            .passivation_timeout(short_timeout)
            .snapshot_strategy(Arc::new(NoSnapshot))
            .with_event_store(store.clone())
            .with_registry(registry)
            .build(),
    );

    // tenant-a activates and mutates entity 42.
    let value_after_a = increment(&runtime_a, ENTITY_ID, 7).await;
    assert_eq!(value_after_a, 7, "tenant-a's own increment must apply");

    // Deterministically drive the actor past its passivation timeout —
    // mirrors builder.rs's own `advance_and_settle` pattern (no real sleeps).
    tokio::time::advance(short_timeout * 4).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        runtime_a.passivated_count(),
        1,
        "the shared actor for entity 42 must have passivated via mark_passivated \
         before the rest of this test can exercise recovery"
    );

    // tenant-b activates the "same" (entity_type, entity_id) after passivation —
    // this goes through the registry's now-empty slot and deactivate_if_mine's
    // teardown, then recovers from the store under tenant-b's own tenant scope.
    let value_seen_by_b = get_value(&runtime_b, ENTITY_ID).await;
    assert_eq!(
        value_seen_by_b, 0,
        "tenant-b must recover a fresh (value=0) entity from its own empty stream, \
         not tenant-a's post-passivation state"
    );

    // tenant-a re-activates after passivation and must recover its own state,
    // not whatever tenant-b just did.
    let value_seen_by_a_again = get_value(&runtime_a, ENTITY_ID).await;
    assert_eq!(
        value_seen_by_a_again, 7,
        "SEC-001: tenant-a re-activating after passivation must recover its own \
         value=7 from its own stream, got {value_seen_by_a_again} — the shared \
         registry's passivation/reactivation cycle let tenant-b's activity leak in"
    );
}

// ---------------------------------------------------------------------------
// C_concurrent_first_access_shared: two tasks race to be the first to
// activate the same (entity_type, entity_id) through the shared registry,
// one per tenant. No timing dependence — a Barrier makes both `entity_ref`
// calls line up, and the single-flight lookup_or_insert (ADR-001) resolves
// the race deterministically to exactly one winner every run.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_registry_concurrent_first_access_spawns_one_actor_per_tenant() {
    let store = Arc::new(InMemoryEventStore::<TestEvent>::new());
    let registry = Arc::new(EntityRegistry::new());

    let runtime_a = Arc::new(runtime_for(
        "tenant-a",
        store.clone(),
        Some(registry.clone()),
    ));
    let runtime_b = Arc::new(runtime_for("tenant-b", store.clone(), Some(registry)));

    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let barrier_a = barrier.clone();
    let runtime_a_task = runtime_a.clone();
    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        increment(&runtime_a_task, ENTITY_ID, 11).await
    });

    let barrier_b = barrier.clone();
    let runtime_b_task = runtime_b.clone();
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        increment(&runtime_b_task, ENTITY_ID, 23).await
    });

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let _value_a = result_a.expect("tenant-a's task must not panic");
    let _value_b = result_b.expect("tenant-b's task must not panic");

    let stream_a = stream_len(&store, "tenant-a").await;
    let stream_b = stream_len(&store, "tenant-b").await;

    assert_eq!(
        stream_a, 1,
        "SEC-001: tenant-a's stream must hold exactly its own one increment event, \
         found {} — a concurrent first-access race through the shared registry can \
         collapse both tenants' commands onto whichever runtime's actor won \
         lookup_or_insert, so the loser's write lands in the winner's stream instead \
         of its own",
        stream_a
    );
    assert_eq!(
        stream_b, 1,
        "SEC-001: tenant-b's stream must hold exactly its own one increment event, \
         found {} — see tenant-a's assertion above for why this can fail",
        stream_b
    );

    let final_value_a = get_value(&runtime_a, ENTITY_ID).await;
    let final_value_b = get_value(&runtime_b, ENTITY_ID).await;
    assert_eq!(
        final_value_a, 11,
        "tenant-a must observe only its own increment (11), got {final_value_a}"
    );
    assert_eq!(
        final_value_b, 23,
        "tenant-b must observe only its own increment (23), got {final_value_b}"
    );
}
