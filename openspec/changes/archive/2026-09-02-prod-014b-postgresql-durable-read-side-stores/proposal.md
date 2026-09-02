# Proposal: PROD-014B — PostgreSQL Durable Read-Side Stores

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers).

## Objective

PROD-014A shipped a `Profile::Production` gate that refuses a read-side projection whose
progress pair is volatile — and left nothing in-tree that can satisfy it. PROD-014B supplies
the missing implementation: a PostgreSQL `OffsetStore` + `DedupStore` pair whose state survives
a process restart, conforming to the SPI exactly as it exists today.

## Intent

A production host adopting PROD-014A has exactly two options in-tree today: fail the gate, or
register a test-only fake (`FakeDurableOffsetStore` / `FakeDurableDedupStore`,
`examples/reference-app/src/read_side/store.rs:150-307`) that claims durability it does not
have. `examples/reference-app`'s own production path takes neither: it passes `None` for
read-side progress, with an explicit "PROD-014A F-1" comment
(`examples/reference-app/src/main.rs:109-114`).

A gate with no satisfying implementation is a refusal, not a guarantee. PROD-014B discharges
PROD-014A's named follow-up F-1 and nothing more: no SPI change, no gate change, no
registration change.

It also carries forward one thing that must not get lost in the word "durable". Durable
storage of dedup records is **not** exactly-once processing, and PROD-014B does not become
one by being backed by PostgreSQL. `ego-rs` targets distributed production, so the honest
statement is an adoption constraint rather than a caveat: **PROD-014B is adoptable in Production
only under single-writer-per-`(projection_id, tag, tenant)`, until an atomic reservation
mechanism or equivalent enforcement exists.** That boundary is stated below as a named, accepted
limitation with its own success criterion — see **Guarantees and Named Limitations**.

## Active Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | **Two tables, not one.** `projection_offsets` with identity `(projection_id, tag, tenant)`; `projection_dedup` with identity `(projection_id, tag, event_id)`. Tenant is **not** part of dedup identity | The two SPIs have genuinely different key shapes and the trait docs say so: `OffsetStore` is "independent per (projection_id, tag, tenant) tuple" (`crates/domain/src/read_side/offset.rs:51-83`); `DedupStore`'s "deduplication scope: (projection_id, tag, event_id)" (`dedup.rs:20-51`). Reference-app's own `OffsetKey`/`DedupKey` tuples confirm both (`read_side/store.rs:143-148`). `crates/effect-store`'s analogous state/dedup pair is likewise two tables |
| D-2 | **Both tenant columns are `NOT NULL`.** The nullable-tenant pattern used by the write-side adapters (`tenant_id IS NOT DISTINCT FROM $N` plus partial unique indexes, `crates/persistence/src/postgres/snapshot.rs:63-137`) is deliberately **not** copied | The read-side SPI's parameter is `tenant: &str`, never `Option<&str>`. The framework's global/"systemwide" tenant concept exists only for write-side stores (`crates/domain/src/persistence/tenant.rs:29-35`) and is structurally absent from the read-side SPI. Every read-side call site is concretely tenant-scoped (`crates/runtime/src/read_side/scheduler.rs:840-882`; `examples/reference-app/src/read_side/mod.rs:199` filters out any tag with no decodable tenant before it reaches a store call). A nullable column would model a state the SPI cannot express |
| D-3 | **`write_offset` is a plain upsert.** No CAS token, no expected-previous-offset, no ordering check | `write_offset` (`offset.rs:77-83`) takes no expected value and its doc language is a plain overwrite; the only caller (`crates/domain/src/read_side/session.rs:91-176`) never checks whether its write won. An adapter that invented a CAS guarantee would be stricter than the contract it implements, and would fail for callers the trait considers valid |
| D-4 | **Dedup retention is explicitly unbounded in this change.** No purge, no TTL, no eviction ships in PROD-014B, and the spec says so outright rather than omitting it | Retention is undefined at every layer today — `scheduler.rs`, `runner.rs`, and `session.rs` contain zero TTL/eviction logic and no coded relationship between dedup removal and offset advancement. The workspace's own precedent is that retention is a deliberate, separately-owned decision: `crates/effect-store`'s `effect_dedup` has an explicit separate cleanup path (`crates/effect-store/src/postgres/mod.rs:285-356`). Silent omission would be the one unacceptable option. Follow-up F-2 |
| D-5 | **Placement: `crates/persistence/src/postgres/`, continuing the flat migration sequence at `013+`**, following `reservation.rs`'s conflict-safe write pattern (`INSERT ... ON CONFLICT DO NOTHING` / conditional `UPDATE`) | Matches every existing golden-path adapter's placement and adds zero dependency-graph edges: `ego-persistence` already depends only on `ego-domain`, where both SPIs live, and `reference-app` already depends on `ego-persistence`. `crates/effect-store`'s independent `001/002` sequence was justified by AD-10 as a property of an already-separate crate, not a general rule |
| D-6 | **No SPI change, no `Profile::Production` gate change, no `AppBuilder::read_side_progress` change.** The adapter gets `is_durable() -> true` and the existing generic `Arc<T>` forwarding impls for free (`offset.rs:91-119`, `dedup.rs:59-86`) | PROD-014A shipped all three surfaces. This change implements against them; changing them here would make it a second governance change wearing an adapter's name |
| D-7 | **The `DedupStore` check-then-act concurrency gap is carried as a named, accepted contractual limitation — not closed, and not silently absorbed.** It is stated in the acceptance criteria (see Guarantees and Named Limitations), not as an implementation note | `seen()` and `mark_seen()` are two separate trait methods (`dedup.rs:37-51`) and `ReadSideSession::execute` runs the handler between the check (`session.rs:116-128`) and the commit (`session.rs:142-149`). No Postgres adapter can close that window from inside `mark_seen`. Closing it needs orchestration-level single-writer-per-tag enforcement or an atomic reserve-style SPI method — both out of scope. Follow-up F-1 |
| D-8 | **Real-PostgreSQL conformance tests live exclusively in `integration-tests/`**, via `ego_integration_tests::isolated_database()`, matching the 19 existing `tests/infrastructure/*_postgres.rs` suites | `ego-rs-testing-strategy`: no unit test may reach a real database. `integration-tests` is a separate workspace and is the only place real infrastructure is admitted |

