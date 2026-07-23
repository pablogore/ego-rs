//! In-memory read side store.
//!
//! Reference implementation of `ReadSideStore` using in-memory storage.
//! Stores events per tag with offset-based pagination.

use std::collections::BTreeMap;

use ego_domain::read_side::event_stream::EventStreamElement;
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::offset::Offset;
use ego_domain::read_side::store::{ReadSideStore, ReadSideStoreError};

type TagStream = Vec<EventStreamElement<serde_json::Value>>;

/// In-memory read side store.
///
/// Stores events per tag with offset-based pagination.
///
/// `ReadSideStore::fetch` takes an explicit `tenant` parameter, so this
/// reference implementation enforces tenant isolation at the store level:
/// `fetch` only ever returns events whose `tenant_id` matches the requested
/// tenant, independent of how the tag was constructed. An empty tenant
/// returns nothing (fail closed).
pub struct InMemoryReadSideStore {
    /// Events indexed by tag, sorted by `event_version` ascending.
    streams: BTreeMap<String, TagStream>,
}

impl InMemoryReadSideStore {
    pub fn new() -> Self {
        InMemoryReadSideStore {
            streams: BTreeMap::new(),
        }
    }

    /// Inserts an event, fanning out to every tag it carries (CORE-005
    /// FR-021: an event with multiple tags is processed independently in
    /// each tag stream).
    pub fn insert(&mut self, event: EventStreamElement<serde_json::Value>) {
        for tag in &event.tags {
            let stream = self.streams.entry(tag.value().to_string()).or_default();
            // Insert in sorted order by event_version.
            let pos = stream
                .iter()
                .position(|e| e.event_version > event.event_version)
                .unwrap_or(stream.len());
            stream.insert(pos, event.clone());
        }
    }

    /// Returns the number of events stored for a tag.
    pub fn len(&self, tag: &str) -> usize {
        self.streams.get(tag).map(|s| s.len()).unwrap_or(0)
    }

    /// Returns true if no events are stored for a tag.
    pub fn is_empty(&self, tag: &str) -> bool {
        self.len(tag) == 0
    }

    /// Returns all events for a tag, in ascending `event_version` order.
    pub fn all_events(&self, tag: &str) -> Vec<EventStreamElement<serde_json::Value>> {
        self.streams.get(tag).cloned().unwrap_or_default()
    }

    /// Returns an iterator over events for a tag, in ascending
    /// `event_version` order, without cloning the whole tag stream — for
    /// callers that build their own pagination via [`paginate`] (e.g.
    /// `SharedReadSideStore::fetch`, which must stay synchronous while
    /// holding its own lock guard).
    pub fn events_for_tag(
        &self,
        tag: &str,
    ) -> impl Iterator<Item = &EventStreamElement<serde_json::Value>> {
        self.streams.get(tag).into_iter().flatten()
    }

    /// Every tag currently holding at least one event, in ascending order.
    /// Used by pollers that discover their tag set dynamically (e.g. one tag
    /// per tenant) instead of knowing it upfront.
    pub fn known_tags(&self) -> Vec<EventTag> {
        self.streams
            .keys()
            .map(|t| EventTag::new(t.clone()))
            .collect()
    }
}

impl Default for InMemoryReadSideStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Synchronous offset/batch-size pagination shared by every `ReadSideStore`
/// impl backed by an in-memory, per-tag ordered event list. Kept as a plain
/// (non-`&self`) function so callers can also invoke it while holding a
/// `std::sync::MutexGuard` without an `.await` in scope — a `MutexGuard` is
/// `!Send`, so holding one across an await point would break a `Send`-bound
/// async trait future; this function never awaits, so that's never a risk.
///
/// Also enforces tenant isolation: only events whose `tenant_id` equals
/// `tenant` are returned, and an empty `tenant` returns nothing (fail closed)
/// so a missing tenant can never surface another tenant's events.
pub fn paginate<'a>(
    events: impl Iterator<Item = &'a EventStreamElement<serde_json::Value>>,
    tenant: &str,
    offset: Option<&Offset>,
    batch_size: usize,
) -> Vec<EventStreamElement<serde_json::Value>> {
    // Fail closed: an empty tenant is treated as "no tenant" and never
    // matches any event, instead of degrading into an unscoped scan.
    if tenant.trim().is_empty() {
        return Vec::new();
    }
    let after = offset.and_then(Offset::as_sequence).unwrap_or(0);
    events
        .filter(|e| e.tenant_id() == tenant && e.event_version > after)
        .take(batch_size)
        .cloned()
        .collect()
}

