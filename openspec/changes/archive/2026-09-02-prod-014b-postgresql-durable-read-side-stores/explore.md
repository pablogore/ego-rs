# Exploration: PROD-014B — PostgreSQL Durable Read-Side Stores

**Phase**: `sdd-explore`
**Change**: `prod-014b-postgresql-durable-read-side-stores`
**Baseline**: `develop` @ `a445e5b`
**Status**: complete — ready for `sdd-propose`

## Intent

Close the gap PROD-014A deliberately left open: provide a durable PostgreSQL
implementation of the read-side `OffsetStore` + `DedupStore` pair, conforming
to the SPI and `Profile::Production` durability gate PROD-014A already
shipped. This is a P0 gap-closing change, not new SPI design.

## Scope

PROD-014B covers exactly one capability: a durable PostgreSQL adapter for
`OffsetStore` and `DedupStore`. Explicitly out of scope:

- Stoolap or any other backend — deferred to a possible future work item
  (identifier not yet assigned) only if a concrete product need appears.
  Note: "PROD-014C" is reserved for a different, already-named follow-up —
  see `proposal.md` §Named Follow-Ups ("PROD-014C — Atomic Read-Side Event
  Claiming").
- Oracle, ClickHouse, MySQL, Redis, RocksDB, DynamoDB, Cassandra/Scylla,
  SQLite — none are productive ego-rs backends today (verified by a full
  codebase audit: zero code, only illustrative prose in archived OpenSpec).
- Any change to the `OffsetStore`/`DedupStore` SPI, the `Profile::Production`
  gate logic, or the `AppBuilder::read_side_progress` registration
  mechanism — all already shipped by PROD-014A.

## 1. Current State

- SPI (`crates/domain/src/read_side/offset.rs:55-84`, `dedup.rs:25-52`):
  `OffsetStore::{read_offset,write_offset}` keyed on `(projection_id, tag,
  tenant: &str)`; `DedupStore::{seen,mark_seen}` keyed on `(projection_id,
  tag, event_id)`. Both default `is_durable()` to `false` (offset.rs:62-64,
  dedup.rs:33-35) and both have a load-bearing generic `Arc<T>` forwarding
  impl (offset.rs:91-119, dedup.rs:59-86) that a new adapter gets for free.
- Only implementations today: `InMemoryOffsetStore`/`InMemoryDedupStore` and
  `FakeDurableOffsetStore`/`FakeDurableDedupStore`
  (`examples/reference-app/src/read_side/store.rs:150-307`) — no real
  durable adapter exists.
- Gate: `RuntimeBuilder::validate_read_side_progress_profile`
  (`crates/service-sdk/src/runtime/builder.rs:879-891`) calls
  `persistent_entity::profile::require_durably_configured`
  (`crates/persistent-entity/src/profile.rs:51-63`) with
  `pair.offset.is_durable() && pair.dedup.is_durable()` — only
  `Profile::Production` with a non-durable pair fails.
- Registration entrypoint:
  `AppBuilder::read_side_progress(projection_id, offset_store, dedup_store)`
  (`crates/service-sdk/src/app/mod.rs:633-651`), rejecting duplicate
  `projection_id`.
- Composition-time value type: `ReadSideProgressStores { offset, dedup }`
  (`examples/reference-app/src/read_side/mod.rs:93-117`) — application code,
  not SPI/`service-sdk`, with `::in_memory()` and `::fake_durable()`
  constructors today.
- Current gap: `examples/reference-app/src/main.rs:85-115` passes `None` for
  read-side progress, with an explicit "PROD-014A F-1" comment
  (main.rs:109-114).

## 2. PostgreSQL Golden Path — conventions found

- `ego-persistence` (`crates/persistence`) depends only on `ego-domain`
  (`crates/persistence/Cargo.toml:6-15`) — no new dependency graph edge
  needed since `OffsetStore`/`DedupStore` live in `ego-domain` too.
- One file per store under `postgres/`, re-exported from `postgres/mod.rs:1-14`.
  Adapters take `PgPool` by constructor injection (`snapshot.rs:39-44`,
  `reservation.rs:83-88`); `is_durable()` returns `true` unconditionally
  (`snapshot.rs:52-54`, `event_store.rs:126-128`).
- Migrations: hand-rolled `include_str!` + `sqlx::raw_sql` runner
  (multi-statement files need this, not `sqlx::query` —
  `migrations.rs:40-57`), single flat numbered sequence `001..012`
  (`crates/persistence/src/postgres/migrations/*.sql`). `crates/effect-store`
  runs its **own independent** `001/002` sequence (AD-10,
  `crates/effect-store/src/postgres/migrations.rs:1-9`) — that precedent
  applies only if a new crate is chosen, not inside `ego-persistence`.
- Tenant-NULL handling precedent: `tenant_id IS NOT DISTINCT FROM $N` plus
  two partial unique indexes (PG14-safe, vs. PG15's `NULLS NOT DISTINCT`) —
  `snapshot.rs:63-137`, `010_create_operation_reservations.sql:1-14,66-72`.
  The read-side SPI's `tenant` is `&str` not `Option<&str>`, so this may not
  transfer directly — flagged as an open question.
- Concurrency-safe write pattern: `reservation.rs` uses
  `INSERT ... ON CONFLICT DO NOTHING` for first-write races
  (reservation.rs:213-219) and conditional `UPDATE ... WHERE <identity +
  expected version>` for contested updates (reservation.rs:313-320) — the
  applicable pattern for a durable offset/dedup adapter, not a plain upsert.
- Health-check precedent: `reservation.rs::probe()` (reservation.rs:537-561)
  queries the real table, not `SELECT 1`, to prove the migration ran — no
  such method exists on `OffsetStore`/`DedupStore` today, confirmed by
  reading both traits in full.
- Testing precedent: `integration-tests/tests/infrastructure/*_postgres.rs`
  (19 files) use `ego_integration_tests::{isolated_database, IsolatedDatabase}`
  (e.g. `durable_entity_progress_postgres.rs:43,73`) — confirmed
  `integration-tests` is a separate workspace (`reference-app/Cargo.toml:19-21`
  comment), matching the `ego-rs-testing-strategy` skill's rule that no unit
  test may reach real Postgres.

## 3. Affected Areas (for `sdd-propose`/`sdd-design`, not touched this phase)

- `crates/persistence/src/postgres/` — new adapter file(s) + migration(s) `013+`
- `crates/persistence/src/postgres/mod.rs` — re-export
- `examples/reference-app/src/read_side/mod.rs:93-117` — new
  `ReadSideProgressStores::postgres(...)` constructor
- `examples/reference-app/src/main.rs:109-114` — rewire `None` → `Some(...)`
- `integration-tests/tests/infrastructure/` — new `*_postgres.rs` conformance suite

## 4. Approaches (crate placement)

1. **Adapter inside `ego-persistence/src/postgres/`** (recommended) — matches
   every existing golden-path adapter's placement, zero new dependency edges
   (`reference-app` already depends on `ego-persistence`), continues the
   existing `013+` migration sequence.
   - Pros: consistent with `event_store.rs`/`snapshot.rs`/`reservation.rs`;
     no new crate; no new dependency wiring.
   - Cons: none identified against the evidence read.
   - Effort: Low (placement decision only; adapter itself is real work).
2. **New standalone crate**, mirroring `effect-store`'s independent-sequence
   pattern — not indicated: that separation was explained by AD-10 as being
   about an already-existing differently-tabled crate, not a general
   "read-side gets its own crate" rule; read-side has no such existing home.
   - Effort: Medium, with no offsetting benefit found.

## 5. Risks / Open Questions for `sdd-design`

1. Schema shape — one table vs. two (offset/dedup have different key
   shapes; effect-store's own two-table split for the analogous
   state/dedup pair is the closer precedent than a merged table).
2. Dedup retention/eviction — neither `DedupStore` trait nor any adapter
   defines one; an unbounded dedup table (as effect-store's own
   `effect_dedup` would be without its separate TTL worker) needs an
   explicit decision, not silent deferral.
3. Concurrency — `TagSchedulerImpl::start_projection`
   (`crates/runtime/src/read_side/scheduler.rs:66-108`) processes tags
   sequentially within one process, but nothing prevents two service
   replicas from polling the same projection/tag/tenant concurrently; the
   adapter must use `ON CONFLICT`/conditional-`UPDATE`, not a naive upsert.
4. Migration numbering — continue `ego-persistence`'s flat `013+` sequence
   unless a different crate is chosen.
5. Tenant-NULL handling — SPI's `tenant: &str` may not need the
   nullable-tenant pattern the existing adapters use; needs confirming
   against actual call sites (reference app's own usage is always
   tenant-scoped).
6. Testing — real-Postgres tests belong exclusively in `integration-tests/`
   via `isolated_database()`, never in a unit test.

## 6. Recommendation

Place the new adapter in `crates/persistence/src/postgres/`, continuing the
existing migration sequence at `013+`, following `reservation.rs`'s
conditional-write concurrency pattern, and add a
`ReadSideProgressStores::postgres(...)` constructor in the reference app
plus the `main.rs` rewiring. Leave schema shape, dedup retention, and
tenant-NULL handling as explicit `sdd-design` decisions.

## 7. Contract & Concurrency Addendum

Follow-up micro-exploration requested before `sdd-propose`, to close three
design-affecting questions the user flagged: table shape, dedup retention,
and tenant nullability — plus two additional questions on the concurrency
guarantees each SPI method actually offers.

**Q1 — Logical identity.**
- `OffsetStore` (`crates/domain/src/read_side/offset.rs:51-83`): doc "reads
  and writes projection offsets per (projection_id, tag, tenant)...
  independent per (projection_id, tag, tenant) tuple." `read_offset`/
  `write_offset` (69-83) take `projection_id: &str, tag: &EventTag, tenant:
  &str`. Value is `Offset::Sequence(i64)` (offset.rs:12-16, "last confirmed
  event_version post-atomic-commit"). **Identity/PK = `(projection_id,
  tag.value(), tenant)`**, confirmed by reference-app's `type OffsetKey =
  (String, String, String)` (`examples/reference-app/src/read_side/store.rs:146-148`).
- `DedupStore` (`crates/domain/src/read_side/dedup.rs:20-51`): doc
  "Deduplication scope: (projection_id, tag, event_id)." `seen`/`mark_seen`
  (37-51) take `projection_id: &str, tag: &EventTag, event_id: &str`,
  boolean presence only. **Identity/PK = `(projection_id, tag.value(),
  event_id)`**, confirmed by `type DedupKey = (String, String, String)`
  (store.rs:143-145). Tenant is NOT part of dedup identity per the SPI —
  frozen, out of scope for PROD-014B.

**Q2 — Dedup retention horizon.** `crates/runtime/src/read_side/scheduler.rs`,
`crates/domain/src/read_side/runner.rs`, `session.rs` have zero TTL/eviction
logic and no coded relationship between dedup removal and offset advancement
(session.rs:91-176 marks every unique event seen then writes offset, forever
accumulating). **Retention is entirely undefined today.** Contrast:
`crates/effect-store`'s analogous `effect_dedup` table has an explicit
separate cleanup path (`crates/effect-store/src/postgres/mod.rs:285-356`) —
the workspace's own precedent is that retention is always a deliberate,
separate decision, never silent. **Decision: PROD-014B's spec states
retention explicitly as unbounded for now** (no purge mechanism shipped in
this change); a follow-up owns eviction if/when table growth becomes a
concern.

**Q3 — Tenant obligatory or optional.** SPI is `tenant: &str`, never
`Option<&str>`. All call sites are tenant-scoped concretely
(`crates/runtime/src/read_side/scheduler.rs:840-882`;
`examples/reference-app/src/read_side/mod.rs:199` filters out any tag with
no decodable tenant before it reaches a store call). The framework's
"systemwide"/global tenant concept exists only for write-side stores via
`Option<&str>` (`crates/domain/src/persistence/tenant.rs:29-35`, consumed by
`event_store.rs`, `repository.rs`, `persistence/snapshot.rs`, implemented as
`tenant_id IS NOT DISTINCT FROM $N` in `crates/persistence/src/postgres/snapshot.rs:67,77`).
That concept's type is structurally absent from the read-side SPI.
**Decision: tenant column is `NOT NULL`** — do not copy the nullable
pattern from write-side stores; there is no framework-level global-tenant
representation at the read-side SPI layer to justify it.

**Q4 — OffsetStore concurrency guarantee.** `write_offset` (offset.rs:77-83)
has no expected-previous-offset param, no CAS token, no ordering language in
its doc — a plain overwrite. The caller (session.rs:91-176) never checks
whether its write "won." Single-writer-per-tag holds only intra-process:
`TagSchedulerImpl::start_projection` (scheduler.rs:66-108) awaits each tag's
session sequentially, no per-tag `tokio::spawn` — but nothing in
`crates/domain/src/read_side/` or `crates/runtime/src/read_side/` prevents a
second replica polling the same `(projection_id, tag, tenant)` concurrently
(no leader election/lock/fencing token found anywhere in read-side code).
**Decision: the Postgres adapter implements `write_offset` as a plain
upsert** (matches the SPI's own overwrite semantics); it does not invent a
CAS guarantee the trait doesn't express.

**Q5 — DedupStore atomicity (highest priority).** `seen`/`mark_seen`
(dedup.rs:37-51) are two separate trait methods, no atomic check-and-insert.
`ReadSideSession::execute`: Phase 2 `seen()` gates (session.rs:116-128),
Phase 3 runs `handler.handle()` (135), Phase 4 `mark_seen()` commits only
after (142-149) — check-then-act with real side effects in the window.
Under two concurrent writers, both can observe `seen()==false` before either
marks, and **both already ran the handler** by the time either commits.
Unexercised today only because `InMemoryDedupStore`/`FakeDurableDedupStore`
(store.rs:196-238,279-307) are process-local `Arc<Mutex<..>>` and the
scheduler processes tags strictly sequentially. Precedent:
`EffectDedupStore::reserve` (`crates/effect-store/src/postgres/mod.rs:699-756`)
is ONE atomic `INSERT ... ON CONFLICT DO NOTHING` call that reserves
**before** any side effect runs — the workspace's own counter-example.

**Verdict on Q5**: the `DedupStore` trait API as written is **not** safe
against concurrent/distributed writers, and no Postgres adapter alone can
fully fix it. A UNIQUE constraint inside `mark_seen` closes the bookkeeping
race (the table converges, no error on a double mark) but not the
double-execution race, since the handler already ran for both callers
before either reaches `mark_seen`. **This is a genuine SPI-level gap
PROD-014A left uncovered, not a Postgres-adapter-level detail.**

### Is PROD-014B unblocked for `sdd-propose`?

Yes, conditionally. Q1–Q4 are fully implementable in a Postgres adapter with
no SPI change (two tables, composite UNIQUE keys, tenant `NOT NULL`,
`write_offset` as plain upsert, retention explicitly unbounded for now).
Q5 does not block PROD-014B, since its scope excludes SPI changes — but the
spec must explicitly document the concurrency limitation as an accepted,
named boundary: **at-least-once delivery with best-effort dedup
bookkeeping, reliable against double-execution only under an unenforced
single-writer-per-tag assumption.** Closing it fully needs an
orchestration-level single-writer-per-tag guarantee or a future atomic
`reserve`-style SPI change — both explicitly out of scope for PROD-014B,
and a candidate for a named follow-up item in the proposal's non-goals.

## Ready for Proposal

Yes — conditional on `sdd-propose` carrying forward the Q5 concurrency
boundary as an explicit, named acceptance-criteria item (not silently
absorbed), and the Q2/Q3/Q4 decisions above as fixed design inputs.
