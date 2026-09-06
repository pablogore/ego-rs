//! Stoolap-backed [`OffsetStore`].
//!
//! `write_offset` is last-write-wins, matching the port's own contract
//! (`ego_persistence_api::read_side::offset::OffsetStore`): no
//! compare-and-swap, no monotonicity, no ordering enforcement. The write
//! path is a three-step UPDATE-first / INSERT-`ON CONFLICT DO NOTHING` /
//! re-UPDATE sequence rather than a single `INSERT ... ON CONFLICT DO
//! UPDATE`, because the latter shape has never been proven against Stoolap
//! 0.4 anywhere in this repository, whereas the two proven upsert shapes
//! this store uses instead are already in production use in
//! `crate::operation::reservation` (design.md AD-4).
//!
//! # Blocking I/O
//!
//! Every `Database::execute`/`query` call is synchronous, disk-touching
//! I/O. All trait methods therefore run their body via
//! [`StoolapOffsetStore::run_blocking`], which hands the closure to
//! [`tokio::task::spawn_blocking`] — never `block_in_place`, which panics
//! outside a multi-threaded Tokio runtime.
//!
//! # Concurrency scope
//!
//! Safe for concurrent use by multiple async tasks within one process
//! holding this store's underlying file. Not tested, documented, or
//! claimed to be safe across multiple OS processes or multiple nodes (see
//! `read_side` module doc).

use std::fmt;
use std::path::Path;

use async_trait::async_trait;
use stoolap::Database;

use ego_persistence_api::read_side::event_tag::EventTag;
use ego_persistence_api::read_side::offset::{Offset, OffsetStore, OffsetStoreError};

use crate::persistence::stoolap_common::{dsn_declares_sync_full, dsn_for, is_write_conflict};

const CREATE_OFFSETS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS projection_offsets (
    projection_id TEXT    NOT NULL,
    tag           TEXT    NOT NULL,
    tenant        TEXT    NOT NULL,
    offset_value  INTEGER NOT NULL,
    UNIQUE (projection_id, tag, tenant)
)";

/// Maps a raw Stoolap error to the port's `Transient`/`Fatal` split
/// (design.md AD-8): a write conflict is retry-safe, everything else is
/// fatal by default (fail-loud).
fn classify_error(e: stoolap::Error) -> OffsetStoreError {
    if is_write_conflict(&e) {
        OffsetStoreError::Transient(e.to_string())
    } else {
        OffsetStoreError::Fatal(e.to_string())
    }
}

/// A durable, single-process [`OffsetStore`] backed by an embedded Stoolap
/// database.
pub struct StoolapOffsetStore {
    db: Database,
}

impl fmt::Debug for StoolapOffsetStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoolapOffsetStore")
            .field("dsn", &self.db.dsn())
            .finish()
    }
}

impl StoolapOffsetStore {
    /// Opens (creating the `projection_offsets` table if absent) a
    /// Stoolap-backed offset store at `path`.
    ///
    /// Fails closed (design.md AD-9, the same pattern
    /// `StoolapOperationReservationStore::open` already follows): only ever
    /// returns a store whose live engine reports `sync=full`, so
    /// `is_durable()` never outlives-lies about how the store was opened.
    pub async fn open(path: &Path) -> Result<Self, OffsetStoreError> {
        let dsn = dsn_for(path);
        let db = Database::open(&dsn).map_err(|e| OffsetStoreError::Fatal(e.to_string()))?;

        if !dsn_declares_sync_full(db.dsn()) {
            return Err(OffsetStoreError::Fatal(format!(
                "stoolap engine at {:?} is not configured for durable sync (sync=full); \
                 refusing to open a StoolapOffsetStore that would misreport is_durable()",
                db.dsn()
            )));
        }

        // Dialect note (AD-3): a composite `PRIMARY KEY` is parsed but
        // silently NOT enforced by Stoolap 0.4.0 (no constraint, no index) —
        // `UNIQUE` is fully enforced and is what `ON CONFLICT` matches
        // against, so it is used here instead.
        db.execute(CREATE_OFFSETS_TABLE, ())
            .map_err(|e| OffsetStoreError::Fatal(e.to_string()))?;

        Ok(Self { db })
    }

