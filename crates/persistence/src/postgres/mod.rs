//! PostgreSQL persistence implementations.

pub mod aggregate_type_backfill;
pub mod event_store;
pub mod migrations;
pub mod repository;
pub mod snapshot;

pub use event_store::PostgreSQLEventStore;
pub use repository::PostgreSQLRepository;
pub use snapshot::PostgreSQLSnapshotStore;

/// Coerce an optional tenant identifier into the value bound to SQL queries.
///
/// The tenant-scope rule lives in the domain — see
/// [`ego_domain::persistence::tenant`] for why. Re-exported so this module's
/// existing `use crate::postgres::resolve_tenant;` call sites keep working.
pub(crate) use ego_domain::persistence::resolve_tenant;
