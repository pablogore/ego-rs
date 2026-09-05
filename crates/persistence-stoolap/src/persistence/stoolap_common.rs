//! Shared Stoolap plumbing: DSN construction, tenant-scope encoding, and
//! error classification. Promoted from `repository.rs` (S1) so `snapshot.rs`
//! (and, in a later PR, the `event_sourcing` module) reuse the exact same
//! DSN string, tenant sentinel, and conflict classifier rather than each
//! defining their own copy (design.md AD-2).

use std::path::Path;

use ego_persistence_api::persistence::PersistenceError;

/// The scope a `None` tenant is stored under. Never returned to a caller,
/// never compared against a caller-supplied value — internal encoding only.
pub(crate) const SYSTEMWIDE_SCOPE: &str = "";

/// Maps a resolved tenant (`None` == systemwide) to its stored scope column
/// value. The sentinel is the empty string, which `resolve_tenant` already
/// rejects as a caller-supplied tenant (`MissingTenant`), so no real tenant
/// can ever collide with it.
pub(crate) fn encode_tenant(resolved: Option<&str>) -> &str {
    resolved.unwrap_or(SYSTEMWIDE_SCOPE)
}

/// Builds the durable-sync DSN every Stoolap-backed store in this crate must
/// open through (design.md AD-3): `sync=full` is what makes `is_durable()`
/// truthful rather than a hardcoded claim.
pub(crate) fn dsn_for(path: &Path) -> String {
    format!("file://{}?sync=full", path.display())
}

pub(crate) fn internal_err(e: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::Internal(e.to_string())
}

/// Classifies a raw Stoolap error as a lost optimistic-concurrency race
/// (`Conflict`) rather than a genuine failure (`Internal`). Default is
/// fail-loud: anything not recognized here stays `Internal`.
pub(crate) fn is_write_conflict(e: &stoolap::Error) -> bool {
    match e {
        stoolap::Error::UniqueConstraint { .. } => true,
        stoolap::Error::TransactionAborted => true,
        stoolap::Error::LockAcquisitionFailed(_) | stoolap::Error::DatabaseLocked => true,
        // Pinned, brittle-but-documented arm (EC-7): Stoolap's MVCC write-claim
        // conflict has no dedicated error variant, only this message text.
        stoolap::Error::Internal { message } => {
            message.contains("uncommitted changes from transaction")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsn_carries_full_sync() {
        assert_eq!(dsn_for(Path::new("/tmp/x")), "file:///tmp/x?sync=full");
    }

    #[test]
    fn encode_tenant_maps_only_the_absent_scope_to_the_sentinel() {
        assert_eq!(encode_tenant(None), SYSTEMWIDE_SCOPE);
        assert_eq!(encode_tenant(Some("t")), "t");
    }

    #[test]
    fn is_write_conflict_recognizes_a_unique_constraint_violation() {
        let e = stoolap::Error::UniqueConstraint {
            index: "idx".into(),
            column: "aggregate_id".into(),
            value: "agg-1".into(),
            row_id: -1,
        };
        assert!(is_write_conflict(&e));
    }

    #[test]
    fn is_write_conflict_recognizes_a_transaction_aborted_error() {
        assert!(is_write_conflict(&stoolap::Error::TransactionAborted));
    }

    #[test]
    fn is_write_conflict_recognizes_lock_failures() {
        assert!(is_write_conflict(&stoolap::Error::LockAcquisitionFailed(
            "held by another writer".into()
        )));
        assert!(is_write_conflict(&stoolap::Error::DatabaseLocked));
    }

    #[test]
    fn is_write_conflict_fails_loud_for_an_unrecognized_error() {
        assert!(!is_write_conflict(&stoolap::Error::TableNotFound(
            "aggregates".into()
        )));
    }
}
