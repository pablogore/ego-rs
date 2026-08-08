use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ego_domain::event::DomainEvent;
use ego_domain::operation::OperationReceipt;
use ego_domain::persistence::resolve_tenant;
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};

type StreamKey = (String, String, Option<String>);

/// The logical identity of a receipt: aggregate scope plus operation key.
///
/// Four components, not two. The same operation key against two different
/// aggregates is two operations, and keying on `(tenant, operation_key)` alone
/// would let one aggregate's completion suppress another's work.
type ReceiptKey = (String, String, Option<String>, String);

/// Refuses a receipt that would replace an existing one carrying a different
/// fingerprint, and reports whether the write is redundant.
///
/// A matching fingerprint is an ordinary retry and is idempotent. A differing
/// one is a *different request* reusing an operation key, and answering it with
/// the stored response would hand one caller another's result — so it is a
/// conflict, never an overwrite.
fn reconcile(
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

/// Builds the lookup key for a receipt.
fn receipt_key(receipt: &OperationReceipt) -> ReceiptKey {
    (
        receipt.aggregate_type().to_string(),
        receipt.aggregate_id().to_string(),
        receipt.tenant().map(|t| t.as_str().to_string()),
        receipt.operation_key().as_str().to_string(),
    )
}

/// In-memory event store.
///
/// Stores events per `(aggregate_type, aggregate_id)` per tenant. Enforces
/// optimistic concurrency.
///
/// The streams live behind a shared lock rather than being owned outright,
/// because a [unit of work](EventStoreUnitOfWork) handed out by
/// [`EventStore::begin`] has to be able to publish into the same store after the
/// borrow that created it has ended. The lock is `std::sync::Mutex` and is only
/// ever held for the duration of a map operation — never across an `.await` — so
/// its guard never needs to be `Send`.
pub struct InMemoryEventStore<E> {
    streams: Arc<Mutex<HashMap<StreamKey, Vec<StoredEvent<E>>>>>,
    /// Committed receipts, shared with every unit of work this store hands out
    /// for the same reason the streams are: a unit of work publishes into the
    /// same store after the borrow that created it has ended.
    receipts: Arc<Mutex<HashMap<ReceiptKey, OperationReceipt>>>,
}

impl<E> InMemoryEventStore<E> {
    pub fn new() -> Self {
        InMemoryEventStore {
            streams: Arc::new(Mutex::new(HashMap::new())),
            receipts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<E> Default for InMemoryEventStore<E> {
    fn default() -> Self {
        Self::new()
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
        let key = (
            aggregate_type.to_string(),
            aggregate_id.to_string(),
            tenant.clone(),
        );

        let mut streams = lock(&self.streams);
        let stream = streams.entry(key).or_default();
        let current = stream.len() as i64;

        if current != expected_version {
            return Err(PersistenceError::Conflict {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                expected: expected_version,
                actual: current,
            });
        }

        let count = events.len() as i64;
        stream.extend(events);
        Ok(current + count)
    }

    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_type.to_string(), aggregate_id.to_string(), tenant);

        match lock(&self.streams).get(&key) {
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
        let mut ids: Vec<(String, String)> = lock(&self.streams)
            .keys()
            .filter(|(_, _, t)| *t == tenant)
            .map(|(atype, aid, _)| (atype.clone(), aid.clone()))
            .collect();
        ids.sort();
        Ok(ids)
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError> {
        Ok(Box::new(InMemoryEventStoreUnitOfWork {
            streams: Arc::clone(&self.streams),
            staged: HashMap::new(),
            receipts: Arc::clone(&self.receipts),
            staged_receipts: HashMap::new(),
        }))
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
        Ok(lock(&self.receipts).get(&key).cloned())
    }
}

/// A unit of work over [`InMemoryEventStore`].
///
/// Appends accumulate in `staged` and are published into the shared streams only
/// on commit, so dropping this without committing discards them — the same
/// observable outcome as abandoning a database transaction, reached by staging
/// rather than by rolling back.
///
/// The version check reads the committed streams *and* whatever this unit of work
/// has already staged, which is what lets two appends to one stream advance
/// instead of colliding. Getting that wrong would make the in-memory store
/// disagree with the durable one about a case the shared conformance harness
/// covers.
struct InMemoryEventStoreUnitOfWork<E> {
    streams: Arc<Mutex<HashMap<StreamKey, Vec<StoredEvent<E>>>>>,
    staged: HashMap<StreamKey, Vec<StoredEvent<E>>>,
    receipts: Arc<Mutex<HashMap<ReceiptKey, OperationReceipt>>>,
    /// Receipts confirmed here but not yet published. Staged rather than written
    /// through, so dropping this unit of work discards them exactly as it
    /// discards appends — the receipt shares the events' fate or it is not a
    /// receipt for them.
    staged_receipts: HashMap<ReceiptKey, OperationReceipt>,
}

#[async_trait]
impl<E: DomainEvent + Clone + Send + Sync + 'static> EventStoreUnitOfWork<E>
    for InMemoryEventStoreUnitOfWork<E>
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
        let key = (aggregate_type.to_string(), aggregate_id.to_string(), tenant);

        let committed = lock(&self.streams).get(&key).map_or(0, Vec::len) as i64;
        let staged = self.staged.get(&key).map_or(0, Vec::len) as i64;
        let current = committed + staged;

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
        reconcile(lock(&self.receipts).get(&key), receipt)?;
        reconcile(self.staged_receipts.get(&key), receipt)?;
        self.staged_receipts.insert(key, receipt.clone());
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
        // Both maps are published under their own locks, taken one at a time and
        // never across an await, so a caller committing on one thread cannot
        // deadlock against a reader on another.
        let mut streams = lock(&self.streams);
        for (key, events) in self.staged {
            streams.entry(key).or_default().extend(events);
        }
        drop(streams);

        let mut receipts = lock(&self.receipts);
        for (key, receipt) in self.staged_receipts {
            receipts.insert(key, receipt);
        }
        Ok(())
    }
}

/// Takes the streams lock, recovering from poisoning rather than propagating it.
///
/// A caller that panicked mid-append must not make the store permanently
/// unusable for every other caller sharing it — the same non-poisoning rationale
/// the persistence facade applies to its own locks. The worst a recovered guard
/// can expose here is a stream that received part of a batch, which is the state
/// the panicking caller left behind either way.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
