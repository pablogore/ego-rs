//! PostgreSQL persistence implementations.

pub mod aggregate_type_backfill;
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
/// `None` is the spec-blessed tenant-less/systemwide (D1) mode: it resolves to
/// `Ok(None)`, bound as SQL NULL. Only an empty-string tenant (`Some("")`) is a
/// real misconfiguration and fails closed with
/// [`PersistenceError::MissingTenant`] rather than being coerced to NULL, which
/// would silently file a caller that *meant* to scope a tenant into the shared,
/// un-scoped NULL partition — a tenant-isolation fail-open. A concrete tenant
/// (`Some(t)`) is returned verbatim.
pub(crate) fn resolve_tenant(tenant_id: Option<&str>) -> Result<Option<String>, PersistenceError> {
    match tenant_id {
        None => Ok(None),
        Some("") => Err(PersistenceError::MissingTenant),
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
        assert_eq!(
            resolve_tenant(Some("")),
            Err(PersistenceError::MissingTenant)
        );
    }

    #[test]
    fn resolve_tenant_preserves_real_tenant() {
        assert_eq!(
            resolve_tenant(Some("real-tenant")),
            Ok(Some("real-tenant".to_string()))
        );
    }

    #[test]
    fn resolve_tenant_allows_none_systemwide() {
        // `None` is the spec-blessed tenant-less/systemwide (D1) mode: it must
        // resolve to SQL NULL, not be rejected. Only an empty-string tenant is a
        // real misconfiguration.
        assert_eq!(resolve_tenant(None), Ok(None));
    }
}
