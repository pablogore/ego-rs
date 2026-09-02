use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ego_persistence_api::read_side::event_tag::EventTag;
use ego_persistence_api::read_side::offset::{Offset, OffsetStore, OffsetStoreError};

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
