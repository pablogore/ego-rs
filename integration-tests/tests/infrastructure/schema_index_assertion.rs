//! **Guarantee:** every table whose identity is unique *per tenant* and *per
//! systemwide stream* carries both halves of the partial-unique-index pair that
//! enforces it, with the exact columns, in the exact order, unique, over exactly
//! complementary NULL predicates — as PostgreSQL itself reports them.
//!
//! **Layers traversed:** the framework's own migrations → a real PostgreSQL →
//! `pg_index`, `pg_class` and `pg_attribute`.
//!
//! # The catalog, not the migration source
//!
//! These queries ask the server what it is actually enforcing. Reading the `.sql`
//! files back and matching text would only prove that a file says what it says,
//! which is not the property anything depends on: a migration can be shadowed by
//! an earlier one, an `IF NOT EXISTS` can silently decline to replace a
//! differently-shaped index that already exists, and a column can be dropped and
//! re-added underneath a name that still matches. What the stores depend on is the
//! shape the catalog reports, so that is what is read.
//!
//! # Why this file exists separately from the behavioural tests
//!
//! `conflict_from_postgres.rs` and the receipt-gate scenarios prove the guarantees
//! hold by provoking them. That is the primary evidence, and it would keep passing
//! if someone replaced a pair of partial indexes with something that happens to
//! reject the same inserts for a different reason — a wider index, a trigger, a
//! constraint with different NULL semantics. This file pins the *shape*: which
//! columns, in which order, unique or not, and how NULL is treated.
//!
//! Those behavioural tests are also scoped by construction.
//! `conflict_from_postgres` runs entirely under one tenant, so it loads exactly
//! one half of one pair and stays green when the systemwide half is deleted —
//! measured, and recorded in that file. This one is the only place the *other*
//! halves are load-bearing.
//!
//! # Where this file went, and why it is back
//!
//! An earlier version of it lived in the old root-workspace integration crate and
//! was deleted wholesale by #274, along with every other target that needed Docker
//! — a deliberate deferral, not a judgement that the coverage was worthless. It is
//! reconstructed here, in the independent workspace that now owns
//! infrastructure-backed tests, and extended: the old registry held `events`
//! alone, because the reservation and receipt tables did not exist yet. Both are
//! registered now.
//!
//! # The strategy being pinned, and the server it is pinned on
//!
//! On PostgreSQL 15 and later a single `CREATE UNIQUE INDEX ... NULLS NOT
//! DISTINCT` expresses "two NULL tenants are the same tenant" directly. This
//! framework declares PostgreSQL 14 as its floor, where that syntax does not exist
//! and `pg_index` has no `indnullsnotdistinct` column to inspect. So the semantics
//! are built from two partial unique indexes over complementary predicates.
//!
//! The runner provisions PostgreSQL 16, not 14. Worth saying plainly rather than
//! leaving implied, because the floor is the entire reason the pair exists and 16
//! on its own would not have required it. What is asserted here is therefore the
//! *strategy* — the pair, its complementarity, and each half's exact column list —
//! which must hold on every supported server. Nothing here reads
//! `indnullsnotdistinct`, so these assertions mean the same thing on 14 and on 16.
//!
//! # Two defences against the same failure, catching opposite halves of it
//!
//! [`every_registered_table_has_a_complete_uniqueness_pair`] walks a hand-written
//! registry. [`no_table_carries_a_lopsided_half_of_a_uniqueness_pair`] walks the
//! catalog instead. Neither subsumes the other:
//!
//! - A table that should have a pair and has **no index at all** is invisible to
//!   any amount of discovery — there is nothing to discover. Only the registry
//!   names it.
//! - A table **nobody registered** that grew one lopsided index is invisible to
//!   the registry. Only discovery finds it. A unique index over
//!   `tenant_id IS NOT NULL` with no companion for the NULL partition silently
//!   permits unlimited duplicates there, which is precisely the failure that
//!   already occurred once in this schema.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::collections::BTreeMap;

