//! Infrastructure glue between `RegisterUser`'s write side and CORE-005's
//! real read-side engine: a shared handle to
//! `ego-infrastructure`'s in-memory `ReadSideStore` adapter, plus the
//! `OffsetStore`/`DedupStore` implementations CORE-005 requires that this
//! workspace does not otherwise provide an in-memory reference for.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ego_domain::read_side::dedup::{DedupStore, DedupStoreError};
use ego_domain::read_side::event_stream::EventStreamElement;
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::offset::{Offset, OffsetStore, OffsetStoreError};
use ego_domain::read_side::store::{ReadSideStore, ReadSideStoreError};
use ego_infrastructure::persistence::in_memory::{paginate, InMemoryReadSideStore};
use serde_json::Value;

/// Shared, cloneable handle to a single `InMemoryReadSideStore` instance.
///
/// `ReadSideStore::fetch` takes `&self` (no interior mutation), but the
/// write side (`ReadSideSink`) needs to insert into the *same* store the
/// scheduler reads from — a plain `Arc<Mutex<_>>` newtype is the smallest
/// way to share one instance across both sides without hand-rolling a new
/// SPI (the underlying `ego_domain::read_side::store::ReadSideStore` and
/// `ego_infrastructure::persistence::in_memory::InMemoryReadSideStore`
/// types are both foreign to this crate, so `Arc<Mutex<_>>` cannot
/// implement the foreign trait directly here — the orphan rule requires
/// this local wrapper).
#[derive(Clone, Default)]
pub struct SharedReadSideStore(Arc<Mutex<InMemoryReadSideStore>>);

impl SharedReadSideStore {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(InMemoryReadSideStore::new())))
    }

    fn insert(&self, event: EventStreamElement<Value>) {
        self.0.lock().expect("SharedReadSideStore lock poisoned").insert(event);
    }

    /// Every tag currently holding at least one event — used by the poll
    /// loop to discover per-tenant tags dynamically (see `super::spawn`).
    pub fn known_tags(&self) -> Vec<EventTag> {
        self.0.lock().expect("SharedReadSideStore lock poisoned").known_tags()
    }
}

#[async_trait]
impl ReadSideStore<Value> for SharedReadSideStore {
    async fn fetch(&self, tag: &EventTag, offset: Option<&Offset>, batch_size: usize) -> Result<Vec<EventStreamElement<Value>>, ReadSideStoreError> {
        // No `.await` inside the lock scope — `paginate` is a plain
        // synchronous function (never yields), so locking a std `Mutex`
        // across it is never a footgun. Delegates to the same pagination
        // logic `InMemoryReadSideStore::fetch` uses, instead of cloning the
        // full tag history via `all_events` and re-filtering it here.
        let guard = self.0.lock().expect("SharedReadSideStore lock poisoned");
        Ok(paginate(guard.events_for_tag(tag.value()), offset, batch_size))
    }
}

/// Writes real `RegisterUser`-emitted domain events into the shared
/// read-side store, tagged for `UsersByTenant` (CORE-005's `EventTagger`
/// role, inlined here since there is exactly one tag this reference app
/// ever computes).
#[derive(Clone)]
pub struct ReadSideSink {
    store: SharedReadSideStore,
    next_version: Arc<AtomicI64>,
}

impl ReadSideSink {
    pub fn new(store: SharedReadSideStore) -> Self {
        Self { store, next_version: Arc::new(AtomicI64::new(1)) }
    }

    /// Records one domain event as an `EventStreamElement` under a
    /// tenant-scoped tag (`"{PROJECTION_TAG}:{tenant_id}"`) — each tenant
    /// gets its own tag stream, so the underlying store structurally
    /// isolates tenants instead of relying solely on the handler filtering
    /// by `EventStreamElement::tenant_id` after the fact. See
    /// `super::tenant_tag`.
    pub fn record(&self, tenant_id: &str, aggregate_id: &str, event_type: &str, payload: Value, occurred_at: DateTime<Utc>) {
        let version = self.next_version.fetch_add(1, Ordering::SeqCst);
        let event_id = format!("{event_type}:{aggregate_id}:{version}");
        self.store.insert(EventStreamElement::new(
            event_id,
            aggregate_id,
            tenant_id,
            event_type,
            payload,
            version,
            occurred_at,
            vec![super::tenant_tag(tenant_id)],
        ));
    }
}

