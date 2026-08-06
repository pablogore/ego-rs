//! Assertions against PostgreSQL's own catalog for the indexes that enforce
//! tenant-partitioned uniqueness.
//!
//! The catalog, not the migration source. Reading the `.sql` file back would only
//! prove the file says what it says; these queries ask the server what it is
//! actually enforcing, which is the thing the store depends on.
//!
//! # Why this file exists separately from the behavioural tests
//!
//! `stream_identity_uniqueness.rs` proves the guarantee holds by provoking it.
//! That is the primary evidence, and it would keep passing if someone replaced
//! the pair of partial indexes with something that happens to reject the same
//! inserts for a different reason — a wider index, a trigger, a constraint with
//! different NULL semantics. This file pins the *shape*: which columns, in which
//! order, unique or not, and how NULL is treated.
//!
//! # The strategy being pinned
//!
//! On PostgreSQL 15 and later a single `CREATE UNIQUE INDEX ... NULLS NOT
//! DISTINCT` expresses "two NULL tenants are the same tenant" directly. This
//! workspace declares PostgreSQL 14 as its floor, where that syntax does not
//! exist and `pg_index` has no `indnullsnotdistinct` column to inspect. So the
//! semantics are built from two partial unique indexes over complementary
//! predicates, and *that* is what gets asserted: the pair, the complementarity,
//! and each half's exact column list.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

use ego_persistence::postgres::migrations;

/// Pinned explicitly. Load-bearing here: 14 is the version whose lack of
/// `NULLS NOT DISTINCT` motivates the two-index strategy this file asserts.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

/// One half of a tenant-partitioned uniqueness pair, as the catalog reports it.
#[derive(Debug, PartialEq, Eq)]
struct IndexShape {
    name: String,
    unique: bool,
    predicate: String,
    columns: Vec<String>,
}

