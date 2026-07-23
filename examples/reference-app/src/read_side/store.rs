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
        self.0
            .lock()
            .expect("SharedReadSideStore lock poisoned")
            .insert(event);
    }

    /// Every tag currently holding at least one event — used by the poll
    /// loop to discover per-tenant tags dynamically (see `super::spawn`).
    pub fn known_tags(&self) -> Vec<EventTag> {
        self.0
            .lock()
            .expect("SharedReadSideStore lock poisoned")
            .known_tags()
    }
}

#[async_trait]
impl ReadSideStore<Value> for SharedReadSideStore {
    async fn fetch(
        &self,
        tenant: &str,
        tag: &EventTag,
        offset: Option<&Offset>,
        batch_size: usize,
    ) -> Result<Vec<EventStreamElement<Value>>, ReadSideStoreError> {
        // The explicit `tenant` is authoritative: the scheduler threads each
        // tag's real tenant into this call (see `super::tenant_from_tag` and
        // `super::spawn`), so we paginate strictly by it. Our tags are
        // tenant-scoped (see `super::tenant_tag`), so `tenant_from_tag` is a
        // defense-in-depth cross-check — if the tag decodes to a *different*
        // tenant than the caller asked for, that is a scoping error and we
        // fail closed to empty rather than risk surfacing another tenant's
        // stream.
        if let Some(tag_tenant) = super::tenant_from_tag(tag) {
            if tag_tenant != tenant {
                return Ok(Vec::new());
            }
        }
        // No `.await` inside the lock scope — `paginate` is a plain
        // synchronous function (never yields), so locking a std `Mutex`
        // across it is never a footgun. Delegates to the same pagination
        // logic `InMemoryReadSideStore::fetch` uses, instead of cloning the
        // full tag history via `all_events` and re-filtering it here.
        // `paginate` fails closed on an empty scope, so a fetch can never
        // return unscoped data.
        let guard = self.0.lock().expect("SharedReadSideStore lock poisoned");
        Ok(paginate(
            guard.events_for_tag(tag.value()),
            tenant,
            offset,
            batch_size,
        ))
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
        Self {
            store,
            next_version: Arc::new(AtomicI64::new(1)),
        }
    }

    /// Records one domain event as an `EventStreamElement` under a
    /// tenant-scoped tag (`"{PROJECTION_TAG}:{tenant_id}"`) — each tenant
    /// gets its own tag stream, so the underlying store structurally
    /// isolates tenants instead of relying solely on the handler filtering
    /// by `EventStreamElement::tenant_id` after the fact. See
    /// `super::tenant_tag`.
    pub fn record(
        &self,
        tenant_id: &str,
        aggregate_id: &str,
        event_type: &str,
        payload: Value,
        occurred_at: DateTime<Utc>,
    ) {
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
    async fn read_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
    ) -> Result<Option<Offset>, OffsetStoreError> {
        let key = (
            projection_id.to_string(),
            tag.value().to_string(),
            tenant.to_string(),
        );
        Ok(self
            .0
            .lock()
            .expect("InMemoryOffsetStore lock poisoned")
            .get(&key)
            .copied())
    }

    async fn write_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
        offset: &Offset,
    ) -> Result<(), OffsetStoreError> {
        let key = (
            projection_id.to_string(),
            tag.value().to_string(),
            tenant.to_string(),
        );
        self.0
            .lock()
            .expect("InMemoryOffsetStore lock poisoned")
            .insert(key, *offset);
        Ok(())
    }
}

/// In-memory `DedupStore` — this workspace has no other in-memory
/// reference implementation of it.
#[derive(Clone, Default)]
pub struct InMemoryDedupStore(Arc<Mutex<HashSet<DedupKey>>>);

#[async_trait]
impl DedupStore for InMemoryDedupStore {
    async fn seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<bool, DedupStoreError> {
        let key = (
            projection_id.to_string(),
            tag.value().to_string(),
            event_id.to_string(),
        );
        Ok(self
            .0
            .lock()
            .expect("InMemoryDedupStore lock poisoned")
            .contains(&key))
    }