use ego_integration_tests::isolated_database;
use sqlx::PgPool;

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
///
/// `probes` travels with the pair on purpose. The complementarity test needs one
/// row on each side of the predicate boundary, and holding those inserts in a
/// second list is how a newly registered table would end up silently skipped by
/// it — present in the shape assertions, absent from the partition ones.
struct ExpectedPair {
    table: &'static str,
    tenant_half: (&'static str, &'static [&'static str]),
    systemwide_half: (&'static str, &'static [&'static str]),
    /// A legal row with a tenant, then a legal row without one, with distinct
    /// identities so neither half of the pair refuses them.
    probes: (&'static str, &'static str),
}

/// The registry of tables that must carry a complete pair.
///
/// Hand-maintained, deliberately — see the module docs for why discovery cannot
/// replace it. The systemwide half of every pair omits `tenant_id` on purpose: its
/// predicate already fixes that column to NULL for every row the index contains,
/// so including it would index a constant and discriminate nothing. The asymmetry
/// is pinned here rather than left to be rediscovered as a surprise.
const EXPECTED_PAIRS: &[ExpectedPair] = &[
    ExpectedPair {
        table: "events",
        tenant_half: (
            "ux_events_identity_tenant",
            &["tenant_id", "aggregate_type", "aggregate_id", "version"],
        ),
        systemwide_half: (
            "ux_events_identity_systemwide",
            &["aggregate_type", "aggregate_id", "version"],
        ),
        probes: (
            "INSERT INTO events \
               (aggregate_type, aggregate_id, tenant_id, version, event_type, payload) \
             VALUES ('order', 'probe-tenant', 'tenant-1', 1, 'Probed', '{}'::jsonb)",
            "INSERT INTO events \
               (aggregate_type, aggregate_id, tenant_id, version, event_type, payload) \
             VALUES ('order', 'probe-systemwide', NULL, 1, 'Probed', '{}'::jsonb)",
        ),
    },
    ExpectedPair {
        table: "operation_reservations",
        tenant_half: (
            "ux_operation_reservations_identity_tenant",
            &["tenant_id", "operation_key"],
        ),
        systemwide_half: (
            "ux_operation_reservations_identity_systemwide",
            &["operation_key"],
        ),
        probes: (
            "INSERT INTO operation_reservations \
               (tenant_id, operation_key, fingerprint, owner_id, fencing_token, \
                lease_until, state) \
             VALUES ('tenant-1', 'probe-tenant', 'fp', 'owner', 1, NOW(), 'in_progress')",
            "INSERT INTO operation_reservations \
               (tenant_id, operation_key, fingerprint, owner_id, fencing_token, \
                lease_until, state) \
             VALUES (NULL, 'probe-systemwide', 'fp', 'owner', 1, NOW(), 'in_progress')",
        ),
    },
    ExpectedPair {
        table: "operation_receipts",
        tenant_half: (
            "ux_operation_receipts_identity_tenant",
            &[
                "tenant_id",
                "aggregate_type",
                "aggregate_id",
                "operation_key",
            ],
        ),
        systemwide_half: (
            "ux_operation_receipts_identity_systemwide",
            &["aggregate_type", "aggregate_id", "operation_key"],
        ),
        probes: (
            "INSERT INTO operation_receipts \
               (tenant_id, aggregate_type, aggregate_id, operation_key, fingerprint, \
                outcome_kind) \
             VALUES ('tenant-1', 'order', 'probe', 'probe-tenant', 'fp', 'no_events')",
            "INSERT INTO operation_receipts \
               (tenant_id, aggregate_type, aggregate_id, operation_key, fingerprint, \
                outcome_kind) \
             VALUES (NULL, 'order', 'probe', 'probe-systemwide', 'fp', 'no_events')",
        ),
    },
];

/// One catalog row as `pg_index` yields it: index name, uniqueness, the partial
/// predicate (absent for a total index) and the ordered column list (absent only
/// if the index has no key columns, which cannot happen here).
type CatalogRow = (String, bool, Option<String>, Option<Vec<String>>);

const TENANT_PREDICATE: &str = "(tenant_id IS NOT NULL)";
const SYSTEMWIDE_PREDICATE: &str = "(tenant_id IS NULL)";

/// Every index on `table`, with its ordered column list, uniqueness and partial
/// predicate, read from the catalog.
///
/// `indkey` holds the columns in index order, so `WITH ORDINALITY` is what
/// preserves that order through the join to `pg_attribute` — without it the column
/// list would come back in whatever order the join produced, and an assertion
/// about ordering would be meaningless rather than merely weaker.
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
#[tokio::test]
async fn every_registered_table_has_a_complete_uniqueness_pair() {
    let db = isolated_database().await;
    let pool = db.pool().await;

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
                "`{name}` must be UNIQUE — a non-unique index over the identity enforces \
                 nothing, which is exactly the state this schema was in before"
            );
            assert_eq!(
                shape.columns, columns,
                "`{name}` must cover exactly these columns in exactly this order; order is \
                 part of the contract because it determines which prefix lookups the index \
                 can serve"
            );
            assert_eq!(
                shape.predicate, predicate,
                "`{name}` must be partial over `{predicate}`. That predicate is how \
                 NULL-tenant semantics are expressed on a server without NULLS NOT \
                 DISTINCT: it is what makes two NULL tenants collide instead of being \
                 treated as distinct"
            );
        }
    }

    db.close().await;
}

/// For every registered table, the two predicates partition it: complementary, so
/// no row escapes and no row is covered twice.
///
/// Asserted against the server rather than by reading the predicates, because
/// "these two strings look like opposites" is a claim about text. This evaluates
/// them over a table populated with both kinds of row and counts.
#[tokio::test]
async fn the_two_predicates_cover_every_registered_table_with_no_gap_and_no_overlap() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    for pair in EXPECTED_PAIRS {
        for probe in [pair.probes.0, pair.probes.1] {
            sqlx::query(probe).execute(&pool).await.unwrap_or_else(|e| {
                panic!(
                    "the probe row must insert into `{}`: {e}\n\n{probe}",
                    pair.table
                )
            });
        }

        // The table name comes from the const registry above, never from input; it
        // cannot be bound as a parameter because it is an identifier, not a value.
        let (total, tenant_half, systemwide_half, both): (i64, i64, i64, i64) =
            sqlx::query_as(&format!(
                "SELECT COUNT(*), \
                        COUNT(*) FILTER (WHERE tenant_id IS NOT NULL), \
                        COUNT(*) FILTER (WHERE tenant_id IS NULL), \
                        COUNT(*) FILTER (WHERE tenant_id IS NOT NULL AND tenant_id IS NULL) \
                 FROM {}",
                pair.table
            ))
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("counting `{}` must succeed: {e}", pair.table));

