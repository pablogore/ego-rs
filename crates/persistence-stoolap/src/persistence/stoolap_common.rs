//! Shared Stoolap plumbing: DSN construction, tenant-scope encoding, and
//! error classification. Promoted from `repository.rs` (S1) so `snapshot.rs`
//! (and, in a later PR, the `event_sourcing` module) reuse the exact same
//! DSN string, tenant sentinel, and conflict classifier rather than each
//! defining their own copy (design.md AD-2).

use std::path::Path;

use ego_persistence_api::operation::reservation::{FencingToken, ReservationError};
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

/// Whether `dsn` declares `sync=full` as an actual query parameter — not
/// merely as a substring anywhere in the string (AD-9, promoted from
/// `crates/effect-store/src/stoolap/mod.rs::dsn_declares_sync_full`).
///
/// A raw `dsn.contains("sync=full")` would also match a path segment that
/// happens to contain that text (e.g. `file:///data/no_sync=full_here/db`).
/// Stoolap's `Database` exposes no structured accessor for the sync mode it
/// parsed (only the raw `dsn()` string), so this parses just the query
/// section — everything after the first `?` — and requires an exact
/// `sync=full` token between `&` separators.
pub(crate) fn dsn_declares_sync_full(dsn: &str) -> bool {
    dsn.split_once('?')
        .map(|(_, query)| query.split('&').any(|param| param == "sync=full"))
        .unwrap_or(false)
}

/// Classifies a raw Stoolap error as a lost optimistic-concurrency race
/// (`Conflict`) rather than a genuine failure (`Internal`). Default is
/// fail-loud: anything not recognized here stays `Internal`.
pub(crate) fn is_write_conflict(e: &stoolap::Error) -> bool {
    match e {
        stoolap::Error::UniqueConstraint { .. } => true,
        stoolap::Error::TransactionAborted => true,
        stoolap::Error::LockAcquisitionFailed(_) | stoolap::Error::DatabaseLocked => true,
        // Pinned, brittle-but-documented arm (EC-7): none of Stoolap's MVCC
        // write conflicts has a dedicated error variant, only message text on
        // `Internal`. Stoolap 0.4.0 raises three, all of them a lost race and
        // all of them retry-safe:
        //
        //   - "row N has uncommitted changes from transaction M" — the
        //     pessimistic write-claim conflict, raised when the write is taken.
        //   - "write conflict: row N was modified by another transaction" and
        //     "write conflict: row N was concurrently inserted by another
        //     transaction" — OCC validation, raised at commit instead.
        //
        // Which one a racer hits depends on whether it collides while claiming
        // the write or only at commit, so recognizing one and not the others
        // makes the classification depend on scheduling: the same lost race
        // surfaces as retry-safe or as fatal from run to run.
        stoolap::Error::Internal { message } => {
            message.contains("uncommitted changes from transaction")
                || message.starts_with("write conflict:")
        }
        _ => false,
    }
}

/// Converts a token into the column's type, refusing rather than wrapping —
/// mirrors `PostgresOperationReservationStore::token_for_storage`.
///
/// Hoisted from `operation/reservation.rs` (design.md AD-11): both callers —
/// `operation::reservation` (behind `#[cfg(feature = "operation-reservation")]`)
/// and `read_side::claim` (behind `#[cfg(feature = "read-side")]`) — need this
/// exact guard, and `operation/reservation.rs` cannot be referenced in place
/// from `read_side` because a cross-feature reference would break under
/// `--features read-side` alone. `FencingToken`/`ReservationError` live in
/// `ego-persistence-api`, an unconditional dependency of this crate, so this
/// module — created to end exactly this kind of per-store duplication — can
/// host both functions with no new edge.
pub(crate) fn token_for_storage(token: FencingToken) -> Result<i64, ReservationError> {
    i64::try_from(token.value()).map_err(|_| ReservationError::FencingExhausted)
}

/// Rebuilds a token from the column, refusing a value no writer of ours could
/// produce — mirrors `PostgresOperationReservationStore::token_from_storage`.
pub(crate) fn token_from_storage(raw: i64) -> Result<FencingToken, ReservationError> {
    if raw <= 0 {
        return Err(ReservationError::Backend(format!(
            "stored fencing_token {raw} is not positive; the sequence starts at 1"
        )));
    }
    let value = u64::try_from(raw).map_err(|_| {
        ReservationError::Backend(format!("stored fencing_token {raw} is not representable"))
    })?;
    Ok(FencingToken::from_value(value))
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

    /// Stoolap raises this when a racer loses the OCC check at commit on an
    /// UPDATE. It is the message the concurrent-takeover claim test hit, and
    /// classifying it as fatal made a lost race look like data corruption.
    #[test]
    fn is_write_conflict_recognizes_an_occ_update_conflict_at_commit() {
        assert!(is_write_conflict(&stoolap::Error::Internal {
            message: "write conflict: row 1 was modified by another transaction".into()
        }));
    }

    /// The INSERT counterpart of the same OCC check.
    #[test]
    fn is_write_conflict_recognizes_an_occ_insert_conflict_at_commit() {
        assert!(is_write_conflict(&stoolap::Error::Internal {
            message: "write conflict: row 7 was concurrently inserted by another transaction"
                .into()
        }));
    }

    /// The pessimistic write-claim conflict, raised before commit.
    #[test]
    fn is_write_conflict_recognizes_a_pessimistic_write_claim_conflict() {
        assert!(is_write_conflict(&stoolap::Error::Internal {
            message: "row 1 has uncommitted changes from transaction 42".into()
        }));
    }

    /// `Internal` is Stoolap's catch-all, so recognizing it wholesale would
    /// turn every genuine internal failure into a retry. Only the conflict
    /// messages above are retry-safe.
    #[test]
    fn is_write_conflict_fails_loud_for_an_unrecognized_internal_error() {
        assert!(!is_write_conflict(&stoolap::Error::Internal {
            message: "checksum mismatch reading page 12".into()
        }));
    }

    #[test]
    fn is_write_conflict_fails_loud_for_an_unrecognized_error() {
        assert!(!is_write_conflict(&stoolap::Error::TableNotFound(
            "aggregates".into()
        )));
    }

    #[test]
    fn dsn_declares_sync_full_rejects_a_path_containing_the_text_without_the_param() {
        assert!(!dsn_declares_sync_full("file:///tmp/db"));
        assert!(!dsn_declares_sync_full("file:///tmp/no_sync=full_here/db"));
        assert!(dsn_declares_sync_full(&dsn_for(Path::new("/tmp/db"))));
    }
}
