# Design: PROD-014B — PostgreSQL Durable Read-Side Stores

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers).
>
> **Inputs**: `proposal.md` (D-1 … D-8, IS-1 … IS-9, OOS-1 … OOS-8, G-1 … G-4, L-1 … L-5,
> R-1 … R-6, F-1 … F-3, SC-1 … SC-12) and `explore.md` (§7 Contract & Concurrency Addendum,
> Q1 … Q5). This document decides **how**: schema, SQL, concurrency approach, error
> handling, placement, and test placement. Observable requirements are `spec.md`'s and are
> not restated here.
>
> **Baseline read**: `develop` @ `a445e5b`. Every file:line below was read on this
> baseline, not recalled.

## Technical Approach

Two tables, two adapters, two migrations, on the golden path `event_store.rs` /
`snapshot.rs` / `reservation.rs` already established: one file per store under
`crates/persistence/src/postgres/`, `PgPool` by constructor injection, `is_durable()`
hardcoded `true`, re-exported from `postgres/mod.rs`, schema delivered as the next numbers
in the flat `include_str!` + `sqlx::raw_sql` sequence.

Both writes are conflict-safe by construction rather than by coordination. `write_offset`
is one upsert; `mark_seen` is one `INSERT … ON CONFLICT … DO NOTHING`. Neither takes a
lock, a lease, or a fencing token, and neither needs a read-modify-write round trip.

What that buys is stated precisely in **AD-6** and is the load-bearing sentence of this
design: the delivered guarantee is **single-writer-per-`(projection_id, tag, tenant)`**
(proposal L-3). Conflict-safe writes make the *bookkeeping* converge. They do not close
the check-then-act window between `seen()` and `mark_seen()`, because the handler already
ran inside it. Closing that is **PROD-014C — Atomic Read-Side Event Claiming** (F-1), and
this design does not attempt it.

No SPI, gate, registration, or scheduler code is touched (OOS-1, OOS-2, D-6).

---

## Evidence Corrections

Both were found by reading the code the inputs point at. Each changes what the
implementation must do.

### EC-1 — `explore.md` §2 recommends `reservation.rs`'s conditional `UPDATE` for the offset write; §7 Q4 supersedes it

`explore.md:83-87` reads the golden path as "conditional `UPDATE … WHERE <identity +
expected version>` for contested updates — the applicable pattern for a durable
offset/dedup adapter, **not a plain upsert**", and §5 item 3 repeats it. The later
addendum reverses this on evidence: `write_offset` (`offset.rs:77-83`) has no
expected-previous parameter and no ordering language, and its only caller
(`session.rs:91-176`) never inspects whether its write won (`explore.md` Q4;
proposal D-3, L-5).

An adapter carrying `reservation.rs`'s conditional `UPDATE` would refuse writes the trait
considers valid — a store stricter than the contract it implements, failing for callers
the SPI admits. **AD-3 implements the plain upsert.** The two texts disagree; this is not
an oversight being implemented around.

### EC-2 — `main.rs`'s pool is *moved* into `EntityEventStores::open`, so IS-6 needs a clone taken before that line

`examples/reference-app/src/main.rs:73-78` connects the pool and then passes it **by
value**: `EntityEventStores::open(pool)`. `PgPool` is `Clone` (an `Arc` internally), so the
fix is one `pool.clone()` — but it has to be taken *before* line 78, not after, and IS-6's
one-line description ("rewire `None` to the real Postgres pair") does not surface that the
binding is gone by then. Resolved in **AD-10**, which also fixes the ordering against
`migrations::run(&pool)` at line 77.

---

## Component Map

```
crates/persistence/src/postgres/
├── migrations/013_create_projection_offsets.sql   NEW   AD-1, AD-2
├── migrations/014_create_projection_dedup.sql     NEW   AD-1, AD-2
├── migrations.rs                     MOD  two const + two registry entries (AD-2)
├── read_side_offset.rs               NEW  PostgreSQLOffsetStore   (AD-3, AD-4)
├── read_side_dedup.rs                NEW  PostgreSQLDedupStore    (AD-5)
└── mod.rs                            MOD  two `pub use` + pub(crate) fn is_fatal (AD-8, AD-9)
                                                ↑ used by
