//! Stoolap-backed [`DedupStore`].
//!
//! Dedup identity is `(projection_id, tag, event_id)` — deliberately **no**
//! tenant column, matching the port's own signature
//! (`ego_persistence_api::read_side::dedup::DedupStore`), which takes no
//! tenant parameter (design.md AD-3). This store never prunes, expires, or
//! evicts a mark: no TTL, no retention, no `seen_at` column — an explicit
//! spec Non-Goal.
//!
//! # Blocking I/O
//!
//! Every `Database::execute`/`query` call is synchronous, disk-touching
//! I/O. All trait methods therefore run their body via
//! [`StoolapDedupStore::run_blocking`], which hands the closure to
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

use ego_persistence_api::read_side::dedup::{DedupStore, DedupStoreError};
use ego_persistence_api::read_side::event_tag::EventTag;

use crate::persistence::stoolap_common::{dsn_declares_sync_full, dsn_for, is_write_conflict};

const CREATE_DEDUP_TABLE: &str = "
CREATE TABLE IF NOT EXISTS projection_dedup (
    projection_id TEXT NOT NULL,
    tag           TEXT NOT NULL,
    event_id      TEXT NOT NULL,
    UNIQUE (projection_id, tag, event_id)
)";

/// Maps a raw Stoolap error to the port's `Transient`/`Fatal` split
/// (design.md AD-8): a write conflict is retry-safe, everything else is
/// fatal by default (fail-loud).
fn classify_error(e: stoolap::Error) -> DedupStoreError {
    if is_write_conflict(&e) {
        DedupStoreError::Transient(e.to_string())
    } else {
        DedupStoreError::Fatal(e.to_string())
    }
}

/// A durable, single-process [`DedupStore`] backed by an embedded Stoolap
/// database.
pub struct StoolapDedupStore {
    db: Database,
}

impl fmt::Debug for StoolapDedupStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoolapDedupStore")
            .field("dsn", &self.db.dsn())
            .finish()
    }
}

impl StoolapDedupStore {
    /// Opens (creating the `projection_dedup` table if absent) a
    /// Stoolap-backed dedup store at `path`.
    ///
    /// Fails closed (design.md AD-9, the same pattern
    /// `StoolapOffsetStore::open` already follows): only ever returns a
    /// store whose live engine reports `sync=full`, so `is_durable()`
    /// never outlives-lies about how the store was opened.
    pub async fn open(path: &Path) -> Result<Self, DedupStoreError> {
        let dsn = dsn_for(path);
        let db = Database::open(&dsn).map_err(|e| DedupStoreError::Fatal(e.to_string()))?;

        if !dsn_declares_sync_full(db.dsn()) {
            return Err(DedupStoreError::Fatal(format!(
                "stoolap engine at {:?} is not configured for durable sync (sync=full); \
                 refusing to open a StoolapDedupStore that would misreport is_durable()",
                db.dsn()
            )));
        }

        // Dialect note (AD-3): a composite `PRIMARY KEY` is parsed but
        // silently NOT enforced by Stoolap 0.4.0 (no constraint, no index) —
        // `UNIQUE` is fully enforced and is what `ON CONFLICT` matches
        // against, so it is used here instead.
        db.execute(CREATE_DEDUP_TABLE, ())
            .map_err(|e| DedupStoreError::Fatal(e.to_string()))?;

        Ok(Self { db })
    }

    /// Runs `f` against a cloned `Database` handle on Tokio's
    /// blocking-thread pool — the same shape as
    /// `StoolapOffsetStore::run_blocking`.
    async fn run_blocking<F, R>(&self, f: F) -> Result<R, DedupStoreError>
    where
        F: FnOnce(&Database) -> Result<R, DedupStoreError> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| DedupStoreError::Fatal(format!("blocking task panicked: {e}")))?
    }

    #[cfg(test)]
    fn dsn(&self) -> &str {
        self.db.dsn()
    }
}

#[async_trait]
impl DedupStore for StoolapDedupStore {
    /// Truthful by construction (design.md AD-9): `open()` only ever
    /// returns a store whose live engine reports `sync=full`, so this
    /// re-derives from the same invariant rather than a hardcoded value.
    fn is_durable(&self) -> bool {
        dsn_declares_sync_full(self.db.dsn())
    }

    async fn seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<bool, DedupStoreError> {
        let projection_id = projection_id.to_string();
        let tag = tag.value().to_string();
        let event_id = event_id.to_string();

        self.run_blocking(move |db| {
            let mut rows = db
                .query(
                    "SELECT 1 FROM projection_dedup
                     WHERE projection_id = $1 AND tag = $2 AND event_id = $3
                     LIMIT 1",
                    (projection_id, tag, event_id),
                )
                .map_err(classify_error)?;

            Ok(rows.next().is_some())
        })
        .await
    }

