//! Deduplication store SPI.

use async_trait::async_trait;
use thiserror::Error;

use super::event_tag::EventTag;

/// Error type for dedup store operations.
#[derive(Debug, Error)]
pub enum DedupStoreError {
    /// Transient error (e.g., connection issue).
    #[error("transient dedup store error: {0}")]
    Transient(String),

    /// Fatal error (e.g., data corruption).
    #[error("fatal dedup store error: {0}")]
    Fatal(String),
}

/// Deduplication store SPI.
///
/// Deduplication scope: (projection_id, tag, event_id).
/// Replay dedup is ON by default.
#[async_trait]
pub trait DedupStore {
    /// Whether dedup marks written through this store survive a process
    /// restart.
    ///
    /// Defaults to `false`: honest for every implementation in this workspace
    /// today, none of which is durable, and for every third-party implementation
    /// that has not considered the question. `Profile::Production` reads this
    /// (PROD-014A); a durable implementation overrides it to `true`.
    fn is_durable(&self) -> bool {
        false
    }

    /// Checks if an event has already been seen.
    async fn seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<bool, DedupStoreError>;

    /// Marks an event as seen.
    async fn mark_seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<(), DedupStoreError>;
}

/// Forwards through a shared handle, so a composition root can hold the
/// pair as `Arc<dyn DedupStore + Send + Sync>` and still hand that exact
/// value to `TagSchedulerImpl::spawn`, whose `D` parameter is taken by
/// value with a `Clone` bound. Without this, the registered pair and the
/// spawned pair could never be the same value (PROD-014A EC-2).
#[async_trait]
impl<T: DedupStore + Send + Sync + ?Sized> DedupStore for std::sync::Arc<T> {
    /// **Load-bearing.** Omitting this silently inherits the trait's `false`
    /// default, and every registered pair would be classified volatile no
    /// matter what the host wrapped — the gate would refuse a correct
    /// durable composition and pass nothing.
    fn is_durable(&self) -> bool {
        (**self).is_durable()
    }

    async fn seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<bool, DedupStoreError> {
        (**self).seen(projection_id, tag, event_id).await
    }

    async fn mark_seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<(), DedupStoreError> {
        (**self).mark_seen(projection_id, tag, event_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct BareDedupStore;

    #[async_trait]
    impl DedupStore for BareDedupStore {
        async fn seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _event_id: &str,
        ) -> Result<bool, DedupStoreError> {
            Ok(false)
        }

        async fn mark_seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _event_id: &str,
        ) -> Result<(), DedupStoreError> {
            Ok(())
        }
    }

    /// A bare implementation that never overrides `is_durable()` must be
    /// classified volatile by default (PROD-014A IS-1/AD-4).
    #[test]
    fn bare_impl_defaults_is_durable_to_false() {
        assert!(!BareDedupStore.is_durable());
    }

    #[derive(Default)]
    struct DurableDedupStore {
        seen_ids: std::sync::Mutex<std::collections::HashSet<String>>,
    }

    #[async_trait]
    impl DedupStore for DurableDedupStore {
        fn is_durable(&self) -> bool {
            true
        }

        async fn seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            event_id: &str,
        ) -> Result<bool, DedupStoreError> {
            Ok(self.seen_ids.lock().unwrap().contains(event_id))
        }

        async fn mark_seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            event_id: &str,
        ) -> Result<(), DedupStoreError> {
            self.seen_ids.lock().unwrap().insert(event_id.to_string());
            Ok(())
        }
    }

    /// PROD-014A EC-2/AD-3 landmine: `Arc<T>` MUST forward `is_durable()`.
    /// Without this, a composition root holding the pair as
    /// `Arc<dyn DedupStore + Send + Sync>` would classify every registered
    /// store volatile no matter what it wraps.
    #[test]
    fn arc_forwards_is_durable() {
        let store: Arc<dyn DedupStore + Send + Sync> = Arc::new(DurableDedupStore::default());
        assert!(store.is_durable(), "Arc<T> must forward is_durable()");
    }

    /// `Arc<T>` must also forward `seen`/`mark_seen`, proving the same `Arc`
    /// handle a composition root registers is fully usable by
    /// `TagSchedulerImpl::spawn` (PROD-014A EC-2).
    #[tokio::test]
    async fn arc_forwards_seen_and_mark_seen() {
        let store: Arc<dyn DedupStore + Send + Sync> = Arc::new(DurableDedupStore::default());
        let tag = EventTag::new("users-by-tenant");

        assert!(!store.seen("proj", &tag, "evt-1").await.unwrap());
        store.mark_seen("proj", &tag, "evt-1").await.unwrap();
        assert!(store.seen("proj", &tag, "evt-1").await.unwrap());
    }
}
