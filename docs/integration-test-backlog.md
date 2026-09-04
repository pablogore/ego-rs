# Integration-test backlog

An inventory of the **invariants** that left this workspace when 13 targets were
removed, and where each one used to be asserted. The count — 52 tests — is a
record of what was deleted, not a target to reach.

**Tracked as issue #275.** This document is the authoritative inventory; the issue
is the counted work item and carries the rebuild's binding constraints. A note in a
document is not a schedule — if you are reading this to find out whether the work is
planned, the answer is in #275, not here.

## Governing rule — read before using anything below

**This is not a mandate to recreate 52 tests.** It is a list of properties that are
currently unverified. The rebuild recovers *invariants*, not cardinality, and the
old topology is explicitly not the target: it took 20–35 minutes per run, and
reproducing it would trade one constitutional problem for an unusable one.

The rebuilt suite is surgical, and every constraint below binds:

- **Invariants, not count.** One test may cover several entries here; several
  entries may collapse into one. Finishing with far fewer than 52 tests is the
  expected outcome, not a shortfall.
- **No duplicated contract coverage.** Anything already proven by the in-process
  contract, conformance, or unit suites must not be re-asserted against
  infrastructure. Infrastructure is for what infrastructure alone can show.
- **Each infrastructure test justifies itself.** Every retained test states the
  property that *cannot* be established in-process. No justification, no test.
- **One PostgreSQL per run.** A single container is reused across the whole run;
  per-test containers are what made the old suite unusable.
- **Time budget:** whole suite **≤5 minutes**; any individual slice **≤1–2 minutes**.
  These are the acceptance criteria of the rebuild, not aspirations.
- **No arbitrary waits.** No `sleep`, no fixed timeouts standing in for a condition.
  Synchronise on observable state — the `pg_locks` poll below is the pattern.

Each **"Rebuild requirement"** note names the invariant that must survive and the
mechanism that made it observable. It is not an instruction to restore the test that
carried it.

## Status of the affected behaviour

Until #275 closes, any report on the behaviour below must read: **"implemented and
contractually tested; real PostgreSQL and transports not verified."** Nothing here
is a claim that the in-process suites cover it.

## Index by category

| Category | What was lost | Where |
|---|---|---|
| **SQL / migrations** | `aggregate_type` backfill: clean split, revert, zero-row commit, store refusing to open until complete; aborts proven to run *before* the first `UPDATE` | §1 `aggregate_type_backfill` |
| **Constraints** | Index shapes read from `pg_index` rather than the `.sql`; complete tenant-partitioned uniqueness pairs with no gap or overlap; the reservation table's AD-1 partial pair; refusal of inconsistent completions and non-positive fencing tokens | §1 `schema_index_assertion`, `reservation_store_postgres` |
| **Concurrency** | Concurrent appends yielding one winner and only conflicts; unique violation surfacing as a conflict with the real version; six contenders racing one expired lease. Determinism came from polling `pg_locks`, never from sleeping | §1 `stream_identity_uniqueness`, `reservation_store_postgres` |
| **Fencing** | A takeover whose `UPDATE` waits on a row lock re-checking the lease it read, with the window forced open via `SELECT … FOR UPDATE` — **rebuilt**, see the note; exhaustion at the storable token limit changing nothing — still missing | §1 `reservation_store_postgres` — was **highest value** |
| **Readiness** | `probe()` against a real database: reachable, empty-table, non-mutating, and the down-and-back-up transition | §1 `reservation_store_readiness_postgres` |
| **Recovery** | Unit-of-work rollback and isolation; recovery of a never-persisted aggregate against both implementations; NULL-tenant streams under SQL three-valued logic | §1 `event_store_uow`, `recovery_of_a_fresh_aggregate`, `systemwide_streams` |
| **Transport / e2e** | Real socket bind and bounded graceful shutdown; OTLP wire round-trip asserting received ids; CORE-018's real-HTTP end-to-end criterion | §2, §3, §4 |

## Why they were removed

