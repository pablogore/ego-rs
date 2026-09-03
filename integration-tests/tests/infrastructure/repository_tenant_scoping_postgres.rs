//! **Guarantee:** `PostgreSQLRepository<A, F>`'s tenant-scoped SQL predicates
//! — `save`'s lock-select and upsert, `load`, and `delete` — treat the
//! systemwide scope (`tenant_id` resolved to SQL `NULL`) the same way two
//! concrete tenants are treated: genuinely isolated from every other scope,
//! and genuinely equal to itself across repeated calls. The same guarantee
//! `event_store.rs` and `snapshot.rs` already hold for their own tables
//! (migrations 008/012) — `aggregates` is fixed to match here.
//!
//! **Layers traversed:** `PostgreSQLRepository::save`/`load`/`delete` → real
//! SQL → the `aggregates` table's tenant-partitioned identity.
//!
//! # Why this needs a real PostgreSQL
//!
//! `NULL = NULL` is never `TRUE` in SQL — a plain `tenant_id = $2` predicate
//! can never match a systemwide row, no matter how the request resolves its
//! own tenant. No in-memory double keyed on `Option<TenantId>` can
//! misrepresent this: Rust's own `Option::eq` treats `None == None` as
//! `true`, so an in-memory repository gets the right answer for the wrong
//! reason and could never catch a regression here. Only a real server
//! evaluating real three-valued SQL logic can show it. Likewise, only a real
//! `INSERT ... ON CONFLICT` against the real catalog can show whether the
//! conflict target actually names a matching unique index.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_domain::persistence::{PersistenceError, Repository};
use ego_integration_tests::isolated_database;
use ego_persistence::postgres::PostgreSQLRepository;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// A minimal aggregate payload — this suite's own type, never depended on by
/// production code. The repository is generic over any `Clone + Serialize`
/// type, so nothing about the identity/tenant behaviour under test depends on
/// what the payload actually is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TestAggregate {
    value: String,
}

type Repo = PostgreSQLRepository<
    TestAggregate,
    fn(serde_json::Value) -> Result<TestAggregate, PersistenceError>,
>;

fn repo(pool: PgPool) -> Repo {
    let deserialize: fn(serde_json::Value) -> Result<TestAggregate, PersistenceError> = |value| {
        serde_json::from_value(value)
            .map_err(|e| PersistenceError::Internal(format!("bad payload: {e}")))
    };
    PostgreSQLRepository::new(pool, deserialize)
}

fn aggregate(value: &str) -> TestAggregate {
    TestAggregate {
        value: value.to_string(),
    }
}

/// The case that is broken today: a systemwide-scoped aggregate must
/// round-trip through `save` → `load` → `save` → `delete` exactly the way a
/// tenant-scoped one does.
///
/// The second `save` is what proves the lock-select's bug specifically, not
/// just the read side: a systemwide row the lock-select cannot find is a row
/// `save` treats as brand new every time, so the returned version would be
/// `1` again instead of `2`, and a stale `expected_version` would never
/// conflict no matter what is passed.
#[tokio::test(flavor = "multi_thread")]
async fn systemwide_scope_round_trips_through_save_load_and_delete() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let mut repo = repo(pool.clone());

    const ID: &str = "systemwide-aggregate";

    let v1 = repo
        .save(ID, aggregate("v1"), None, 0)
        .expect("the first systemwide save succeeds and starts at version 1");
    assert_eq!(v1, 1);

    let loaded = repo.load(ID, None).expect(
        "a systemwide load must find the row it just wrote — a plain \
         `tenant_id = NULL` predicate can never match it",
    );
    assert_eq!(loaded, aggregate("v1"));

    let v2 = repo.save(ID, aggregate("v2"), None, 1).expect(
        "the second systemwide save must see version 1 from the first — the \
         lock-select must find the existing row, not silently treat it as new",
    );
    assert_eq!(
        v2, 2,
        "a version of 1 here means the lock-select never found the existing \
         row and the upsert inserted a fresh one instead of updating it"
    );

    let loaded_v2 = repo
        .load(ID, None)
        .expect("the systemwide load still finds the row after the update");
    assert_eq!(loaded_v2, aggregate("v2"));

    // A stale expected_version must still conflict under the systemwide
    // scope, the same way it does for a tenant-scoped aggregate — proving the
    // optimistic-concurrency check is genuinely wired to a real prior read,
    // not silently bypassed because the row was never found.
    let err = repo
        .save(ID, aggregate("v3"), None, 1)
        .expect_err("a stale expected_version must be refused, not silently accepted");
    assert!(
        matches!(err, PersistenceError::Conflict { .. }),
        "must be reported as a conflict, not swallowed or mapped to something else: {err:?}"
    );

    repo.delete(ID, None)
        .expect("a systemwide delete must find and remove the row it just wrote");

    let not_found = repo
        .load(ID, None)
        .expect_err("the row must be gone after a successful delete");
    assert!(
        matches!(not_found, PersistenceError::NotFound { .. }),
        "must report NotFound, not something else: {not_found:?}"
    );

    db.close().await;
}

