//! Provider composition validation (PROD-002 PR5 Phase 7.5): proves a REAL
//! `PostgresEffectStore`, registered through `RuntimeBuilder::with_effect_store`
//! exactly as a host would, is what the runtime's external-effects pipeline
//! ACTUALLY dispatches through.
//!
//! Deliberately NOT a `claim_due`/`mark_succeeded`/lease/retention semantics
//! re-test — that is Tier 1/2/3 conformance, already covered by
//! `effect_store_postgres_unit.rs` and `effect_store_postgres_conformance.rs`
//! alongside this file. This test's whole job is the wiring: does
//! `with_effect_store` compose with a real durable provider end to end, or
//! does dispatch silently land somewhere else.
//!
//! **Layers traversed:** `RuntimeBuilder::with_effect_store` → the real
//! `RuntimeEffectAcceptor`/delivery runner → `PostgresEffectStore` → real SQL
//! → PostgreSQL.
//!
//! **Why in-process cannot show this.** `crates/service-sdk/tests/
//! effect_store_composition.rs` already proves this exact seam generically,
//! against an in-process test double (`RecordingEffectStore`) and, for
//! Phase 7.5, a real embedded `StoolapEffectStore`. Neither can prove a real
//! *networked, multi-node* provider composes the same way: sqlx's real
//! connection pool, schema creation and migration path never run against a
//! double, and Stoolap has no separate network hop to get wrong. This test
//! drives one effect through a real `PostgresEffectStore` end to end, then
//! reads the row back with a second, completely independent connection —
//! proof the effect landed in PostgreSQL, not in some silently-substituted
//! `InMemoryEffectStore`.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::Duration;
use ego_domain::{ExternalEffectDescription, IdempotencyKey, SystemClock};
use ego_effect_store::PostgresEffectStore;
use ego_integration_tests::isolated_database;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use ego_testkit::RecordingExecutor;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::testing::{create_test_context, TestCommand, TestEvent, TestState};

/// Fixed rather than `uuid`-suffixed (same reasoning as
/// `effect_store_postgres_unit.rs`'s `SCHEMA`): this test's isolated database
/// is already exclusive to it, so nothing else can ever share this schema.
const SCHEMA: &str = "effect_store_composition";

/// This file's own fixture (module convention: each file in this suite keeps
/// its own), mirroring `crates/service-sdk/tests/effect_store_composition.rs`'s
/// `EffectDescribingEntity` exactly — `Increment` describes one external
/// effect, everything else describes none.
#[derive(Debug)]
struct EffectDescribingEntity;

#[async_trait]
impl PersistentEntity for EffectDescribingEntity {
    type Command = TestCommand;
    type Event = TestEvent;
    type State = TestState;

    fn initial_state(&self) -> TestState {
        TestState::new(0)
    }

    async fn handle_command(
        &self,
        command: &TestCommand,
        _state: &TestState,
        _context: &CommandContext,
    ) -> Result<Vec<TestEvent>, EntityError> {
        match command {
            TestCommand::Increment(v) => Ok(vec![TestEvent::Incremented(*v)]),
            TestCommand::Decrement(v) => Ok(vec![TestEvent::Decremented(*v)]),
            TestCommand::GetState => Ok(vec![]),
        }
    }

    async fn apply_event(
        &self,
        state: &TestState,
        event: &TestEvent,
    ) -> Result<TestState, EntityError> {
        Ok(match event {
            TestEvent::Incremented(v) => TestState {
                value: state.value + v,
                version: state.version + 1,
            },
            TestEvent::Decremented(v) => TestState {
                value: state.value.saturating_sub(*v),
                version: state.version + 1,
            },
        })
    }

    async fn apply_events(
        &self,
        state: &TestState,
        events: &[TestEvent],
    ) -> Result<TestState, EntityError> {
        let mut s = state.clone();
        for event in events {
            s = self.apply_event(&s, event).await?;
        }
        Ok(s)
    }

    async fn external_effects(
        &self,
        command: &TestCommand,
        _new_state: &TestState,
        events: &[TestEvent],
        _context: &CommandContext,
    ) -> Vec<ExternalEffectDescription> {
        if events.is_empty() {
            return Vec::new();
        }
        match command {
            TestCommand::Increment(_) => vec![ExternalEffectDescription {
                idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
                effect_type: "invoice.created".to_string(),
                payload: vec![],
                destination: "https://example.com".to_string(),
            }],
            _ => Vec::new(),
        }
    }
}

#[tokio::test]
async fn a_real_postgres_effect_store_registered_via_with_effect_store_actually_receives_the_dispatch(
) {
    let db = isolated_database().await;
    let store = Arc::new(
        PostgresEffectStore::connect(
            db.url(),
            SCHEMA,
            Duration::seconds(30),
            Arc::new(SystemClock),
        )
        .await
        .expect("connect a real PostgresEffectStore"),
    );
    let executor = Arc::new(RecordingExecutor::always_succeeds());

    let sdk_runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_store(store.clone())
        .register_effect_executor(["invoice.created"], executor.clone())
        .unwrap()
        .build();
    sdk_runtime
        .start_effects()
        .await
        .expect("an executor was registered — start_effects must succeed");

    let acceptor = sdk_runtime
        .effect_acceptor()
        .expect("start_effects must make build()'s wired acceptor available");
    let entity_runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .with_effect_acceptor(acceptor)
        .build();
    let entity_ref = entity_runtime
        .entity_ref(
            "probe",
            "postgres-composition-1",
            Arc::new(EffectDescribingEntity),
        )
        .expect("spawning a fresh actor must succeed");

    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .expect("the command itself must succeed regardless of effect delivery timing");
    assert!(
        matches!(result, CommandResult::Events { .. }),
        "expected a normal Events commit, got {result:?}"
    );

    // Independent inspection: a completely separate connection, opened
    // straight from the isolated-database fixture rather than through
    // `PostgresEffectStore`'s own internal pool, reading the raw row with
    // plain SQL. A row landing here — in PostgreSQL, in the schema THIS
    // store was constructed with — is evidence dispatch went through the
    // REGISTERED store, not that the runtime merely *believes* it did.
    // Polled, not asserted once, because `mark_succeeded` lands
    // asynchronously, a moment after `send_command` above already returned.
    let inspector = db.pool().await;
    // security review: `SCHEMA` is interpolated, not bound — Postgres cannot
    // bind an identifier (schema/table name) as a `$N` parameter. Injection-
    // safe here because `SCHEMA` is this file's own compile-time `&'static
    // str` constant above, never external input — the same posture already
    // documented at `crates/effect-store/src/postgres/mod.rs`'s
    // `search_path`/`CREATE SCHEMA` interpolations.
    let sql = format!("SELECT state FROM \"{SCHEMA}\".effect_state WHERE state = 'succeeded'");
    tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let row: Option<(String,)> = sqlx::query_as(&sql)
                .fetch_optional(&inspector)
                .await
                .expect("query the raw effect_state table");
            if row.is_some() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
    })
    .await
    .expect("the registered PostgresEffectStore must independently show the effect as succeeded");

    assert!(
        !executor.attempts().is_empty(),
        "the registered executor must actually have been invoked"
    );

    db.close().await;
}
