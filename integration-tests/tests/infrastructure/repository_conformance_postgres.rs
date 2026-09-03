//! S1 (design AD-9): the shared `Repository` conformance harness, run against
//! `PostgreSQLRepository`. `integration-tests` already dev-depends on both
//! `ego-testkit` and `ego-persistence` (`Cargo.toml`), so this run adds no new
//! dependency edge.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_domain::persistence::PersistenceError;
use ego_integration_tests::isolated_database;
use ego_persistence::postgres::PostgreSQLRepository;
use ego_testkit::{assert_repository_conformance, ConformanceAggregate};
use sqlx::PgPool;

type Repo = PostgreSQLRepository<
    ConformanceAggregate,
    fn(serde_json::Value) -> Result<ConformanceAggregate, PersistenceError>,
>;

fn repo(pool: PgPool) -> Repo {
    let deserialize: fn(serde_json::Value) -> Result<ConformanceAggregate, PersistenceError> =
        |value| {
            serde_json::from_value(value)
                .map_err(|e| PersistenceError::Internal(format!("bad payload: {e}")))
        };
    PostgreSQLRepository::new(pool, deserialize)
}

#[tokio::test(flavor = "multi_thread")]
async fn postgresql_repository_satisfies_the_shared_conformance_suite() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let mut repository = repo(pool);

    assert_repository_conformance(&mut repository);

    db.close().await;
}
