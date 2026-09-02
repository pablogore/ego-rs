use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ego_persistence_api::read_side::dedup::{DedupStore, DedupStoreError};
use ego_persistence_api::read_side::event_tag::EventTag;

/// Dedup-state key: `(projection_id, tag, event_id)` (CORE-005's exact
/// dedup scope).
type DedupKey = (String, String, String);
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