examples/reference-app/
├── src/read_side/mod.rs              MOD  ReadSideProgressStores::postgres(pool)  (AD-10)
└── src/main.rs                       MOD  pool.clone() before open; None → Some   (AD-10, EC-2)

integration-tests/tests/infrastructure/
└── read_side_progress_postgres.rs    NEW  the whole conformance suite  (AD-12)

crates/domain/src/read_side/{offset,dedup,session,runner}.rs   UNTOUCHED  (OOS-1)
crates/service-sdk/src/{runtime/builder.rs, app/mod.rs}        UNTOUCHED  (OOS-1)
crates/runtime/src/read_side/scheduler.rs                      UNTOUCHED  (OOS-2)
crates/effect-store/                                           UNTOUCHED  (own 001/002 sequence)
```

## Data Flow

```
ReadSideSession::execute (session.rs, UNCHANGED)          PostgreSQL
─────────────────────────────────────────────────         ──────────
 Phase 2  dedup.seen(pid, tag, event_id) ──────────▶  SELECT 1 FROM projection_dedup
             │  false                                  WHERE pid=$1 AND tag=$2 AND event_id=$3
             ▼                                                    │
 Phase 3  handler.handle(event)   ◀── ⚠ THE WINDOW ───────────────┘
             │      two writers can both be here, both having
             │      read false, before either reaches Phase 4
             ▼                                        INSERT INTO projection_dedup …
 Phase 4  dedup.mark_seen(...) ────────────────────▶  ON CONFLICT (pid,tag,event_id) DO NOTHING
             │                                        → converges to ONE row, no error (G-2)
             ▼                                        INSERT INTO projection_offsets …
          offset.write_offset(..., Sequence(n)) ───▶  ON CONFLICT (pid,tag,tenant)
                                                      DO UPDATE SET offset_value = EXCLUDED…
                                                      → last write wins (L-5)
