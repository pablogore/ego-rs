//! A simple persistence facade.
//!
//! This module provides a [`PersistenceFacade`] that wraps concrete
//! [`EventStore`] and [`Snapshot`] implementations behind `Arc<Mutex<dyn ...>>`
//! so both production and test code can supply any backing store.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::Mutex as AsyncMutex;

use ego_domain::persistence::{EventStore, PersistenceError, Snapshot, StoredEvent};
use ego_domain::DomainEvent;

// ---------------------------------------------------------------------------
// Re-exported domain types used by actor.rs
// ---------------------------------------------------------------------------

/// Snapshot data loaded during recovery.
#[derive(Debug)]
pub struct SnapshotData {
    /// The snapshot payload as a JSON value.
    pub data: serde_json::Value,
    /// The aggregate version at which the snapshot was taken.
    pub version: u64,
}

// We re-export `StoredEvent` so that actor.rs only imports from this module.
pub use ego_domain::persistence::StoredEvent as StoredEventAlias;

// ---------------------------------------------------------------------------
// No-op event store / snapshot store (used when no store is provided)
// ---------------------------------------------------------------------------

struct NoopEventStore<E> {
    _phantom: PhantomData<E>,
}

impl<E: DomainEvent> NoopEventStore<E> {
    fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<E: DomainEvent + Clone + Send + Sync + 'static> EventStore<E> for NoopEventStore<E> {
    async fn append(
        &mut self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        Ok(0)
    }

    async fn load(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        Ok(Vec::new())
    }
}

struct NoopSnapshotStore;

impl Snapshot for NoopSnapshotStore {
    fn save_snapshot(
        &mut self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _version: i64,
        _payload: serde_json::Value,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn load_snapshot(
        &self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// PersistenceFacade
// ---------------------------------------------------------------------------

/// A facade for persistence operations.
///
/// Wraps an [`EventStore`] and a [`Snapshot`] implementation behind
/// `Arc<Mutex<dyn ...>>` so that any backing store (in-memory, database,
/// test stub) can be injected at construction time.
///
/// Neither lock is `std::sync::Mutex`, for the same reason as `BoundedMailbox`'s
/// queue and `EntityRegistry`'s map: a backing store that panics mid-call (a
/// malformed adapter, a deserialization panic, a deliberately-injected test
/// failure) must not permanently poison persistence for every other entity
/// sharing this facade.
///
/// The two locks are different types, and the asymmetry is deliberate rather
/// than an oversight. `EventStore`'s storage methods are asynchronous, so its
/// guard has to be held across an `.await` — `parking_lot`'s guard is not `Send`,
/// which would make every future that touches the store non-`Send` and therefore
/// unspawnable. `tokio::sync::Mutex` yields a `Send` guard and, like
/// `parking_lot`, does not poison. `Snapshot` is still synchronous, so its lock
/// has nothing to hold across and stays on the cheaper, non-async one. When that
/// trait becomes asynchronous it converts too; converting it now would add an
/// `.await` to acquire a lock nothing waits on.
///
/// The default constructor (`PersistenceFacade::new()`) creates a no-op
/// facade that accepts writes but never persists anything.  Use
/// [`PersistenceFacade::with_stores`] to supply real stores.
pub struct PersistenceFacade<E> {
    event_store: Arc<AsyncMutex<dyn EventStore<E> + Send>>,
    snapshot_store: Arc<Mutex<dyn Snapshot + Send>>,
    _event: PhantomData<E>,
}

impl<E> std::fmt::Debug for PersistenceFacade<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistenceFacade").finish()
    }
}

impl<E: DomainEvent + Clone + Send + Sync + 'static> PersistenceFacade<E> {
    /// Creates a new no-op facade.
    ///
    /// Writes succeed but are discarded; loads return empty results.
    pub fn new() -> Self {
        Self {
            event_store: Arc::new(AsyncMutex::new(NoopEventStore::new())),
            snapshot_store: Arc::new(Mutex::new(NoopSnapshotStore)),
            _event: PhantomData,
        }
    }

