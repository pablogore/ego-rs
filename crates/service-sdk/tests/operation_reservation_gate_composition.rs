//! STOOLAP-S3 Phase 5 (cross-backend gate proof): the operation reservation
//! store gate (`RuntimeBuilder::validate_operation_reservation_profile`,
//! wired via `App::builder().operation_reservation_store(..)`) must classify
//! the workspace's three REAL `OperationReservationStore` implementations
//! correctly under `Profile::Production` — not a synthetic
//! `StubReservationStore(bool)` double. The gate mechanism itself (the
//! {Dev,Production} x {none,volatile,durable} matrix, the actionable
//! message, and `build()`/`try_build()` parity) is already proven against
//! that stub in `crates/service-sdk/src/runtime/builder.rs` (Phase 4); this
//! file is the missing end-to-end half — mirrors
//! `read_side_progress_composition.rs`'s `App::builder()` surfacing pattern.
//!
//! Run with: cargo test -p ego-service-sdk --test operation_reservation_gate_composition

use std::sync::Arc;

use ego_domain::time::SystemClock;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore;
use ego_persistence_stoolap::StoolapOperationReservationStore;
use ego_service_sdk::app::App;
use ego_service_sdk::app::CompositionError;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, Profile, RuntimeError};

/// Task 5.1 (spec `persistence-memory-adapter`): a real, non-durable
/// `InMemoryOperationReservationStore` registered under `Profile::Production`
/// is rejected — not because it is a stub that answers `false`, but because
/// the real in-memory adapter genuinely never overrides `is_durable()`.
#[test]
fn a_real_in_memory_reservation_store_is_rejected_under_production_profile() {
    let store = Arc::new(InMemoryOperationReservationStore::new(Arc::new(
        SystemClock,
    )));

    let err = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .profile(Profile::Production)
        .operation_reservation_store(store)
        .build()
        .err()
        .expect("Production with a real in-memory (volatile) reservation store must refuse");

    match err {
        CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_)) => {}
        other => panic!("expected Validation(PersistenceNotConfigured), got {other:?}"),
    }
}

/// Task 5.2 (spec `idempotent-command-processing`): a real
/// `PostgresOperationReservationStore` is accepted under `Profile::Production`.
/// `connect_lazy` only parses the DSN and never opens a socket (mirrors
/// `PostgresOperationReservationStore`'s own `store_without_a_live_connection`
/// test) — `is_durable()` is pure and reads no connection, so no live
/// database is required to prove the gate accepts it. `#[tokio::test]`, not
/// plain `#[test]`: `PgPoolOptions::connect_lazy` starts the pool's
/// background reaper task, which needs an active Tokio context even though
/// it opens no socket.
#[tokio::test]
async fn a_real_postgres_reservation_store_is_accepted_under_production_profile() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://user:pass@localhost/db")
        .expect("connect_lazy only parses the DSN, it does not connect");
    let store = Arc::new(PostgresOperationReservationStore::new(
        pool,
        Arc::new(SystemClock),
    ));

    let app = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .profile(Profile::Production)
        .operation_reservation_store(store)
        .build();

    assert!(
        app.is_ok(),
        "Production with a real, durable Postgres reservation store must succeed: {:?}",
        app.err()
    );
}

/// Task 5.3: a real `StoolapOperationReservationStore`, opened against a real
/// temp-file DSN with `sync=full` (the same setup WU2/WU3's conformance
/// suite uses), is accepted under `Profile::Production`. Honest only because
/// the reopen-durability test (`tests/reservation_conformance.rs`, Phase 3
/// task 3.1) already proved the underlying durability claim — this test does
/// not re-prove durability, only that the gate reads `is_durable()` on the
/// real store correctly.
#[tokio::test]
async fn a_real_stoolap_reservation_store_is_accepted_under_production_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(
        StoolapOperationReservationStore::open(dir.path(), Arc::new(SystemClock))
            .await
            .expect("open a real embedded StoolapOperationReservationStore"),
    );

    let app = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .profile(Profile::Production)
        .operation_reservation_store(store)
        .build();

    assert!(
        app.is_ok(),
        "Production with a real, durable Stoolap reservation store must succeed: {:?}",
        app.err()
    );
}
