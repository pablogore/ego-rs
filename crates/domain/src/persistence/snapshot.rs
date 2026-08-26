//! Aggregate snapshot contract.
//!
//! Defines `Snapshot` — the domain SPI for saving and loading aggregate
//! snapshots. Snapshots enable fast reconstruction without replaying all
//! events.

use crate::persistence::PersistenceError;
use serde_json::Value;

/// Trait for saving and loading aggregate snapshots.
///
/// Snapshots capture the full state of an aggregate at a given version,
/// allowing faster reconstruction without replaying all events.
pub trait Snapshot {
    /// Declares whether this store's writes survive a process restart
    /// (PROD-013). Defaulted to `false` so no existing or third-party
    /// implementation is silently promoted to "durable" by omission — a
    /// durable provider overrides this to report its truthful capability.
    fn is_durable(&self) -> bool {
        false
    }

    /// Save a snapshot for the given aggregate.
    ///
    /// - `aggregate_id`: The unique identifier of the aggregate.
    /// - `tenant_id`: Optional tenant scope. `Some("")` (empty string) is treated as missing tenant.
    /// - `version`: The aggregate version at which the snapshot was taken.
    /// - `payload`: The serialized aggregate state.
    fn save_snapshot(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: i64,
        payload: Value,
    ) -> Result<(), PersistenceError>;

    /// Load the latest snapshot for the given aggregate.
    ///
    /// Returns `Ok(None)` if no snapshot exists.
    /// Returns `Ok(Some((version, payload)))` with the highest version snapshot.
    fn load_snapshot(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<(i64, Value)>, PersistenceError>;
}