    /// Runs `f` against a cloned `Database` handle on Tokio's
    /// blocking-thread pool — the same shape as
    /// `StoolapOperationReservationStore::run_blocking`.
    async fn run_blocking<F, R>(&self, f: F) -> Result<R, OffsetStoreError>
    where
        F: FnOnce(&Database) -> Result<R, OffsetStoreError> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| OffsetStoreError::Fatal(format!("blocking task panicked: {e}")))?
    }

    #[cfg(test)]
    fn dsn(&self) -> &str {
        self.db.dsn()
    }
}

#[async_trait]
impl OffsetStore for StoolapOffsetStore {
    /// Truthful by construction (design.md AD-9): `open()` only ever
    /// returns a store whose live engine reports `sync=full`, so this
    /// re-derives from the same invariant rather than a hardcoded value.
    fn is_durable(&self) -> bool {
        dsn_declares_sync_full(self.db.dsn())
    }

    async fn read_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
    ) -> Result<Option<Offset>, OffsetStoreError> {
        let projection_id = projection_id.to_string();
        let tag = tag.value().to_string();
        let tenant = tenant.to_string();

        self.run_blocking(move |db| {
            let mut rows = db
                .query(
                    "SELECT offset_value FROM projection_offsets
                     WHERE projection_id = $1 AND tag = $2 AND tenant = $3",
                    (projection_id, tag, tenant),
                )
                .map_err(classify_error)?;

            let row = match rows.next() {
                Some(row) => row.map_err(classify_error)?,
                None => return Ok(None),
            };

            let value: i64 = row.get(0).map_err(classify_error)?;
            Ok(Some(Offset::Sequence(value)))
        })
        .await
    }

    async fn write_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
        offset: &Offset,
    ) -> Result<(), OffsetStoreError> {
        let projection_id = projection_id.to_string();
        let tag = tag.value().to_string();
        let tenant = tenant.to_string();
        let value = offset
            .as_sequence()
            .expect("Offset has exactly one variant, Sequence(i64)");

        self.run_blocking(move |db| {
            // Step 1 (AD-4): steady-state path — the row already exists for
            // every write after the first.
            let affected = db
                .execute(
                    "UPDATE projection_offsets SET offset_value = $1
                     WHERE projection_id = $2 AND tag = $3 AND tenant = $4",
                    (value, projection_id.clone(), tag.clone(), tenant.clone()),
                )
                .map_err(classify_error)?;

            if affected >= 1 {
                return Ok(());
            }

            // Step 2: first write for this key. `ON CONFLICT DO NOTHING`
            // makes two racing first writers resolve without either seeing
            // a unique violation: exactly one inserts, the other falls
            // through to step 3.
            let inserted = db
                .execute(
                    "INSERT INTO projection_offsets (projection_id, tag, tenant, offset_value)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (projection_id, tag, tenant) DO NOTHING",
                    (projection_id.clone(), tag.clone(), tenant.clone(), value),
                )
                .map_err(classify_error)?;

            if inserted == 1 {
                return Ok(());
            }

            // Step 3: a concurrent writer inserted the row in the window
            // between step 1's UPDATE and step 2's INSERT conflict — repeat
            // step 1. Whichever racer's value survives is correct under the
            // port's last-write-wins contract.
            let affected = db
                .execute(
                    "UPDATE projection_offsets SET offset_value = $1
                     WHERE projection_id = $2 AND tag = $3 AND tenant = $4",
                    (value, projection_id, tag, tenant),
                )
                .map_err(classify_error)?;

            if affected == 0 {
                return Err(OffsetStoreError::Transient(
                    "write_offset: row vanished between the fallback insert and the \
                     re-UPDATE; retry"
                        .to_string(),
                ));
            }
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test that touches the database serializes on stoolap's own
    /// failpoint lock, matching `reservation.rs`'s guard: `WAL_WRITE_FAIL`
    /// is a process-wide `AtomicBool`, so an unguarded test here can
    /// observe another module's failpoint test armed mid-run.
    fn db_test_guard() -> stoolap::test_failpoints::FailpointGuard {
        stoolap::test_failpoints::FailpointGuard::new()
    }

    async fn fresh_store() -> (StoolapOffsetStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = StoolapOffsetStore::open(dir.path())
            .await
            .expect("open StoolapOffsetStore");
        (store, dir)
    }

    #[tokio::test]
    async fn an_opened_store_requested_full_sync() {
        let _fp = db_test_guard();
        let (store, dir) = fresh_store().await;
        assert_eq!(store.dsn(), dsn_for(dir.path()));
        assert!(store.is_durable());
    }

    /// AD-9 regression guard, mirroring `StoolapOperationReservationStore`'s
    /// identical test: a live, weakly-configured engine already holding
    /// `path` when `open()` is called must be refused, not silently
    /// accepted.
    #[tokio::test]
    async fn open_refuses_a_path_already_locked_by_a_non_durable_engine() {
        let _fp = db_test_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let weak_dsn = format!("file://{}", dir.path().display());
        let _weak_db = Database::open(&weak_dsn).unwrap();

        let err = StoolapOffsetStore::open(dir.path())
            .await
            .expect_err("expected open() to refuse a path locked by a non-durable engine");
        assert!(matches!(err, OffsetStoreError::Fatal(_)));
    }

    #[tokio::test]
    async fn read_offset_of_a_never_written_key_is_none() {
        let _fp = db_test_guard();
        let (store, _dir) = fresh_store().await;
        let tag = EventTag::new("users-by-tenant");
        assert_eq!(
            store.read_offset("proj", &tag, "tenant-a").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_write_is_isolated_to_its_key() {
        let _fp = db_test_guard();
        let (store, _dir) = fresh_store().await;
        let tag_a = EventTag::new("tag-a");
        let tag_b = EventTag::new("tag-b");

        store
            .write_offset("proj-1", &tag_a, "tenant-1", &Offset::sequence(10))
            .await
            .unwrap();
        store
            .write_offset("proj-1", &tag_b, "tenant-1", &Offset::sequence(20))
            .await
            .unwrap();
        store
            .write_offset("proj-2", &tag_a, "tenant-1", &Offset::sequence(30))
            .await
            .unwrap();
        store
            .write_offset("proj-1", &tag_a, "tenant-2", &Offset::sequence(40))
            .await
            .unwrap();

        assert_eq!(
            store
                .read_offset("proj-1", &tag_a, "tenant-1")
                .await
                .unwrap(),
            Some(Offset::sequence(10))
        );
        assert_eq!(
            store
                .read_offset("proj-1", &tag_b, "tenant-1")
                .await
                .unwrap(),
            Some(Offset::sequence(20))
        );
        assert_eq!(
            store
                .read_offset("proj-2", &tag_a, "tenant-1")
                .await
                .unwrap(),
            Some(Offset::sequence(30))
        );
        assert_eq!(
            store
                .read_offset("proj-1", &tag_a, "tenant-2")
                .await
                .unwrap(),
            Some(Offset::sequence(40))
        );
        // A key never written stays absent even though its siblings exist.
        assert_eq!(
            store
                .read_offset("proj-1", &tag_a, "tenant-3")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_repeat_write_overwrites_without_ordering_enforcement() {
        let _fp = db_test_guard();
        let (store, _dir) = fresh_store().await;
        let tag = EventTag::new("tag");

        store
            .write_offset("proj", &tag, "tenant", &Offset::sequence(5))
            .await
            .unwrap();
        store
            .write_offset("proj", &tag, "tenant", &Offset::sequence(2))
            .await
            .unwrap();

        // Last-write-wins: no compare-and-swap, no monotonicity — a smaller
        // value overwrites a larger one exactly as the trait allows.
        assert_eq!(
            store.read_offset("proj", &tag, "tenant").await.unwrap(),
            Some(Offset::sequence(2))
        );
    }
}