/// Two concrete tenants, same `aggregate_id`: independent rows, each keeping
/// its own state, neither visible under the other's scope. This also proves
/// the identity is genuinely tenant-scoped rather than globally unique on
/// `aggregate_id` alone — the pre-fix schema's `aggregate_id`-only
/// `PRIMARY KEY` would refuse the second tenant's save outright.
#[tokio::test(flavor = "multi_thread")]
async fn two_tenants_with_the_same_aggregate_id_do_not_collide() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let mut repo = repo(pool.clone());

    const ID: &str = "shared-aggregate-id";

    repo.save(ID, aggregate("tenant-a"), Some("tenant-a"), 0)
        .expect("tenant A's save succeeds");
    repo.save(ID, aggregate("tenant-b"), Some("tenant-b"), 0)
        .expect(
            "tenant B's save must succeed independently — same aggregate_id, \
             a different tenant, so this must not collide with tenant A's row",
        );

    assert_eq!(
        repo.load(ID, Some("tenant-a"))
            .expect("tenant A's row is found"),
        aggregate("tenant-a")
    );
    assert_eq!(
        repo.load(ID, Some("tenant-b"))
            .expect("tenant B's row is found"),
        aggregate("tenant-b"),
        "tenant B's own state, not tenant A's — a collision would return tenant A's value"
    );

    // Cross-scope isolation: neither tenant's row may surface under a request
    // that resolves to no tenant at all.
    let not_found = repo
        .load(ID, None)
        .expect_err("a systemwide load must not find either tenant's row");
    assert!(matches!(not_found, PersistenceError::NotFound { .. }));

    db.close().await;
}

/// A tenant-scoped row and a systemwide row sharing the same `aggregate_id`:
/// independent, neither visible under the other's scope. The complementary
/// half of tenant isolation to the two-concrete-tenants case above.
#[tokio::test(flavor = "multi_thread")]
async fn a_scoped_tenant_and_the_systemwide_scope_do_not_collide() {
    let db = isolated_database().await;
    let pool = db.pool().await;
    let mut repo = repo(pool.clone());

    const ID: &str = "scoped-vs-systemwide";

    repo.save(ID, aggregate("scoped"), Some("tenant-a"), 0)
        .expect("the tenant-scoped save succeeds");
    repo.save(ID, aggregate("systemwide"), None, 0).expect(
        "the systemwide save must succeed independently — same aggregate_id, \
         no tenant, so this must not collide with the tenant-scoped row",
    );

    assert_eq!(
        repo.load(ID, Some("tenant-a"))
            .expect("the tenant-scoped row is found"),
        aggregate("scoped")
    );
    assert_eq!(
        repo.load(ID, None).expect("the systemwide row is found"),
        aggregate("systemwide"),
        "the systemwide scope's own state, not the tenant-scoped one's"
    );

    // Deleting the systemwide row must not touch the tenant-scoped one.
    repo.delete(ID, None)
        .expect("the systemwide row is deleted");
    assert_eq!(
        repo.load(ID, Some("tenant-a"))
            .expect("the tenant-scoped row survives the systemwide delete"),
        aggregate("scoped")
    );

    db.close().await;
}
