//! App-level tenant isolation for persistent entities (found while
//! investigating SEC-001, issue #466; tracked separately).
//!
//! SEC-001 itself is the registry key: two `EntityRuntime`s pinned to
//! different tenants that share one `EntityRegistry` collide, because the
//! registry drops the tenant from its key. That is covered, and fixed, by
//! `crates/persistent-entity/tests/cross_tenant_routing_tests.rs`.
//!
//! This file checks a different question: whether a request's tenant ever
//! reaches entity routing when an application is composed through
//! `AppBuilder`. It does not. Both requests below build the *same* full
//! `EntityTriple` (the registered runtime's fixed tenant, the entity type
//! and the id), so keying the registry by the full triple does not change
//! the outcome. The assertions state the isolation a multi-tenant
//! deployment needs and fail today.
//!
//! ## What the public API makes impossible
//!
//! - `RuntimeBuilder::with_entity<E>(runtime: Arc<EntityRuntime<E::Event>>)`
//!   (`crates/service-sdk/src/runtime/builder.rs:401-417`, mirrored by
//!   `AppBuilder::entity` at `crates/service-sdk/src/app/mod.rs:403-429`)
//!   registers **at most one** `EntityRuntime` per aggregate type `E`,
//!   keyed by `TypeId::of::<E>()`. A second `.entity::<E>(..)` /
//!   `.with_entity::<E>(..)` call for the same `E` fails closed with
//!   `DuplicateEntity` (proved by
//!   `crates/service-sdk/src/app/mod.rs`'s own
//!   `entity_rejects_duplicate_registration_at_build` test). There is no
//!   supported way to register two `EntityRuntime`s — one per tenant — for
//!   the same aggregate type `E` through this API.
//! - `RuntimeInner::resolve_entity::<E>()`
//!   (`crates/service-sdk/src/runtime/runtime_builder.rs:138-153`,
//!   `:613-625`) always resolves that single registered runtime for `E`,
//!   with no tenant parameter anywhere in its signature.
//! - `EntityRuntimeRef::entity_ref(entity_type, entity_id, handler)`
//!   (`crates/service-sdk/src/di/mod.rs:80-91`) — the handle a service
//!   obtains through DI and the only way it reaches an entity — also takes
//!   no tenant parameter. A request handler holding a `ServiceContext` with
//!   a resolved tenant (`ctx.tenant_hint()`) has no parameter on this call
//!   through which to pass it.
//!
//! So the single `EntityRuntime` a real app registers for an aggregate type
//! is pinned to exactly one tenant identity for the process lifetime (its
//! own fixed `tenant_id`, or `"default"` under `single_tenant_mode`), and
//! every request — whatever tenant `ServiceContext` it carries — is
//! dispatched through that same runtime. This test does not invent a
//! tenant-routing mechanism the framework doesn't have; it drives the real
//! pipeline (`AppBuilder::entity` → `Injectable`-resolved
//! `EntityRuntimeRef<E>` → `entity_ref` → `send_command`) with two
//! different tenant `ServiceContext`s and asserts the isolation a
//! multi-tenant deployment needs. Given the limitation above, that
//! isolation cannot hold, and the assertions below are expected to fail —
//! which is itself the evidence: at this layer, two tenants addressing the
//! same aggregate type and id always reach the same entity.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ego_domain::persistence::{EventStore, StoredEvent};
use ego_service_sdk::app::App;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::di::{DepKey, EntityRuntimeRef, Injectable};
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeError, RuntimeInner};
#[allow(unused_imports)]
use ego_service_sdk_macros::{operation, service};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::persistence::InMemoryEventStore;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{TestCommand, TestEvent, TestState};

const ENTITY_TYPE: &str = "order";
const ENTITY_ID: &str = "42";

fn handler(
) -> Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>> {
    Arc::new(TestEntity::new())
}

// ---------------------------------------------------------------------------
// A tenant_scoped service, resolved through DI, that dispatches to the
// registered entity runtime per request.
// ---------------------------------------------------------------------------

#[service(version = "1.0.0")]
pub trait OrderService {
    /// `#[tenant_scoped]` is intentionally NOT applied here: the point of
    /// this reproduction is that even a handler that receives a
    /// tenant-carrying `ServiceContext` has no `entity_ref` parameter to
    /// forward that tenant through (see module doc). Adding
    /// `#[tenant_scoped]` would only add an authorization-time check on
    /// `ctx` — it would not create a routing path into the entity layer
    /// that does not exist.
    #[operation]
    async fn increment(
        &self,
        ctx: ServiceContext,
        entity_id: String,
        amount: u64,
    ) -> Result<u64, ServiceError>;

    #[operation]
    async fn get_value(&self, ctx: ServiceContext, entity_id: String) -> Result<u64, ServiceError>;
}

struct OrderServiceImpl {
    entity: EntityRuntimeRef<TestEntity>,
}

impl Injectable for OrderServiceImpl {
    fn dependencies() -> Vec<DepKey> {
        vec![DepKey::Entity(
            std::any::TypeId::of::<TestEntity>(),
            std::any::type_name::<TestEntity>(),
        )]
    }

    fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
        Ok(Self {
            entity: rt.resolve_entity::<TestEntity>()?,
        })
    }
}