    /// Creates a facade backed by the supplied event and snapshot stores.
    pub fn with_stores(
        event_store: Arc<AsyncMutex<dyn EventStore<E> + Send>>,
        snapshot_store: Arc<Mutex<dyn Snapshot + Send>>,
    ) -> Self {
        Self {
            event_store,
            snapshot_store,
            _event: PhantomData,
        }
    }

    /// Loads snapshot and events for recovery.
    ///
    /// `aggregate_type` and `aggregate_id` are the structural identity
    /// components the event store now requires; the snapshot store still
    /// keys on the single joined string, since only the event stream's
    /// identity is split in this change.
    ///
    /// Returns `(Option<SnapshotData>, Vec<StoredEventRow<E>>)` where
    /// `StoredEventRow` is the local wrapper holding version metadata.
    pub async fn load_for_recovery(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(Option<SnapshotData>, Vec<StoredEventRow<E>>), String> {
        let snapshot_key = format!("{aggregate_type}-{aggregate_id}");
        let snap = {
            let store = self.snapshot_store.lock();
            store
                .load_snapshot(&snapshot_key, tenant_id)
                .map_err(|e| e.to_string())?
        };

        let snap_data = snap.map(|(version, data)| SnapshotData {
            data,
            version: version as u64,
        });

        let snap_version = snap_data.as_ref().map(|s| s.version).unwrap_or(0);

        let (stored, stored_base): (Vec<StoredEvent<E>>, u64) = {
            let store = self.event_store.lock().await;
            // TODO: EventStore::load returns the full stream; events before snap_version are loaded
            // then discarded. Adding load_from_version(since: u64) to the trait would allow stores
            // to skip pre-snapshot events server-side and avoid the O(N) full load.
            let events = store
                .load(aggregate_type, aggregate_id, tenant_id)
                .await
                .map_err(|e| e.to_string())?;
            let base = store.stream_version_offset(aggregate_type, aggregate_id, tenant_id);
            (events, base)
        };

        // Physical index `idx` maps to logical version `idx + stored_base`; events
        // with logical version < snap_version are already covered by the snapshot.
        let rows: Vec<StoredEventRow<E>> = stored
            .into_iter()
            .enumerate()
            .filter(|(idx, _)| (*idx as u64 + stored_base) >= snap_version)
            .map(|(idx, se)| StoredEventRow {
                event: se.event,
                version: idx as u64 + stored_base + 1,
            })
            .collect();

        Ok((snap_data, rows))
    }

    /// Persists a batch of events.
    ///
    /// Returns the new aggregate version.
    pub async fn persist_events(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: u64,
        events: &[E],
    ) -> Result<u64, String> {
        let stored: Vec<StoredEvent<E>> = events
            .iter()
            .cloned()
            .map(StoredEvent::without_correlation)
            .collect();

        let new_version = {
            let mut store = self.event_store.lock().await;
            store
                .append(
                    aggregate_type,
                    aggregate_id,
                    tenant_id,
                    version as i64,
                    stored,
                )
                .await
                .map_err(|e| e.to_string())?
        };

        Ok(new_version as u64)
    }

    /// Stores a snapshot.
    pub async fn store_snapshot(
        &self,
        entity_id: &str,
        tenant_id: Option<&str>,
        version: u64,
        data: &serde_json::Value,
    ) -> Result<(), String> {
        let mut store = self.snapshot_store.lock();
        store
            .save_snapshot(entity_id, tenant_id, version as i64, data.clone())
            .map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// StoredEventRow — local wrapper used by actor.rs
// ---------------------------------------------------------------------------

/// A stored event with version metadata, as returned by [`PersistenceFacade`].
#[derive(Debug)]
pub struct StoredEventRow<E> {
    /// The event payload.
    pub event: E,
    /// The version number of this event in the stream.
    pub version: u64,
}

// ---------------------------------------------------------------------------
// InMemory stores (re-export for builder/tests convenience)
// ---------------------------------------------------------------------------

/// The in-memory store's stream key: `(aggregate_type, aggregate_id)`, the
/// same split identity the `EventStore` trait now requires — never a joined
/// string, so two streams can never be confused by this store even if their
/// components would join to the same text.
type StreamKey = (String, String);

/// In-memory event store backed by a `HashMap`.
///
/// Suitable for tests and development. Enforces optimistic concurrency via
/// version checks on `append`.
///
/// # Tenant isolation
///
/// This implementation does **not** scope streams by `tenant_id`.  All
/// tenants share the same `(aggregate_type, aggregate_id)`-keyed namespace.
/// This is intentional for testing purposes where single-tenant behaviour is
/// the default.  Production deployments that require tenant isolation must
/// supply an implementation that incorporates `tenant_id` into the stream
/// key.
pub struct InMemoryEventStore<E> {
    streams: HashMap<StreamKey, Vec<StoredEvent<E>>>,
    /// Per-stream version offset — simulates events already covered by a snapshot.
    version_offsets: HashMap<StreamKey, i64>,
}

impl<E> InMemoryEventStore<E> {
    /// Creates an empty in-memory event store.
    pub fn new() -> Self {
        InMemoryEventStore {
            streams: HashMap::new(),
            version_offsets: HashMap::new(),
        }
    }

    /// Declares that `offset` events were already persisted for
    /// `(aggregate_type, aggregate_id)` before this store was created (e.g.
    /// covered by a pre-seeded snapshot). The store treats those events as
    /// implicitly present for version-check purposes without requiring dummy
    /// event payloads to be added.
    pub fn with_version_offset(
        mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        offset: i64,
    ) -> Self {
        self.version_offsets.insert(
            (aggregate_type.to_string(), aggregate_id.to_string()),
            offset,
        );
        self
    }
}

#[async_trait]
impl<E: DomainEvent + Clone + Send + Sync + 'static> EventStore<E> for InMemoryEventStore<E> {
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        _tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let key = (aggregate_type.to_string(), aggregate_id.to_string());
        let offset = self.version_offsets.get(&key).copied().unwrap_or(0);
        let stream = self.streams.entry(key).or_default();

        let current_version = stream.len() as i64 + offset;
        if current_version != expected_version {
            return Err(PersistenceError::Conflict {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                expected: expected_version,
                actual: current_version,
            });
        }

        for event in events {
            stream.push(event);
        }

        Ok(stream.len() as i64 + offset)
    }

    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let key = (aggregate_type.to_string(), aggregate_id.to_string());
        Ok(self.streams.get(&key).cloned().unwrap_or_default())
    }

    async fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        Ok(self.streams.keys().cloned().collect())
    }

    fn stream_version_offset(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> u64 {
        let key = (aggregate_type.to_string(), aggregate_id.to_string());
        self.version_offsets.get(&key).copied().unwrap_or(0).max(0) as u64
    }
}

/// In-memory snapshot store backed by a `HashMap`.
///
/// Stores the latest snapshot per aggregate.
pub struct InMemorySnapshotStore {
    snapshots: HashMap<String, (i64, serde_json::Value)>,
}

impl InMemorySnapshotStore {
    /// Creates an empty in-memory snapshot store.
    pub fn new() -> Self {
        InMemorySnapshotStore {
            snapshots: HashMap::new(),
        }
    }
}

impl Snapshot for InMemorySnapshotStore {
    fn save_snapshot(
        &mut self,
        stream_id: &str,
        _tenant_id: Option<&str>,
        version: i64,
        payload: serde_json::Value,
    ) -> Result<(), PersistenceError> {
        self.snapshots
            .insert(stream_id.to_string(), (version, payload));
        Ok(())
    }

    fn load_snapshot(
        &self,
        stream_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError> {
        Ok(self.snapshots.get(stream_id).cloned())
    }
}