```

The window marked ⚠ is L-2/L-3, unchanged by this design and owned by **PROD-014C —
Atomic Read-Side Event Claiming** (F-1). See AD-6.

---

## Architecture Decisions

### AD-1 — Schema: two tables, composite primary keys, `tenant` `NOT NULL` on offsets only

**Decision** — `013_create_projection_offsets.sql`:

```sql
CREATE TABLE IF NOT EXISTS projection_offsets (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    tenant        VARCHAR(255) NOT NULL,
    offset_value  BIGINT       NOT NULL,
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_offsets_identity
        PRIMARY KEY (projection_id, tag, tenant)
);
```

`014_create_projection_dedup.sql`:

```sql
CREATE TABLE IF NOT EXISTS projection_dedup (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    event_id      VARCHAR(255) NOT NULL,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_dedup_identity
        PRIMARY KEY (projection_id, tag, event_id)
);
```

**Criteria**:

1. **The primary key *is* the UNIQUE identity IS-2 asks for**, and it is also the index
   `seen()` reads. A `BIGSERIAL id` + separate unique index (`010`/`011`'s shape) exists
   there only because a nullable `tenant_id` forced two *partial* unique indexes, which a
   table constraint cannot express. That reason is structurally absent here (D-2), so the
   surrogate key would be a column nothing reads and a second index nothing needs.
2. **`tenant` is `NOT NULL`, and the `tenant_id IS NOT DISTINCT FROM $N` pattern of
   `reservation.rs:182` / `snapshot.rs:67,77` is deliberately not reused** (D-2, Q3). The
   read-side SPI's parameter is `tenant: &str`, never `Option<&str>`; the framework's
   systemwide-tenant concept (`crates/domain/src/persistence/tenant.rs:29-35`) is a
   write-side type with no read-side counterpart. A nullable column would model a state
   the SPI cannot produce, and would drag two partial indexes behind it to do so.
3. **`offset_value`, never `offset`.** `OFFSET` is a reserved word in PostgreSQL; a column
   named `offset` is unusable unquoted, and quoting an identifier for the rest of the
   table's life to save four characters is a trap, not a convention.
4. **`BIGINT` is exactly `Offset::Sequence(i64)`** (`offset.rs:12-16`) — a total mapping in
   both directions, so no `token_for_storage`-style checked conversion is required (AD-4).
5. **`VARCHAR(255)` follows the existing sequence** (`001`, `010`, `011`) rather than
   inventing `TEXT`. Its ceiling, stated rather than discovered: an identifier longer than
   255 characters is **refused** by the column (SQLSTATE `22001`), not truncated, and is
   classified `Fatal` by AD-8. No call site produces one — `projection_id` is a `const`,
   `tag` is composed by the host, `tenant` is a validated `TenantId`.
6. **`created_at` / `updated_at` are operational, not functional.** No query reads them.
   `created_at` on `projection_dedup` is what a future F-2 retention pass would scan; this
   change ships no index for it, because indexing for a scan nobody performs is designing
   F-2 in advance (AD-11).

**Rejected — one merged table.** The two identities differ in a column that is not
optional on either side: an offset row has no `event_id` and a dedup row has no `tenant`
(D-1). A merged table needs both nullable plus a discriminator, which turns two primary
keys into two partial unique indexes and a `CHECK` — strictly more schema to express
strictly less.

### AD-2 — Two migration files at `013`/`014`, registered in the existing flat sequence

**Decision**: two `include_str!` constants and two entries appended to `migrations()`
(`migrations.rs:60-90`), in ascending order, run by the unchanged `sqlx::raw_sql` loop
(`migrations.rs:43-57`).

**Criteria**: (a) every existing file in the sequence creates or alters exactly one thing —
`010` and `011` are two tables in two files for the same capability, which is this case
precisely; (b) `crates/effect-store`'s independent `001/002` sequence is a property of a
separate crate (AD-10 there), not a rule that transfers into `ego-persistence` (D-5);
(c) **the existing tests in `migrations.rs` already cover the new files at no cost** —
`every_migration_file_is_registered_and_every_registration_has_a_file` fails if a `.sql`
file is added without being registered (the exact defect that once left three migrations
inert), and `registration_order_ascends_by_numeric_prefix` fails if `013`/`014` are
misordered. No new migration test is written; R-4 is caught by machinery that exists.

### AD-3 — `write_offset` is one upsert, last-write-wins (resolves EC-1)

**Decision**:

```rust
sqlx::query(
    r#"INSERT INTO projection_offsets (projection_id, tag, tenant, offset_value)
       VALUES ($1, $2, $3, $4)
       ON CONFLICT (projection_id, tag, tenant)
       DO UPDATE SET offset_value = EXCLUDED.offset_value, updated_at = NOW()"#,
)
.bind(projection_id)
.bind(tag.value())
.bind(tenant)
.bind(offset.as_sequence().expect("Offset has exactly one variant"))
.execute(&self.pool)
.await
.map_err(offset_error)?;
```

**Criteria**: (a) EC-1 — the SPI expresses overwrite, and the adapter implements the
contract it was handed, not a stricter one; (b) one statement, so there is no
check-then-write window of the adapter's own making — two concurrent writers both succeed
and the later commit wins, which is `Ok(())` under this SPI (L-5); (c) `tag.value()`
(`event_tag.rs:29-31`) is the bound `$2`, never the `Display` form and never interpolated.

**`.expect(...)` on `as_sequence()` is correct here**, not a lurking panic: `Offset` is a
single-variant enum (`offset.rs:12-16`) whose FR-014 constraint is that it stays one. If a
variant is ever added, this is exactly where the compiler-adjacent failure should surface,
rather than silently writing a fabricated sequence.

### AD-4 — `read_offset` is a scalar point lookup; absent means `Ok(None)`

**Decision**:

```rust
let stored: Option<i64> = sqlx::query_scalar(
    r#"SELECT offset_value FROM projection_offsets
       WHERE projection_id = $1 AND tag = $2 AND tenant = $3"#,
)
.bind(projection_id).bind(tag.value()).bind(tenant)
.fetch_optional(&self.pool).await.map_err(offset_error)?;

