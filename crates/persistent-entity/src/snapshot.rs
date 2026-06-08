//! Snapshot strategy definitions.
//!
//! This module defines the snapshot strategy trait and built-in implementations.

use crate::error::EntityError;
use serde::{Deserialize, Serialize};

/// A snapshot strategy for determining when to take snapshots.
#[async_trait::async_trait]
pub trait SnapshotStrategy: Send + Sync {
    /// Determine if a snapshot should be taken.
    ///
    /// # Arguments
    /// * `version` - The current version of the entity
    /// * `event_count` - The number of events since the last snapshot
    ///
    /// # Returns
    /// * `bool` - True if a snapshot should be taken
    async fn should_take_snapshot(
        &self,
        version: u64,
        event_count: u64,
    ) -> Result<bool, EntityError>;
}

/// A snapshot strategy that takes snapshots every N events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodicSnapshotStrategy {
    /// The number of events between snapshots.
    pub events_between_snapshots: u64,
}

impl PeriodicSnapshotStrategy {
    /// Create a new periodic snapshot strategy.
    pub fn new(events_between_snapshots: u64) -> Self {
        Self {
            events_between_snapshots,
        }
    }
}

#[async_trait::async_trait]
impl SnapshotStrategy for PeriodicSnapshotStrategy {
    async fn should_take_snapshot(
        &self,
        _version: u64,
        event_count: u64,
    ) -> Result<bool, EntityError> {
        Ok(event_count >= self.events_between_snapshots)
    }
}

/// A snapshot strategy that takes snapshots based on version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionBasedSnapshotStrategy {
    /// The version interval between snapshots.
    pub version_interval: u64,
}

impl VersionBasedSnapshotStrategy {
    /// Create a new version-based snapshot strategy.
    pub fn new(version_interval: u64) -> Self {
        Self {
            version_interval,
        }
    }
}

#[async_trait::async_trait]
impl SnapshotStrategy for VersionBasedSnapshotStrategy {
    async fn should_take_snapshot(
        &self,
        version: u64,
        _event_count: u64,
    ) -> Result<bool, EntityError> {
        Ok(version % self.version_interval == 0)
    }
}

/// A snapshot strategy that never takes snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoSnapshot;

#[async_trait::async_trait]
impl SnapshotStrategy for NoSnapshot {
    async fn should_take_snapshot(
        &self,
        _version: u64,
        _event_count: u64,
    ) -> Result<bool, EntityError> {
        Ok(false)
    }
}