`scripts/detect-integration-tests.sh` — the CI guard for CC-R11 (No Infrastructure
Dependency), CC-R12 (Unit-Test Enforcement), UT-R2 (No Real Infrastructure) and
UT-R4 (No Testcontainers) — **was failing** against `crates/integration-tests`. The
crate declared `testcontainers` and `testcontainers-modules` as dev-dependencies,
which the constitution forbids outright. The repository was enforcing a rule it
also violated, and the guard's failure was simply being tolerated.

This removal resolves that by taking the violating targets out of this workspace
rather than by weakening the guard. It is not a decision that the coverage was
worthless — most of it tested properties that genuinely cannot be established
in-process, which is exactly why it needs somewhere else to live.

## Where they go

**Outside the root Cargo workspace, inside this repository, at `integration-tests/`**
— an independent Cargo workspace that is not a member of the root one. Infrastructure
dependencies therefore never enter the workspace the constitution governs, while the
invariants stay versioned alongside the code they cover.

`cargo test --workspace` keeps running with no Docker and never compiles or runs that
directory; the suite is invoked explicitly with
`cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`. Nothing here may be
reconstructed inside the **root workspace** — that is what re-breaks the constitution.
Nothing here belongs in another repository either.

It is not built yet. Until it exists, every property in the tables below is
**unverified**. That is a real, accepted, temporary loss — not a claim that the
in-process suites cover it.

## Scope of the removal

Only targets that touch something outside the process were removed: a database, a
socket, a container. The ~97 remaining targets under `tests/` are in-process — they
are "integration tests" only in cargo's sense (a separate crate compiled against the
public API) and stay exactly where they are.

Explicitly retained, untouched:

- unit tests in `src` (`#[cfg(test)]`), including `crates/testkit`'s own suites
- doctests
- `trybuild` compile-fail/compile-pass contracts in `crates/service-sdk/tests/`
- in-process contract and conformance suites (`persistent-entity`, `ego-scheduler`,
  `security-sdk`, `security-jwt`, the rest of `service-sdk` and `reference-app`)

The shared conformance harnesses in `ego-testkit` were **not** removed and are still
exercised: `assert_event_store_conformance` by
`crates/infrastructure/tests/in_memory_event_store_conformance.rs` and
`crates/persistent-entity/tests/default_store_conformance.rs`, and
`assert_reservation_store_conformance` (with its three group functions) by
`crates/testkit/src/reservation.rs`'s own in-memory test. The rebuilt Postgres suites
must run these same definitions rather than re-deriving them — that pairing is the
only thing that keeps the two implementations honest against each other.

---

## 1. `crates/integration-tests/` — removed entirely (10 targets, 47 tests)

Needed Docker and a real PostgreSQL 14 container.

### `postgres_event_store_conformance.rs` (1 test)

Runs `ego-testkit`'s shared `EventStore` conformance contract against
`PostgresEventStore`. Paired deliberately with the in-memory run of the identical
harness; that pairing previously surfaced four divergences between the two
implementations.

**Rebuild requirement:** the durable store must satisfy the same shared definitions
as the in-memory one, not a copy of them.

### `event_store_characterization.rs` (3 tests)

Pinned today's observable behaviour of the synchronous `append` path before that
contract changed: version advancement and load ordering, rejection of a stale
`expected_version` via the explicit check, and that the `events` table enforces
stream-identity uniqueness.

### `event_store_uow.rs` (4 tests)

Unit-of-work semantics that cannot be checked without a real transaction: dropping
a unit of work without committing persists nothing; committing makes every append
durable; a failure on the second stream discards the first; an open unit of work is
invisible to other readers.

**Rebuild requirement:** transactional isolation and rollback are the whole point —
an in-memory double cannot stand in.

### `stream_identity_uniqueness.rs` (5 tests)

Database-enforced uniqueness of the stream identity (migration 008) and the store's
translation of the violation it makes reachable: a duplicate identity refused for a
tenant and in the systemwide partition, two tenants permitted to hold the same
identity, a unique violation surfacing as a conflict that reports the real version,
and concurrent appends for one version producing exactly one winner with the rest
as conflicts.

