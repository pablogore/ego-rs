//! Stoolap-backed implementation of `ego_persistence_api::persistence::Snapshot`.
//!
//! Synchronous, like S1's `StoolapRepository` — the `Snapshot` trait has no
//! `async` methods, so this store needs nothing beyond what S1 already
//! established (design.md: "Two new stores ... `Snapshot` is synchronous, so
//! it needs nothing S1 does not already have").
//!
//! The `snapshots` table mirrors S1's `aggregates` table shape verbatim
//! (design.md AD-2): `tenant_id`/`aggregate_id`/`version`/`payload` with a
//! `UNIQUE (tenant_id, aggregate_id)` constraint — one row per aggregate, the
//! latest snapshot always overwrites the previous one.

use std::fmt;
use std::path::Path;

use ego_persistence_api::persistence::{resolve_tenant, PersistenceError, Snapshot};
use serde_json::Value;
use stoolap::Database;

use super::stoolap_common::{dsn_for, encode_tenant, internal_err};

const CREATE_SNAPSHOTS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS snapshots (
    tenant_id    TEXT    NOT NULL,
    aggregate_id TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    payload      TEXT    NOT NULL,
    UNIQUE (tenant_id, aggregate_id)
)";

const SELECT_EXISTS: &str =
    "SELECT version FROM snapshots WHERE tenant_id = $1 AND aggregate_id = $2";
const SELECT_SNAPSHOT: &str =
    "SELECT version, payload FROM snapshots WHERE tenant_id = $1 AND aggregate_id = $2";
const INSERT_SNAPSHOT: &str =
    "INSERT INTO snapshots (tenant_id, aggregate_id, version, payload) VALUES ($1, $2, $3, $4)";
const UPDATE_SNAPSHOT: &str = "UPDATE snapshots SET version = $1, payload = $2 \
     WHERE tenant_id = $3 AND aggregate_id = $4";

/// Stoolap-backed implementation of `Snapshot`.
///
/// Every write goes through the shared `dsn_for()` DSN (`sync=full`), and
/// `open()` refuses to hand back a store whose live engine is not actually
/// configured that way (design.md AD-3) — `is_durable()` is then backed by
/// that construction invariant, never a hardcoded value.
pub struct StoolapSnapshotStore {
    db: Database,
}

impl fmt::Debug for StoolapSnapshotStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoolapSnapshotStore")
            .field("dsn", &self.db.dsn())
            .finish()
    }
}

impl StoolapSnapshotStore {
    /// Opens (creating the `snapshots` table if absent) a Stoolap-backed
    /// snapshot store at `path`.
    ///
    /// Fails closed (design.md AD-3 criterion 2) if the live engine handed
    /// back for this DSN — Stoolap's `Database::open` shares a process-wide
    /// registry entry per open handle — is not actually configured for
    /// `sync=full`. A durability claim this store makes later
    /// (`is_durable()`) must never outlive-lie about how it was opened.
    pub fn open(path: &Path) -> Result<Self, PersistenceError> {
        let dsn = dsn_for(path);
        let db = Database::open(&dsn).map_err(internal_err)?;

        if !db.dsn().contains("sync=full") {
            return Err(PersistenceError::Internal(format!(
                "stoolap engine at {:?} is not configured for durable sync (sync=full); \
                 refusing to open a Snapshot store that would misreport is_durable()",
                db.dsn()
            )));
        }

        db.execute(CREATE_SNAPSHOTS_TABLE, ())
            .map_err(internal_err)?;

        Ok(Self { db })
    }

    #[cfg(test)]
    fn dsn(&self) -> &str {
        self.db.dsn()
    }
}

impl Snapshot for StoolapSnapshotStore {
    /// Truthful by construction (design.md AD-3 criterion 3): `open()` only
    /// ever returns a store whose live engine reports `sync=full`, so this
    /// re-derives from the same `db.dsn()` invariant rather than a fixed
    /// `true`.
    fn is_durable(&self) -> bool {
        self.db.dsn().contains("sync=full")
    }

    fn save_snapshot(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: i64,
        payload: Value,
    ) -> Result<(), PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref());
        let payload_str = serde_json::to_string(&payload).map_err(internal_err)?;

        let mut tx = self.db.begin().map_err(internal_err)?;

        let existing: Option<i64> = tx
            .query_opt(SELECT_EXISTS, (scope, aggregate_id))
            .map_err(internal_err)?;

        match existing {
            None => tx.execute(
                INSERT_SNAPSHOT,
                (scope, aggregate_id, version, payload_str.as_str()),
            ),
            Some(_) => tx.execute(
                UPDATE_SNAPSHOT,
                (version, payload_str.as_str(), scope, aggregate_id),
            ),
        }
        .map_err(internal_err)?;

        // AD-3 criterion 4: exactly one commit, no deferred/batched path — a
        // failed WAL sync here must surface as an `Err`, never silent success.
        tx.commit().map_err(internal_err)
    }

    fn load_snapshot(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<(i64, Value)>, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref());

        let mut rows = self
            .db
            .query(SELECT_SNAPSHOT, (scope, aggregate_id))
            .map_err(internal_err)?;

        let row = match rows.next() {
            None => return Ok(None),
            Some(row) => row.map_err(internal_err)?,
        };

        let version: i64 = row.get(0).map_err(internal_err)?;
        let payload_str: String = row.get(1).map_err(internal_err)?;
        let payload: Value = serde_json::from_str(&payload_str).map_err(internal_err)?;

        Ok(Some((version, payload)))
    }
}

