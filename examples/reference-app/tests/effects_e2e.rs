//! CORE-019 Phase 12.2 E2E: describe → deliver → retry → dedup, through
//! `examples/reference-app`'s REAL actor-spawning path
//! (`EntityRuntimeBuilder` -> `EntityRuntime` -> `TokioEntityRef::new` -> a
//! real `tokio::spawn`-ed `EntityActor`), with a real `service-sdk`
//! `RuntimeBuilder`-constructed `RuntimeEffectAcceptor` plugged in via
//! `EntityRuntimeBuilder::with_effect_acceptor` — the exact PR3/PR4-documented
//! gap this PR closes (design.md Phase 9 notes: "Actually plumbing that
//! acceptor into `persistent_entity::builder::EntityRuntimeBuilder` /
//! `EntityRuntime` / `TokioEntityRef::new` ... is left to whichever host
//! constructs both runtimes ... Phase 12/PR5's explicit scope").
//!
//! Uses `UserEntity`'s real `external_effects` override (one "welcome email"
//! effect per registration, `domain/user.rs`) and `ego-testkit`'s
//! `RecordingExecutor` registered via `service-sdk`'s
//! `register_effect_executor` — no synthetic unit-test-only actor.

use std::sync::Arc;
use std::time::Duration;

use ego_runtime::effects::AttemptOutcome;
use ego_service_sdk::RuntimeBuilder;
use ego_testkit::RecordingExecutor;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::CommandResult;
use reference_app::domain::user::{UserCommand, UserEntity, UserRegistered, UserState};

fn ctx() -> CommandContext {
    CommandContext::new("user".to_string())
}

fn register_command(user_id: &str) -> UserCommand {
    UserCommand::Register {
        user_id: user_id.to_string(),
        email: "e2e@example.com".to_string(),
        tenant_id: "default".to_string(),
    }
}

#[tokio::test]
async fn describe_deliver_retry_through_the_real_actor_spawn_path_and_repeat_register_is_a_noop() {
    // describe: `UserEntity::external_effects` describes one
    // "user.welcome_email" effect per committed `UserRegistered` event.
    let executor = Arc::new(RecordingExecutor::with_outcomes(vec![
        AttemptOutcome::RetryableFailure("simulated transient failure".to_string()),
        AttemptOutcome::Success,
    ]));

    let rt = RuntimeBuilder::new()
        .register_effect_executor(["user.welcome_email"], executor.clone())
        .unwrap()
        .build();
    rt.start_effects()
        .await
        .expect("an executor was registered — start_effects must succeed");
    let acceptor = rt.effect_acceptor().unwrap().clone();

    // The real actor-spawning path: EntityRuntimeBuilder -> EntityRuntime ->
    // entity_ref() -> TokioEntityRef::new() -> tokio::spawn-ed EntityActor.
    let user_runtime = Arc::new(
        EntityRuntimeBuilder::<UserRegistered>::new()
            .with_effect_acceptor(acceptor)
            .build(),
    );

    let user_ref = user_runtime
        .entity_ref::<UserCommand, UserState>("user", "user-e2e-1", Arc::new(UserEntity::new()))
        .unwrap();

    let result: Result<CommandResult<UserRegistered, UserState>, EntityError> = user_ref
        .send_command(register_command("user-e2e-1"), ctx())
        .await;
    assert!(
        matches!(result, Ok(CommandResult::Events { .. })),
        "registration must succeed: {result:?}"
    );

    // deliver + retry: the Deferred runner's first attempt fails with the
    // scripted RetryableFailure and is marked Retryable in the store; the
    // redispatch only happens on the runner's own reclaim tick
    // (`RECLAIM_INTERVAL`, `effects/runner.rs` — a real production 5s
    // constant, not configurable through `DeliveryConfig`), so this must
    // wait past at least one such tick before the scripted `Success` on
    // attempt 2 can be observed.
    tokio::time::timeout(Duration::from_secs(8), async {
        while executor.attempts().len() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the effect is delivered: retried once (past one reclaim tick), then succeeds");

    let attempts = executor.attempts();
    assert_eq!(attempts.len(), 2, "exactly one retry before success");
    assert_eq!(attempts[0].attempt, 1);
    assert_eq!(attempts[1].attempt, 2);
    assert_eq!(attempts[0].effect_type, "user.welcome_email");
    assert_eq!(attempts[0].destination, "mailer://welcome/user-e2e-1");

    // Re-registering the same user_id against the same already-rehydrated
    // handle no-ops at the entity level — see
    // `domain/user.rs::UserEntity::handle_command`. `CommandResult::NoEvents`
    // means `external_effects` is never invoked a second time, so no new
    // "welcome email" effect is even described: the executor call count below
    // stays bounded by construction, not by delivery-runner dedup.
    //
    // Stated so nobody reads coverage into this test that is not here: this
    // case used to reach the delivery runner twice and exercise its dedup path
    // end to end, which is what the old `..._then_dedup_...` name described.
    // The entity-level no-op removed that trigger, so the name went with it.
    // Delivery-runner dedup is covered by unit tests in
    // `crates/runtime/src/effects/runner.rs`
    // (`happy_path_success_marks_succeeded_and_commits_dedup`,
    // `dedup_conflict_marks_invalid_effect_terminal`,
    // `dedup_reserve_transient_failure_retries_then_succeeds`,
    // `dedup_reserve_permanent_error_is_immediately_terminal_without_retry`,
    // `dedup_other_succeeded_on_a_fresh_submission_is_marked_succeeded_not_terminal_failed`)
    // and by `cross_tenant_dedup_never_collides_even_with_identical_type_and_key`
    // in `crates/runtime/src/effects/store.rs`.
    let second_result: Result<CommandResult<UserRegistered, UserState>, EntityError> = user_ref
        .send_command(register_command("user-e2e-1"), ctx())
        .await;
    assert!(matches!(second_result, Ok(CommandResult::NoEvents { .. })));

    // Give the single-consumer runner a bounded window to drain the second
    // accept(); a dedup short-circuit never calls the executor, so the count
    // must never exceed 2. Poll instead of a single blind sleep: a fixed
    // sleep only checks once, at one arbitrary point in time — under slow
    // scheduling a real dedup regression could still land after that single
    // check, producing a false pass. Polling checks repeatedly across the
    // whole window and fails the instant a regression shows up, while still
    // returning fast in the (expected) case where nothing ever changes.
    let poll_until = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let count = executor.attempts().len();
        assert!(
            count <= 2,
            "a duplicate idempotency key must dedupe, never reach the executor a second time"
        );
        if tokio::time::Instant::now() >= poll_until {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        executor.attempts().len(),
        2,
        "a duplicate idempotency key must dedupe, never reach the executor a second time"
    );
}