Ok(stored.map(Offset::Sequence))
```

**Criteria**: (a) `fetch_optional`, so "never written" is `Ok(None)` and not an error —
the resume-from-zero case is normal, not exceptional; (b) all three identity columns are
bound `$N`, including `tenant`, which is SC-7 and `ego-rs-security` Rule 2 satisfied by
the query shape rather than by a review promise; (c) the primary key makes at most one row
matchable, so `fetch_optional` cannot see the multi-row error path; (d) `i64` → `Offset`
needs no guard (AD-1 criterion 4).

### AD-5 — `mark_seen` is one `INSERT … ON CONFLICT (…) DO NOTHING` with an explicit target

**Decision**:

```rust
// mark_seen
sqlx::query(
    r#"INSERT INTO projection_dedup (projection_id, tag, event_id)
       VALUES ($1, $2, $3)
       ON CONFLICT (projection_id, tag, event_id) DO NOTHING"#,
)
.bind(projection_id).bind(tag.value()).bind(event_id)
.execute(&self.pool).await.map_err(dedup_error)?;
Ok(())

// seen
let hit: Option<i32> = sqlx::query_scalar(
    r#"SELECT 1 FROM projection_dedup
       WHERE projection_id = $1 AND tag = $2 AND event_id = $3"#,
)
.bind(projection_id).bind(tag.value()).bind(event_id)
.fetch_optional(&self.pool).await.map_err(dedup_error)?;
Ok(hit.is_some())
```

**Criteria**: (a) one statement, no `rows_affected` inspection — the SPI returns
`Result<(), _>`, so "inserted" and "already there" are the same success, and reading the
count would only tempt a caller-visible distinction the trait cannot carry;
(b) **an explicit conflict target, unlike `reservation.rs:213-219`'s bare
`ON CONFLICT DO NOTHING`** — that form is forced there by partial indexes, which cannot be
named as a target; here the identity is a plain primary key, and naming it means a
violation of some *other* future constraint surfaces as an error instead of being silently
swallowed; (c) `seen()` is a primary-key point lookup, so no `LIMIT` and no `COUNT(*)`.

**Studied and deliberately not copied: `EffectDedupStore::reserve`**
(`crates/effect-store/src/postgres/mod.rs:699-756`). It is the same single atomic insert,
but it *returns the outcome* — `rows_affected() == 1` means the caller won the claim, and
it runs **before** any side effect. That is the shape F-1 needs and the shape this SPI
cannot express: `mark_seen` returns `Result<()>`, is called only after
`handler.handle()` (`session.rs:135`), and has no vocabulary for "you lost". Reproducing
`reserve`'s statement here without `reserve`'s return type and call position would look
like exclusion while providing none.

### AD-6 — The delivered guarantee is single-writer-per-`(projection_id, tag, tenant)`; the primary key is idempotent storage, not exclusion

**Decision**: this pair is designed, documented, and tested as **at-least-once processing
with best-effort dedup bookkeeping under an unenforced single-writer assumption**
(proposal L-1/L-2/L-3). Nothing in the adapters, their rustdoc, the persistence README, or
the configuration docs may describe them as exactly-once, concurrency-safe, or safe for a
multi-replica projection writer (SC-8, SC-12, IS-8).

**The distinction this design is required to make explicit**:

| | What the `projection_dedup` primary key does | What it does not do |
|---|---|---|
| Effect | Two concurrent `mark_seen` calls for the same identity converge to **one row**, with no unique-violation error surfacing to either caller (G-2) | Prevent **two handlers from having already run** |
| Why | `ON CONFLICT DO NOTHING` resolves the write race inside one statement | The race is upstream: `seen()` (`session.rs:116-128`) and `mark_seen()` (`:142-149`) are separate SPI methods with `handler.handle()` (`:135`) between them. Both writers read `false` and execute **before** either marks (`explore.md` Q5) |
| Scope | Storage idempotence | Nothing about execution exclusion |

A constraint on a table is a predicate about rows. It cannot retroactively un-run an
effect that already happened, and no PostgreSQL adapter can close this window from inside
`mark_seen` — it is an SPI-level gap (Q5 verdict, D-7).

Single-writer-per-tag holds **inside one process today** because
`TagSchedulerImpl::start_projection` (`scheduler.rs:66-108`) awaits each tag's session
sequentially. Across replicas nothing enforces it: read-side code contains no leader
election, no lock, no lease, and no fencing token. This change neither detects nor refuses
a second replica (OOS-2).

**Where a future atomic claim would hook, named only**: the seam is Phase 2/3 of
`ReadSideSession::execute` — a claim must be *obtained* before `handler.handle()` runs and
must return whether the caller won, which means a new SPI method (or orchestration-level
single-writer enforcement), not a different SQL statement behind the existing one. That is
**PROD-014C — Atomic Read-Side Event Claiming** (F-1). It is named here and designed
nowhere in this document.

### AD-7 — `projection_dedup` carries no `tenant` column, and that is not a tenant-isolation defect

**Decision**: dedup identity is `(projection_id, tag, event_id)` exactly (D-1, Q1). Tenant
is absent from the table, from every dedup query, and from the index.

**Criteria**: the SPI's `seen`/`mark_seen` (`dedup.rs:37-51`) take no tenant, and the trait
doc states the scope outright ("Deduplication scope: (projection_id, tag, event_id)").
Adding a column no SPI method can populate would make every row's tenant a fabrication.

**What a security reviewer must be told rather than left to infer** (`ego-rs-security`
Rule 2 asks every tenant-scoped table to bind `tenant_id`): this table is **not**
tenant-scoped data. It stores no tenant-owned value — only the presence of an event
identifier — and `projection_offsets`, which *is* tenant-scoped, binds `tenant` in every
statement (AD-3, AD-4). The consequence is real and required by SC-4: the same `event_id`
under the same `(projection_id, tag)` is already-seen regardless of tenant. In the
reference composition that is inert, because `tag` is itself tenant-derived
(`tenant_tag`, `read_side/mod.rs:199`), so two tenants never share a tag. A host that
chooses a tenant-independent tag **does** share dedup rows across tenants — a property of
the SPI's identity, adopted knowingly here, not introduced by this adapter.

### AD-8 — One shared SQLSTATE-based `is_fatal` predicate; `Transient` is the default

**Decision**: `crates/persistence/src/postgres/mod.rs` gains one `pub(crate)` pure
function, and each adapter maps it into its own error type.

```rust
/// Whether a storage failure will fail the same way on every retry.
///
/// `Transient` is the default because a retryable failure misreported as `Fatal`
/// stops a projection that would have recovered on its own. The four codes below
/// are the ones a retry cannot help: the migration did not run, the schema drifted,
/// a value does not fit its column, or a row cannot be decoded into the type this
/// crate wrote.
pub(crate) fn is_fatal(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => matches!(
            db.code().as_deref(),
            Some("42P01") // undefined_table — migration 013/014 not applied
                | Some("42703") // undefined_column — schema drift
                | Some("22001") // string_data_right_truncation — over VARCHAR(255) (AD-1)
                | Some("23514") // check_violation
        ),
        sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => true,
        _ => false,
    }
}
```

```rust
fn offset_error(err: sqlx::Error) -> OffsetStoreError {
    let text = err.to_string();
    if is_fatal(&err) { OffsetStoreError::Fatal(text) } else { OffsetStoreError::Transient(text) }
}
// structurally identical `dedup_error` → DedupStoreError::{Fatal, Transient}
```

**Criteria**: (a) both SPIs declare the same two-variant split (`offset.rs:42-49`,
`dedup.rs:9-18`) and neither defines which failures are which — leaving it to a coin flip
per call site is how one adapter ends up retrying a missing table forever; (b) the
predicate is pure and takes no pool, so it is the **one genuinely unit-testable surface**
this change adds (AD-12), matching `reservation.rs`'s own `#[cfg(test)]` shape, which tests
only pure helpers; (c) it lives in `postgres/mod.rs` because two adapter files need it and
that module already hosts a shared crate-internal item (`pub(crate) use resolve_tenant`,
`mod.rs:16-21`) — one definition, not a copy per file.