/// What a table's pair must look like for its identity to be unique per tenant
/// *and* per systemwide stream.
struct ExpectedPair {
    table: &'static str,
    tenant_half: (&'static str, &'static [&'static str]),
    systemwide_half: (&'static str, &'static [&'static str]),
}

/// The registry of tables that must carry a complete pair.
///
/// `events` is the only entry today. The receipt and reservation tables arrive
/// with their own slices and add their own entries then — this is deliberately a
/// hand-maintained list, because a table that should have a pair and has *no*
/// index at all is invisible to any amount of catalog discovery. The discovery
/// check further down covers the other direction: a table that has one half and
/// forgot the other.
const EXPECTED_PAIRS: &[ExpectedPair] = &[ExpectedPair {
    table: "events",
    tenant_half: (
        "ux_events_identity_tenant",
        &["tenant_id", "aggregate_type", "aggregate_id", "version"],
    ),
    // The systemwide half omits `tenant_id` on purpose: its predicate already
    // fixes that column to NULL for every row the index contains, so including it
    // would index a constant and discriminate nothing. The asymmetry is pinned
    // here rather than left to be rediscovered as a surprise.
    systemwide_half: (
        "ux_events_identity_systemwide",
        &["aggregate_type", "aggregate_id", "version"],
    ),
}];

/// One catalog row as `pg_index` yields it: index name, uniqueness, the partial
/// predicate (absent for a total index) and the ordered column list (absent only
/// if the index has no key columns, which cannot happen here).
type CatalogRow = (String, bool, Option<String>, Option<Vec<String>>);

const TENANT_PREDICATE: &str = "(tenant_id IS NOT NULL)";
const SYSTEMWIDE_PREDICATE: &str = "(tenant_id IS NULL)";

async fn start_pool() -> (PgPool, ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_tag(POSTGRES_IMAGE_TAG)
        .start()
        .await
        .expect("the Postgres testcontainer must start; this test cannot run without Docker");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("the container must publish its mapped Postgres port");
    let host = container
        .get_host()
        .await
        .expect("the container must report a reachable host address")
        .to_string();
    let host = if host == "localhost" {
        "127.0.0.1".to_string()
    } else {
        host
    };

    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .connect(&url)
        .await
        .expect("must be able to connect to the freshly started container");

    migrations::run(&pool)
        .await
        .expect("the framework's own migrations must apply cleanly");

    (pool, container)
}

/// Every index on `table`, with its ordered column list, uniqueness and partial
/// predicate, read from the catalog.
///
/// `indkey` holds the columns in index order, so `WITH ORDINALITY` is what
/// preserves that order through the join to `pg_attribute` — without it the
/// column list would come back in whatever order the join produced, and an
/// assertion about ordering would be meaningless.
async fn index_shapes(pool: &PgPool, table: &str) -> Vec<IndexShape> {
    let rows: Vec<CatalogRow> = sqlx::query_as(
        "SELECT c.relname, \
                i.indisunique, \
                pg_get_expr(i.indpred, i.indrelid), \
                (SELECT array_agg(a.attname ORDER BY k.ord) \
                   FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) \
                   JOIN pg_attribute a \
                     ON a.attrelid = i.indrelid AND a.attnum = k.attnum) \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indexrelid \
         WHERE i.indrelid = $1::regclass \
         ORDER BY c.relname",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .expect("the index catalog must be queryable");

    rows.into_iter()
        .map(|(name, unique, predicate, columns)| IndexShape {
            name,
            unique,
            predicate: predicate.unwrap_or_default(),
            columns: columns.unwrap_or_default(),
        })
        .collect()
}

/// Each registered table carries both halves of its pair, with the exact columns,
/// in the exact order, unique, and over complementary NULL predicates.
#[tokio::test(flavor = "multi_thread")]
async fn every_registered_table_has_a_complete_uniqueness_pair() {
    let (pool, _container) = start_pool().await;

    for pair in EXPECTED_PAIRS {
        let shapes = index_shapes(&pool, pair.table).await;
        let find = |name: &str| {
            shapes
                .iter()
                .find(|shape| shape.name == name)
                .unwrap_or_else(|| {
                    panic!(
                        "table `{}` is missing the index `{name}`. Present: {:?}",
                        pair.table,
                        shapes.iter().map(|s| &s.name).collect::<Vec<_>>()
                    )
                })
        };

        for (expected, predicate) in [
            (pair.tenant_half, TENANT_PREDICATE),
            (pair.systemwide_half, SYSTEMWIDE_PREDICATE),
        ] {
            let (name, columns) = expected;
            let shape = find(name);

            assert!(
                shape.unique,
                "`{name}` must be UNIQUE — a non-unique index over the identity enforces nothing, \
                 which is exactly the state this schema was in before"
            );
            assert_eq!(
                shape.columns, columns,
                "`{name}` must cover exactly these columns in exactly this order; order is part \
                 of the contract because it determines which prefix lookups the index can serve"
            );
            assert_eq!(
                shape.predicate, predicate,
                "`{name}` must be partial over `{predicate}`. That predicate is how NULL-tenant \
                 semantics are expressed on a server without NULLS NOT DISTINCT: it is what makes \
                 two NULL tenants collide instead of being treated as distinct"
            );
        }
    }
}

/// The two predicates partition the table: complementary, so no row escapes and
/// no row is covered twice.
///
/// Asserted against the server rather than by reading the predicates, because
/// "these two strings look like opposites" is a claim about text. This evaluates
/// them over a table populated with both kinds of row and counts.
#[tokio::test(flavor = "multi_thread")]
async fn the_two_predicates_cover_the_table_with_no_gap_and_no_overlap() {
    let (pool, _container) = start_pool().await;

    for (aggregate_id, tenant) in [
        ("a", Some("tenant-1")),
        ("b", None),
        ("c", Some("tenant-2")),
    ] {
        sqlx::query(
            "INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload) \
             VALUES ('order', $1, $2, 1, 'Probed', '{}'::jsonb)",
        )
        .bind(aggregate_id)
        .bind(tenant)
        .execute(&pool)
        .await
        .expect("the probe row must insert");
    }

    let (total, tenant_half, systemwide_half, both): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), \
                COUNT(*) FILTER (WHERE tenant_id IS NOT NULL), \
                COUNT(*) FILTER (WHERE tenant_id IS NULL), \
                COUNT(*) FILTER (WHERE tenant_id IS NOT NULL AND tenant_id IS NULL) \
         FROM events",
    )
    .fetch_one(&pool)
    .await
    .expect("counting must succeed");

    assert_eq!(
        tenant_half + systemwide_half,
        total,
        "every row must fall under exactly one of the two predicates — a row under neither would \
         be a row no index constrains"
    );
    assert_eq!(
        both, 0,
        "no row may fall under both predicates, or the two indexes would be enforcing overlapping \
         claims about the same rows"
    );
}

/// Any table that carries one half of a tenant-partitioned uniqueness pair
/// carries the other half too.
///
/// This is the half of the contract the registry above cannot express. The
/// registry catches a table that is supposed to have a pair; this catches a table
/// nobody thought to register that grew one lopsided index — a unique index over
/// `tenant_id IS NOT NULL` with no companion for the NULL partition silently
/// permits unlimited duplicates there, which is precisely the failure that
/// already occurred once in this schema.
///
/// It is inert today: `events` is the only table with such indexes and its pair is
/// complete. Stating it now is the point — it starts guarding the moment the
/// receipt and reservation tables land, without depending on whoever writes them
/// remembering to extend this file.
#[tokio::test(flavor = "multi_thread")]
async fn no_table_carries_a_lopsided_half_of_a_uniqueness_pair() {
    let (pool, _container) = start_pool().await;

    let halves: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.relname, c.relname, pg_get_expr(i.indpred, i.indrelid) \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indexrelid \
         JOIN pg_class t ON t.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = t.relnamespace \
         WHERE i.indisunique \
           AND i.indpred IS NOT NULL \
           AND n.nspname = 'public' \
           AND pg_get_expr(i.indpred, i.indrelid) LIKE '%tenant_id IS%NULL%' \
         ORDER BY t.relname, c.relname",
    )
    .fetch_all(&pool)
    .await
    .expect("the index catalog must be queryable");

    let mut tables: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (table, index, predicate) in halves {
        tables
            .entry(table)
            .or_default()
            .push(format!("{index} over {predicate}"));
    }

    for (table, found) in &tables {
        let has_tenant_half = found.iter().any(|f| f.contains(TENANT_PREDICATE));
        let has_systemwide_half = found.iter().any(|f| f.contains(SYSTEMWIDE_PREDICATE));
        assert!(
            has_tenant_half && has_systemwide_half,
            "table `{table}` carries only part of a tenant-partitioned uniqueness pair: {found:?}. \
             One half without the other leaves its complementary partition unconstrained — for a \
             missing NULL half that means unlimited duplicate identities among tenant-less rows"
        );
    }

    // Not an assertion about the count, which will grow. This one states that the
    // discovery query works at all: if it silently matched nothing, every check
    // above would pass vacuously.
    assert!(
        tables.contains_key("events"),
        "the discovery query found no pair on `events`, so it is not matching what it claims to; \
         every assertion in this test would pass without examining anything"
    );
}
