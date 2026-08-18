//! A simple persistence facade.
//!
//! This module provides a [`PersistenceFacade`] that holds concrete
//! [`EventStore`] and [`Snapshot`] implementations behind trait objects, so both
//! production and test code can supply any backing store. The event store is held
//! as `Arc<dyn EventStore<..> + Send + Sync>`; the snapshot store still needs a
//! lock, and the facade's own documentation says why.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use ego_domain::operation::OperationReceipt;
use ego_domain::persistence::resolve_tenant;
use ego_domain::persistence::{
    EventStore, EventStoreUnitOfWork, PersistenceError, Snapshot, StoredEvent,
};
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
        &self,
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

    /// A unit of work that discards everything, which is this store's whole
    /// contract: it accepts writes and persists nothing. Returning an error
    /// instead would make the no-op facade unusable by any caller that opens a
    /// unit of work, and the no-op facade exists precisely so such a caller can
    /// run without a configured store.
    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError> {
        Ok(Box::new(DiscardingUnitOfWork {
            _phantom: PhantomData,
        }))
    }

    /// Always `None`, and that is the honest answer rather than a limitation.
    ///
    /// This facade retains nothing, so no operation has ever completed *here*.
    /// Reporting a miss is what keeps the no-op store from pretending: a caller
    /// that confirms a receipt and then looks it up sees it absent, which is
    /// exactly what "discarded" means made observable.
    async fn find_receipt(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        Ok(None)
    }
}

/// The [`NoopEventStore`]'s unit of work. Accepts appends, reports the versions
/// they would have produced, and persists nothing on commit.
struct DiscardingUnitOfWork<E> {
    _phantom: PhantomData<E>,
}