    async fn mark_seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<(), DedupStoreError> {
        let key = (
            projection_id.to_string(),
            tag.value().to_string(),
            event_id.to_string(),
        );
        self.0
            .lock()
            .expect("InMemoryDedupStore lock poisoned")
            .insert(key);
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
    // AND `ReadSideStore::fetch` now takes an explicit tenant that scopes the
    // returned events by `tenant_id`, so isolation is both structural (per
    // tag) and type-enforced (per fetch) — a tag's stream can never contain
    // another tenant's events, and even if it did, `fetch` would filter them
    // out.
    #[tokio::test]
    async fn tenants_are_structurally_isolated_at_the_store_level() {
        let store = SharedReadSideStore::new();
        let sink = ReadSideSink::new(store.clone());

        sink.record(
            "tenant-a",
            "user-1",
            "UserRegistered",
            serde_json::json!({ "email": "a@example.com" }),
            Utc::now(),
        );
        sink.record(
            "tenant-b",
            "user-2",
            "UserRegistered",
            serde_json::json!({ "email": "b@example.com" }),
            Utc::now(),
        );
        sink.record(
            "tenant-a",
            "user-3",
            "UserRegistered",
            serde_json::json!({ "email": "c@example.com" }),
            Utc::now(),
        );

        let tags = store.known_tags();
        assert_eq!(
            tags.len(),
            2,
            "each tenant gets its own tag stream, not one shared tag"
        );

        for tag in &tags {
            // `fetch` scopes by its explicit `tenant` argument (authoritative),
            // and our tags are tenant-scoped, so pass the tag's own tenant to
            // read that stream back.
            let tenant = super::super::tenant_from_tag(tag).expect("tenant-scoped tag");
            let events = store.fetch(tenant, tag, None, 100).await.unwrap();
            assert!(!events.is_empty());
            let tenants: HashSet<&str> = events.iter().map(|e| e.tenant_id()).collect();
            assert_eq!(
                tenants.len(),
                1,
                "a single tag's stream must contain exactly one tenant's events, got: {tenants:?}"
            );
        }
    }

    // The `tenant` argument is authoritative (see the `ReadSideStore::fetch`
    // contract): the tag may only ever narrow to that same tenant, never pick
    // a different one. A fetch whose explicit tenant disagrees with the
    // tag-encoded tenant returns nothing (fail closed), while a matching pair
    // returns exactly that tenant's events.
    #[tokio::test]
    async fn fetch_honors_explicit_tenant_over_tag() {
        let store = SharedReadSideStore::new();
        let sink = ReadSideSink::new(store.clone());

        sink.record(
            "tenant-a",
            "user-1",
            "UserRegistered",
            serde_json::json!({ "email": "a@example.com" }),
            Utc::now(),
        );
        sink.record(
            "tenant-b",
            "user-2",
            "UserRegistered",
            serde_json::json!({ "email": "b@example.com" }),
            Utc::now(),
        );

        let tag_a = super::super::tenant_tag("tenant-a");
        let tag_b = super::super::tenant_tag("tenant-b");

        // Matching tenant + tag: returns exactly tenant-a's events.
        let matched = store.fetch("tenant-a", &tag_a, None, 100).await.unwrap();
        assert!(
            !matched.is_empty(),
            "matching tenant + tag must return events"
        );
        assert!(
            matched.iter().all(|e| e.tenant_id() == "tenant-a"),
            "only tenant-a events may be returned"
        );

        // Mismatched: explicit tenant-a but tenant-b's tag must return nothing —
        // the tag may never override the explicit tenant.
        let crossed = store.fetch("tenant-a", &tag_b, None, 100).await.unwrap();
        assert!(
            crossed.is_empty(),
            "a tag for a different tenant must never surface that tenant's events, got: {crossed:?}"
        );
    }
}