#[async_trait]
impl OrderService for OrderServiceImpl {
    async fn increment(
        &self,
        ctx: ServiceContext,
        entity_id: String,
        amount: u64,
    ) -> Result<u64, ServiceError> {
        // `ctx.tenant_hint()` is available here but there is nowhere to hand
        // it to `entity_ref` — see the module doc's third bullet.
        let _ = ctx.tenant_hint();
        let entity_ref = self
            .entity
            .entity_ref::<TestCommand, TestState>(ENTITY_TYPE, entity_id, handler())
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        let result: Result<CommandResult<TestEvent, TestState>, _> = entity_ref
            .send_command(
                TestCommand::Increment(amount),
                CommandContext::new(ENTITY_TYPE.to_string()),
            )
            .await;
        match result.map_err(|e| ServiceError::internal(e.to_string()))? {
            CommandResult::Events { new_state, .. } => Ok(new_state.value),
            CommandResult::NoEvents { state } => Ok(state.value),
            other => Err(ServiceError::internal(format!(
                "unexpected CommandResult: {other:?}"
            ))),
        }
    }

    async fn get_value(&self, ctx: ServiceContext, entity_id: String) -> Result<u64, ServiceError> {
        let _ = ctx.tenant_hint();
        let entity_ref = self
            .entity
            .entity_ref::<TestCommand, TestState>(ENTITY_TYPE, entity_id, handler())
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        let result: Result<CommandResult<TestEvent, TestState>, _> = entity_ref
            .send_command(
                TestCommand::GetState,
                CommandContext::new(ENTITY_TYPE.to_string()),
            )
            .await;
        match result.map_err(|e| ServiceError::internal(e.to_string()))? {
            CommandResult::NoEvents { state } => Ok(state.value),
            CommandResult::Events { new_state, .. } => Ok(new_state.value),
            other => Err(ServiceError::internal(format!(
                "unexpected CommandResult: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// D_app_level: real AppBuilder pipeline, two tenant ServiceContexts.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn app_level_requests_from_two_tenants_reach_separate_entities() {
    let store = Arc::new(InMemoryEventStore::<TestEvent>::new());

    // The public API allows registering exactly ONE EntityRuntime for
    // `TestEntity` (see module doc). It must be pinned to some fixed
    // tenant identity; "tenant-a" is chosen arbitrarily — the point this
    // test makes does not depend on which one.
    let entity_runtime = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            .single_tenant(false)
            .tenant_id("tenant-a")
            .passivation_timeout(Duration::from_secs(3600))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .with_event_store(store.clone())
            .build(),
    );

    let app = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .entity::<TestEntity>(entity_runtime)
        .service_with_tag::<OrderServiceImpl, OrderServiceTag>(|arc| arc)
        .build()
        .expect("all declared dependencies are registered, build must succeed");

    let proxy = app
        .resolve::<OrderServiceTag>()
        .expect("OrderServiceImpl was registered and must resolve via its Tag");

    let ctx_tenant_a = ServiceContext::new().with_tenant_id("tenant-a");
    let ctx_tenant_b = ServiceContext::new().with_tenant_id("tenant-b");

    // Tenant A's request mutates entity 42 through the resolved runtime.
    let value_after_a = proxy
        .increment(ctx_tenant_a.clone(), ENTITY_ID.to_string(), 5)
        .await
        .expect("tenant-a's request must succeed");
    assert_eq!(value_after_a, 5, "tenant-a's own increment must apply");

    // Tenant B's request reads the "same" entity id through the SAME
    // resolved runtime — the DI layer has no other one to give it.
    let value_seen_by_b = proxy
        .get_value(ctx_tenant_b.clone(), ENTITY_ID.to_string())
        .await
        .expect("tenant-b's request must succeed");
    assert_eq!(
        value_seen_by_b, 0,
        "App-level tenant isolation: a request carrying ServiceContext::with_tenant_id(\"tenant-b\") \
         observed value={value_seen_by_b} instead of a fresh entity (0). The DI-resolved \
         EntityRuntimeRef<TestEntity> (di/mod.rs:80-91) has no tenant parameter, and \
         AppBuilder::entity/RuntimeBuilder::with_entity registers only one EntityRuntime per \
         aggregate type (app/mod.rs:403-429, runtime/builder.rs:401-417), so this request was \
         necessarily dispatched through tenant-a's fixed-tenant runtime"
    );

    // Tenant B's own write, through the same shared runtime.
    let value_after_b = proxy
        .increment(ctx_tenant_b.clone(), ENTITY_ID.to_string(), 100)
        .await
        .expect("tenant-b's request must succeed");

    // Tenant A's own subsequent read must be unaffected by tenant B's write.
    let value_seen_by_a_after = proxy
        .get_value(ctx_tenant_a.clone(), ENTITY_ID.to_string())
        .await
        .expect("tenant-a's request must succeed");
    assert_eq!(
        value_seen_by_a_after, 5,
        "App-level tenant isolation: tenant-a's state changed to {value_seen_by_a_after} (tenant-b's \
         write produced {value_after_b}) because both ServiceContexts were dispatched through \
         the one EntityRuntime the app registered for TestEntity"
    );

    // The event store must hold one stream per tenant.
    let stream_a: Vec<StoredEvent<TestEvent>> = store
        .load(ENTITY_TYPE, ENTITY_ID, Some("tenant-a"))
        .await
        .expect("tenant-a stream must be loadable");
    let stream_b_result = store.load(ENTITY_TYPE, ENTITY_ID, Some("tenant-b")).await;
    assert_eq!(
        stream_a.len(),
        1,
        "App-level tenant isolation: tenant-a's stream must hold exactly tenant-a's own event, \
         found {} — every write in this test, whichever ServiceContext requested it, was \
         persisted under the single registered runtime's fixed tenant_id",
        stream_a.len()
    );
    assert!(
        stream_b_result
            .as_ref()
            .map(|events| !events.is_empty())
            .unwrap_or(false),
        "App-level tenant isolation: tenant-b's own stream must hold tenant-b's own event; got \
         {stream_b_result:?} instead — this app-level composition provides no mechanism for a \
         request to reach a tenant-b-scoped stream for this aggregate type at all"
    );
}
