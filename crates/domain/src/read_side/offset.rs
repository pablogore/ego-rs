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

#[cfg(test)]
mod tests {
    use super::*;

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
