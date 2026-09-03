# Proposal: PROD-014C — Atomic Read-Side Event Claiming

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers).

## Intent

PROD-014B shipped durable read-side progress, but only under an **unenforced** adoption
constraint: single-writer-per-`(projection_id, tag, tenant)`. Nothing detects or refuses a
second replica. `ReadSideSession::execute()` (`crates/domain/src/read_side/session.rs:91-176`)
runs `handler.handle()` between `dedup_store.seen()` and `dedup_store.mark_seen()` with no
shared transaction, lock, or lease — two replicas can both observe `seen() == false` and both
invoke the handler. `ego-rs` targets distributed production, so its own documented target
deployment is today outside its own guarantee.

PROD-014C obtains **exclusion before the handler runs**: for one claim identity, at most one
worker holds a valid processing claim at a time, across processes, with crash recovery and
stale-owner rejection. This is *single valid processing ownership*, never exactly-once.

## Active Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | **Claim identity is exactly `(projection_id, tag, tenant)`** — not per-event, not per-projection, not global (IS-1) | This triple is already the unit that owns one monotonically advancing offset: `OffsetStore`'s own doc states "Offsets are independent per (projection_id, tag, tenant) tuple" (`crates/persistence-api/src/read_side/offset.rs:53`) and `013_create_projection_offsets.sql:24-25` declares it as `PRIMARY KEY (projection_id, tag, tenant)`. Claiming at exactly this granularity preserves the existing ordering promise trivially — `ReadSideStore::fetch` returns "events with `event_version > offset` in ascending version order" per `(tenant, tag)` (`crates/persistence-api/src/read_side/store.rs:30`), and only the claim holder ever calls `fetch`/`handle` for that stream. Per-event claiming would serialize nothing that per-stream ordering does not already serialize, and would multiply round trips per batch (R-3). A coarser claim (global, or per-`projection_id`) would serialize unrelated tenants and tags against each other. Dedup's narrower `(projection_id, tag, event_id)` identity (migration `014`, no tenant column) is deliberately **not** the claim identity: dedup tracks event presence, claiming owns tenant-scoped stream progress |
| D-2 | **A new port, not an added method on `OffsetStore` or `DedupStore`** — as a matter of principle. The exact method set and the acquire mechanism stay open for `sdd-design` (see Approach) | `OffsetStore` today is three methods — `is_durable`, `read_offset`, `write_offset` (`crates/persistence-api/src/read_side/offset.rs:55-84`) — and its `write_offset` is contractually a plain overwrite with "no compare-and-swap, no expected-previous-offset check" (`crates/persistence/src/postgres/read_side_offset.rs:5-14`, PROD-014B D-3/L-5). Exclusion is a different property from position bookkeeping, and dedup bookkeeping is a third. Both traits are shipped and archived (PROD-014A/PROD-014B), both carry blanket `Arc<T>` forwarding impls (`offset.rs:86+`), both have in-memory implementations, and both are read live by the Production gate (`crates/service-sdk/src/runtime/builder.rs:879-891`) — a new required method breaks every existing implementor, and a defaulted one ships a default no honest implementation can satisfy. Note this is **not** chosen to avoid a spec delta: extending a shipped contract is a MODIFIED-requirement delta either way (exploration §Recommendation). It is chosen so the delta lands on one new capability instead of reopening two archived ones |
| D-3 | **Ownership proof (fencing) is carried as a requirement of this change, not as optional hardening deferred to a follow-up** (IS-3, SC-3) | Expiry-based takeover (IS-2) without ownership proof is strictly worse than no takeover. `write_offset` is last-write-wins by contract (D-2's citation), so a worker whose lease expired mid-batch and then resumed would still be accepted by `write_offset` and `mark_seen` — converting a double-execution bug into an offset-regression bug where a stale owner rewinds a position a live owner already advanced. The workspace already solved the analogous write-side problem exactly this way: `crates/persistence/src/postgres/reservation.rs` mints a strictly-greater fencing token inside the same conditional takeover `UPDATE` that re-owns the row, and its shared `mutate_owned` helper verifies the full `(tenant, key, owner, fencing_token, lease_until > now)` tuple in one `WHERE` clause per statement — never a separate check-then-write window. Shipping takeover without fencing would ship a mechanism that *reads* as exclusion while a stale owner writes through it |
| D-4 | **Lease expiry is measured against an injected `Clock`, never the database's `now()`** (R-5) | `PostgresOperationReservationStore` holds `clock: Arc<dyn Clock>` injected at construction (`crates/persistence/src/postgres/reservation.rs:70-73, 85-87`) and compares expiry against it — PROD-012A AD-8, already proven under real contention. The same two reasons apply unchanged: expiry becomes deterministically testable, and takeover timing stops depending silently on each replica's own wall clock |
| D-5 | **`Profile::Production` fails closed when no durable claim mechanism is registered** (IS-6) | This is the only thing that discharges the Intent. PROD-014B's own binding limitation states the framework "is adoptable in Production only under single-writer-per-`(projection_id, tag, tenant)`" and that "nothing in this change detects or refuses" a second replica (L-3). A PROD-014C that shipped the mechanism but left it opt-in would leave the identical silent-misconfiguration hole one layer up. The gate's shape already exists and needs no invention: `require_durably_configured` (`crates/persistent-entity/src/profile.rs:51-63`) plus the read-side branch `validate_read_side_progress_profile` (`crates/service-sdk/src/runtime/builder.rs:879-891`), reached through `AppBuilder::read_side_progress`'s registration (`crates/service-sdk/src/app/mod.rs:633-651`). PROD-014A R-3 established the precedent that a refusal is strictly better than silent volatility. Whether the gate reuses `require_durably_configured` verbatim or needs a sibling predicate is design's call |
| D-6 | **Placement is `crates/persistence/src/postgres/`, continuing the flat migration sequence at `016+` — not `015`** | PROD-014B D-5 established both the placement and the flat-sequence rule (`crates/effect-store`'s independent `001/002` sequence was justified as a property of an already-separate crate, not a general rule). The sequence on `develop` now ends at `015_fix_aggregates_tenant_null_uniqueness.sql`, with `013_create_projection_offsets.sql` and `014_create_projection_dedup.sql` immediately before it, so the next free number is `016`. **This corrects an error in the pre-amendment draft, which said `015+`** — that number is taken and would have collided at apply time. This decision binds only if design selects a claim-row mechanism; if it selects a PG advisory lock with no claim row (an option Approach leaves open), no migration ships and this decision is inert |
| D-7 | **Contention is proven only against real PostgreSQL, in `integration-tests/`** (IS-7, SC-7) | `ego-rs-testing-strategy` forbids any unit test from reaching a real database; `integration-tests` is a separate workspace and the only place real infrastructure is admitted (PROD-014B D-8). 22 `*_postgres.rs` suites already live under `integration-tests/tests/infrastructure/` and obtain their database from `isolated_database()`; four of them — `concurrent_replicas_postgres.rs`, `lease_contention_postgres.rs`, `fencing_window_postgres.rs`, `takeover_fencing_postgres.rs` — already race real contenders against `operation_reservations`, and `read_side_progress_postgres.rs` is PROD-014B's own read-side conformance suite. There is also a reason specific to this change: an atomicity claim is a claim about what the database does under real contention. A unit-level simulation asserts the behavior of a test double, so it can pass while the real mechanism is broken |
| D-8 | **The boundary is handler-execution count, never external-effect count — and the delivered change must never be worded as exactly-once** (OOS-2, R-1, SC-6) | `Handler<E>` (`crates/domain/src/read_side/handler.rs`) places zero restriction on what a handler does; its doc only says implementations "should be idempotent where possible", enforced nowhere. So a perfect claim bounds how many times the framework *invokes* the handler and can say nothing about an effect the handler performed — the identical caveat already documented for the write-side mechanism at `crates/persistence/src/postgres/reservation.rs:22-26`. Independently, crash recovery alone forces at-least-once with one replica and zero concurrency: dying during the handler, or after it succeeds but before `mark_seen`, re-invokes the handler on resume with no second worker involved. Exactly-once is therefore not merely out of scope here — it is unreachable at this layer. That makes naming discipline acceptance criteria, the same weight PROD-014B gave L-1 |
| D-9 | **No distributed consensus, leader election, or broker — and no backend but PostgreSQL** (OOS-1, OOS-6) | The problem is exclusion over one row identity, which a single PostgreSQL row already serializes; `reservation.rs` proves the full lease-plus-fencing shape with no consensus protocol, no leader election, and no broker. Introducing one would solve a materially harder problem than the Intent names, and would add operational surface the framework does not otherwise have. On backends: a full codebase audit under PROD-014B (OOS-4) found zero productive code for any non-PostgreSQL store, only illustrative prose in archived OpenSpec. A second backend is a second adapter's own change, gated on the port this one defines |
| D-10 | **Retry/backoff for `Transient` errors is excluded** (OOS-3) | Real, pre-existing, and independent. `crates/domain/src/read_side/error.rs:9` documents "Retry batch with exponential backoff (max 3 retries, 100ms base, 10s max)"; a grep for `backoff`/`retry` across `crates/domain/src/read_side/` and `crates/runtime/src/read_side/` returns zero hits outside that one comment — the scheduler's `on_error` callback only logs, and the loop waits for the next poll tick. The two concerns are orthogonal in both directions: retry decides *when* a batch is re-attempted, claiming decides *who* may attempt it, and either ships without the other |
| D-11 | **Cross-table atomicity between dedup and offset writes is excluded** (OOS-4) | It is already absent today, before any concurrency exists. `mark_seen` and `write_offset` live on two separate structs against two separate tables, each running its own auto-commit `.execute(&self.pool)`; `crates/domain` is generic over the two ports with no transaction handle threaded through, by hexagonal design. Claiming neither creates nor closes this: a crash mid-`mark_seen`-loop before `write_offset` re-invokes the handler on resume with a smaller batch, and that happens with one worker holding a valid claim the entire time. Closing it would require a shared transaction across two ports — redesigning the PROD-014B contracts this change consumes unchanged (D-2) |
| D-12 | **Intra-process cross-tag concurrency is excluded** (OOS-5) | There is none to protect today. `TagSchedulerImpl::start_projection` awaits each tag's session sequentially in a for-loop, and `Backpressure::acquire()` is awaited inline inside `execute_session` rather than gating spawned tasks — so the scheduler's "respecting concurrency limits" wording currently describes throttling, not parallelism. Claiming makes cross-*process* concurrency safe; exploiting cross-tag parallelism inside one process is a scheduler change with its own ordering and backpressure obligations. Excluded so this change's acceptance criteria stay about exclusion rather than about throughput |

## Atomicity Gate

**Run, and it cut scope four times.** Retry/backoff for `Transient` errors was considered and
removed (D-10): it is independently shippable in either order and answers a different question.
Cross-table dedup/offset atomicity was considered and removed (D-11): closing it means a shared
transaction across two archived ports, which would turn "obtain exclusion" into "redesign the
read-side persistence contracts". Intra-process cross-tag concurrency was considered and removed
(D-12): it is a scheduler throughput change, and folding it in would let a throughput regression
fail an exclusion proposal. A second, non-PostgreSQL backend was considered and removed (D-9):
it is a second adapter against the port this change defines.

What remains is one indivisible capability, because no in-scope item is independently shippable
with value:

- **IS-1** is the identity decision every other item is keyed on — a decision, not a deliverable.
- **IS-2** alone is a port nobody calls: dead code, and worse than absent because the trait's
  existence reads as a guarantee.
- **IS-5** alone is a table and an adapter that nothing claims through.
- **IS-4** alone has nothing to acquire — it cannot exist without IS-2 and IS-5.
- **IS-3** cannot be deferred without shipping a false guarantee. Takeover without ownership
  proof lets a stale owner write offsets through a mechanism that reads as exclusion (D-3), so
  the version of this change without IS-3 is not a smaller guarantee — it is a wrong one.
- **IS-6** is what makes IS-2..IS-5 non-optional. Without it the mechanism exists and a
  production composition can still silently run the exact multi-replica configuration PROD-014B's
  L-3 named as outside the guarantee, which leaves the Intent undischarged. Conversely IS-6
  without IS-2..IS-5 is PROD-014A's R-3 failure mode: a refusal with nothing in-tree to satisfy it.
- **IS-7** is the only way any of SC-1, SC-2 or SC-3 is observable at all (D-7).
- **IS-8** is not documentation garnish. The shipped adapters' own rustdoc, `ARCHITECTURE.md:211-219`,
  and `examples/reference-app/src/read_side/mod.rs:118-126` currently name PROD-014C as the gap
  closer and state the single-writer constraint as unenforced. Landing the enforcement without
  IS-8 leaves the delivered documentation asserting something false.

Every item names the same mechanism — one claim per `(projection_id, tag, tenant)`, acquired
before `fetch`, held across the batch through `write_offset`, proven by fencing — and the same
acceptance criterion.

R-4's stacked-PR forecast is not a counter-argument to this gate. Atomicity governs whether this
is one capability; slicing governs how many reviewable diffs deliver it. PROD-014A carried the
same pairing (its R-6 alongside its own PASS), and the slices there were delivery units of one
capability, not separate proposals.

**ATOMICITY: PASS**

## Scope

### In Scope

- **IS-1** — Claim identity `(projection_id, tag, tenant)` — the `projection_offsets` PK and
  `OffsetStore`'s own documented identity; claiming per stream preserves `ReadSideStore::fetch`'s
  per-`(tenant, tag)` ordering trivially.
- **IS-2** — An atomic claim port: acquire-or-refuse, lease renewal, release, and expiry-based
  takeover so a dead worker cannot block the stream forever.
- **IS-3** — Ownership proof so a worker that lost its claim cannot keep writing as owner. The
  `operation_reservations` precedent (`crates/persistence/src/postgres/reservation.rs`) already
  solves the analogous write-side problem with a monotonic fencing token + injected `Clock`;
  PROD-014C adopts the same shape unless design proves it unnecessary.
- **IS-4** — `ReadSideSession::execute()` acquires the claim before `fetch` and holds it through
  `write_offset`.
- **IS-5** — A durable PostgreSQL adapter + migration `016+` (the next free number; `015` is taken
  — D-6), mirroring `reservation.rs`'s conditional-`UPDATE`-with-CAS shape.
- **IS-6** — `Profile::Production` fails closed when no durable claim mechanism is registered.
  Post-change, multi-replica read-side becomes **SUPPORTED WITH EXPLICIT OPERATIONAL CONSTRAINT**
  (durable claim store registered; handler effects still at-least-once). The gating mechanism —
  and whether it mirrors PROD-014A's `require_durably_configured` idiom — is design's call.
- **IS-7** — Real-Postgres contention tests under `integration-tests/`, modelled on
  `concurrent_replicas_postgres.rs` / `takeover_fencing_postgres.rs`.
- **IS-8** — Spec deltas per Capabilities; adapter/README docs replace the single-writer
  constraint with the new, enforced one.

### Out of Scope

- **OOS-1** — Distributed consensus, global leader election, a distributed transaction
  coordinator, a Kafka consumer-group replacement, EventStore redesign.
- **OOS-2** — Exactly-once **external** side effects. `Handler<E>` permits arbitrary I/O, so
  claiming bounds handler-execution count only; the effect boundary must carry its own fence —
  the identical caveat already documented at `reservation.rs:22-26`.
- **OOS-3** — Retry/backoff for `Transient` errors (documented at
  `crates/domain/src/read_side/error.rs:9`, unimplemented). Adjacent, separate.
- **OOS-4** — Dedup/offset cross-table atomicity (two independent upserts today).
- **OOS-5** — Intra-process tag concurrency (`TagSchedulerImpl` is sequential today).
- **OOS-6** — Any backend but PostgreSQL; removing in-memory pairs.

## Capabilities

### New Capabilities

- `read-side-event-claiming`: the observable exclusion contract — claim identity, acquisition
  refusal under a live claim, expiry-based takeover, stale-owner rejection, and what it still
  does not promise.

### Modified Capabilities

- `read-side`: "Prevention of Double Handler Execution Rests on an Explicit, Unenforced
  Single-Writer Adoption Constraint" becomes enforced; "The Concurrency Gap Has a Named, Distinct
  Follow-Up" is discharged. "Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler
  Execution" stays true and MUST survive unchanged.

`read-side-durable-progress` needs no delta — its non-goals already assign claiming here.

## Approach

Add a **new port** rather than evolving `OffsetStore`/`DedupStore`, whose contracts are shipped
and archived and whose semantics (offset overwrite; dedup bookkeeping) are orthogonal to
exclusion. Shape it after `OperationReservationStore` — the only proven concurrent-claim
mechanism in this workspace — but keyed on `(projection_id, tag, tenant)` and lifecycle-shaped
for a continuous poll loop rather than a one-shot command.

**Open question for `sdd-design`**: new port vs. evolved `OffsetStore`; the exact method set
(an illustrative `try_claim`/`renew`/`complete`/`release` is a hint, not a mandate); and whether
a PG advisory lock or `FOR UPDATE SKIP LOCKED` beats a claim table for this poll-loop model.

## Required Semantics

```
Given two workers polling the same (projection_id, tag, tenant)
When both attempt to acquire the claim at the same time
Then exactly one MUST obtain it; the other MUST be refused, and the refused
     worker MUST NOT call fetch or invoke the handler for that stream on
     that tick.

Given a worker holding a valid claim on a stream
When it is still processing a long batch and its lease is approaching expiry
Then it MUST be able to extend the lease and continue, and no other worker
     may take the stream over while that lease remains valid.

Given a worker that acquired a claim and then stopped — crashed, was killed,
      or was paused indefinitely — without releasing it
When its lease expires
Then another worker MUST be able to take the stream over without operator
     intervention and without waiting indefinitely, so a dead worker cannot
     block a stream forever.

Given a worker whose claim was taken over by another worker after its lease
      expired
When that first worker resumes and attempts to write offset or dedup state
      as the owner
Then the write MUST be rejected as a stale owner and MUST leave the stored
     state unmodified — in particular it MUST NOT rewind an offset the new
     owner already advanced.

Given a worker holding a valid claim
When it finishes its batch and releases the claim normally
Then the stream MUST become immediately claimable again, without waiting for
     the lease to expire.

Given a composition declaring Profile::Production that registers read-side
      progress but no durable claim mechanism
When build() is called
Then it MUST be refused at composition/bootstrap time — never deferred to the
     first poll or the first batch — with an error naming the missing
     capability and the exact call that fixes it.

Given a composition declaring Profile::Production that registers a durable
      claim mechanism
When build() is called
Then it MUST succeed, and multi-replica read-side becomes supported under the
     stated operational constraint.

Given a stream whose claim is held by one worker
When that worker processes a batch
Then events MUST still be handled in ascending version order per
     (tenant, tag), exactly as before this change — claiming MUST NOT
     reorder, interleave, or skip events within a stream.

Given a single worker holding a valid claim for the whole batch
When it crashes after the handler succeeds but before the batch is fully
     recorded
Then the handler MAY run again for those events on resume. This change does
     NOT prevent that, and no delivered artifact may describe it as
     exactly-once processing or exactly-once external effects.
```

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistence-api/src/read_side/` | New | Claim port (IS-2, IS-3) |
| `crates/domain/src/read_side/session.rs` | Modified | Claim wraps the batch (IS-4) |
| `crates/persistence/src/postgres/` + `migrations/016+` | New | Durable adapter + table (IS-5, D-6) |
| `crates/service-sdk` composition gate | Modified | Production fail-closed (IS-6) |
| `crates/runtime/src/read_side/scheduler.rs` | Modified | Claim lifecycle across polls |
| `examples/reference-app/src/read_side/mod.rs:118-126` | Modified | Retire the PROD-014C promise |
| `integration-tests/tests/infrastructure/` | New | Contention suite (IS-7) |
| `openspec/specs/{read-side-event-claiming,read-side}/spec.md` | New / Modified | IS-8 |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| R-1 | "Atomic claiming" is read as "exactly-once" | High | OOS-2 is acceptance criteria (SC-6); grep-gate the delivered change, as PROD-014B's verify pass did |
| R-2 | A lease too short evicts a live slow worker mid-batch | Med | Renewal during long batches + fencing rejects the evicted writer's late writes (IS-3) |
| R-3 | Claim overhead per poll tick degrades throughput | Med | Claim once per stream and hold across the batch, not per event |
| R-4 | Touching `session.rs` + scheduler + gate + adapter exceeds the 400-line budget | High | `sdd-tasks` forecasts stacked slices: port + adapter, then session/scheduler wiring, then gate + docs |
| R-5 | Clock skew across replicas mis-times expiry | Med | Injected `Clock`, never DB `now()` — `reservation.rs`'s AD-8 precedent |

## Rollback Plan

Additive at the port and table level. Revert = drop the claim port and adapter, restore
`session.rs` to the unguarded sequence, revert the Production gate, drop migration `016+`, and
restore the single-writer adoption constraint in specs and docs. The claim table is referenced by
nothing else and may be dropped or left in place; discarding it degrades behavior back to
PROD-014B's, not to corruption.

## Dependencies

- PROD-014A / PROD-014B (archived) — durability gate and the durable progress pair, consumed.
- `crates/persistence/src/postgres/reservation.rs` — SQL-shape precedent only, not imported.
- `ego_integration_tests::isolated_database()`.
- No new external dependency, crate, or service.

## Success Criteria

- [ ] **SC-1** — Two concurrent workers on one `(projection_id, tag, tenant)`: exactly one holds a
      valid claim; the other is refused and does not invoke the handler.
- [ ] **SC-2** — A worker that dies holding a claim releases it by expiry; another worker takes
      over without operator action and without waiting indefinitely.
- [ ] **SC-3** — A worker whose claim was taken over cannot write offset or dedup state as owner.
- [ ] **SC-4** — `Profile::Production` refuses a read-side composition with no durable claim
      mechanism, and succeeds with one.
- [ ] **SC-5** — Per-`(tag, tenant)` ascending ordering and PROD-014B's durability guarantees are
      unchanged; `cargo test --workspace` shows zero new failures.
- [ ] **SC-6** — No delivered artifact describes this as exactly-once processing or exactly-once
      external effects; docs state multi-replica as supported under the stated operational
      constraint.
- [ ] **SC-7** — Contention is proven against real PostgreSQL with multiple concurrent contenders
      in `integration-tests/`, never a unit-test simulation.