### AD-9 — Two adapter files, both re-exported; no `probe()` method

**Decision**: `read_side_offset.rs` (`PostgreSQLOffsetStore`) and `read_side_dedup.rs`
(`PostgreSQLDedupStore`), each `pub struct { pool: PgPool }` with `pub fn new(pool: PgPool)`,
a manual `Debug` printing only the pool (matching `snapshot.rs:31-37`,
`reservation.rs:75-81`), `is_durable() -> true` unconditionally
(`snapshot.rs:52-54`, `event_store.rs:126-128`), and `pub use` from `postgres/mod.rs`
(IS-4).

**Criteria**: one file per store is the existing shape (`event_store.rs`, `snapshot.rs`,
`reservation.rs`, `repository.rs`), and the two prefixed names keep them from reading as
write-side stores in a directory that already has a `snapshot.rs`.

**No `probe()`, deliberately.** `reservation.rs:537-561` queries its real table instead of
`SELECT 1` precisely so a missing migration is found at readiness rather than at first
write — good, and reachable there because `OperationReservationStore` declares the method.
`OffsetStore`/`DedupStore` declare no health method, and adding an inherent one nothing
calls is scaffolding. The property `probe()` protects is instead preserved by AD-8: an
unapplied migration surfaces as `42P01` → `Fatal`, distinguishable from a transient
outage. A real readiness probe belongs to whatever change adds the SPI method, not here.

