//! Resolution of a caller-supplied tenant scope into the value a store files a
//! row under.
//!
//! This lives in the domain because it is a rule about what a tenant identifier
//! *means*, not a detail of how any one store keeps its rows. It had been copied
//! into four adapters — the PostgreSQL module and three in-memory ones — which is
//! four places for the rule to drift and, at the point this was consolidated, a
//! fifth about to be added.

use crate::persistence::PersistenceError;

/// Resolves a caller's tenant argument into the scope a store files under.
///
/// Three inputs, three distinct meanings, and the middle one is the reason this
/// function exists rather than a cast:
///
/// - `None` — the tenant-less ("systemwide") scope. A legitimate mode, resolved
///   to the absent scope.
/// - `Some("")` — a **misconfiguration**, rejected. An empty tenant almost always
///   means a value that should have been populated and was not; coercing it to
///   the absent scope would file the row into the shared systemwide partition, so
///   a configuration mistake would become a silent tenant-isolation failure.
/// - `Some(t)` — that tenant.
///
/// Stores must compare the resolved scope null-safely. Two absent scopes are the
/// same scope, and an absent scope is not any concrete tenant — in SQL that means
/// `IS NOT DISTINCT FROM` rather than `=`, and in a keyed collection it means the
/// key must carry the `Option` rather than flattening it.
pub fn resolve_tenant(tenant_id: Option<&str>) -> Result<Option<String>, PersistenceError> {
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
    fn an_empty_tenant_is_rejected_rather_than_coerced_to_systemwide() {
        // Fail closed. Coercing this to `None` would file the row into the
        // shared, unscoped partition — a configuration mistake becoming a
        // tenant-isolation failure with no error raised anywhere.
        assert_eq!(
            resolve_tenant(Some("")),
            Err(PersistenceError::MissingTenant)
        );
    }

    #[test]
    fn a_real_tenant_is_preserved_exactly() {
        assert_eq!(
            resolve_tenant(Some("real-tenant")),
            Ok(Some("real-tenant".to_string()))
        );
    }

    #[test]
    fn absent_is_the_systemwide_scope_and_is_allowed() {
        // `None` is a supported mode, not a missing value. Only the empty string
        // is a misconfiguration.
        assert_eq!(resolve_tenant(None), Ok(None));
    }

    #[test]
    fn whitespace_is_a_tenant_name_not_an_empty_one() {
        // Documenting the boundary rather than leaving it to be discovered: only
        // the exactly-empty string is rejected. Trimming here would silently
        // rename tenants, and deciding that " " is a mistake is a policy this
        // layer has no basis for.
        assert_eq!(resolve_tenant(Some(" ")), Ok(Some(" ".to_string())));
    }
}
