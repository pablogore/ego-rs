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
//!   already occurred once in this schema. Discovery pairs halves by the identity
//!   they enforce, not merely by the predicate they carry — two indexes, one per
//!   predicate, guarding *different* identities is not a pair.
//!
//! # What "exact columns" has to mean to be worth asserting
//!
//! Two ways a catalog read can report the expected column list while the index
//! enforces something else, both closed here:
//!
//! - **An expression key.** `indkey` stores `0` for one, and `pg_attribute` has no
//!   `attnum = 0`, so an inner join drops it silently. Appending
//!   `lower(fingerprint)` to a pair half narrows the identity — two rows alike in
//!   every registered column can then coexist — while the reported list is
//!   unchanged. The join is outer, the element holds its position, and
//!   `indexprs` is read as a second independent signal.
//! - **`INCLUDE` payload.** `indkey` carries it after the keys, and only the first
//!   `indnkeyatts` enforce uniqueness. Comparing the whole vector would fail on an
//!   added `INCLUDE` column that changes nothing — the mirror mistake, a false red
//!   instead of a false green. Keys and payload are separated and only keys are
//!   compared.
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
    /// The key elements that enforce uniqueness, in index order. An expression
    /// key appears as [`EXPRESSION_KEY`], holding its position rather than
    /// vanishing.
    key_columns: Vec<String>,
    /// The non-key `INCLUDE` payload. Carried so a failure can say what is
    /// present, never compared: it does not participate in uniqueness.
    included_columns: Vec<String>,
    /// Whether any key is an expression, read from `indexprs` independently of
    /// the positional marker above.
    has_expression_key: bool,
    /// The server's own rendering of the index, for failure messages.
    definition: String,
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
/// predicate (absent for a total index), the ordered key elements, the ordered
/// `INCLUDE` payload, whether any key is an expression, and the server's own
/// rendering of the whole index.
type CatalogRow = (
    String,
    bool,
    Option<String>,
    Option<Vec<String>>,
    Option<Vec<String>>,
    bool,
    String,
);

/// One row of the discovery sweep: table name, index name, the partial predicate,
/// and the ordered key elements.
type DiscoveryRow = (String, String, Option<String>, Option<Vec<String>>);

const TENANT_PREDICATE: &str = "(tenant_id IS NOT NULL)";
const SYSTEMWIDE_PREDICATE: &str = "(tenant_id IS NULL)";

/// Stands in for a key that is an expression rather than a plain column.
///
/// An expression key has no name to report, so it needs a placeholder that
/// occupies its position. Anything else silently shortens the key list, which is
/// exactly the failure this constant exists to make impossible — see
/// [`index_shapes`].
const EXPRESSION_KEY: &str = "(expression)";