### AD-10 — Reference-app wiring: `postgres(pool)` constructor, and the pool is cloned *before* `EntityEventStores::open` (resolves EC-2)

**Decision** — `examples/reference-app/src/read_side/mod.rs`, beside `in_memory()` and
`fake_durable()`:

```rust
/// Durable, and the only pair a `Profile::Production` composition can register
/// and satisfy (PROD-014B IS-5). `pool` must already have had
/// `ego_persistence::postgres::migrations::run` applied — migrations `013`/`014`
/// create the two tables these stores write to.
///
/// Adoption constraint (PROD-014B L-3): safe only where exactly one writer per
/// `(projection_id, tag, tenant)` exists. Two replicas of this projection are
/// outside the guarantee and nothing here detects it — see PROD-014C.
pub fn postgres(pool: PgPool) -> Self {
    Self {
        offset: Arc::new(PostgreSQLOffsetStore::new(pool.clone())),
        dedup: Arc::new(PostgreSQLDedupStore::new(pool)),
    }
}
```

**Decision** — `examples/reference-app/src/main.rs:73-114`, ordering made explicit:

```rust
let pool = PgPoolOptions::new()…connect(&config.database.url).await?;
ego_persistence::postgres::migrations::run(&pool).await?;   // 013/014 applied here
let read_side_progress = ReadSideProgressStores::postgres(pool.clone());  // EC-2: before the move
let stores = EntityEventStores::open(pool).await?;          // pool moved
…
build_runtime_with(…, Some(read_side_progress))?;           // was None + "PROD-014A F-1" comment
```

**Criteria**: (a) EC-2 — `open` takes the pool by value, so the clone must precede line 78;
`PgPool` is `Clone` over a shared `Arc`, so both stores and the event stores share one
connection pool rather than opening a second; (b) `migrations::run` already precedes both
(line 77), so no ordering changes and no new migration call is added; (c) the host is
already `Profile::Production` (`EntityEventStores::open`), so `Some(pair)` now routes
through `validate_read_side_progress_profile` on a real durable backend for the first time
— the gate's own logic is untouched (SC-5, D-6); (d) `build_runtime_with` already threads a
stated pair into both the registration and `ProjectionSpec` from one value (`lib.rs:793`,
`:869-875`), so IS-6 is one argument change, not new plumbing.

### AD-11 — No retention, and the growth statement is one operational line

**Decision**: no TTL, no purge, no eviction, no partitioning, no retention index (D-4,
OOS-3). `projection_dedup` grows monotonically, linearly with unique events processed
(L-4). The adapters' rustdoc carries one operational note: row count is a signal to
observe, and the escalation trigger and the mechanism belong to **F-2**.

**Criteria**: retention needs a horizon, and the rule tying dedup removal to offset
advancement does not exist at any layer today (`scheduler.rs`, `runner.rs`, `session.rs`
contain none — Q2). Shipping a cleanup path here would invent that rule inside a
persistence adapter, where no caller could see it. `crates/effect-store`'s `effect_dedup`
sets the workspace precedent: retention is separately owned
(`effect-store/src/postgres/mod.rs:285-356`).

