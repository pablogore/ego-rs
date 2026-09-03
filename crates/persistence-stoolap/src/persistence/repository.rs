use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

use ego_persistence_api::persistence::{resolve_tenant, PersistenceError, Repository};
use stoolap::Database;

const CREATE_AGGREGATES_TABLE: &str = "
CREATE TABLE IF NOT EXISTS aggregates (
    tenant_id    TEXT    NOT NULL,
    aggregate_id TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    payload      TEXT    NOT NULL,
    UNIQUE (tenant_id, aggregate_id)
)";

const SELECT_VERSION: &str =
    "SELECT version FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2";
const INSERT_AGGREGATE: &str =
    "INSERT INTO aggregates (tenant_id, aggregate_id, version, payload) VALUES ($1, $2, 1, $3)";
const UPDATE_AGGREGATE: &str = "UPDATE aggregates SET version = $1, payload = $2 \
     WHERE tenant_id = $3 AND aggregate_id = $4 AND version = $5";
const LOAD_PAYLOAD: &str =
    "SELECT payload FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2";
const DELETE_AGGREGATE: &str = "DELETE FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2";

/// Classifies a raw Stoolap error as a lost optimistic-concurrency race
/// (`Conflict`) rather than a genuine failure (`Internal`). Default is
/// fail-loud: anything not recognized here stays `Internal`.
fn is_write_conflict(e: &stoolap::Error) -> bool {
    match e {
        stoolap::Error::UniqueConstraint { .. } => true,
        stoolap::Error::TransactionAborted => true,
        stoolap::Error::LockAcquisitionFailed(_) | stoolap::Error::DatabaseLocked => true,
        // Pinned, brittle-but-documented arm (EC-7): Stoolap's MVCC write-claim
        // conflict has no dedicated error variant, only this message text.
        stoolap::Error::Internal { message } => {
            message.contains("uncommitted changes from transaction")
        }
        _ => false,
    }
}

/// The scope a `None` tenant is stored under. Never returned to a caller,
/// never compared against a caller-supplied value — internal encoding only.
const SYSTEMWIDE_SCOPE: &str = "";

/// Maps a resolved tenant (`None` == systemwide) to its stored scope column
/// value. The sentinel is the empty string, which `resolve_tenant` already
/// rejects as a caller-supplied tenant (`MissingTenant`), so no real tenant
/// can ever collide with it.
fn encode_tenant(resolved: Option<&str>) -> &str {
    resolved.unwrap_or(SYSTEMWIDE_SCOPE)
}

fn dsn_for(path: &Path) -> String {
    format!("file://{}?sync=full", path.display())
}

fn internal_err(e: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::Internal(e.to_string())
}

/// Stoolap-backed implementation of `Repository<A>`.
///
/// `F` deserializes a stored payload (`serde_json::Value`) back into `A`.
pub struct StoolapRepository<A, F> {
    db: Database,
    deserialize: F,
    _marker: PhantomData<A>,
}

impl<A, F> fmt::Debug for StoolapRepository<A, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoolapRepository")
            .field("dsn", &self.db.dsn())
            .finish()
    }
}

impl<A, F> StoolapRepository<A, F>
where
    F: Fn(serde_json::Value) -> Result<A, PersistenceError>,
{
    pub fn new(path: &Path, deserialize: F) -> Result<Self, PersistenceError> {
        let db = Database::open(&dsn_for(path)).map_err(internal_err)?;
        db.execute(CREATE_AGGREGATES_TABLE, ())
            .map_err(internal_err)?;
        Ok(Self {
            db,
            deserialize,
            _marker: PhantomData,
        })
    }

    #[cfg(test)]
    fn dsn(&self) -> &str {
        self.db.dsn()
    }
}