**Rebuild requirement:** the concurrency test uses a `pg_locks` poll to know a
statement is genuinely blocked rather than sleeping. Keep that; a race test that
sometimes exercises nothing reports success either way.

### `systemwide_streams.rs` (5 tests)

The NULL-tenant ("systemwide") mode. `resolve_tenant(None)` resolves to SQL NULL and
plain `=` never matches it under three-valued logic — a defect only a real database
exhibits. Covers version advancement across appends, load returning the stream
rather than reporting it absent, stale `expected_version` still rejected, a
systemwide and a tenant stream with the same identity staying separate, and
`list_aggregate_ids` covering the systemwide partition.

**Rebuild requirement:** this class of bug is invisible to any in-memory store,
which uses `Option` equality rather than SQL NULL semantics.

### `schema_index_assertion.rs` (3 tests)

Queries PostgreSQL's own catalog — not the migration source — for the indexes
enforcing tenant-partitioned uniqueness: every registered table has a complete
uniqueness pair, the two predicates cover the table with no gap and no overlap, and
no table carries a lopsided half of a pair.

**Rebuild requirement:** must read `pg_index`, not the `.sql` file. Reading the file
back only proves the file says what it says.

### `aggregate_type_backfill.rs` (9 tests)

The offline `events.aggregate_type` backfill, driven with raw SQL so each scenario
can construct stored shapes the store's own API would never produce. Covers: clean
data splitting every row while preserving row count and stream integrity; ambiguous
aggregate id aborting the whole run and naming the row; whitespace-only remainder
aborting and naming the row; an aggregate id matching no registered type aborting;
post-split identity collision aborting and naming both rows; a stream with a version
gap rolling the whole transformation back; zero rows committing trivially and still
setting the column `NOT NULL`; the store refusing to open until the backfill has
completed; and `revert` exactly rejoining what the backfill split.

**Rebuild requirement:** the abort paths must be proven to run **before** the first
`UPDATE`, and the rollback paths must leave the table byte-identical. This is the
largest single block of lost coverage.

### `recovery_of_a_fresh_aggregate.rs` (3 tests)

Recovering an aggregate that has never been persisted, through the facade the actor
uses, against **both** store implementations: the durable store reports a
never-written aggregate as absent, and recovery succeeds against the durable and the
in-memory store alike.

**Rebuild requirement:** keep the both-implementations shape. The defect this caught
was a divergence, not an absolute behaviour.

### `reservation_store_postgres.rs` (10 tests)

`PostgresOperationReservationStore` against `ego-testkit`'s shared reservation
contract (whole contract plus the three groups in isolation), the AD-1 complementary
partial index pair read from the catalog, the table refusing an inconsistent
completion and a non-positive fencing token, and three properties the sequential
scenarios cannot reach:

- six contenders racing one expired lease yield exactly one winner whose token
  advanced by exactly one;
- a takeover whose `UPDATE` waits on a row lock re-checks the lease it read rather
  than the lease it remembers (the window is forced open with `SELECT … FOR UPDATE`,
  not raced for);
- a takeover at `i64::MAX` reports exhaustion and changes owner, token and lease not
  at all.

**Rebuild requirement:** the `UPDATE`'s own `lease_until <= $N` guard is only ever
exercised by the forced-window test. It was verified by neutralising the guard and
watching the conformance suite stay green — so a rebuild that drops this test loses
the only check on the fencing guarantee under contention.

**REBUILT** as `integration-tests/tests/fencing_window_postgres.rs`. The forced
window is reproduced with `SELECT … FOR UPDATE`, the lease is renewed inside the
holding transaction while the contender blocks, and the refusal is required.
Re-measured on the rebuild: neutralising the predicate fails that test while
`replay_from_postgres`, `conflict_from_postgres` and `takeover_fencing_postgres`
all stay green. So the note above still describes the situation exactly — it is the
only check, and it now exists. The other tests are named rather than counted: a
count would read as a re-measured claim the moment another test lands, when in fact
nobody had re-run it.

