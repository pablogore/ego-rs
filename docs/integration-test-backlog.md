# Integration-test backlog

Every test target removed from this workspace, and the coverage each one held.

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

**A separate integration crate/workspace, outside this one**, so that infrastructure
dependencies never enter the workspace the constitution governs. It is not built
yet. Nothing here should be reconstructed inside `ego-rs` itself.

Until it exists, every property in the tables below is **unverified**. That is a
real, accepted, temporary loss — not a claim that the in-process suites cover it.

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

`tests/support/mod.rs` was **kept** — `make_token` is used by four retained tests
(`http_route`, `ingress_trace_context`, `ingress_trace_wiring`, and the support
module itself).

---

## Known stale references left in place

`docs/test-audit.md` is a dated audit snapshot that predates
`crates/integration-tests` and recommends creating it. It is a historical record,
not live wiring, and was deliberately not rewritten — amending a dated report to
match a later decision would falsify it. Read it as of its date.