impl<A, F> Repository<A> for StoolapRepository<A, F>
where
    A: serde::Serialize,
    F: Fn(serde_json::Value) -> Result<A, PersistenceError>,
{
    fn save(
        &mut self,
        aggregate_id: &str,
        aggregate: A,
        tenant_id: Option<&str>,
        expected_version: i64,
    ) -> Result<i64, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref());
        let payload = serde_json::to_string(&aggregate).map_err(internal_err)?;

        let mut tx = self.db.begin().map_err(internal_err)?;

        let current: Option<i64> = tx
            .query_opt(SELECT_VERSION, (scope, aggregate_id))
            .map_err(internal_err)?;

        let new_version = match current {
            None if expected_version == 0 => 1,
            None => {
                return Err(PersistenceError::Conflict {
                    aggregate_id: aggregate_id.to_string(),
                    expected: expected_version,
                    actual: 0,
                });
            }
            Some(c) if c == expected_version => expected_version + 1,
            Some(c) => {
                return Err(PersistenceError::Conflict {
                    aggregate_id: aggregate_id.to_string(),
                    expected: expected_version,
                    actual: c,
                });
            }
        };

        let write_result = match current {
            None => tx.execute(INSERT_AGGREGATE, (scope, aggregate_id, payload.as_str())),
            Some(_) => tx.execute(
                UPDATE_AGGREGATE,
                (
                    new_version,
                    payload.as_str(),
                    scope,
                    aggregate_id,
                    expected_version,
                ),
            ),
        };

        let affected = match write_result {
            Ok(affected) => affected,
            Err(e) if is_write_conflict(&e) => {
                return Err(PersistenceError::Conflict {
                    aggregate_id: aggregate_id.to_string(),
                    expected: expected_version,
                    actual: current.unwrap_or(0),
                });
            }
            Err(e) => return Err(internal_err(e)),
        };

        if affected != 1 {
            let actual: Option<i64> = tx
                .query_opt(SELECT_VERSION, (scope, aggregate_id))
                .map_err(internal_err)?;
            return Err(PersistenceError::Conflict {
                aggregate_id: aggregate_id.to_string(),
                expected: expected_version,
                actual: actual.unwrap_or(0),
            });
        }

        match tx.commit() {
            Ok(()) => Ok(new_version),
            Err(e) if is_write_conflict(&e) => Err(PersistenceError::Conflict {
                aggregate_id: aggregate_id.to_string(),
                expected: expected_version,
                actual: current.unwrap_or(0),
            }),
            Err(e) => Err(internal_err(e)),
        }
    }

    fn load(&self, aggregate_id: &str, tenant_id: Option<&str>) -> Result<A, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref());

        let payload: Option<String> = self
            .db
            .query_opt(LOAD_PAYLOAD, (scope, aggregate_id))
            .map_err(internal_err)?;

        let payload = payload.ok_or_else(|| PersistenceError::NotFound {
            aggregate_id: aggregate_id.to_string(),
        })?;

        let value: serde_json::Value = serde_json::from_str(&payload).map_err(internal_err)?;
        (self.deserialize)(value)
    }

    fn delete(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref());

        let affected = self
            .db
            .execute(DELETE_AGGREGATE, (scope, aggregate_id))
            .map_err(internal_err)?;

        if affected == 0 {
            return Err(PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            });
        }

        Ok(())
    }
}

/// Colocated unit tests open a real embedded Stoolap database per test, each
/// against its own `tempfile` path. Documented architectural exception under
/// `skills/testing/SKILL.md` Rule 1, per `design.md` AD-3 criterion 4, AD-4,
/// AD-7, and the same embedded/file-backed reasoning as AD-9 criterion 1
/// (see `crates/effect-store/src/stoolap/mod.rs`'s `fresh_store()`).
#[cfg(test)]
mod tests {
    use super::*;
    use ego_persistence_api::persistence::Repository;
    use serde::{Deserialize, Serialize};
    use std::path::Path;
    use tempfile::TempDir;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestAggregate {
        value: String,
    }

    fn deserialize_test_aggregate(v: serde_json::Value) -> Result<TestAggregate, PersistenceError> {
        serde_json::from_value(v).map_err(|e| PersistenceError::Internal(e.to_string()))
    }

    type TestRepo =
        StoolapRepository<TestAggregate, fn(serde_json::Value) -> Result<TestAggregate, PersistenceError>>;

    fn new_repo(path: &Path) -> TestRepo {
        StoolapRepository::new(path, deserialize_test_aggregate as fn(_) -> _).unwrap()
    }