#[async_trait]
impl<E: DomainEvent + Clone + Send + Sync + 'static> EventStoreUnitOfWork<E>
    for DiscardingUnitOfWork<E>
{
    async fn append(
        &mut self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        Ok(expected_version + events.len() as i64)
    }

    /// Accepts the receipt and discards it, exactly as this unit of work
    /// discards appends.
    ///
    /// It answers `Ok` rather than erroring so a caller running without a
    /// configured store is not broken by the very method that records success —
    /// the same reason `append` answers `Ok`. It is **not** a durability claim,
    /// and nothing here is retained: [`NoopEventStore::find_receipt`] reports
    /// every lookup as a miss, so a caller can observe the discard rather than
    /// having to trust this comment. Any caller that needs the receipt to
    /// survive must configure a real store; this facade exists for the callers
    /// that do not.
    async fn confirm_receipt(
        &mut self,
        _receipt: &OperationReceipt,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
        Ok(())
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
/// Holds an [`EventStore`] and a [`Snapshot`] implementation behind trait objects,
/// so that any backing store (in-memory, database, test stub) can be injected at
/// construction time. The two are held differently, and the next two paragraphs are
/// the reason.
///
/// The event store is held **without** a lock. It used to sit behind one only
/// because `EventStore::append` demanded `&mut self`, and the lock existed to
/// produce that exclusive borrow — which then serialised every append in the
/// process, holding the lock across a full database round trip. Narrowing `append`
/// to `&self` removed the reason, so the lock went with it: a store that reaches
/// its own state through a pool handle or an interior lock does not need a second
/// one wrapped around it.
///
/// The snapshot store still has one — the only lock this facade holds — because
/// `Snapshot` is still synchronous and still takes `&mut self`, so a shared facade
/// has no other way to produce the exclusive borrow it asks for.
///
/// That lock is `parking_lot::Mutex`, not `std::sync::Mutex`, for the same reason as
/// `BoundedMailbox`'s queue and `EntityRegistry`'s map: it does not poison, and a
/// backing store that panics mid-call (a malformed adapter, a deserialization panic,
/// a deliberately-injected test failure) must not permanently poison persistence for
/// every other entity sharing this facade. When `Snapshot` narrows the way
/// `EventStore` just did, this lock goes too.
///
/// The default constructor (`PersistenceFacade::new()`) creates a no-op
/// facade that accepts writes but never persists anything.  Use
/// [`PersistenceFacade::with_stores`] to supply real stores.
pub struct PersistenceFacade<E> {
    event_store: Arc<dyn EventStore<E> + Send + Sync>,
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
            event_store: Arc::new(NoopEventStore::new()),
            snapshot_store: Arc::new(Mutex::new(NoopSnapshotStore)),
            _event: PhantomData,
        }
    }

    /// Creates a facade backed by the supplied event and snapshot stores.
    pub fn with_stores(
        event_store: Arc<dyn EventStore<E> + Send + Sync>,
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
            // TODO: EventStore::load returns the full stream; events before snap_version are loaded
            // then discarded. Adding load_from_version(since: u64) to the trait would allow stores
            // to skip pre-snapshot events server-side and avoid the O(N) full load.
            // An aggregate with no events yet is the ordinary first state of
            // every entity, not a failure. Recovery absorbs `NotFound` as "no
            // history" whatever store reported it, and does so unconditionally:
            // propagating it meant no entity could be activated for the first
            // time against any store that reports absence.
            //
            // Every real implementation of the port does report it that way. The
            // no-op store instead returns an empty stream, and that is not a
            // divergence — it persists nothing, so an empty stream is the truth
            // about it rather than a claim that a stream is missing. Both shapes
            // mean the same thing here, which is why this handles both rather
            // than requiring one.
            //
            // Only `NotFound` is absorbed. A connection failure or a
            // deserialization error still fails recovery, because those mean the
            // history could not be *read* — not that there is none. Recovering an
            // unreadable stream as a fresh entity would append from version zero
            // over history it never saw.
            let events = match self
                .event_store
                .load(aggregate_type, aggregate_id, tenant_id)
                .await
            {
                Ok(events) => events,
                Err(PersistenceError::NotFound { .. }) => Vec::new(),
                Err(other) => return Err(other.to_string()),
            };
            let base =
                self.event_store
                    .stream_version_offset(aggregate_type, aggregate_id, tenant_id);
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
        let stored: Vec<StoredEvent<E>> = events.iter().cloned().map(StoredEvent::new).collect();

        let new_version = {
            self.event_store
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

    /// Persists a batch of events **and** the receipt recording that the
    /// operation ran, as one transaction.
    ///
    /// `events` may be empty. A success that produced nothing still writes a
    /// receipt, and still writes it through a real unit of work: that case has
    /// no event in the stream to carry its completion, so without the receipt it
    /// is indistinguishable from a command that never ran.
    ///
    /// # Why this is separate from [`persist_events`](Self::persist_events)
    ///
    /// `persist_events` uses the direct append path, which owns and commits its
    /// own transaction — nothing can be made to land atomically *with* it. A
    /// receipt written afterwards could survive a rollback of the events it
    /// describes, or be lost while they survive; either way the store would
    /// disagree with itself about whether the operation happened.
    ///
    /// The two paths are kept apart rather than merged because a command with no
    /// idempotency identity has no receipt to write, and must keep the behaviour
    /// it has today. Routing it through a unit of work would change which store
    /// method it depends on for no benefit.
    ///
    /// # Ordering
    ///
    /// Append, then confirm, then commit — once. A commit placed before the
    /// confirmation would make the events durable while the receipt is still
    /// staged, which is the split this method exists to prevent. A confirmation
    /// error aborts before the commit, so neither becomes visible.
    pub async fn persist_events_with_receipt(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: u64,
        events: &[E],
        receipt: &OperationReceipt,
    ) -> Result<u64, String> {
        let stored: Vec<StoredEvent<E>> = events.iter().cloned().map(StoredEvent::new).collect();

        let mut uow = self.event_store.begin().await.map_err(|e| e.to_string())?;

        let new_version = if stored.is_empty() {
            version as i64
        } else {
            uow.append(
                aggregate_type,
                aggregate_id,
                tenant_id,
                version as i64,
                stored,
            )
            .await
            .map_err(|e| e.to_string())?
        };

        // Before the commit, deliberately. Dropping `uow` on this error path is
        // the rollback: nothing staged above becomes visible.
        uow.confirm_receipt(receipt)
            .await
            .map_err(|e| e.to_string())?;

        uow.commit().await.map_err(|e| e.to_string())?;

        Ok(new_version as u64)
    }

    /// Looks up the receipt for one operation against one aggregate.
    ///
    /// A minimal delegation to [`EventStore::find_receipt`], and deliberately
    /// nothing more. It does not expose the store, and it has no fallback: a
    /// read error is returned as an error, never softened into "no receipt".
    /// "No receipt" means *run the command*, so a facade that swallowed a read
    /// failure would re-execute an operation that already completed — the exact
    /// duplicate the receipt exists to prevent.
    pub async fn find_receipt(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        self.event_store
            .find_receipt(aggregate_type, aggregate_id, tenant_id, operation_key)
            .await
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

/// The in-memory store's stream key: `(aggregate_type, aggregate_id, tenant)`, the
/// same split identity the `EventStore` trait requires — never a joined string, so
/// two streams can never be confused by this store even if their components would
/// join to the same text.
///
/// The tenant is `Option<String>`, and carrying the `Option` in the key rather
/// than flattening it is what makes the tenant-less scope its own partition: in a
/// keyed collection `None == None`, so two systemwide streams find each other,
/// while `None` never equals `Some(_)`. That is the same semantics the durable
/// store expresses in SQL with `IS NOT DISTINCT FROM`.
type StreamKey = (String, String, Option<String>);

/// The logical identity of a receipt: aggregate scope plus operation key.
///
/// Four components, not two — the same operation key against two different
/// aggregates is two operations.
type ReceiptKey = (String, String, Option<String>, String);

/// Everything a committed unit of work makes visible, behind **one** lock.
///
/// Streams and receipts share a lock rather than holding one each, because
/// `commit` must publish as a single step. A reader that could see the events
/// without the receipt would find no evidence the operation ran, and would be
/// free to run it again. Two locks taken in sequence leave exactly that window
/// open between them. The durable store inherits this from its transaction;
/// here it has to be constructed, and the way to make it impossible to get wrong
/// is to leave a reader nothing to acquire halfway.
struct Committed<E> {
    streams: HashMap<StreamKey, Vec<StoredEvent<E>>>,
    receipts: HashMap<ReceiptKey, OperationReceipt>,
}

impl<E> Default for Committed<E> {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
            receipts: HashMap::new(),
        }
    }
}

/// Builds the lookup key for a receipt.
fn receipt_key(receipt: &OperationReceipt) -> ReceiptKey {
    (
        receipt.aggregate_type().to_string(),
        receipt.aggregate_id().to_string(),
        receipt.tenant().map(|t| t.as_str().to_string()),
        receipt.operation_key().as_str().to_string(),
    )
}

/// Refuses a receipt that would replace an existing one under a different
/// fingerprint. A matching fingerprint is an ordinary, idempotent retry.
fn reconcile_receipt(
    existing: Option<&OperationReceipt>,
    incoming: &OperationReceipt,
) -> Result<(), PersistenceError> {
    match existing {
        Some(found) if found.fingerprint() != incoming.fingerprint() => {
            Err(PersistenceError::Conflict {
                aggregate_id: format!("{}-{}", incoming.aggregate_type(), incoming.aggregate_id()),
                expected: 0,
                actual: 0,
            })
        }
        _ => Ok(()),
    }
}

/// The key for a declared version offset: `(aggregate_type, aggregate_id)`.
///
/// Deliberately narrower than [`StreamKey`]. `with_version_offset` takes no tenant
/// argument, so an offset declared for a stream identity applies in every tenant
/// partition of it. That is the granularity the API offers, stated rather than
/// implied — a test that needs per-tenant offsets would need the setter to grow a
/// parameter first.
type OffsetKey = (String, String);

/// In-memory event store backed by a `HashMap`.
///
/// Suitable for tests and development. Enforces optimistic concurrency via
/// version checks on `append`.
///
/// # Tenant isolation
///
/// Streams are scoped by tenant, and this is not a convenience — it is what keeps
/// this store from disagreeing with the durable one. It had previously shared one
/// `(aggregate_type, aggregate_id)` namespace across every tenant, so two streams
/// with the same type and id in different tenants collided into one. Since this is
/// also the store the runtime builder installs when none is supplied, the
/// divergence was reachable without anyone choosing it.
///
/// The tenant argument goes through the shared
/// [`resolve_tenant`](ego_domain::persistence::resolve_tenant), so an empty-string
/// tenant fails closed here exactly as it does in the durable store rather than
/// being filed into the systemwide partition.
pub struct InMemoryEventStore<E> {
    /// Behind a shared lock rather than owned outright, so a unit of work handed
    /// out by [`EventStore::begin`] can publish into the same store after the
    /// borrow that created it has ended.
    ///
    /// `parking_lot::Mutex`, like the facade's snapshot lock: it does not poison,
    /// so a caller that panics mid-append cannot make the store permanently
    /// unusable for everyone sharing it. Its guard is never held across an
    /// `.await` — only for the duration of a map operation — so it never needs to
    /// be `Send`.
    state: Arc<Mutex<Committed<E>>>,
    /// Per-stream version offset — simulates events already covered by a snapshot.
    version_offsets: HashMap<OffsetKey, i64>,
}

impl<E> InMemoryEventStore<E> {
    /// Creates an empty in-memory event store.
    pub fn new() -> Self {
        InMemoryEventStore {
            state: Arc::new(Mutex::new(Committed::default())),
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
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let offset_key = (aggregate_type.to_string(), aggregate_id.to_string());
        let offset = self.version_offsets.get(&offset_key).copied().unwrap_or(0);
        let key = (aggregate_type.to_string(), aggregate_id.to_string(), tenant);
        let mut state = self.state.lock();
        let streams = &mut state.streams;
        let stream = streams.entry(key).or_default();

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

    /// Reports a stream this store has never seen as absent, matching the durable
    /// store rather than returning an empty list.
    ///
    /// It used to return an empty stream, and that difference hid a defect for as
    /// long as it existed: recovery propagated the durable store's `NotFound`, so
    /// no entity could be activated for the first time against PostgreSQL, while
    /// every recovery test used this store and never saw it. Recovery now absorbs
    /// absence explicitly, which is what lets this store answer honestly.
    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_type.to_string(), aggregate_id.to_string(), tenant);
        match self.state.lock().streams.get(&key) {
            Some(events) => Ok(events.clone()),
            None => Err(PersistenceError::NotFound {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
            }),
        }
    }

    async fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let mut ids: Vec<(String, String)> = self
            .state
            .lock()
            .streams
            .keys()
            .filter(|(_, _, t)| *t == tenant)
            .map(|(atype, aid, _)| (atype.clone(), aid.clone()))
            .collect();
        ids.sort();
        Ok(ids)
    }

    async fn find_receipt(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (
            aggregate_type.to_string(),
            aggregate_id.to_string(),
            tenant,
            operation_key.to_string(),
        );
        Ok(self.state.lock().receipts.get(&key).cloned())
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError> {
        Ok(Box::new(StagingUnitOfWork {
            state: Arc::clone(&self.state),
            staged_receipts: HashMap::new(),
            // Cloned rather than shared, and exact: offsets are declared through
            // `with_version_offset`, a builder that consumes `self`, so they are
            // fixed before the store can be used and cannot change while a unit
            // of work is open.
            version_offsets: self.version_offsets.clone(),
            staged: HashMap::new(),
        }))
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

/// [`InMemoryEventStore`]'s unit of work.
///
/// Appends accumulate in `staged` and reach the shared streams only on commit, so
/// dropping this without committing discards them — the same observable outcome
/// as abandoning a database transaction, reached by staging rather than by
/// rolling back.
///
/// The version check counts committed events *and* what this unit of work has
/// already staged, which is what lets two appends to one stream advance instead
/// of colliding. Getting that wrong is how the in-memory store would come to
/// disagree with the durable one about a case the shared conformance harness
/// covers.
///
/// Version offsets are part of that arithmetic, not an exception to it. They exist
/// so a test can pretend a snapshot already covers earlier events, and
/// [`EventStore::append`] adds them to the stream length when deciding whether an
/// expected version matches. A unit of work that left them out would reject an
/// append the direct path accepts, on the same stream, with the same argument —
/// so the version here is `offset + committed + staged`.
struct StagingUnitOfWork<E> {
    state: Arc<Mutex<Committed<E>>>,
    version_offsets: HashMap<OffsetKey, i64>,
    staged: HashMap<StreamKey, Vec<StoredEvent<E>>>,
    /// Receipts confirmed here but not yet published. Staged, never written
    /// through, so dropping this unit of work discards them exactly as it
    /// discards appends.
    staged_receipts: HashMap<ReceiptKey, OperationReceipt>,
}

#[async_trait]
impl<E: DomainEvent + Clone + Send + Sync + 'static> EventStoreUnitOfWork<E>
    for StagingUnitOfWork<E>
{
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let offset_key = (aggregate_type.to_string(), aggregate_id.to_string());
        let key = (aggregate_type.to_string(), aggregate_id.to_string(), tenant);

        let offset = self.version_offsets.get(&offset_key).copied().unwrap_or(0);
        let committed = self.state.lock().streams.get(&key).map_or(0, Vec::len) as i64;
        let staged = self.staged.get(&key).map_or(0, Vec::len) as i64;
        let current = offset + committed + staged;

        if current != expected_version {
            return Err(PersistenceError::Conflict {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                expected: expected_version,
                actual: current,
            });
        }

        let count = events.len() as i64;
        self.staged.entry(key).or_default().extend(events);
        Ok(current + count)
    }

    async fn confirm_receipt(
        &mut self,
        receipt: &OperationReceipt,
    ) -> Result<(), PersistenceError> {
        let key = receipt_key(receipt);
        reconcile_receipt(self.state.lock().receipts.get(&key), receipt)?;
        reconcile_receipt(self.staged_receipts.get(&key), receipt)?;
        self.staged_receipts.insert(key, receipt.clone());
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
        // One acquisition, both maps, then release. An earlier version took the
        // two locks in sequence and released the first before taking the second,
        // leaving a window where another task could observe the events without
        // the receipt saying they already happened — and act on it by running the
        // operation again.
        let mut state = self.state.lock();
        for (key, events) in self.staged {
            state.streams.entry(key).or_default().extend(events);
        }
        for (key, receipt) in self.staged_receipts {
            state.receipts.insert(key, receipt);
        }
        Ok(())
    }
}