Determinism came from polling `pg_stat_activity` for a backend blocked on a lock,
not `pg_locks.relation`: a row-lock wait is a `transactionid` lock with a NULL
`relation`, so the obvious join matches nothing. Worth carrying forward to the
remaining concurrency items on this list, which the original note describes as
`pg_locks` polls.

Still missing from this group: six contenders racing one expired lease, and
exhaustion at the storable token limit.

### `reservation_store_readiness_postgres.rs` (4 tests)

Added by B3.7. The durable store's `probe()` against a real database: a reachable
store reports ready; an empty table is still a reachable store; twenty probes leave
every row byte-identical; and readiness follows Postgres down and back up.

**Rebuild requirement, measured not assumed:** Docker re-allocates a dynamically
published port on `stop`/`start` (verified: 33838 → 33839), so a restarted container
comes back on a different host port and the same pool can never recover. The
down-and-up transition was driven through a TCP forwarder the test owned — binding
`127.0.0.1:0`, severing in-flight connections on the way down so the pool's
*established* connections break too, and closing new ones on accept for a
deterministic error instead of a hang. Reproduce that mechanism; `stop`/`start` will
not work.

B3.7's unit-level coverage in `crates/service-sdk` is unaffected and still runs: the
contributor's mapping, its wiring, instance identity via the probe counter, the
credential-redaction assertions, and readiness-vs-liveness separation.

---

## 2. `crates/infrastructure/tests/otlp_export_roundtrip.rs` — removed (2 tests)

Stood up a minimal in-process OTLP collector per protocol on a real socket, pointed
a real `OtlpTracer` at it, exported one span, and asserted the **received** span's
`trace_id`/`span_id`/`parent_span_id` equalled the domain ids — not merely that
something arrived. One test per protocol: gRPC and HTTP.

**Rebuild requirement:** assert the received ids, not arrival. `OtlpConfig`'s
`Grpc`/`Http` selection is otherwise unproven end-to-end.

Dev-dependencies removed with it: `opentelemetry-proto`, `tonic`, `tokio-stream`,
`prost`, `axum`. `opentelemetry-otlp` stays — it is a real dependency of the crate,
not test-only.

## 3. `crates/transport/tests/server.rs` — removed (1 test)

`serve()` binds a real ephemeral socket, serves a trivial router, accepts one real
client request, then stops within a bounded timeout once its shutdown signal
resolves.

**Rebuild requirement:** graceful shutdown is the property — that it stops, bounded,
after the signal. `axum` and `tokio` stay; they are real dependencies of the crate
and the two retained transport tests use them.

## 4. `examples/reference-app/tests/e2e_register.rs` — removed (2 tests)

Full end-to-end acceptance against a real `axum::serve()` socket with a real HTTP
client and a real HS256 JWT: a request without a JWT returns 401 and never reaches
the operation, and a request with a valid JWT registers both entities end to end.

This was the named success criterion of the CORE-018 proposal ("A real HTTP request
against a running axum server completes registration end-to-end"), so its removal
un-verifies a shipped acceptance claim until the integration workspace exists.

**Resolved (CORE-018, `develop@037628a`):** the integration workspace now exists.
`integration-tests/tests/infrastructure/wire_register_postgres.rs` re-proves this
claim over the real wire — real TCP, real HTTP client, real JWT auth, real
PostgreSQL — through the reference app's real composition root, run via
`cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`. The
paragraph above is left as written: an accurate record of the gap as it stood
at the time this backlog entry was written.

`tests/support/mod.rs` was **kept** — `make_token` is used by four retained tests
(`http_route`, `ingress_trace_context`, `ingress_trace_wiring`, and the support
module itself).

---

## Known stale references left in place

`docs/test-audit.md` is a dated audit snapshot that predates
`crates/integration-tests` and recommends creating it. It is a historical record,
not live wiring, and was deliberately not rewritten — amending a dated report to
match a later decision would falsify it. Read it as of its date.
