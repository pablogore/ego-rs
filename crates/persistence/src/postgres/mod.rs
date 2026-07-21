//! PostgreSQL persistence implementations.

pub mod event_store;
pub mod migrations;
pub mod repository;
pub mod snapshot;

pub use event_store::PostgreSQLEventStore;
pub use repository::PostgreSQLRepository;
pub use snapshot::PostgreSQLSnapshotStore;

use ego_domain::persistence::PersistenceError;

/// Coerce an optional tenant identifier into the value bound to SQL queries.
///
/// Fail closed on any absent tenant: both `Some("")` and `None` return
/// [`PersistenceError::MissingTenant`] instead of being coerced to SQL NULL,
/// which would file the row into the shared, un-scoped NULL partition — a
/// tenant-isolation fail-open. This matches the `tenant_id NOT NULL` schema
/// constraint (migration 007): the database refuses un-scoped rows, so the
/// backend rejects them up front with a clean error rather than surfacing a raw
/// constraint violation. There is no tenant-less/systemwide write path on the
/// Postgres backend.
///
/// The return type stays `Option<String>` (always `Some` on success) so the
/// SQL bind sites remain unchanged.
pub(crate) fn resolve_tenant(
    tenant_id: Option<&str>,
) -> Result<Option<String>, PersistenceError> {
    match tenant_id {
        Some("") | None => Err(PersistenceError::MissingTenant),
        Some(t) => Ok(Some(t.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_tenant_rejects_empty_string() {
        // An empty-string tenant must fail closed instead of being coerced to
        // SQL NULL (which would file the row into the shared, un-scoped NULL
        // partition — a tenant-isolation fail-open).
        assert_eq!(resolve_tenant(Some("")), Err(PersistenceError::MissingTenant));
    }

    #[test]
    fn resolve_tenant_preserves_real_tenant() {
        assert_eq!(
            resolve_tenant(Some("real-tenant")),
            Ok(Some("real-tenant".to_string()))
        );
    }

    #[test]
    fn resolve_tenant_rejects_none() {
        // Fail closed: an absent tenant must be rejected up front, matching the
        // `tenant_id NOT NULL` schema (migration 007). There is no tenant-less/
        // systemwide write path on the Postgres backend.
        assert_eq!(resolve_tenant(None), Err(PersistenceError::MissingTenant));
    }
}