/// The `SELECT` list shared by the two catalog queries.
///
/// # Why `LEFT JOIN`, and why this is not a stylistic choice
///
/// `indkey` stores **`0`** for a key that is an expression rather than a column.
/// `pg_attribute` has no row with `attnum = 0` — real columns are positive and
/// system columns negative — so an **inner** join drops that element without a
/// trace, and `array_agg` never sees it. An earlier version of this file did
/// exactly that, and the consequence was a false green rather than merely thin
/// coverage:
///
/// ```sql
/// CREATE UNIQUE INDEX ux_operation_receipts_identity_systemwide
///     ON operation_receipts (aggregate_type, aggregate_id, operation_key,
///                            lower(fingerprint))
///     WHERE tenant_id IS NULL;
/// ```
///
/// That index no longer enforces the required identity — two receipts identical
/// in every registered column can coexist if `lower(fingerprint)` differs — and
/// the old query still reported exactly `aggregate_type, aggregate_id,
/// operation_key`, so the "exact columns" assertion passed while the guarantee
/// this file exists to pin was gone.
///
/// So the join is outer, the expression element is preserved in position as
/// [`EXPRESSION_KEY`], and `indexprs IS NOT NULL` is read as an independent
/// signal. `pg_get_indexdef` comes along so a failure can show the real
/// definition instead of a list with a hole in it.
///
/// # Key columns and `INCLUDE` are different things
///
/// `indkey` holds the `INCLUDE` payload after the key columns, and only the first
/// `indnkeyatts` of them participate in uniqueness. Lumping them together
/// compares a list that is partly irrelevant to the guarantee: adding
/// `INCLUDE (fingerprint)` would have failed the column assertion while changing
/// nothing about what the index enforces — a false red, the mirror of the false
/// green above. They are split here and only the keys are compared.
const CATALOG_COLUMNS: &str = "c.relname, \
     i.indisunique, \
     pg_get_expr(i.indpred, i.indrelid), \
     (SELECT array_agg(CASE WHEN k.attnum = 0 THEN '(expression)' ELSE a.attname END \
                       ORDER BY k.ord) \
        FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) \
        LEFT JOIN pg_attribute a \
          ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
       WHERE k.ord <= i.indnkeyatts), \
     (SELECT array_agg(CASE WHEN k.attnum = 0 THEN '(expression)' ELSE a.attname END \
                       ORDER BY k.ord) \
        FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) \
        LEFT JOIN pg_attribute a \
          ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
       WHERE k.ord > i.indnkeyatts), \
     i.indexprs IS NOT NULL, \
     pg_get_indexdef(i.indexrelid)";

