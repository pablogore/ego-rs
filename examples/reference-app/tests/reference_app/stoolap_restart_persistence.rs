//! PROD-002 Phase 8 — the "kill-the-process" success criterion
//! (proposal.md): an accepted external effect survives a real process
//! restart and is delivered exactly as the spec's reconstructability
//! requirement demands.
//!
//! "Process A" and "process B" are each built through
//! [`reference_app::build_runtime_with`] — the real public composition
//! path, never `StoolapEffectStore`'s own methods directly for the write
//! path (only [`StoolapEffectStore::open`], which every real host, and the
//! independent verification reader below, must call too).
//!
//! Between the two, process A's `App` is dropped WITHOUT ever calling
//! `App::start()`. That is the whole point: `build_runtime_with` wires the
//! `User` entity runtime's effect acceptor directly from a
//! never-`.start()`ed `RuntimeEffectAcceptor` (see its doc comment), which
//! durably WRITES the accepted effect into the real Stoolap store without
//! ever spawning a delivery task of its own — so nothing in process A ever
//! attempts, let alone completes, delivery. Only process B, which reopens
//! the same on-disk store fresh and actually calls `App::start()`, spawns a
//! real `DeliveryRunner` that claims and delivers it.
//!
//! Scope: proves ONLY the accepted-but-undelivered-at-"crash"-time case
//! (proposal.md criterion #1). It deliberately does not exercise
//! in-flight-at-crash redispatch (`recover_in_flight`, criterion #2 — no
//! production caller anywhere in the workspace, PROD-002 tasks.md 14.3) or
//! multi-node claim exclusivity (criterion #3) — both already covered by
//! Tier 2/3 conformance elsewhere.

use std::sync::Arc;
use std::time::Duration;

use ego_effect_store::StoolapEffectStore;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::persistent_entity::CommandResult;
use reference_app::domain::user::{UserCommand, UserEntity, UserRegistered, UserState};
use reference_app::effects::WelcomeEmailExecutor;
use reference_app::{
    build_runtime_with, AppConfig, BuiltRuntime, EntityEventStores, ExternalEffectsWiring,
    IdempotencyWiring,
};

fn ctx() -> CommandContext {
    CommandContext::new("user".to_string())
}

fn register_command(user_id: &str) -> UserCommand {
    UserCommand::Register {
        user_id: user_id.to_string(),
        email: "restart-test@example.com".to_string(),
        tenant_id: "default".to_string(),
    }
}

/// Queries the raw `effect_state` table through a brand-new, independent
/// Stoolap handle — the same DSN, but never the handle either process built
/// its `App` from. `InMemoryEffectStore` has no such table to even ask
/// about, so a row showing up here is real evidence about the REGISTERED
/// Stoolap store, never that a process merely *believes* delivery happened.
fn succeeded_row_exists(dsn: &str) -> bool {
    let inspector =
        stoolap::Database::open(dsn).expect("open an independent handle on the same DSN");
    let mut rows = inspector
        .query(
            "SELECT state FROM effect_state WHERE state = 'succeeded'",
            (),
        )
        .expect("query the raw effect_state table");
    matches!(rows.next(), Some(Ok(_)))
}

#[tokio::test]
async fn an_effect_accepted_before_a_restart_is_delivered_only_by_the_process_that_restarts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dsn = format!("file://{}", dir.path().display());

    // --- "Process A": register a real user, accept the effect, never start.
    {
        let store = Arc::new(
            StoolapEffectStore::open(dir.path())
                .await
                .expect("open a real embedded StoolapEffectStore"),
        );
        let BuiltRuntime { entities, .. } = build_runtime_with(
            &AppConfig::default(),
            EntityEventStores::in_memory(),
            IdempotencyWiring::Compatibility,
            None,
            ExternalEffectsWiring::Stoolap {
                store,
                executor: Arc::new(WelcomeEmailExecutor),
            },
            None,
            None,
        )
        .expect("process A builds through the real reference-app composition path");

        let user_ref = entities
            .user
            .entity_ref::<UserCommand, UserState>(
                "user",
                "restart-user-1",
                Arc::new(UserEntity::new()),
            )
            .expect("entity_ref");

        let result: CommandResult<UserRegistered, UserState> = user_ref
            .send_command(register_command("restart-user-1"), ctx())
            .await
            .expect("registration must succeed");

        assert!(
            matches!(result, CommandResult::Events { .. }),
            "the welcome-email effect must be ACCEPTED (CommandResult::Events, \
             not EffectsAcceptanceFailed) — a real EffectAcceptor is wired here, \
             even though process A never starts a delivery runner: {result:?}"
        );

        // Process A ends here — dropped without ever calling `App::start()`.
    }

    assert!(
        !succeeded_row_exists(&dsn),
        "process A must not have delivered the effect itself — surviving the \
         restart is the exact thing under test"
    );

    // --- "Process B": reopen the SAME on-disk store fresh, and actually start.
    let store_b = Arc::new(
        StoolapEffectStore::open(dir.path())
            .await
            .expect("reopen the same on-disk Stoolap store as a fresh handle"),
    );
    let BuiltRuntime { app, .. } = build_runtime_with(
        &AppConfig::default(),
        EntityEventStores::in_memory(),
        IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::Stoolap {
            store: store_b,
            executor: Arc::new(WelcomeEmailExecutor),
        },
        None,
        None,
    )
    .expect("process B builds through the same real composition path");

    let running = app
        .start()
        .await
        .expect("process B starts and spawns its real DeliveryRunner");

    // Polled, not asserted once: delivery lands asynchronously, a moment
    // after `claim_due` picks the row up — a fixed sleep would only check at
    // one arbitrary point. `effect_id` was accepted directly into the store
    // by process A, never through process B's own queue, so process B can
    // only ever discover it via `DeliveryRunner`'s periodic reclaim tick
    // (`RECLAIM_INTERVAL`, `effects/runner.rs` — a real production 5s
    // constant, deliberately skipped on the very first tick so a fresh
    // runner never reclaims before anything could possibly be due). This
    // must wait past at least one such tick — same margin `tests/
    // effects_e2e.rs` uses for the same reason.
    tokio::time::timeout(Duration::from_secs(8), async {
        while !succeeded_row_exists(&dsn) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect(
        "process B's real DeliveryRunner must deliver the effect accepted \
         by process A before the restart",
    );

    running.shutdown().await.expect("graceful shutdown");
}
