//! Offset tracking and offset store SPI.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::event_tag::EventTag;

/// Tracks how far a projection has progressed within a tag stream.
///
/// Only `Sequence` variant is allowed (FR-014).
/// Represents the last confirmed event_version post-atomic-commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Offset {
    /// Last confirmed event_version. Resume from `offset + 1` after restart.
    Sequence(i64),
}

impl Offset {
    /// Creates a new `Sequence` offset.
    pub fn sequence(version: i64) -> Self {
        Self::Sequence(version)
    }

    /// Returns the sequence value, or `None` if this is not a `Sequence` offset.
    pub fn as_sequence(&self) -> Option<i64> {
        match self {
            Self::Sequence(v) => Some(*v),
        }
    }
}

impl std::fmt::Display for Offset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequence(v) => write!(f, "Sequence({})", v),
        }
    }
}

/// Error type for offset store operations.
#[derive(Debug, Error)]
pub enum OffsetStoreError {
    /// Transient error (e.g., connection issue).
    #[error("transient offset store error: {0}")]
    Transient(String),
    /// Fatal error (e.g., data corruption).
    #[error("fatal offset store error: {0}")]
    Fatal(String),
}

/// Offset store SPI — reads and writes projection offsets per (projection_id, tag, tenant).
///
/// Offsets are independent per (projection_id, tag, tenant) tuple.
#[async_trait::async_trait]
pub trait OffsetStore {
    /// Whether offsets written through this store survive a process restart.
    ///
    /// Defaults to `false`: honest for every implementation in this workspace
    /// today, none of which is durable, and for every third-party implementation
    /// that has not considered the question. `Profile::Production` reads this
    /// (PROD-014A); a durable implementation overrides it to `true`.
    fn is_durable(&self) -> bool {
        false
    }

    /// Reads the offset for a projection on a tag in a tenant scope.
    ///
    /// Returns `Ok(None)` if no offset has been written yet.
    async fn read_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
    ) -> Result<Option<Offset>, OffsetStoreError>;

    /// Writes the offset for a projection on a tag in a tenant scope.
    async fn write_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
        offset: &Offset,
    ) -> Result<(), OffsetStoreError>;
}

/// Forwards through a shared handle, so a composition root can hold the
/// pair as `Arc<dyn OffsetStore + Send + Sync>` and still hand that exact
/// value to `TagSchedulerImpl::spawn`, whose `O` parameter is taken by
/// value with a `Clone` bound. Without this, the registered pair and the
/// spawned pair could never be the same value (PROD-014A EC-2).
#[async_trait::async_trait]
impl<T: OffsetStore + Send + Sync + ?Sized> OffsetStore for std::sync::Arc<T> {
    /// **Load-bearing.** Omitting this silently inherits the trait's `false`
    /// default, and every registered pair would be classified volatile no
    /// matter what the host wrapped — the gate would refuse a correct
    /// durable composition and pass nothing.
    fn is_durable(&self) -> bool {
        (**self).is_durable()
    }

    async fn read_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
    ) -> Result<Option<Offset>, OffsetStoreError> {
        (**self).read_offset(projection_id, tag, tenant).await
    }

    async fn write_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
        offset: &Offset,
    ) -> Result<(), OffsetStoreError> {
        (**self).write_offset(projection_id, tag, tenant, offset).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct BareOffsetStore;

    #[async_trait::async_trait]
    impl OffsetStore for BareOffsetStore {
        async fn read_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
        ) -> Result<Option<Offset>, OffsetStoreError> {
            Ok(None)
        }

        async fn write_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
            _offset: &Offset,
        ) -> Result<(), OffsetStoreError> {
            Ok(())
        }
    }

    /// A bare implementation that never overrides `is_durable()` must be
    /// classified volatile by default (PROD-014A IS-1/AD-4).
    #[test]
    fn bare_impl_defaults_is_durable_to_false() {
        assert!(!BareOffsetStore.is_durable());
    }

    #[derive(Default)]
    struct DurableOffsetStore {
        written: std::sync::Mutex<Option<Offset>>,
    }

    #[async_trait::async_trait]
    impl OffsetStore for DurableOffsetStore {
        fn is_durable(&self) -> bool {
            true
        }

        async fn read_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
        ) -> Result<Option<Offset>, OffsetStoreError> {
            Ok(*self.written.lock().unwrap())
        }

        async fn write_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
            offset: &Offset,
        ) -> Result<(), OffsetStoreError> {
            *self.written.lock().unwrap() = Some(*offset);
            Ok(())
        }
    }

    /// PROD-014A EC-2/AD-3 landmine: `Arc<T>` MUST forward `is_durable()`.
    /// Without this, a composition root holding the pair as
    /// `Arc<dyn OffsetStore + Send + Sync>` would classify every registered
    /// store volatile no matter what it wraps.
    #[test]
    fn arc_forwards_is_durable() {
        let store: Arc<dyn OffsetStore + Send + Sync> = Arc::new(DurableOffsetStore::default());
        assert!(store.is_durable(), "Arc<T> must forward is_durable()");
    }

    /// `Arc<T>` must also forward `read_offset`/`write_offset`, proving the
    /// same `Arc` handle a composition root registers is fully usable by
    /// `TagSchedulerImpl::spawn` (PROD-014A EC-2).
    #[tokio::test]
    async fn arc_forwards_read_and_write_offset() {
        let store: Arc<dyn OffsetStore + Send + Sync> = Arc::new(DurableOffsetStore::default());
        let tag = EventTag::new("users-by-tenant");

        assert_eq!(store.read_offset("proj", &tag, "tenant").await.unwrap(), None);

        store
            .write_offset("proj", &tag, "tenant", &Offset::sequence(7))
            .await
            .unwrap();

        assert_eq!(
            store.read_offset("proj", &tag, "tenant").await.unwrap(),
            Some(Offset::sequence(7))
        );
    }

    #[test]
    fn test_offset_sequence() {
        let offset = Offset::sequence(42);
        assert_eq!(offset.as_sequence(), Some(42));
        assert_eq!(format!("{}", offset), "Sequence(42)");
    }

    #[test]
    fn test_offset_equality() {
        let o1 = Offset::sequence(10);
        let o2 = Offset::sequence(10);
        let o3 = Offset::sequence(20);
        assert_eq!(o1, o2);
        assert_ne!(o1, o3);
    }
}