/// Every index on `table`, with its ordered key elements, `INCLUDE` payload,
/// uniqueness and partial predicate, read from the catalog.
///
/// `indkey` holds the elements in index order, so `WITH ORDINALITY` is what
/// preserves that order through the join to `pg_attribute` — without it the list
/// would come back in whatever order the join produced, and an assertion about
/// ordering would be meaningless rather than merely weaker.
async fn index_shapes(pool: &PgPool, table: &str) -> Vec<IndexShape> {
    let rows: Vec<CatalogRow> = sqlx::query_as(&format!(
        "SELECT {CATALOG_COLUMNS} \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indexrelid \
         WHERE i.indrelid = $1::regclass \
         ORDER BY c.relname"
    ))
    .bind(table)
    .fetch_all(pool)
    .await
    .expect("the index catalog must be queryable");

    rows.into_iter()
        .map(
            |(
                name,
                unique,
                predicate,
                key_columns,
                included_columns,
                has_expression_key,
                definition,
            )| IndexShape {
                name,
                unique,
                predicate: predicate.unwrap_or_default(),
                key_columns: key_columns.unwrap_or_default(),
                included_columns: included_columns.unwrap_or_default(),
                has_expression_key,
                definition,
            },
        )
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
            // Before the column list, what kind of thing the keys are.
            //
            // An extra expression key narrows the identity without changing any
            // registered column name: two rows identical in every column below can
            // coexist when the expression differs. Asserted from `indexprs` and from
            // the positional marker independently, because the first is the
            // catalog's own answer and the second proves the key list was not
            // silently shortened on its way here.
            assert!(
                !shape.has_expression_key,
                "`{name}` must key on plain columns only. The catalog reports an expression \
                 key, which constrains something no registered column names — so the \
                 identity below is not the identity being enforced. Server definition:\n  {}",
                shape.definition
            );
            assert!(
                !shape.key_columns.iter().any(|c| c == EXPRESSION_KEY),
                "`{name}` has an expression among its keys at a position the column list \
                 cannot express. Server definition:\n  {}",
                shape.definition
            );
            assert_eq!(
                shape.key_columns, columns,
                "`{name}` must key on exactly these columns in exactly this order; order is \
                 part of the contract because it determines which prefix lookups the index \
                 can serve. Non-key INCLUDE payload, which enforces nothing and is not \
                 compared: {:?}. Server definition:\n  {}",
                shape.included_columns, shape.definition
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

/// For every registered table, the pair's own predicates partition it:
/// complementary, so no row escapes and no row is covered twice.
///
/// # The predicates are the catalog's, not this file's
///
/// Load-bearing, and an earlier version of this test got it wrong in a way worth
/// recording. It hardcoded `tenant_id IS NOT NULL` and `tenant_id IS NULL` into
/// the `FILTER` clauses. Those two conditions are exhaustive and mutually
/// exclusive *by definition* — `IS NULL` is two-valued and never yields UNKNOWN —
/// so `tenant_half + systemwide_half == total` and an overlap of `0` were
/// tautologies. They held for any table with a `tenant_id` column, including one
/// carrying no indexes at all, while the doc comment claimed the check ran
/// "against the server rather than by reading the predicates". It did neither: no
/// catalog value ever reached the query.
///
/// The tell was already in the evidence. The mutation battery attributed every
/// detection to the other two tests and none to this one, which is what an inert
/// assertion looks like from the outside.
///
/// So the predicates are now read from `pg_get_expr` for this table's two named
/// indexes and substituted back into the query. A half whose predicate stops
/// being the complement of the other's changes the counts: two identical
/// predicates make the overlap non-zero, and a predicate narrower than the
/// partition leaves rows under neither, so the halves no longer sum to the total.
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

        // What the server says this table's two halves are actually partial over.
        let shapes = index_shapes(&pool, pair.table).await;
        let predicate_of = |name: &str| -> String {
            let shape = shapes
                .iter()
                .find(|shape| shape.name == name)
                .unwrap_or_else(|| {
                    panic!(
                        "table `{}` is missing the index `{name}`, so there is no predicate to \
                         partition it by. Present: {:?}",
                        pair.table,
                        shapes.iter().map(|s| &s.name).collect::<Vec<_>>()
                    )
                });
            // A total index reports no predicate. Substituting the empty string
            // would produce `FILTER (WHERE )` — a syntax error rather than a
            // failed assertion, which reports the wrong thing.
            assert!(
                !shape.predicate.is_empty(),
                "`{name}` is not a partial index, so it cannot be one half of a \
                 predicate-partitioned pair"
            );
            shape.predicate.clone()
        };
        let tenant_predicate = predicate_of(pair.tenant_half.0);
        let systemwide_predicate = predicate_of(pair.systemwide_half.0);

        // Only the table name and the two catalog-read predicates are interpolated;
        // all three come from the server or the const registry, never from input.
        // None can be bound as a parameter: a table name is an identifier and a
        // predicate is an expression, not a value.
        let (total, tenant_half, systemwide_half, both): (i64, i64, i64, i64) =
            sqlx::query_as(&format!(
                "SELECT COUNT(*), \
                        COUNT(*) FILTER (WHERE {tenant_predicate}), \
                        COUNT(*) FILTER (WHERE {systemwide_predicate}), \
                        COUNT(*) FILTER (WHERE ({tenant_predicate}) AND ({systemwide_predicate})) \
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
            "every row in `{}` must fall under exactly one of its pair's predicates, \
             `{tenant_predicate}` and `{systemwide_predicate}` — a row under neither is a \
             row no index constrains",
            pair.table
        );
        assert_eq!(
            both, 0,
            "no row in `{}` may fall under both `{tenant_predicate}` and \
             `{systemwide_predicate}`, or the two indexes would be enforcing overlapping \
             claims about the same rows",
            pair.table
        );
    }

    db.close().await;
}

/// One half of a pair as discovery finds it, before anything is known about
/// whether it has a partner.
#[derive(Debug)]
struct DiscoveredHalf {
    index: String,
    predicate: String,
    keys: Vec<String>,
}

impl DiscoveredHalf {
    /// The identity this half enforces, with the tenancy discriminator removed so
    /// the two halves become comparable.
    ///
    /// The systemwide half omits `tenant_id` because its predicate already fixes
    /// that column to NULL for every row it contains. Dropping `tenant_id` from
    /// the tenant half is therefore what puts both halves in the same terms — and
    /// two halves are a genuine pair only if what remains is *identical*.
    fn identity(&self) -> Vec<&str> {
        self.keys
            .iter()
            .map(String::as_str)
            .filter(|key| *key != "tenant_id")
            .collect()
    }

    fn describe(&self) -> String {
        format!(
            "{} over {} keying {:?}",
            self.index, self.predicate, self.keys
        )
    }
}

/// Any table that carries one half of a tenant-partitioned uniqueness pair
/// carries a *matching* other half.
///
/// This is the half of the contract the registry cannot express: it catches a
/// table nobody thought to register. See the module docs for why both defences are
/// kept.
///
/// # Matching, not merely present
///
/// An earlier version asked only whether some index existed under each predicate.
/// That let a table pass whose two halves guard **different** identities:
///
/// ```sql
/// UNIQUE (tenant_id, aggregate_id) WHERE tenant_id IS NOT NULL;
/// UNIQUE (operation_key)           WHERE tenant_id IS NULL;
/// ```
///
/// Two halves, one per predicate, and no coherent guarantee between them: the
/// tenant partition constrains `aggregate_id` while the systemwide partition
/// constrains `operation_key`, so neither identity is enforced across both
/// tenancy modes. The registry catches that on the three tables it names, which is
/// precisely why discovery must catch it on the tables it does not — that is
/// discovery's whole reason to exist.
///
/// So halves are paired by [`DiscoveredHalf::identity`]: strip `tenant_id` from
/// the tenant half and require what remains to equal the systemwide half's keys
/// exactly.
#[tokio::test]
async fn no_table_carries_a_lopsided_half_of_a_uniqueness_pair() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    let rows: Vec<DiscoveryRow> = sqlx::query_as(
        "SELECT t.relname, \
                c.relname, \
                pg_get_expr(i.indpred, i.indrelid), \
                (SELECT array_agg(CASE WHEN k.attnum = 0 THEN '(expression)' \
                                       ELSE a.attname END ORDER BY k.ord) \
                   FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) \
                   LEFT JOIN pg_attribute a \
                     ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
                  WHERE k.ord <= i.indnkeyatts) \
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

    let mut tables: BTreeMap<String, Vec<DiscoveredHalf>> = BTreeMap::new();
    for (table, index, predicate, keys) in rows {
        tables.entry(table).or_default().push(DiscoveredHalf {
            index,
            predicate: predicate.unwrap_or_default(),
            keys: keys.unwrap_or_default(),
        });
    }

    for (table, found) in &tables {
        let tenant_halves: Vec<&DiscoveredHalf> = found
            .iter()
            .filter(|half| half.predicate == TENANT_PREDICATE)
            .collect();
        let systemwide_halves: Vec<&DiscoveredHalf> = found
            .iter()
            .filter(|half| half.predicate == SYSTEMWIDE_PREDICATE)
            .collect();

        let described: Vec<String> = found.iter().map(DiscoveredHalf::describe).collect();

        assert!(
            !tenant_halves.is_empty() && !systemwide_halves.is_empty(),
            "table `{table}` carries only part of a tenant-partitioned uniqueness pair: \
             {described:?}. One half without the other leaves its complementary partition \
             unconstrained — for a missing NULL half that means unlimited duplicate \
             identities among tenant-less rows"
        );

        // Present on both sides is not the same as coherent across them.
        let matched = tenant_halves.iter().any(|tenant| {
            systemwide_halves
                .iter()
                .any(|systemwide| tenant.identity() == systemwide.identity())
        });
        assert!(
            matched,
            "table `{table}` has an index under each tenancy predicate, but no two of them \
             guard the same identity, so nothing is enforced across both modes: \
             {described:?}. Compared with `tenant_id` stripped from the tenant half — \
             tenant candidates {:?} against systemwide candidates {:?}",
            tenant_halves
                .iter()
                .map(|h| h.identity())
                .collect::<Vec<_>>(),
            systemwide_halves
                .iter()
                .map(|h| h.identity())
                .collect::<Vec<_>>()
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