    async fn mark_seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<(), DedupStoreError> {
        let projection_id = projection_id.to_string();
        let tag = tag.value().to_string();
        let event_id = event_id.to_string();

        self.run_blocking(move |db| {
            // Idempotent by construction (AD-4): a repeat mark affects zero
            // rows and is not an error — there is no mutable payload to
            // re-UPDATE, unlike offset's last-write-wins value.
            db.execute(
                "INSERT INTO projection_dedup (projection_id, tag, event_id)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (projection_id, tag, event_id) DO NOTHING",
                (projection_id, tag, event_id),
            )
            .map_err(classify_error)?;

            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test that touches the database serializes on stoolap's own
    /// failpoint lock, matching `offset.rs`'s guard: `WAL_WRITE_FAIL` is a
    /// process-wide `AtomicBool`, so an unguarded test here can observe
    /// another module's failpoint test armed mid-run.
    fn db_test_guard() -> stoolap::test_failpoints::FailpointGuard {
        stoolap::test_failpoints::FailpointGuard::new()
    }

    async fn fresh_store() -> (StoolapDedupStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = StoolapDedupStore::open(dir.path())
            .await
            .expect("open StoolapDedupStore");
        (store, dir)
    }

    #[tokio::test]
    async fn an_opened_store_requested_full_sync() {
        let _fp = db_test_guard();
        let (store, dir) = fresh_store().await;
        assert_eq!(store.dsn(), dsn_for(dir.path()));
        assert!(store.is_durable());
    }

    /// AD-9 regression guard, mirroring `StoolapOffsetStore`'s identical
    /// test: a live, weakly-configured engine already holding `path` when
    /// `open()` is called must be refused, not silently accepted.
    #[tokio::test]
    async fn open_refuses_a_path_already_locked_by_a_non_durable_engine() {
        let _fp = db_test_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let weak_dsn = format!("file://{}", dir.path().display());
        let _weak_db = Database::open(&weak_dsn).unwrap();

        let err = StoolapDedupStore::open(dir.path())
            .await
            .expect_err("expected open() to refuse a path locked by a non-durable engine");
        assert!(matches!(err, DedupStoreError::Fatal(_)));
    }

    #[tokio::test]
    async fn seen_of_an_unmarked_triple_is_false() {
        let _fp = db_test_guard();
        let (store, _dir) = fresh_store().await;
        let tag = EventTag::new("users-by-tenant");
        assert!(!store.seen("proj", &tag, "evt-1").await.unwrap());
    }

    #[tokio::test]
    async fn mark_seen_is_idempotent() {
        let _fp = db_test_guard();
        let (store, _dir) = fresh_store().await;
        let tag = EventTag::new("users-by-tenant");

        store.mark_seen("proj", &tag, "evt-1").await.unwrap();
        // A repeat mark_seen for the identical triple must succeed without
        // error (idempotent by construction, AD-4).
        store.mark_seen("proj", &tag, "evt-1").await.unwrap();

        assert!(store.seen("proj", &tag, "evt-1").await.unwrap());
    }

    #[tokio::test]
    async fn no_dedup_entry_is_ever_pruned() {
        let _fp = db_test_guard();
        let (store, _dir) = fresh_store().await;
        let tag = EventTag::new("users-by-tenant");

        // No time-based cleanup path exists anywhere in this store: there is
        // no `seen_at` column and no TTL/retention mechanism to invoke, so a
        // mark written arbitrarily long ago is simulated simply by never
        // pruning it — `seen()` must still return `true`.
        store.mark_seen("proj", &tag, "evt-old").await.unwrap();
        assert!(store.seen("proj", &tag, "evt-old").await.unwrap());
    }

    #[tokio::test]
    async fn the_same_event_id_under_a_different_projection_and_tag_is_independent() {
        let _fp = db_test_guard();
        let (store, _dir) = fresh_store().await;
        let tag_a = EventTag::new("tag-a");
        let tag_b = EventTag::new("tag-b");

        store.mark_seen("proj-1", &tag_a, "evt-1").await.unwrap();

        // Isolation across the full key: the identical event_id under a
        // different projection_id or a different tag is a distinct mark.
        assert!(store.seen("proj-1", &tag_a, "evt-1").await.unwrap());
        assert!(!store.seen("proj-2", &tag_a, "evt-1").await.unwrap());
        assert!(!store.seen("proj-1", &tag_b, "evt-1").await.unwrap());
    }
}