## Atomicity Gate

**Run, and it cut scope twice.** Dedup retention/eviction was considered and removed (D-4 → F-2):
it is independently shippable, needs its own horizon decision, and PROD-014B is useful and
testable without it. Closing the dedup concurrency gap was considered and removed (D-7 → F-1):
it is an SPI/orchestration change, not an adapter change, and folding it in would silently turn
"implement the durable pair" into "redesign the read-side dedup contract".

What remains is one indivisible capability. The offset store alone cannot satisfy the gate — it
validates the **pair**. The dedup store alone likewise cannot. Both share one migration
sequence, one placement decision, one durability declaration, and one conformance suite.

**ATOMICITY: PASS**

## Scope

**Boundary at a glance**

| | |
|---|---|
| **PROD-014B includes** | PostgreSQL durable read-side progress · migrations · adapters · conformance tests · reference-app production wiring |
| **PROD-014B excludes** | Multi-writer correctness · atomic reservation · retention/cleanup · replica detection |

### In Scope

- **IS-1** — `PostgreSQLOffsetStore`: durable `OffsetStore` over `projection_offsets`, identity
  `(projection_id, tag, tenant)`, tenant `NOT NULL` (D-1, D-2), `write_offset` as a plain upsert
  (D-3), `is_durable() -> true`.
- **IS-2** — `PostgreSQLDedupStore`: durable `DedupStore` over `projection_dedup`, identity
  `(projection_id, tag, event_id)` with a UNIQUE constraint so a repeated `mark_seen` converges
  instead of erroring (D-1), `is_durable() -> true`.
- **IS-3** — Migration(s) `013+` in `crates/persistence/src/postgres/migrations/`, continuing the
  existing flat sequence and run by the existing `include_str!` + `sqlx::raw_sql` runner (D-5).
- **IS-4** — Re-export from `crates/persistence/src/postgres/mod.rs`, matching the existing
  one-file-per-store shape.
- **IS-5** — A `ReadSideProgressStores::postgres(pool)` constructor in
  `examples/reference-app/src/read_side/mod.rs`, alongside the existing `::in_memory()` and
  `::fake_durable()`.
- **IS-6** — Rewire `examples/reference-app/src/main.rs:109-114` from `None` to the real
  Postgres pair, retiring the "PROD-014A F-1" comment. **Part of the Definition of Done, not a
  deferrable slice**: without it PROD-014B ships infrastructure that no reference composition
  path proves usable.
