//! Integration test for CORE-016 Phase 5 — the Host -> AppConfig ->
//! service construction -> RuntimeBuilder pipeline (see design.md "Data Flow").

use ego_domain::Validate;
use reference_app::read_side::UsersByTenantStore;
use reference_app::{build_runtime_in_memory, AppConfig};

#[test]
fn valid_app_config_passes_validate_and_builds_runtime() {
    let config = AppConfig::default();

    assert!(
        config.validate().is_ok(),
        "default AppConfig should be valid"
    );

    let runtime = build_runtime_in_memory(&config);
    assert!(
        runtime.is_ok(),
        "build_runtime should construct services from a valid AppConfig"
    );
}

#[test]
fn invalid_subtree_config_fails_validate_before_any_service_is_constructed() {
    let mut config = AppConfig::default();
    // Invalidate a single subtree (EventBusConfig requires non-zero capacity).
    config.scheduler.capacity = 0;

    let validate_err = config.validate();
    assert!(
        validate_err.is_err(),
        "AppConfig::validate must reject an invalid subtree"
    );

    // build_runtime calls config.validate() before constructing any service
    // (see lib.rs `build_runtime` — `config.validate()?` is the first line),
    // so the same invalid config must fail the pipeline the same way.
    let pipeline_err = build_runtime_in_memory(&config);
    assert!(
        pipeline_err.is_err(),
        "build_runtime must return Err before constructing any service"
    );
}

// CORE-028 Stage 2 (task 5.2, design.md Testing Strategy): the design doc's
// own test plan for this feature is exactly this cheap, non-async assertion
// — the query handle `build_runtime`'s `.projection(...)` call registers
// must be resolvable through the DI path. That the resolved handle observes
// live engine writes needs the full HTTP/JWT stack over a real socket, and is
// no longer proven anywhere in this workspace — see
// `docs/integration-test-backlog.md`. Reachability alone does not establish it.
#[test]
fn build_runtime_registers_the_read_model_as_a_resolvable_projection() {
    let config = AppConfig::default();

    let runtime = build_runtime_in_memory(&config).expect("build_runtime succeeds");
    assert!(
        runtime
            .app
            .resolve_projection::<UsersByTenantStore>()
            .is_ok(),
        "UsersByTenantStore must be resolvable via the projection DI path after build"
    );
}

// CORE-028 Stage 2C (task 6.2, AD-7 item 2): mirrors
// `build_runtime_registers_the_read_model_as_a_resolvable_projection` — the
// entity-runtime DI path for `UserEntity`, registered via `.entity(...)` in
// `build_runtime`, must be resolvable through `App::resolve_entity` after
// build.
#[test]
fn build_runtime_registers_the_user_entity_runtime_as_resolvable() {
    use reference_app::domain::user::UserEntity;

    let config = AppConfig::default();

    let runtime = build_runtime_in_memory(&config).expect("build_runtime succeeds");
    assert!(
        runtime.app.resolve_entity::<UserEntity>().is_ok(),
        "UserEntity's runtime must be resolvable via the entity DI path after build"
    );
}