/// Colocated unit tests open a real embedded Stoolap database per test, each
/// against its own `tempfile` path — same documented exception `repository.rs`
/// relies on (`skills/testing/SKILL.md` Rule 1, design.md AD-3/AD-4).
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Every test that touches the database serializes on stoolap's own
    /// failpoint lock, matching `repository.rs`'s guard — one of these tests
    /// arms the process-wide `WAL_WRITE_FAIL` failpoint.
    fn db_test_guard() -> stoolap::test_failpoints::FailpointGuard {
        stoolap::test_failpoints::FailpointGuard::new()
    }

    fn new_store(path: &Path) -> StoolapSnapshotStore {
        StoolapSnapshotStore::open(path).unwrap()
    }

    #[test]
    fn an_opened_store_requested_full_sync() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path());
        assert_eq!(store.dsn(), dsn_for(dir.path()));
        assert!(store.is_durable());
    }

    #[test]
    fn is_durable_reflects_the_instance_configuration_not_a_hardcoded_value() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();

        // A store built from a weak (no sync=full) DSN — constructed directly
        // rather than through `open()`, whose guard would refuse it — proves
        // `is_durable()` is derived per-instance, not a fixed `true`.
        let weak_dsn = format!("file://{}", dir.path().display());
        let weak_db = Database::open(&weak_dsn).unwrap();
        let weak_store = StoolapSnapshotStore { db: weak_db };
        assert!(!weak_store.is_durable());
        drop(weak_store);

        let durable_dir = TempDir::new().unwrap();
        let durable_store = new_store(durable_dir.path());
        assert!(durable_store.is_durable());
    }

    #[test]
    fn open_refuses_a_path_already_locked_by_a_non_durable_engine() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();

        // A live, weakly-configured engine holds the on-disk lock for this
        // path (design.md AD-3 criterion 2: Stoolap's process-global engine
        // registry must never let a durable-sounding store silently share, or
        // quietly coexist with, a weaker-than-`sync=full` engine on the same
        // file). `open()` must fail closed rather than succeed and later
        // report `is_durable() == true` untruthfully.
        let weak_dsn = format!("file://{}", dir.path().display());
        let _weak_db = Database::open(&weak_dsn).unwrap();

        let err = StoolapSnapshotStore::open(dir.path()).unwrap_err();
        match err {
            PersistenceError::Internal(_) => {}
            other => panic!("expected Internal (refused open), got {other:?}"),
        }
    }

    #[test]
    fn a_saved_snapshot_survives_close_and_reopen() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        {
            let mut store = new_store(path);
            store
                .save_snapshot("agg-1", None, 3, serde_json::json!({"value": "hello"}))
                .unwrap();
        }

        let store = new_store(path);
        let (version, payload) = store.load_snapshot("agg-1", None).unwrap().unwrap();
        assert_eq!(version, 3);
        assert_eq!(payload, serde_json::json!({"value": "hello"}));
    }

    #[test]
    fn loading_a_missing_snapshot_returns_none() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path());
        assert_eq!(store.load_snapshot("never-saved", None).unwrap(), None);
    }

    #[test]
    fn a_second_save_overwrites_the_first_as_the_latest_snapshot() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let mut store = new_store(dir.path());
        store
            .save_snapshot("agg-2", None, 1, serde_json::json!({"value": "a"}))
            .unwrap();
        store
            .save_snapshot("agg-2", None, 2, serde_json::json!({"value": "b"}))
            .unwrap();

        let (version, payload) = store.load_snapshot("agg-2", None).unwrap().unwrap();
        assert_eq!(version, 2);
        assert_eq!(payload, serde_json::json!({"value": "b"}));
    }

    #[test]
    fn a_tenants_snapshot_is_isolated_from_another_tenant_sharing_the_same_aggregate_id() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let mut store = new_store(dir.path());

        store
            .save_snapshot(
                "shared-agg",
                Some("tenant-a"),
                1,
                serde_json::json!({"owner": "a"}),
            )
            .unwrap();
        store
            .save_snapshot(
                "shared-agg",
                Some("tenant-b"),
                7,
                serde_json::json!({"owner": "b"}),
            )
            .unwrap();

        let a = store
            .load_snapshot("shared-agg", Some("tenant-a"))
            .unwrap()
            .unwrap();
        let b = store
            .load_snapshot("shared-agg", Some("tenant-b"))
            .unwrap()
            .unwrap();
        assert_eq!(a, (1, serde_json::json!({"owner": "a"})));
        assert_eq!(b, (7, serde_json::json!({"owner": "b"})));
    }

    #[test]
    fn a_tenants_snapshot_is_isolated_from_the_systemwide_scope() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let mut store = new_store(dir.path());

        store
            .save_snapshot(
                "shared-agg-2",
                Some("tenant-a"),
                4,
                serde_json::json!({"owner": "tenant-a"}),
            )
            .unwrap();

        // No systemwide row was ever saved for this aggregate id — the
        // tenant-scoped row must not be visible under `None`.
        assert_eq!(store.load_snapshot("shared-agg-2", None).unwrap(), None);
    }

    #[test]
    fn a_commit_time_wal_failure_surfaces_as_an_error_not_silent_success() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let mut store = new_store(dir.path());

        stoolap::test_failpoints::WAL_WRITE_FAIL
            .store(true, std::sync::atomic::Ordering::Release);

        let err = store
            .save_snapshot("agg-3", None, 1, serde_json::json!({"value": "a"}))
            .unwrap_err();

        match err {
            PersistenceError::Internal(message) => {
                assert!(
                    message.contains("failpoint"),
                    "expected the WAL failpoint message, got: {message}"
                );
            }
            other => panic!("expected Internal, got {other:?}"),
        }

        // The failed write must not be visible — no silent partial success.
        stoolap::test_failpoints::WAL_WRITE_FAIL
            .store(false, std::sync::atomic::Ordering::Release);
        assert_eq!(store.load_snapshot("agg-3", None).unwrap(), None);
    }
}
