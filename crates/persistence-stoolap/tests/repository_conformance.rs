//! S3 (design AD-9, AD-11): the shared `Repository` conformance harness, run
//! against `StoolapRepository` — Stoolap becomes the harness's third subject,
//! alongside `InMemoryRepository` and `PostgreSQLRepository`.
//!
//! Each run gets a fresh `tempfile::TempDir`, so no state ever survives
//! across tests or across runs.

use ego_persistence_api::persistence::PersistenceError;
use ego_persistence_stoolap::StoolapRepository;
use ego_testkit::{assert_repository_conformance, ConformanceAggregate};

type Repo = StoolapRepository<
    ConformanceAggregate,
    fn(serde_json::Value) -> Result<ConformanceAggregate, PersistenceError>,
>;

fn deserialize(value: serde_json::Value) -> Result<ConformanceAggregate, PersistenceError> {
    serde_json::from_value(value)
        .map_err(|e| PersistenceError::Internal(format!("bad payload: {e}")))
}

#[test]
fn stoolap_repository_satisfies_the_shared_conformance_suite() {
    let dir = tempfile::tempdir().expect("must create a temp dir for the Stoolap database");
    let mut repository: Repo =
        StoolapRepository::new(dir.path(), deserialize as fn(_) -> _).unwrap();

    assert_repository_conformance(&mut repository);
}