#[async_trait::async_trait]
impl ReadSideStore<serde_json::Value> for InMemoryReadSideStore {
    async fn fetch(
        &self,
        tenant: &str,
        tag: &EventTag,
        offset: Option<&Offset>,
        batch_size: usize,
    ) -> Result<Vec<EventStreamElement<serde_json::Value>>, ReadSideStoreError> {
        Ok(paginate(
            self.events_for_tag(tag.value()),
            tenant,
            offset,
            batch_size,
        ))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn element(tag: &str, version: i64) -> EventStreamElement<serde_json::Value> {
        element_for("tenant-a", tag, version)
    }

    fn element_for(tenant: &str, tag: &str, version: i64) -> EventStreamElement<serde_json::Value> {
        EventStreamElement::new(
            format!("evt-{version}"),
            "agg-1",
            tenant,
            "TestEvent",
            serde_json::json!({ "v": version }),
            version,
            Utc::now(),
            vec![EventTag::new(tag)],
        )
    }

    #[tokio::test]
    async fn fetch_returns_events_after_offset_up_to_batch_size() {
        let mut store = InMemoryReadSideStore::new();
        for v in 1..=5 {
            store.insert(element("order", v));
        }

        let tag = EventTag::new("order");
        let page = store
            .fetch("tenant-a", &tag, Some(&Offset::sequence(2)), 2)
            .await
            .unwrap();

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].event_version, 3);
        assert_eq!(page[1].event_version, 4);
    }

    #[tokio::test]
    async fn fetch_from_none_offset_starts_from_the_beginning() {
        let mut store = InMemoryReadSideStore::new();
        store.insert(element("order", 1));
        store.insert(element("order", 2));

        let tag = EventTag::new("order");
        let page = store.fetch("tenant-a", &tag, None, 10).await.unwrap();

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].event_version, 1);
    }

    #[tokio::test]
    async fn fetch_for_unknown_tag_returns_empty() {
        let store = InMemoryReadSideStore::new();
        let tag = EventTag::new("unknown");

        let page = store.fetch("tenant-a", &tag, None, 10).await.unwrap();
        assert!(page.is_empty());
    }

    // Security: tenant isolation is enforced at the store level, not left to
    // the convention of folding the tenant into the tag. Two tenants share a
    // single tag stream here; `fetch` must still only ever return the
    // requested tenant's events.
    #[tokio::test]
    async fn fetch_only_returns_events_for_the_requested_tenant() {
        let mut store = InMemoryReadSideStore::new();
        store.insert(element_for("tenant-a", "order", 1));
        store.insert(element_for("tenant-b", "order", 2));
        store.insert(element_for("tenant-a", "order", 3));

        let tag = EventTag::new("order");

        let a = store.fetch("tenant-a", &tag, None, 10).await.unwrap();
        assert_eq!(a.len(), 2, "only tenant-a's events");
        assert!(a.iter().all(|e| e.tenant_id() == "tenant-a"));

        let b = store.fetch("tenant-b", &tag, None, 10).await.unwrap();
        assert_eq!(b.len(), 1, "only tenant-b's events");
        assert!(b.iter().all(|e| e.tenant_id() == "tenant-b"));
    }

    // Fail closed: an empty tenant must never surface another tenant's data,
    // even when the tag stream is non-empty.
    #[tokio::test]
    async fn fetch_with_empty_tenant_returns_empty_fail_closed() {
        let mut store = InMemoryReadSideStore::new();
        store.insert(element_for("tenant-a", "order", 1));
        store.insert(element_for("tenant-b", "order", 2));

        let tag = EventTag::new("order");
        let page = store.fetch("", &tag, None, 10).await.unwrap();
        assert!(page.is_empty(), "empty tenant must return no events");
    }

    #[test]
    fn insert_fans_out_across_multiple_tags() {
        let mut store = InMemoryReadSideStore::new();
        let event = EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-a",
            "TestEvent",
            serde_json::json!({}),
            1,
            Utc::now(),
            vec![EventTag::new("order"), EventTag::new("payment")],
        );
        store.insert(event);

        assert_eq!(store.len("order"), 1);
        assert_eq!(store.len("payment"), 1);
        assert!(store.is_empty("shipping"));
    }
}
