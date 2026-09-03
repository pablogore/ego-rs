//! PostgreSQL persistence implementations.

pub mod aggregate_type_backfill;
pub mod event_store;
pub mod migrations;
/// The durable [`ego_domain::read_side::DedupStore`].
pub mod read_side_dedup;
/// The durable [`ego_domain::read_side::OffsetStore`].
pub mod read_side_offset;
/// The durable [`ego_persistence_api::read_side::claim::ReadSideClaimStore`]
/// (PROD-014C).
pub mod read_side_claim;
pub mod repository;

/// The durable operation-reservation store.
pub mod reservation;
pub mod snapshot;

pub use event_store::PostgreSQLEventStore;
pub use read_side_claim::PostgreSQLReadSideClaimStore;
pub use read_side_dedup::PostgreSQLDedupStore;
pub use read_side_offset::PostgreSQLOffsetStore;
pub use repository::PostgreSQLRepository;
pub use snapshot::PostgreSQLSnapshotStore;

/// Coerce an optional tenant identifier into the value bound to SQL queries.
///
/// The tenant-scope rule lives in the domain — see
/// [`ego_domain::persistence::tenant`] for why. Re-exported so this module's
/// existing `use crate::postgres::resolve_tenant;` call sites keep working.
pub(crate) use ego_domain::persistence::resolve_tenant;

/// Whether a storage failure will fail the same way on every retry.
///
/// `Transient` is the default because a retryable failure misreported as
/// `Fatal` stops a projection that would have recovered on its own. The four
/// codes below are the ones a retry cannot help: the migration did not run,
/// the schema drifted, a value does not fit its column, or a row cannot be
/// decoded into the type this crate wrote.
///
/// Shared between [`read_side_offset`] and [`read_side_dedup`] (PROD-014B
/// AD-8) — both `OffsetStore`/`DedupStore` declare the same two-variant
/// `Transient`/`Fatal` split with no defined boundary between them, and
/// leaving each adapter to draw that line on its own is how one of them ends
/// up retrying a missing table forever.
pub(crate) fn is_fatal(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => matches!(
            db.code().as_deref(),
            Some("42P01") // undefined_table — migration 013/014 not applied
                | Some("42703") // undefined_column — schema drift
                | Some("22001") // string_data_right_truncation — over VARCHAR(255)
                | Some("23514") // check_violation
        ),
        sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod is_fatal_tests {
    use super::is_fatal;

    /// A constructed `sqlx::Error::Database` carrying the given SQLSTATE, with
    /// no pool and no connection — `is_fatal` is a pure function over the
    /// error value alone (AD-8, AD-12).
    fn database_error(code: &'static str) -> sqlx::Error {
        #[derive(Debug)]
        struct FakeDbError {
            code: &'static str,
        }

        impl std::fmt::Display for FakeDbError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "fake database error {}", self.code)
            }
        }

        impl std::error::Error for FakeDbError {}

        impl sqlx::error::DatabaseError for FakeDbError {
            fn message(&self) -> &str {
                "fake database error"
            }

            fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
                Some(std::borrow::Cow::Borrowed(self.code))
            }

            fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
                self
            }

            fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
                self
            }

            fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
                self
            }

            fn kind(&self) -> sqlx::error::ErrorKind {
                sqlx::error::ErrorKind::Other
            }
        }

        sqlx::Error::Database(Box::new(FakeDbError { code }))
    }

    #[test]
    fn undefined_table_is_fatal() {
        assert!(is_fatal(&database_error("42P01")));
    }

    #[test]
    fn undefined_column_is_fatal() {
        assert!(is_fatal(&database_error("42703")));
    }

    #[test]
    fn string_data_right_truncation_is_fatal() {
        assert!(is_fatal(&database_error("22001")));
    }

    #[test]
    fn check_violation_is_fatal() {
        assert!(is_fatal(&database_error("23514")));
    }

    #[test]
    fn an_unrelated_sqlstate_is_transient() {
        assert!(!is_fatal(&database_error("40001"))); // serialization_failure
    }

    #[test]
    fn column_decode_is_fatal() {
        let err = sqlx::Error::ColumnDecode {
            index: "offset_value".to_string(),
            source: Box::new(std::io::Error::other("bad column")),
        };
        assert!(is_fatal(&err));
    }

    #[test]
    fn decode_is_fatal() {
        let err = sqlx::Error::Decode(Box::new(std::io::Error::other("bad row")));
        assert!(is_fatal(&err));
    }

    #[test]
    fn pool_timed_out_is_transient() {
        assert!(!is_fatal(&sqlx::Error::PoolTimedOut));
    }

    #[test]
    fn io_error_is_transient() {
        let err = sqlx::Error::Io(std::io::Error::other("connection reset"));
        assert!(!is_fatal(&err));
    }

    #[test]
    fn protocol_error_is_transient() {
        let err = sqlx::Error::Protocol("unexpected message".to_string());
        assert!(!is_fatal(&err));
    }
}