- **IS-7** — A real-Postgres conformance suite under `integration-tests/tests/infrastructure/`
  using `isolated_database()` (D-8), covering: round-trip, restart survival, absent-key reads,
  tenant isolation on offsets, repeated `mark_seen` convergence, and dedup identity being
  tenant-independent.
- **IS-8** — The concurrency boundary is stated as an explicit, named limitation in the spec and
  in the adapters' own public documentation — rustdoc, README, and configuration docs — in words
  an operator can read (D-7). Those docs MUST state the single-writer adoption constraint and
  MUST NOT present a multi-replica projection configuration as officially supported. The code
  does not enforce it; the documentation must at least make it legible.
- **IS-9** — Spec deltas per the Capabilities section.

### Out of Scope

- **OOS-1** — Any change to `OffsetStore` / `DedupStore`, `Profile::Production` gate logic, or
  `AppBuilder::read_side_progress` (D-6).
- **OOS-2** — Closing the dedup check-then-act gap: no atomic reserve method, no
  single-writer-per-tag enforcement, no leader election, no fencing token, no lease, and no
  peer/replica detection (D-7 → F-1). Detecting a concurrent peer from inside a Postgres adapter
  would drag leases and distributed coordination into a persistence spec; that belongs to F-1.
- **OOS-3** — Dedup retention, TTL, purge, or eviction of any kind (D-4 → F-2).
- **OOS-4** — Any backend other than PostgreSQL. Stoolap, Oracle, ClickHouse, MySQL, Redis,
  RocksDB, DynamoDB, Cassandra/Scylla and SQLite are excluded — a full codebase audit found zero
  productive code for any of them, only illustrative prose in archived OpenSpec.
- **OOS-5** — A durable `ReadSideStore` (the event view a projection polls). Inherited unchanged
  from PROD-014A OOS-8 / F-2.
- **OOS-6** — Removing, deprecating, or hiding `InMemoryOffsetStore` / `InMemoryDedupStore` or the
  fake durable pair. They stay valid and explicit for Dev and tests.
- **OOS-7** — Multi-worker ownership, partition leasing, HA, exactly-once delivery, and
  projection rebuild orchestration. Inherited unchanged from PROD-014A OOS-4.
- **OOS-8** — Governing a projection spawned outside the composition root. Inherited unchanged
  from PROD-014A OOS-7.

## Capabilities

### New Capabilities

- `read-side-durable-progress`: the observable durability contract for read-side progress state —
  what survives a restart, what identity each record has, what retention is promised, and
  explicitly what concurrency guarantee is **not** offered.

### Modified Capabilities

- `read-side`: the named concurrency boundary — durable dedup bookkeeping does not imply
  exactly-once handler execution, and prevention of double execution rests on an unenforced
  single-writer-per-tag assumption.

If the spec phase finds an existing requirement already implies one of these, it folds rather
than manufacturing a delta.

## Approach

Follow the golden path already established by `event_store.rs`, `snapshot.rs`, and
`reservation.rs`: one file per store under `crates/persistence/src/postgres/`, `PgPool` by
constructor injection, `is_durable()` returning `true` unconditionally, re-exported from
`postgres/mod.rs`, and schema delivered by the next numbers in the existing flat migration
sequence.

Both writes are conflict-safe by construction rather than by coordination. `mark_seen` is an
`INSERT ... ON CONFLICT DO NOTHING` against a UNIQUE identity, so a repeated or concurrent mark
converges to one row with no error — the same shape `reservation.rs:213-219` already uses.
`write_offset` is an upsert on the offset identity, which is exactly the overwrite semantics the
SPI expresses (D-3). Every query binds its parameters; no identifier or value is interpolated
into SQL text, and every offset query carries `tenant` as a bound parameter.

Nothing else changes. The adapters inherit the generic `Arc<T>` forwarding impls, satisfy the
existing gate through the existing `is_durable()` mechanism, and are registered through the
existing `AppBuilder::read_side_progress` call.

## Guarantees and Named Limitations

**This section is acceptance criteria, not commentary.** It states what a host may rely on and,
with equal weight, what it may not.

> **Adoption constraint.** `ego-rs` targets distributed production, so multi-replica is the real
> deployment target — and that is exactly the configuration this change does not make safe.
> **PROD-014B is adoptable in Production only under single-writer-per-`(projection_id, tag,
> tenant)`, until an atomic reservation mechanism or equivalent enforcement exists (F-1).**
> This is a stated adoption constraint, not a caveat: a host running two replicas of the same
> projection is outside the guarantee, and nothing in this change detects or refuses it.

