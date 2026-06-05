use crate::event::DomainEvent;
use crate::persistence::{PersistenceError, StoredEvent};

/// Trait for appending and loading domain events.
///
/// Implementations provide event-sourced persistence backed by any storage system.
pub trait EventStore<E: DomainEvent> {
    /// Append events to the event stream for the given aggregate.
    ///
    /// - `aggregate_id`: The unique identifier of the aggregate.
    /// - `tenant_id`: Optional tenant scope. `Some("")` (empty string) is treated as missing tenant.
    /// - `expected_version`: Optimistic concurrency check. Use `0` for new aggregates.
    /// - `events`: The events to append, wrapped with optional metadata.
    ///
    /// Returns the new stream version on success, or a `PersistenceError`.
    fn append(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError>;

    /// Load all events for the given aggregate in the given tenant.
    ///
    /// Returns `PersistenceError::NotFound` if the aggregate stream does not exist.
    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError>;

    /// List all aggregate IDs known to this store, optionally scoped to a tenant.
    fn list_aggregate_ids(&self, tenant_id: Option<&str>) -> Result<Vec<String>, PersistenceError>;
}
