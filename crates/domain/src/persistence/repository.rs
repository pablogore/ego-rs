//! Aggregate repository contract.
//!
//! Defines `Repository<A>` — the domain SPI for saving, loading, and
//! deleting aggregates. Optimistic concurrency is enforced via
//! version checks.

use crate::persistence::PersistenceError;

/// Trait for saving, loading, and deleting aggregates.
///
/// Implementations provide persistence for aggregates using any storage system.
pub trait Repository<A> {
    /// Save (upsert) an aggregate.
    ///
    /// - `aggregate_id`: The unique identifier of the aggregate.
    /// - `aggregate`: The aggregate to save.
    /// - `tenant_id`: Optional tenant scope. `Some("")` (empty string) is treated as missing tenant.
    /// - `expected_version`: Optimistic concurrency check. Use `0` for new aggregates.
    ///
    /// Returns the new version on success, or a `PersistenceError`.
    fn save(
        &mut self,
        aggregate_id: &str,
        aggregate: A,
        tenant_id: Option<&str>,
        expected_version: i64,
    ) -> Result<i64, PersistenceError>;

    /// Load an aggregate by ID.
    ///
    /// Returns `PersistenceError::NotFound` if the aggregate does not exist.
    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<A, PersistenceError>;

    /// Delete an aggregate by ID.
    fn delete(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), PersistenceError>;
}
