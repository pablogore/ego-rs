//! A simple persistence facade.
//!
//! This module provides a [`PersistenceFacade`] that wraps concrete
//! [`EventStore`] and [`Snapshot`] implementations behind `Arc<Mutex<dyn ...>>`
//! so both production and test code can supply any backing store.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use ego_domain::persistence::{EventStore, PersistenceError, Snapshot, StoredEvent};
use ego_domain::DomainEvent;

// ---------------------------------------------------------------------------
// Re-exported domain types used by actor.rs
// ---------------------------------------------------------------------------

/// Snapshot data loaded during recovery.
#[derive(Debug)]
pub struct SnapshotData {
    /// The snapshot payload as raw bytes (JSON-serialised state).
    pub data: Vec<u8>,
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

impl<E: DomainEvent + Clone + Send + Sync + 'static> EventStore<E> for NoopEventStore<E> {
    fn append(
        &mut self,
        _aggregate_id: &str,
        tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        Ok(0)
    }

    fn load(
        &self,
        _aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        Ok(Vec::new())
    }

    fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

struct NoopSnapshotStore;

impl Snapshot for NoopSnapshotStore {
    fn save_snapshot(
        &mut self,
        _aggregate_id: &str,
        tenant_id: Option<&str>,
        _version: i64,
        _payload: serde_json::Value,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn load_snapshot(
        &self,
        _aggregate_id: &str,
        tenant_id: Option<&str>,
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
/// The default constructor (`PersistenceFacade::new()`) creates a no-op
/// facade that accepts writes but never persists anything.  Use
/// [`PersistenceFacade::with_stores`] to supply real stores.
pub struct PersistenceFacade<E> {
    event_store: Arc<Mutex<dyn EventStore<E> + Send>>,
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
            event_store: Arc::new(Mutex::new(NoopEventStore::new())),
            snapshot_store: Arc::new(Mutex::new(NoopSnapshotStore)),
            _event: PhantomData,
        }
    }

    /// Creates a facade backed by the supplied event and snapshot stores.
    pub fn with_stores(
        event_store: Arc<Mutex<dyn EventStore<E> + Send>>,
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
    /// Returns `(Option<SnapshotData>, Vec<StoredEventRow<E>>)` where
    /// `StoredEventRow` is the local wrapper holding version metadata.
    pub async fn load_for_recovery(
        &self,
        entity_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(Option<SnapshotData>, Vec<StoredEventRow<E>>), String> {
        let snap = {
            let store = self.snapshot_store.lock().unwrap();
            store
                .load_snapshot(entity_id, tenant_id)
                .map_err(|e| e.to_string())?
        };

        let snap_data = snap.map(|(version, data)| SnapshotData {
            data: serde_json::to_vec(&data).unwrap_or_default(),
            version: version as u64,
        });

        let snap_version = snap_data.as_ref().map(|s| s.version).unwrap_or(0);

        let stored: Vec<StoredEvent<E>> = {
            let store = self.event_store.lock().unwrap();
            store
                .load(entity_id, tenant_id)
                .unwrap_or_default()
        };

        // Skip events that are part of the snapshot (already applied).
        let rows: Vec<StoredEventRow<E>> = stored
            .into_iter()
            .enumerate()
            .filter(|(idx, _)| (*idx as u64) >= snap_version)
            .map(|(idx, se)| StoredEventRow {
                event: se.event,
                version: idx as u64 + 1,
            })
            .collect();

        Ok((snap_data, rows))
    }

    /// Persists a batch of events.
    ///
    /// Returns the new aggregate version.
    pub async fn persist_events(
        &self,
        entity_id: &str,
        tenant_id: Option<&str>,
        version: u64,
        events: Vec<E>,
    ) -> Result<u64, String> {
        let stored: Vec<StoredEvent<E>> = events
            .into_iter()
            .map(StoredEvent::without_correlation)
            .collect();

        let new_version = {
            let mut store = self.event_store.lock().unwrap();
            store
                .append(entity_id, tenant_id, version as i64, stored)
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
        let mut store = self.snapshot_store.lock().unwrap();
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

/// In-memory event store backed by a `HashMap`.
///
/// Suitable for tests and development. Enforces optimistic concurrency via
/// version checks on `append`.
///
/// # Tenant isolation
///
/// This implementation does **not** scope streams by `tenant_id`.  All
/// tenants share the same `aggregate_id`-keyed namespace.  This is
/// intentional for testing purposes where single-tenant behaviour is the
/// default.  Production deployments that require tenant isolation must
/// supply an implementation that incorporates `tenant_id` into the stream
/// key (e.g. `"{tenant}/{aggregate_id}"`).
pub struct InMemoryEventStore<E> {
    streams: HashMap<String, Vec<StoredEvent<E>>>,
}

impl<E> InMemoryEventStore<E> {
    /// Creates an empty in-memory event store.
    pub fn new() -> Self {
        InMemoryEventStore {
            streams: HashMap::new(),
        }
    }
}

impl<E: DomainEvent + Clone + Send + Sync + 'static> EventStore<E> for InMemoryEventStore<E> {
    fn append(
        &mut self,
        stream_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let stream = self
            .streams
            .entry(stream_id.to_string())
            .or_default();

        if stream.len() as i64 != expected_version {
            return Err(PersistenceError::Conflict {
                aggregate_id: stream_id.to_string(),
                expected: expected_version,
                actual: stream.len() as i64,
            });
        }

        for event in events {
            stream.push(event);
        }

        Ok(stream.len() as i64)
    }

    fn load(
        &self,
        stream_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        Ok(self
            .streams
            .get(stream_id)
            .cloned()
            .unwrap_or_default())
    }

    fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(self.streams.keys().cloned().collect())
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
        tenant_id: Option<&str>,
        version: i64,
        payload: serde_json::Value,
    ) -> Result<(), PersistenceError> {
        self.snapshots.insert(stream_id.to_string(), (version, payload));
        Ok(())
    }

    fn load_snapshot(
        &self,
        stream_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError> {
        Ok(self.snapshots.get(stream_id).cloned())
    }
}