### AD-12 — Behaviour is proved only against real PostgreSQL in `integration-tests/`; the crate keeps exactly one pure unit-test surface

**Decision**: `is_fatal` (AD-8) is the only new `#[cfg(test)]` unit test in
`crates/persistence`. Every behavioural claim is proved in
`integration-tests/tests/infrastructure/read_side_progress_postgres.rs` via
`ego_integration_tests::isolated_database()`. No test constructs a `PgPool` inside a
`crates/` unit test, including `connect_lazy` (D-8, SC-10).

**Criteria**: `ego-rs-testing` Rule 1 and Rule 2 forbid a real pool in a unit test, and
Rule 3's documented architectural exception is exactly the root-level `integration-tests/`
workspace with per-test `isolated_database()`. `reservation.rs`'s own `#[cfg(test)]` block
(`:608-669`) tests only pure conversion helpers — the same line this design draws.
`is_durable()` returns a constant but is asserted in the conformance suite, where a real
store exists, rather than through a pool faked into being for one assertion.

---

## Integration Points

| Boundary | Direction | Mechanism | Verified at |
|---|---|---|---|
| `ego-domain` → `ego-persistence` | up | already the crate's only dependency | `crates/persistence/Cargo.toml:6-15` |
| adapters → `Arc<dyn …>` erasure | out | PROD-014A's generic `Arc<T>` forwarding impls, inherited free | `offset.rs:91-119`, `dedup.rs:59-86` |
| `is_durable()` → `Profile::Production` gate | in | unchanged `validate_read_side_progress_profile` | `builder.rs:879-891`; D-6 |
| `ego-persistence` → `reference-app` | up | dependency already declared | `reference-app/Cargo.toml:44,51` |
| pair → registration **and** `ProjectionSpec` | out | one value, two destinations, already wired | `lib.rs:793-796`, `:869-875` |
| schema → runtime | in | existing `include_str!` + `raw_sql` runner | `migrations.rs:43-57`; AD-2 |
| adapters → scheduler / SPI | **none** | no path added, none exists | OOS-1, OOS-2 |

Zero new plumbing: every crossing above already exists.

## Testing Strategy

Strict TDD — the conformance suite is written RED, against types that do not compile yet,
before either adapter body. Each error assertion names the specific variant, never
`is_err()`.

| Level | Location | What it proves |
|---|---|---|
| Unit | `crates/persistence/src/postgres/mod.rs` `#[cfg(test)]` | AD-8: `42P01`/`42703`/`22001`/`23514` and `ColumnDecode`/`Decode` classify `Fatal`; pool timeout, I/O and protocol errors classify `Transient`. Pure function, constructed `sqlx::Error` values, **no pool** |
| Unit | `crates/persistence/src/postgres/migrations.rs` (existing tests, no new code) | AD-2: `013`/`014` are registered and ordered — the existing bidirectional registry test fails if a `.sql` file ships unregistered |
| Integration (real PG) | `integration-tests/tests/infrastructure/read_side_progress_postgres.rs`, `isolated_database()` | **SC-1** restart survival: write an offset, **drop the store and its pool**, open a *new* pool against the same database, rebuild the store, read `N` back — the in-process value is never the evidence (R-3). **SC-2** unwritten `(projection_id, tag, tenant)` → `None`, and tenant B's read never returns tenant A's offset. **SC-3** `mark_seen` twice sequentially *and* twice concurrently (`tokio::join!`) → both `Ok`, `SELECT COUNT(*)` is exactly `1`, `seen()` is `true`. **SC-4** the same `event_id` under a different tenant, same `(projection_id, tag)` → already seen. **SC-5** both `is_durable()` are `true`, and `build_runtime_with(…, Some(ReadSideProgressStores::postgres(pool)))` builds under `Profile::Production` — which is also **SC-6**, the reference production path proved usable. Plus: `read_offset` against a database with no migration applied returns `Fatal`, not `Transient` (AD-8, and the property `probe()` would otherwise have covered — AD-9) |
| — | `examples/reference-app/tests/` | Nothing added. Those binaries reach no external service; the production wiring claim is proved in the row above, through `build_runtime_with` rather than around it |

