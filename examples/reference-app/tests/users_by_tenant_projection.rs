//! `UsersByTenant` read-side projection — new capability, not in CORE-018's
//! original tasks.md. Proves the projection is fed by CORE-005's real
//! read-side engine (`ego-runtime`'s `TagSchedulerImpl` + `ReadSideStore` +
//! `OffsetStore`/`DedupStore`), not a hand-constructed read model, and that
//! the query path returns exactly what `RegisterUser` actually wrote.

mod support;

use std::time::Duration;

use ego_testkit::{PrincipalBuilder, ServiceTestFixture};
use reference_app::application::{RegisterInput, RegisterUser, RegisterUserTag};
use reference_app::read_side::{
    ReadSideHandles, ReadSideProgressStores, ReadSideSink, SharedReadSideStore, UsersByTenantStore,
};

fn input(user_id: &str, email: &str, tenant_id: &str, org_name: &str) -> RegisterInput {
    RegisterInput {
        user_id: user_id.to_string(),
        email: email.to_string(),
        tenant_id: tenant_id.to_string(),
        org_name: org_name.to_string(),
    }
}

/// Real registration through the full guarded service path
/// (`ServiceTestFixture` -> `RegisterUserTag` proxy), exactly like
/// `register_user_guard_chain.rs`, but with a `ReadSideSink` wired so the
/// write side actually feeds the read-side engine under test.
async fn register(store: &SharedReadSideStore, tenant_id: &str, input: RegisterInput) {
    let sink = ReadSideSink::new(store.clone());
    let (service, _org_runtime) = support::make_register_user_full(None, Some(sink));

    let principal = PrincipalBuilder::new().tenant(tenant_id).build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .principal(principal)
        .build();
    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");
    let ctx = fixture.context().with_tenant_id(tenant_id);

    proxy
        .register(ctx, input)
        .await
        .expect("registration succeeds");
}

/// Polls `condition` until true or `timeout` elapses — the scheduler is a
/// real background poller (CORE-005 is pull-based), so the read model
/// catches up asynchronously after the write returns.
async fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition was not met within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn projection_populates_from_real_registration_events_not_a_hand_built_read_model() {
    let store = SharedReadSideStore::new();
    register(
        &store,
        "tenant-a",
        input("user-1", "user@example.com", "tenant-a", "Acme"),
    )
    .await;

    // The projection is constructed AFTER the write and reads only from the
    // shared store the write already populated — proving the read model
    // comes from real emitted events, not a value asserted by the test.
    //
    // PROD-014A AD-3/AD-8: the progress pair arrives here type-erased as
    // `Arc<dyn OffsetStore + Send + Sync>` / `Arc<dyn DedupStore + Send +
    // Sync>` (`fake_durable()`, not `in_memory()`), proving the `Arc<T>`
    // forwarding impls work against the real `TagSchedulerImpl`, not just in
    // isolation.
    let handles = ReadSideHandles::new(store, ReadSideProgressStores::fake_durable());
    let query: UsersByTenantStore = handles.query.clone();
    let runtime = handles.spawn();

    wait_until(
        || !query.view("tenant-a").users.is_empty(),
        Duration::from_secs(2),
    )
    .await;
    let _ = runtime.stop().await;

    let view = query.view("tenant-a");
    assert_eq!(view.org_name.as_deref(), Some("Acme"));
    assert_eq!(view.users.len(), 1);
    assert_eq!(view.users[0].user_id, "user-1");
    assert_eq!(view.users[0].email, "user@example.com");
}

#[tokio::test]
async fn query_returns_only_what_was_registered_for_that_tenant() {
    let store = SharedReadSideStore::new();
    register(
        &store,
        "tenant-a",
        input("user-1", "a@example.com", "tenant-a", "Acme"),
    )
    .await;
    register(
        &store,
        "tenant-b",
        input("user-2", "b@example.com", "tenant-b", "Globex"),
    )
    .await;

    let handles = ReadSideHandles::new(store, ReadSideProgressStores::in_memory());
    let query = handles.query.clone();
    let runtime = handles.spawn();

    wait_until(
        || !query.view("tenant-a").users.is_empty() && !query.view("tenant-b").users.is_empty(),
        Duration::from_secs(2),
    )
    .await;
    let _ = runtime.stop().await;

    let tenant_a = query.view("tenant-a");
    assert_eq!(tenant_a.org_name.as_deref(), Some("Acme"));
    assert_eq!(tenant_a.users.len(), 1);
    assert_eq!(tenant_a.users[0].user_id, "user-1");

    let tenant_b = query.view("tenant-b");
    assert_eq!(tenant_b.org_name.as_deref(), Some("Globex"));
    assert_eq!(tenant_b.users.len(), 1);
    assert_eq!(tenant_b.users[0].user_id, "user-2");

    // Never-registered tenant stays empty — no cross-tenant leakage.
    assert!(query.view("tenant-c").users.is_empty());
    assert!(query.view("tenant-c").org_name.is_none());
}

// Finding 2 (correctness fix): the read-side engine used to hardcode
// `session.execute(None)` on every poll instead of reading the persisted
// offset — once a tag stream accumulated more than `batch_size` (default
// 20) events, every subsequent poll re-fetched the same first batch,
// which was already fully deduped, so the projection stalled forever short
// of the full stream. This drives 50 real events (25 registrations x 2
// events each: `OrganizationEnsured` + `UserRegistered`) through the real
// engine at the real default batch size and asserts every one of them is
// eventually reflected, not just the first 20.
#[tokio::test]
async fn projection_catches_up_past_the_first_poll_batch() {
    let store = SharedReadSideStore::new();

    // One shared service/sink for all 25 registrations (not `register()`'s
    // one-sink-per-call helper): `ReadSideSink`'s version counter is only
    // monotonic within a single sink instance, so reusing one across every
    // call is what gives the 50 events distinct, non-colliding versions.
    let sink = ReadSideSink::new(store.clone());
    let (service, _org_runtime) = support::make_register_user_full(None, Some(sink));
    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .principal(principal)
        .build();
    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");
    let ctx = fixture.context().with_tenant_id("tenant-a");

    for i in 1..=25 {
        proxy
            .register(
                ctx.clone(),
                input(
                    &format!("user-{i}"),
                    &format!("user{i}@example.com"),
                    "tenant-a",
                    "Acme",
                ),
            )
            .await
            .expect("registration succeeds");
    }

    let handles = ReadSideHandles::new(store, ReadSideProgressStores::in_memory());
    let query = handles.query.clone();
    let runtime = handles.spawn();

    wait_until(
        || query.view("tenant-a").users.len() == 25,
        Duration::from_secs(5),
    )
    .await;
    let _ = runtime.stop().await;

    assert_eq!(query.view("tenant-a").users.len(), 25);
}