        // Before the partition claim, the premise it rests on. With no row on one
        // side of the boundary the two assertions below would hold over an empty
        // half and prove nothing about it.
        assert!(
            tenant_half >= 1 && systemwide_half >= 1,
            "`{}` must hold at least one row on each side of the predicate boundary for \
             this test to mean anything; got {tenant_half} with a tenant and \
             {systemwide_half} without",
            pair.table
        );
        assert_eq!(
            tenant_half + systemwide_half,
            total,
            "every row in `{}` must fall under exactly one of the two predicates — a row \
             under neither would be a row no index constrains",
            pair.table
        );
        assert_eq!(
            both, 0,
            "no row in `{}` may fall under both predicates, or the two indexes would be \
             enforcing overlapping claims about the same rows",
            pair.table
        );
    }

    db.close().await;
}

/// Any table that carries one half of a tenant-partitioned uniqueness pair
/// carries the other half too.
///
/// This is the half of the contract the registry cannot express: it catches a
/// table nobody thought to register that grew one lopsided index. See the module
/// docs for why both defences are kept.
#[tokio::test]
async fn no_table_carries_a_lopsided_half_of_a_uniqueness_pair() {
    let db = isolated_database().await;
    let pool = db.pool().await;

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

    let mut tables: BTreeMap<String, Vec<String>> = BTreeMap::new();
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
            "table `{table}` carries only part of a tenant-partitioned uniqueness pair: \
             {found:?}. One half without the other leaves its complementary partition \
             unconstrained — for a missing NULL half that means unlimited duplicate \
             identities among tenant-less rows"
        );
    }

    // Not an assertion about the count, which will grow. This states that the
    // discovery query works at all: if it silently matched nothing — a renamed
    // catalog column, a predicate rendered differently, a changed schema name —
    // every check above would pass over an empty map, and this test would report
    // success having examined nothing.
    //
    // Anchored to the registry rather than to one table, so a discovery query that
    // regressed into matching only some pairs is caught too.
    for pair in EXPECTED_PAIRS {
        assert!(
            tables.contains_key(pair.table),
            "the discovery query found no pair on `{}`, so it is not matching what it \
             claims to. Found pairs on: {:?}",
            pair.table,
            tables.keys().collect::<Vec<_>>()
        );
    }

    db.close().await;
}