    #[test]
    fn dsn_carries_full_sync() {
        assert_eq!(dsn_for(Path::new("/tmp/x")), "file:///tmp/x?sync=full");
    }

    #[test]
    fn encode_tenant_maps_only_the_absent_scope_to_the_sentinel() {
        assert_eq!(encode_tenant(None), SYSTEMWIDE_SCOPE);
        assert_eq!(encode_tenant(Some("t")), "t");
    }

    #[test]
    fn an_opened_repository_requested_full_sync() {
        let dir = TempDir::new().unwrap();
        let repo = new_repo(dir.path());
        assert_eq!(repo.dsn(), dsn_for(dir.path()));
    }

    #[test]
    fn a_committed_save_survives_close_and_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        {
            let mut repo = new_repo(path);
            let version = repo
                .save(
                    "agg-1",
                    TestAggregate {
                        value: "hello".into(),
                    },
                    None,
                    0,
                )
                .unwrap();
            assert_eq!(version, 1);
        }

        let repo = new_repo(path);
        let loaded = repo.load("agg-1", None).unwrap();
        assert_eq!(loaded.value, "hello");
    }

    #[test]
    fn two_systemwide_saves_leave_exactly_one_row() {
        let dir = TempDir::new().unwrap();
        let mut repo = new_repo(dir.path());
        repo.save("agg-2", TestAggregate { value: "a".into() }, None, 0)
            .unwrap();
        let second = repo
            .save("agg-2", TestAggregate { value: "b".into() }, None, 1)
            .unwrap();
        assert_eq!(second, 2);

        let rows = repo
            .db
            .query(
                "SELECT version FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2",
                (SYSTEMWIDE_SCOPE, "agg-2"),
            )
            .unwrap();
        assert_eq!(rows.count(), 1);
    }

    #[test]
    fn a_fresh_aggregate_with_a_nonzero_expected_version_is_a_conflict() {
        let dir = TempDir::new().unwrap();
        let mut repo = new_repo(dir.path());

        let err = repo
            .save(
                "agg-never-saved",
                TestAggregate { value: "a".into() },
                None,
                5,
            )
            .unwrap_err();

        match err {
            PersistenceError::Conflict {
                aggregate_id,
                expected,
                actual,
            } => {
                assert_eq!(aggregate_id, "agg-never-saved");
                assert_eq!(expected, 5);
                assert_eq!(actual, 0);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn a_stale_expected_version_is_a_conflict() {
        let dir = TempDir::new().unwrap();
        let mut repo = new_repo(dir.path());
        repo.save("agg-3", TestAggregate { value: "a".into() }, None, 0)
            .unwrap();
        repo.save("agg-3", TestAggregate { value: "b".into() }, None, 1)
            .unwrap();

        let err = repo
            .save("agg-3", TestAggregate { value: "c".into() }, None, 1)
            .unwrap_err();

        match err {
            PersistenceError::Conflict {
                aggregate_id,
                expected,
                actual,
            } => {
                assert_eq!(aggregate_id, "agg-3");
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn race_between_two_transactions_is_a_conflict() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        let mut repo1 = new_repo(path);
        repo1
            .save("agg-4", TestAggregate { value: "a".into() }, None, 0)
            .unwrap();

        // A real, still-open Stoolap transaction claims the row and has not
        // committed yet — a genuine concurrent writer mid-flight, not a mock.
        let mut claimant = repo1.db.begin().unwrap();
        claimant
            .execute(
                UPDATE_AGGREGATE,
                (2i64, "claimed", SYSTEMWIDE_SCOPE, "agg-4", 1i64),
            )
            .unwrap();

        // A second repository handle on the same DSN (same underlying engine,
        // per Stoolap's DSN registry) races against the still-uncommitted
        // claim above through the real public `save()` path.
        let mut repo2 = new_repo(path);
        let err = repo2
            .save("agg-4", TestAggregate { value: "b".into() }, None, 1)
            .unwrap_err();

        match err {
            PersistenceError::Conflict {
                aggregate_id,
                expected,
                ..
            } => {
                assert_eq!(aggregate_id, "agg-4");
                assert_eq!(expected, 1);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }

        claimant.commit().unwrap();
    }
}