### What PROD-014B guarantees

- **G-1** — Offsets and dedup records written through these adapters survive a process restart:
  after a restart a projection resumes from its last persisted offset instead of replaying the
  whole stream with no dedup memory.
- **G-2** — The dedup bookkeeping table converges. Marking the same `(projection_id, tag,
  event_id)` more than once — sequentially or concurrently — yields exactly one row and no
  error.
- **G-3** — A `Profile::Production` composition registering this pair passes PROD-014A's gate on
  the strength of a real durable backend rather than a test fake. Passing that gate means the
  progress state is durable; it does not mean the deployment is multi-writer safe (see the
  adoption constraint above).
- **G-4** — Offsets are isolated per `(projection_id, tag, tenant)`; one tenant's progress is
  never observable as another's.

### What PROD-014B does NOT guarantee

- **L-1 — Not exactly-once.** PROD-014B delivers at-least-once processing with best-effort dedup
  bookkeeping. Nothing in this change makes read-side event handling exactly-once, and nothing
  in the change may be documented as if it did.
- **L-2 — Not safe deduplication under true multi-node concurrent writers.** `seen()` and
  `mark_seen()` are separate trait methods (`crates/domain/src/read_side/dedup.rs:37-51`) and
  `ReadSideSession::execute` runs the event handler **between** the check
  (`session.rs:116-128`) and the commit (`session.rs:142-149`). Two writers on the same
  `(projection_id, tag, tenant)` can both observe `seen() == false` and **both already have run
  the handler** before either marks. The UNIQUE constraint (G-2) fixes the bookkeeping — the
  table converges, no duplicate row, no error — and does nothing about the double execution.
  This is an SPI-level gap; no PostgreSQL adapter can close it from inside `mark_seen`.
- **L-3 — Prevention of double handler execution depends on an external, unenforced
  assumption.** `TagSchedulerImpl::start_projection`
  (`crates/runtime/src/read_side/scheduler.rs:66-108`) awaits each tag's session sequentially, so
  single-writer-per-tag happens to hold **inside one process today**. Nothing enforces it across
  replicas: read-side code contains no leader election, no lock, no lease, and no fencing token.
  A host running two replicas of the same projection is outside the guarantee, and this change
  neither detects nor refuses that configuration. Because multi-replica is the real deployment
  target for `ego-rs`, this is the binding adoption constraint stated above — PROD-014B is
  adoptable in Production **only** under single-writer-per-`(projection_id, tag, tenant)` until
  F-1 exists.
- **L-4 — Dedup storage growth is unbounded.** No purge, TTL, or eviction ships here (D-4).
  `projection_dedup` grows **linearly with the number of unique events processed** by a
  projection, monotonically and without an upper bound. Operators must observe that row count
  as an operational signal, not discover it as an incident.
  **Escalation trigger**: if a production projection reaches millions of rows within a short
  window, F-2 (retention and eviction) escalates to P0/P1 and is scheduled independently of this
  change — it does not wait on PROD-014B's own lifecycle or on F-1. Retention is excluded here
  because there is not yet real volume data to size a horizon against, and a cleanup path would
  change lifecycle, indexes, operations, and probably the API.
- **L-5 — `write_offset` is last-write-wins.** No CAS, no ordering guarantee, no detection of a
  concurrent overwrite (D-3) — this is the SPI's own semantics, faithfully implemented, not an
  adapter shortcoming.

## Required Semantics