// CORE-028 Stage 2C review fix (resilience + reliability, 2 WARNINGs): the
// test above only proves `resolve_entity` *reachability* — it never calls
// `.entity_ref(...)` on the resolved handle, so real dispatch through the
// DI path was never exercised. And `build_runtime`'s own production wiring
// (`.entity::<UserEntity>(user_runtime.clone())`) is never independently
// proven to share the SAME live `EntityRuntime` as the hand-wired handle
// `RegisterUserImpl` writes through — two resolutions could both be `Ok`
// while secretly pointing at unconnected runtimes.
//
// This test builds its own `user_runtime` the same way `build_runtime` does
// (`Arc::new(EntityRuntimeBuilder::new().build())`) and hands the identical
// `Arc` to BOTH `RegisterUserImpl` (the production write path) and
// `App::builder().entity::<UserEntity>(...)` (the DI read path) — exactly
// mirroring `build_runtime`'s own wiring, not inventing a new one. Proof of
// sharing goes through `EntityRuntime::active_count()` (already-public,
// used elsewhere in `persistent-entity`'s own test suite) rather than
// `UserCommand::Register`'s resulting `UserState`: `UserEntity::handle_command`
// unconditionally overwrites state from whatever command it's given
// (`domain/user.rs`), so re-sending `Register` through the DI-resolved
// handle for the SAME already-registered id would trivially "match" the
// values the test itself supplies, regardless of whether the runtime is
// truly shared. Dispatching to a FRESH id through the DI-resolved handle and
// observing `user_runtime`'s OWN `active_count()` increment is what actually
// distinguishes "the DI handle writes into our runtime" from "the DI handle
// writes into some independent, disconnected runtime".
#[tokio::test]
async fn di_resolved_entity_runtime_ref_dispatches_and_shares_state_with_production_register_flow()
{
    use std::sync::Arc;

    use ego_service_sdk::App;
    use ego_testkit::{PrincipalBuilder, ServiceTestFixture};
    use persistent_entity::builder::EntityRuntimeBuilder;
    use persistent_entity::command_context::CommandContext;
    use persistent_entity::entity_ref::EntityRef;
    use persistent_entity::persistent_entity::CommandResult;
    use reference_app::application::{
        RegisterInput, RegisterUser, RegisterUserImpl, RegisterUserTag,
    };
    use reference_app::domain::user::{UserCommand, UserEntity, UserRegistered, UserState};

    // Mirrors `build_runtime`'s AD-4 construction exactly (lib.rs:228-229) —
    // test-side reuse of the same construction, not new production wiring.
    let org_runtime = Arc::new(EntityRuntimeBuilder::new().build());
    let user_runtime = Arc::new(EntityRuntimeBuilder::new().build());
    let service: Arc<dyn RegisterUser> = Arc::new(RegisterUserImpl::new(
        org_runtime,
        user_runtime.clone(),
        None,
    ));

    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .principal(principal)
        .build();
    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    // The real production register flow (guards + dual write), not a
    // test-only shortcut.
    let ctx = fixture.context().with_tenant_id("tenant-a");
    let input = RegisterInput {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        tenant_id: "tenant-a".to_string(),
        org_name: "Acme".to_string(),
    };
    let result = proxy.register(ctx, input).await;
    assert!(
        result.is_ok(),
        "production register flow must succeed: {result:?}"
    );
    assert_eq!(
        user_runtime.active_count(),
        1,
        "production register must have spawned exactly one live User actor"
    );

    // DI path: register the SAME `user_runtime` Arc through
    // `App::builder().entity::<UserEntity>(...)`, exactly as `build_runtime`
    // does, then resolve it back out via `App::resolve_entity`.
    let app = App::builder()
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .entity::<UserEntity>(user_runtime.clone())
        .build()
        .expect("build succeeds");
    let entity_runtime_ref = app
        .resolve_entity::<UserEntity>()
        .expect("UserEntity runtime resolves via the entity DI path");

    // Real dispatch through the DI-resolved handle (closes "no test exercises
    // real entity_ref() dispatch") — a fresh id, so the resulting state is
    // unambiguous proof of correct dispatch (not just "didn't panic").
    let fresh_ref = entity_runtime_ref
        .entity_ref::<UserCommand, UserState>("user", "user-2", Arc::new(UserEntity::new()))
        .expect("entity_ref succeeds through the DI-resolved handle");
    let fresh_result: CommandResult<UserRegistered, UserState> = fresh_ref
        .send_command(
            UserCommand::Register {
                user_id: "user-2".to_string(),
                email: "second@example.com".to_string(),
                tenant_id: "tenant-a".to_string(),
            },
            CommandContext::new("user".to_string()),
        )
        .await
        .expect("dispatch through the DI-resolved handle succeeds");
    // `UserEntity::external_effects` describes a "welcome email" effect on
    // every registration (domain/user.rs) — this test wires no
    // `EffectAcceptor` (mirroring `build_runtime`, which registers none
    // either), so a real, successful write here is `EffectsAcceptanceFailed`,
    // not `Events` — a committed write with a post-commit warning attached,
    // never a command failure (same as `register_user_partial_failure.rs`
    // handles it).
    match fresh_result {
        CommandResult::Events { new_state, .. }
        | CommandResult::EffectsAcceptanceFailed { new_state, .. } => assert_eq!(
            new_state,
            UserState::Registered {
                email: "second@example.com".to_string(),
                tenant_id: "tenant-a".to_string(),
            },
            "the DI-resolved handle must observe the command it was actually sent"
        ),
        other => panic!("expected a committed registration, got {other:?}"),
    }

    // The distinguishing proof: dispatching a FRESH id through the
    // DI-resolved handle only increments `user_runtime`'s OWN active count
    // if the DI handle is genuinely backed by the identical `EntityRuntime`
    // the production flow wrote through above — a disconnected/independent
    // instance would leave this count unchanged at 1.
    assert_eq!(
        user_runtime.active_count(),
        2,
        "the DI-resolved EntityRuntimeRef must share the exact same live \
         EntityRuntime as the production hand-wired handle, not an \
         independent instance"
    );
}

#[test]
fn build_runtime_wires_real_kit_config_output() {
    let config = AppConfig::default();

    let runtime = build_runtime_in_memory(&config);
    assert!(
        runtime.is_ok(),
        "build_runtime should materialize configuration through the real kit-config \
         loader (ConfigLoader -> ConfigurationProvider -> build_logger -> with_logger)"
    );
}

#[test]
fn invalid_cross_domain_rule_fails_validate() {
    let mut config = AppConfig::default();
    // Each subtree is individually valid, but the cross-domain rule (see
    // lib.rs `AppConfig::validate`) requires more database connections once
    // the runtime is multi-tenant.
    config.runtime.single_tenant_mode = false;
    config.runtime.tenant_id = "tenant-a".to_string();
    config.database.max_connections = 1;

    assert!(
        config.database.validate().is_ok(),
        "the database subtree alone is valid"
    );
    assert!(
        config.runtime.validate().is_ok(),
        "the runtime subtree alone is valid"
    );
    assert!(
        config.validate().is_err(),
        "the cross-domain rule must reject this combination even though each subtree is valid alone"
    );
}