Two properties are diff properties, not test properties, and are checked by reading the
change: **SC-7** (no interpolation anywhere; every `$N` bound, every offset statement binds
`tenant`) and **SC-11** (`crates/domain/src/read_side/`, `crates/service-sdk`'s gate and
registration, and `crates/runtime/src/read_side/scheduler.rs` appear in no file list here).

## Threat Matrix

N/A — no routing, shell command, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. This change adds two SQL adapters, two
DDL files, one host constructor, and one test suite; no external process is invoked and no
file is executed or classified.

The applicable security surface is `ego-rs-security` Rules 1 and 2, and it is closed by
construction: every value is a bound `$N` (AD-3, AD-4, AD-5), no identifier or value is
interpolated into SQL text, and no allowlist carve-out is used — unlike
`isolated_database()`'s `CREATE DATABASE`, this change interpolates nothing at all. Rule 2
is satisfied for `projection_offsets` (every statement binds `tenant`) and explicitly
scoped for `projection_dedup` in AD-7.

## Migration / Rollout

Two additive `CREATE TABLE IF NOT EXISTS` migrations, referenced by nothing else and
referencing nothing else. No existing table, column, index, or query changes. Deploy order
is the existing one: `migrations::run` already precedes every store construction in
`main.rs` and in the integration suite's template database.

Rollback is the proposal's, unchanged: delete both adapters and their re-exports, delete
`013`/`014` and their registry entries, delete the conformance suite, remove
`ReadSideProgressStores::postgres`, restore `main.rs` to `None`. The two tables may be
dropped or left; no data written before this change exists, and state written between
deploy and revert degrades a projection back to today's replay-from-scratch behaviour
rather than corrupting anything.

## Traceability

| Proposal item | Resolved by | Note |
|---|---|---|
| IS-1, D-1, D-2, D-3 | AD-1, AD-3, AD-4 | `NOT NULL` tenant; plain upsert |
| IS-2, D-1, G-2 | AD-1, AD-5 | primary key **is** the UNIQUE identity |
| IS-3, D-5, R-4 | AD-2 | `013`/`014`; existing registry tests cover them |
| IS-4 | AD-9 | one file per store, both re-exported |
| IS-5 | AD-10 | `ReadSideProgressStores::postgres(pool)` |
| IS-6, SC-6 | AD-10 + **EC-2** | clone before `EntityEventStores::open` |
| IS-7, D-8, SC-10 | AD-12 | `isolated_database()`; one pure unit test only |
| IS-8, SC-8, SC-12, L-1/L-2/L-3 | **AD-6** | the guarantee is single-writer-per-`(projection_id, tag, tenant)` |
| D-4, L-4, OOS-3 | AD-11 | unbounded; one operational line; F-2 owns retention |
| D-6, OOS-1, SC-5 | AD-9, AD-10 | `is_durable() -> true`; gate untouched |
| D-7, OOS-2, F-1, SC-9 | AD-6 | PROD-014C named; hook point identified, not designed |
| L-5, Q4 | AD-3 + **EC-1** | supersedes `explore.md` §2's conditional-`UPDATE` reading |
| G-1, SC-1, R-3 | AD-12 | restart proved across a new pool, not an in-process value |
| G-4, SC-2, SC-7 | AD-4 | `tenant` bound in every offset statement |
| Q1, Q3, R-6 | AD-1 | identity and `NOT NULL`; relaxing to nullable stays a forward migration |
| Q5, R-1 | AD-6, AD-7 | bookkeeping vs execution stated as a table, not a footnote |
| — | AD-8 | new: `Transient`/`Fatal` split was undefined at both SPIs |
| R-5 | — | `sdd-tasks` owns the 400-line forecast; not pre-empted here |

## Open Questions

- [ ] AD-1 criterion 5 — `VARCHAR(255)` follows convention and refuses (SQLSTATE `22001`)
      rather than truncating an over-long identifier. No call site produces one today;
      confirm the ceiling is acceptable rather than switching this pair to `TEXT`.
- [ ] AD-9 — no `probe()`. `reservation.rs` has one because its port declares it; here the
      unapplied-migration case is covered by `Fatal` classification instead. Confirm no
      readiness surface is expected from these adapters in this change.
