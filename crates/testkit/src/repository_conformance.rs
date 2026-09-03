//! `assert_repository_conformance` — a shared conformance check for
//! [`Repository`] implementations.
//!
//! It belongs here rather than beside any single adapter, for the same reason
//! `assert_event_store_conformance` does (`event_store.rs`): `Repository` has
//! had exactly two implementations — `InMemoryRepository` and
//! `PostgreSQLRepository` — with no shared harness judging them against one
//! contract, and a third (`StoolapRepository`) is about to exist. Adding a
//! third implementation without a harness triples the surface for the same
//! class of divergence `event_store.rs`'s own doc comment records.

use ego_domain::persistence::{PersistenceError, Repository};
use serde::{Deserialize, Serialize};

/// A minimal aggregate the harness owns, so all three `Repository`
/// implementations are judged against the same payload shape and construct
/// the same deserializer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformanceAggregate {
    /// The aggregate's only field — content is not under test, identity and
    /// versioning are.
    pub value: String,
}

/// Builds a [`ConformanceAggregate`] carrying `value`.
pub fn conformance_aggregate(value: &str) -> ConformanceAggregate {
    ConformanceAggregate {
        value: value.to_string(),
    }
}

/// Asserts that a [`Repository`] implementation honours the parts of the
/// contract that are about *identity and versioning* — which rows belong to
/// which tenant scope, how a version advances, and what a lost race reports.
///
/// The caller passes one repository instance; every scenario uses its own
/// aggregate id, so the harness needs no way to build a fresh repository —
/// which matters because construction is fallible for Stoolap, infallible for
/// Memory, and pool-dependent for PostgreSQL.
///
/// # What is checked, and what is deliberately not
///
/// Checked: a fresh save starts at version one, sequential saves advance the
/// version by exactly one, a stale expected version is rejected as a
/// truthful conflict, a load returns the most recently saved content, an
/// absent load and an absent delete both report not-found, delete is
/// permanent, an empty tenant identifier is rejected on every method, the
/// systemwide (tenant-less) scope round-trips exactly like a named tenant,
/// two different tenants sharing one aggregate id never collide, and a
/// tenant scope and the systemwide scope sharing one aggregate id never
/// collide either.
///
/// Deliberately **not** checked, each for a stated reason:
///
/// - **A fresh aggregate saved with a non-zero expected version.** The two
///   previously-shipped implementations disagree on this exact case:
///   `InMemoryRepository` reports a conflict, `PostgreSQLRepository` silently
///   accepts the write. Reconciling that disagreement is its own change, not
///   a scenario this harness may assert either side of.
/// - **Durability.** Not part of `Repository`'s contract — `is_durable()`
///   does not exist on this trait. Pinned per-adapter instead.
/// - **Concurrency.** A shared harness cannot construct a second handle onto
///   the same backing store without knowing the backend. Pinned per-adapter.
/// - **Payload shape.** `Repository` is generic over the aggregate type;
///   asserting a serialization format would test `serde`, not the port.
///
/// # Panics
///
/// Panics with a descriptive message on the first divergence from the
/// contract.
pub fn assert_repository_conformance<R>(repository: &mut R)
where
    R: Repository<ConformanceAggregate> + ?Sized,
{
    let tenant = Some("conformance-tenant");

    // --- A fresh save starts at version one, and sequential saves advance
    //     the version by exactly one each time ---------------------------
    let first = repository
        .save("advances", conformance_aggregate("v1"), tenant, 0)
        .expect("a fresh aggregate saved with expected_version 0 must succeed");
    assert_eq!(first, 1, "a fresh save must start at version one");

    let second = repository
        .save("advances", conformance_aggregate("v2"), tenant, 1)
        .expect("a sequential save at the current version must succeed");
    assert_eq!(
        second, 2,
        "each successful save must advance the version by exactly one"
    );

    let third = repository
        .save("advances", conformance_aggregate("v3"), tenant, 2)
        .expect("a second sequential save at the current version must succeed");
    assert_eq!(third, 3, "the version must keep advancing by exactly one");

    // --- A stale expected_version conflicts, truthfully -------------------
    let stale = repository.save("advances", conformance_aggregate("stale"), tenant, 1);
    match stale {
        Err(PersistenceError::Conflict {
            expected, actual, ..
        }) => {
            assert_eq!(
                expected, 1,
                "the conflict must report the version the caller expected"
            );
            assert_eq!(
                actual, 3,
                "the conflict must report the version actually stored, not the expected one"
            );
        }
        other => panic!("a stale expected_version must be rejected as a conflict, got {other:?}"),
    }

    // --- A load returns the most recently saved content --------------------
    let loaded = repository
        .load("advances", tenant)
        .expect("a saved aggregate must be loadable");
    assert_eq!(
        loaded,
        conformance_aggregate("v3"),
        "load must return the most recently saved content, not an earlier one"
    );

    // --- Loading and deleting an absent aggregate both report not-found ----
    match repository.load("never-saved", tenant) {
        Err(PersistenceError::NotFound { aggregate_id }) => {
            assert_eq!(aggregate_id, "never-saved");
        }
        other => panic!("loading an absent aggregate must report not-found, got {other:?}"),
    }
    match repository.delete("never-saved", tenant) {
        Err(PersistenceError::NotFound { aggregate_id }) => {
            assert_eq!(aggregate_id, "never-saved");
        }
        other => panic!("deleting an absent aggregate must report not-found, got {other:?}"),
    }

    // --- Delete removes the aggregate permanently ---------------------------
    repository
        .save("to-delete", conformance_aggregate("v1"), tenant, 0)
        .expect("saving a fresh aggregate to delete must succeed");
    repository
        .delete("to-delete", tenant)
        .expect("deleting an aggregate that exists must succeed");
    match repository.load("to-delete", tenant) {
        Err(PersistenceError::NotFound { .. }) => {}
        other => panic!("a deleted aggregate must load as not-found, got {other:?}"),
    }

    // --- An empty tenant identifier is rejected, never coerced, on every
    //     method ------------------------------------------------------------
    let empty_tenant = Some("");
    assert!(
        matches!(
            repository.save("empty-tenant", conformance_aggregate("v1"), empty_tenant, 0),
            Err(PersistenceError::MissingTenant)
        ),
        "save must reject an empty tenant identifier as MissingTenant"
    );
    assert!(
        matches!(
            repository.load("empty-tenant", empty_tenant),
            Err(PersistenceError::MissingTenant)
        ),
        "load must reject an empty tenant identifier as MissingTenant"
    );
    assert!(
        matches!(
            repository.delete("empty-tenant", empty_tenant),
            Err(PersistenceError::MissingTenant)
        ),
        "delete must reject an empty tenant identifier as MissingTenant"
    );

    // --- The systemwide scope round-trips through save, load, save, and
    //     delete, exactly as a named tenant scope does ----------------------
    const SYSTEMWIDE_ID: &str = "systemwide-aggregate";
    let systemwide_first = repository
        .save(SYSTEMWIDE_ID, conformance_aggregate("v1"), None, 0)
        .expect("the first systemwide save must succeed and start at version one");
    assert_eq!(systemwide_first, 1);

    let systemwide_loaded = repository
        .load(SYSTEMWIDE_ID, None)
        .expect("a systemwide load must find the row it just wrote");
    assert_eq!(systemwide_loaded, conformance_aggregate("v1"));

    let systemwide_second = repository
        .save(SYSTEMWIDE_ID, conformance_aggregate("v2"), None, 1)
        .expect(
            "the second systemwide save must see version one from the first — a \
             scope invisible to its own version check would return one twice",
        );
    assert_eq!(systemwide_second, 2);

    let systemwide_stale = repository.save(SYSTEMWIDE_ID, conformance_aggregate("v3"), None, 1);
    assert!(
        matches!(systemwide_stale, Err(PersistenceError::Conflict { .. })),
        "a stale expected_version must conflict under the systemwide scope too, \
         got {systemwide_stale:?}"
    );

    repository
        .delete(SYSTEMWIDE_ID, None)
        .expect("a systemwide delete must find and remove the row it just wrote");
    match repository.load(SYSTEMWIDE_ID, None) {
        Err(PersistenceError::NotFound { .. }) => {}
        other => panic!(
            "the systemwide aggregate must be gone after delete, got {other:?}"
        ),
    }

    // --- Two different tenants sharing one aggregate identity do not
    //     collide -----------------------------------------------------------
    const SHARED_ID: &str = "shared-aggregate-id";
    repository
        .save(SHARED_ID, conformance_aggregate("tenant-a"), Some("tenant-a"), 0)
        .expect("tenant A's save must succeed");
    repository
        .save(SHARED_ID, conformance_aggregate("tenant-b"), Some("tenant-b"), 0)
        .expect(
            "tenant B's save must succeed independently — same aggregate id, a \
             different tenant, so this must not collide with tenant A's row",
        );
    assert_eq!(
        repository
            .load(SHARED_ID, Some("tenant-a"))
            .expect("tenant A's row must be found"),
        conformance_aggregate("tenant-a")
    );
    assert_eq!(
        repository
            .load(SHARED_ID, Some("tenant-b"))
            .expect("tenant B's row must be found"),
        conformance_aggregate("tenant-b"),
        "tenant B's own content, not tenant A's — a collision would return tenant A's value"
    );
    match repository.load(SHARED_ID, None) {
        Err(PersistenceError::NotFound { .. }) => {}
        other => panic!(
            "a systemwide load must not find either tenant's row, got {other:?}"
        ),
    }

    // --- A tenant scope and the systemwide scope sharing one aggregate
    //     identity do not collide --------------------------------------------
    const SHARED_ID_2: &str = "scoped-vs-systemwide";
    repository
        .save(SHARED_ID_2, conformance_aggregate("scoped"), Some("tenant-a"), 0)
        .expect("the tenant-scoped save must succeed");
    repository
        .save(SHARED_ID_2, conformance_aggregate("systemwide"), None, 0)
        .expect(
            "the systemwide save must succeed independently — same aggregate id, \
             no tenant, so this must not collide with the tenant-scoped row",
        );
    assert_eq!(
        repository
            .load(SHARED_ID_2, Some("tenant-a"))
            .expect("the tenant-scoped row must be found"),
        conformance_aggregate("scoped")
    );
    assert_eq!(
        repository
            .load(SHARED_ID_2, None)
            .expect("the systemwide row must be found"),
        conformance_aggregate("systemwide"),
        "the systemwide scope's own content, not the tenant-scoped one's"
    );
    repository
        .delete(SHARED_ID_2, None)
        .expect("the systemwide row must be deleted");
    assert_eq!(
        repository
            .load(SHARED_ID_2, Some("tenant-a"))
            .expect("the tenant-scoped row must survive the systemwide delete"),
        conformance_aggregate("scoped")
    );
}