/// Dedup-state key: `(projection_id, tag, event_id)` (CORE-005's exact
/// dedup scope).
type DedupKey = (String, String, String);
/// Offset key: `(projection_id, tag, tenant)` (CORE-005's exact offset
/// scope).
type OffsetKey = (String, String, String);

/// In-memory `OffsetStore` — this workspace has no other in-memory
/// reference implementation of it.
#[derive(Clone, Default)]
pub struct InMemoryOffsetStore(Arc<Mutex<HashMap<OffsetKey, Offset>>>);

#[async_trait]
impl OffsetStore for InMemoryOffsetStore {
    async fn read_offset(&self, projection_id: &str, tag: &EventTag, tenant: &str) -> Result<Option<Offset>, OffsetStoreError> {
        let key = (projection_id.to_string(), tag.value().to_string(), tenant.to_string());
        Ok(self.0.lock().expect("InMemoryOffsetStore lock poisoned").get(&key).copied())
    }

    async fn write_offset(&self, projection_id: &str, tag: &EventTag, tenant: &str, offset: &Offset) -> Result<(), OffsetStoreError> {
        let key = (projection_id.to_string(), tag.value().to_string(), tenant.to_string());
        self.0.lock().expect("InMemoryOffsetStore lock poisoned").insert(key, *offset);
        Ok(())
    }
}

/// In-memory `DedupStore` — this workspace has no other in-memory
/// reference implementation of it.
#[derive(Clone, Default)]
pub struct InMemoryDedupStore(Arc<Mutex<HashSet<DedupKey>>>);

#[async_trait]
impl DedupStore for InMemoryDedupStore {
    async fn seen(&self, projection_id: &str, tag: &EventTag, event_id: &str) -> Result<bool, DedupStoreError> {
        let key = (projection_id.to_string(), tag.value().to_string(), event_id.to_string());
        Ok(self.0.lock().expect("InMemoryDedupStore lock poisoned").contains(&key))
    }

    async fn mark_seen(&self, projection_id: &str, tag: &EventTag, event_id: &str) -> Result<(), DedupStoreError> {
        let key = (projection_id.to_string(), tag.value().to_string(), event_id.to_string());
        self.0.lock().expect("InMemoryDedupStore lock poisoned").insert(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    // Finding 5 (security hardening): `InMemoryReadSideStore`'s key was
    // narrowed from `(tenant, tag)` (the original CORE-005 shape — see git
    // history at e5b4074) to tag-only, removing store-level tenant
    // isolation. `ReadSideSink::record` now tags each event with a
    // tenant-scoped tag (`tenant_tag`) instead of one shared constant tag,
    // so isolation is structural again — a tag's stream can never contain
    // another tenant's events, even under adversarial conditions (a
    // hypothetical handler that ignored `EventStreamElement::tenant_id`
    // entirely still could not see cross-tenant data, because `fetch` is
    // scoped by tag alone).
    #[tokio::test]
    async fn tenants_are_structurally_isolated_at_the_store_level() {
        let store = SharedReadSideStore::new();
        let sink = ReadSideSink::new(store.clone());

        sink.record("tenant-a", "user-1", "UserRegistered", serde_json::json!({ "email": "a@example.com" }), Utc::now());
        sink.record("tenant-b", "user-2", "UserRegistered", serde_json::json!({ "email": "b@example.com" }), Utc::now());
        sink.record("tenant-a", "user-3", "UserRegistered", serde_json::json!({ "email": "c@example.com" }), Utc::now());

        let tags = store.known_tags();
        assert_eq!(tags.len(), 2, "each tenant gets its own tag stream, not one shared tag");

        for tag in &tags {
            let events = store.fetch(tag, None, 100).await.unwrap();
            assert!(!events.is_empty());
            let tenants: HashSet<&str> = events.iter().map(|e| e.tenant_id()).collect();
            assert_eq!(
                tenants.len(),
                1,
                "a single tag's stream must contain exactly one tenant's events, got: {tenants:?}"
            );
        }
    }
}