```
Given a PostgreSQL offset store and a projection that has written offset N
When the process restarts and read_offset is called for the same
     (projection_id, tag, tenant)
Then it MUST return N — not None, and not a replay from the beginning.

Given a PostgreSQL offset store
When read_offset is called for a (projection_id, tag, tenant) never written
Then it MUST return None, and MUST NOT return another tenant's offset.

Given a PostgreSQL dedup store
When mark_seen is called twice for the same (projection_id, tag, event_id),
     sequentially or concurrently
Then both calls MUST succeed, exactly one row MUST exist, and a subsequent
     seen() MUST return true.

Given a PostgreSQL dedup store
When the same event_id is marked under two different tenants for the same
     (projection_id, tag)
Then it MUST be treated as already seen — tenant is not part of dedup identity.

Given two concurrent writers on the same (projection_id, tag, tenant)
When both observe seen() == false before either marks
Then the handler MAY run twice. This change does NOT prevent that, and the
     spec MUST state it as an accepted, named limitation rather than implying
     exactly-once semantics.

Given a composition declaring Profile::Production
When it registers this PostgreSQL pair through AppBuilder::read_side_progress
Then build() MUST succeed with no change to the gate's own logic.
```

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistence/src/postgres/` (new store file(s)) | New | `PostgreSQLOffsetStore`, `PostgreSQLDedupStore` (IS-1, IS-2) |
| `crates/persistence/src/postgres/migrations/013+` | New | Two tables with their UNIQUE identities, `NOT NULL` tenant on offsets (IS-3, D-1, D-2) |
| `crates/persistence/src/postgres/mod.rs` | Modified | Re-export (IS-4) |
| `examples/reference-app/src/read_side/mod.rs:93-117` | Modified | `ReadSideProgressStores::postgres(pool)` (IS-5) |
| `examples/reference-app/src/main.rs:109-114` | Modified | `None` → real Postgres pair; PROD-014A F-1 comment retired (IS-6) |
| `integration-tests/tests/infrastructure/` | New | Conformance suite via `isolated_database()` (IS-7, D-8) |
| `crates/domain/src/read_side/{offset,dedup,session,runner}.rs` | Untouched | No SPI change (OOS-1) |
| `crates/service-sdk/src/runtime/builder.rs`, `app/mod.rs` | Untouched | No gate or registration change (OOS-1) |
| `crates/runtime/src/read_side/scheduler.rs` | Untouched | No concurrency/ownership change (OOS-2) |
| `openspec/specs/{read-side-durable-progress,read-side}/spec.md` | New / Modified | Deltas per IS-9 |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| R-1 | "Durable PostgreSQL read-side" is later read as "read-side concurrency is solved", and a host deploys multiple replicas of one projection believing dedup protects it | High | The whole point of **Guarantees and Named Limitations**: L-1/L-2/L-3 are acceptance criteria with their own success criterion (SC-8), stated in the spec and in the adapters' public docs, not in a commit message. F-1 is named as the distinct atomic follow-up |
| R-2 | Unbounded `projection_dedup` growth becomes an operational incident | Med | Stated outright (D-4, L-4) rather than omitted, with F-2 named **and given a written escalation trigger**: millions of rows per projection in a short window escalates F-2 to P0/P1 independently of this change. Row count is an operational signal to observe, not a surprise. The workspace's own `effect_dedup` precedent shows retention is always separately owned |
| R-3 | The conformance suite proves round-trip but never restart survival — the single property that distinguishes this change from the in-memory pair | Med | IS-7 names restart survival as a required case; SC-1 asserts it explicitly |
| R-4 | Migration `013+` collides with another in-flight change touching the same sequence | Low | Flat sequence is single-owner and re-checked at apply; `crates/effect-store`'s independent sequence is unaffected (D-5) |
| R-5 | Review budget: two adapters, migrations, reference-app rewiring and an integration suite plausibly exceed 400 changed lines | Med | **Resolved in favour of scope, not splitting.** IS-5/IS-6 are Definition of Done: without the reference wiring this change ships infrastructure no composition path proves usable. If the forecast exceeds the budget, the resolution is to raise the budget for this change or trim accessory code — never to move the functional wiring into a later spec. `sdd-tasks` forecasts it under that constraint |
| R-6 | Tenant `NOT NULL` (D-2) is later found too strict if a global-tenant read-side concept ever appears | Low | It cannot be expressed today: `tenant: &str` has no null. Relaxing a column to nullable is a forward migration; the reverse would not be |

## Named Follow-Ups (deliberately not folded in)

- **F-1 — PROD-014C — Atomic Read-Side Event Claiming.** Close the L-2/L-3 gap so double handler
  execution is prevented rather than merely unlikely, and lift the single-writer adoption
  constraint. The name is deliberate: the real problem is not persisting dedup bookkeeping — this
  change already does that durably — it is **obtaining exclusion before the handler executes**.
  A writer must claim the event, not record afterwards that it processed one. The shape a real
  fix would take is already proven in this workspace: `EffectDedupStore::reserve`
  (`crates/effect-store/src/postgres/mod.rs:699-756`) is **one** atomic
  `INSERT ... ON CONFLICT DO NOTHING` that reserves **before** any side effect runs. Two routes
  are open — orchestration-level single-writer-per-tag enforcement, or a future atomic
  claim/reserve SPI method — and choosing between them is that change's work, not this one's.
  Peer/replica detection and enforcement belong here too (OOS-2). This follow-up must exist so
  "PostgreSQL durable" is never later confused with "read-side concurrency correctness".
  *Identifier note*: `explore.md` §Scope speculatively used "PROD-014C" for a possible second
  backend (Stoolap). That identifier is claimed here for atomic event claiming; a second backend,
  if it is ever wanted, takes a different identifier.
- **F-2 — Read-side dedup retention and eviction.** A horizon, a purge path, and the rule tying
  dedup removal to offset advancement (D-4, L-4). `crates/effect-store`'s separate cleanup path
  is the precedent. **Escalation trigger**: millions of rows per projection in a short production
  window raises this to P0/P1, scheduled independently of PROD-014B and of F-1.
- **F-3 — A durable `ReadSideStore`.** Inherited unchanged from PROD-014A (OOS-5); still open,
  still separate.

## Rollback Plan

Additive. Reverting is: delete the two adapter files and their re-export, delete migrations
`013+`, delete the conformance suite, remove `ReadSideProgressStores::postgres`, and restore
`main.rs` to `None` for read-side progress. No existing call site is touched by either the change
or the revert, and no SPI, gate, or registration signature changes in either direction.

The two new tables are additive and referenced by nothing else — a rollback may drop them or
leave them in place harmlessly. Because they are new, no data written before this change exists
to migrate or lose; state written by the adapters between deploy and revert is discarded, which
degrades a reverted projection back to today's replay-from-scratch behavior rather than
corrupting anything.

## Dependencies

- PROD-014A (archived) — the `is_durable()` SPI methods, the `Profile::Production` read-side gate,
  and `AppBuilder::read_side_progress`. All consumed unchanged.
- PROD-013 (archived) — `require_durably_configured` and the gate it established. Not touched.
- `crates/persistence`'s existing migration runner and `PgPool` conventions.
- `ego_integration_tests::isolated_database()` for the conformance suite.
- No new external dependency, crate, service, or infrastructure. `sqlx` and PostgreSQL are
  already workspace dependencies.

## Success Criteria

- [ ] **SC-1** — After a restart, a projection using the PostgreSQL pair resumes from its last
      persisted offset. A test proves this by dropping and rebuilding the store against the same
      database, not by asserting on an in-process value.
- [ ] **SC-2** — `read_offset` for an unwritten `(projection_id, tag, tenant)` returns `None`, and
      never another tenant's offset.
- [ ] **SC-3** — Marking the same `(projection_id, tag, event_id)` twice succeeds both times,
      leaves exactly one row, and leaves `seen()` returning `true`.
- [ ] **SC-4** — Dedup identity is tenant-independent: the same `event_id` under a different
      tenant for the same `(projection_id, tag)` is reported as already seen.
- [ ] **SC-5** — Both adapters report `is_durable() == true`, and a `Profile::Production`
      composition registering them builds successfully with no change to the gate's logic.
- [ ] **SC-6** — `examples/reference-app`'s production path registers the real Postgres pair; the
      "PROD-014A F-1" `None` placeholder is gone.
- [ ] **SC-7** — Every SQL statement binds its parameters; no value or identifier is interpolated
      into SQL text, and every offset query carries `tenant` as a bound parameter.
- [ ] **SC-8** — L-1, L-2, L-3 and L-4 appear as an explicit named limitation in the spec **and**
      in the adapters' public documentation, in prose a human can read. Nowhere in the delivered
      change is this pair described as exactly-once, concurrency-safe, or safe for multi-replica
      projection writers.
- [ ] **SC-9** — PROD-014C — Atomic Read-Side Event Claiming (F-1) is recorded as a distinct
      atomic follow-up, referencing `EffectDedupStore::reserve` as the proven shape, so the gap
      has a named owner rather than an implicit one.
- [ ] **SC-10** — No real-PostgreSQL test exists outside `integration-tests/`, and every new one
      obtains its database from `isolated_database()`.
- [ ] **SC-11** — `crates/domain/src/read_side/`, `crates/service-sdk`'s gate and registration, and
      `crates/runtime/src/read_side/scheduler.rs` are unmodified; `cargo test --workspace` shows
      zero new failures.
- [ ] **SC-12** — The adapters' rustdoc, the persistence README, and the configuration docs state
      the single-writer-per-`(projection_id, tag, tenant)` adoption constraint, and none of them
      presents a multi-replica projection configuration as officially supported. The code does not
      enforce this; the documentation makes it legible to an operator, and detection/enforcement
      is explicitly deferred to F-1.
