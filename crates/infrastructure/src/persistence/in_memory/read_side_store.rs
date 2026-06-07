//! In-memory read side store.
//!
//! Reference implementation of `ReadSideStore` using in-memory storage.
//! Stores events per tag with offset-based pagination.

use std::collections::{BTreeMap, BTreeSet};

use ego_domain::read_side::{
    EventStreamElement,
    EventTag,
    Offset,
    ReadSideStore as ReadSideStoreTrait,
    ReadSideStoreError,
};

type TagStream = Vec<EventStreamElement<serde_json::Value>>;

/// In-memory read side store.
///
/// Stores events per tag with offset-based pagination.
/// All data is tenant-isolated.
pub struct InMemoryReadSideStore {
    /// Events indexed by (tenant, tag) -> sorted by event_version
    streams: BTreeMap<(String, String), TagStream>,
}

impl InMemoryReadSideStore {
    pub fn new() -> Self {
        InMemoryReadSideStore {
            streams: BTreeMap::new(),
        }
    }

    /// Inserts an event into the store.
    pub fn insert(&mut self, tenant: String, event: EventStreamElement<serde_json::Value>) {
        for tag in &event.tags {
            let key = (tenant.clone(), tag.value().to_string());
            let stream = self.streams.entry(key).or_default();
            // Insert in sorted order by event_version
            let pos = stream
                .iter()
                .position(|e| e.event_version > event.event_version)
                .unwrap_or(stream.len());
            stream.insert(pos, event.clone());
        }
    }

    /// Returns the number of events for a tag.
    pub fn len(&self, tenant: &str, tag: &str) -> usize {
        self.streams
            .get(&(tenant.to_string(), tag.to_string()))
            .map(|s| s.len())
            .unwrap_or(0)
    }

    /// Returns all events for a tag.
    pub fn all_events(&self, tenant: &str, tag: &str) -> Vec<EventStreamElement<serde_json::Value>> {
        self.streams
            .get(&(tenant.to_string(), tag.to_string()))
            .cloned()
            .unwrap_or_default()
    }
}

impl Default for InMemoryReadSideStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ReadSideStore<serde_json::Value> for InMemoryReadSideStore {
    async fn fetch(
        &self,
        tag: &EventTag,
        offset: Option<&Offset>,
        batch_size: usize,
    ) -> Result<Vec<EventStreamElement<serde_json::Value>>, ReadSideStoreError> {
        // This implementation is for testing only.
        // In production, this would query a database.
        Err(ReadSideStoreError::Transient(
            "InMemoryReadSideStore::fetch not implemented for testing".to_string(),
        ))
    }
}
