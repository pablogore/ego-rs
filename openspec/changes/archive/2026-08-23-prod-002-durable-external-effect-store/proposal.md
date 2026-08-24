# Proposal: PROD-002 — Durable External Effect Store

## Metadata

| Field | Value |
|-------|-------|
| Change ID | PROD-002 |
| Title | Durable External Effect Store |
| Type | Production hardening (write-side effect delivery durability) |
| Date | 2026-07-19 |
| Parent | CORE-019 Reliable External Effects (shipped, archived at `openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/`; living spec `openspec/specs/external-effects/spec.md`) |
| Related | CORE-030 Transactional Outbox (`ROADMAP.md` §5.1, PLANNED — distinct capability, see §Boundary) |
| Roadmap | `ROADMAP.md` §7.2, Priority P0 |
| Status | PROPOSING |

## Intent

CORE-019 shipped the complete at-least-once delivery pipeline (accept → queue
→ delivery runner → executor) around two public ports — `EffectStateStore`
and `EffectDedupStore` — but the only implementation is `InMemoryEffectStore`,
explicitly labeled convenience-only. The pipeline's honesty clause states the
consequence verbatim: with the in-memory store, the guarantee **degrades to
at-most-once across a crash**. Every `Pending`/`InFlight` effect dies with the
process.

The problem is the ambiguity window along the chain

```
business decision → event commit → effect registration → effect execution → execution confirmation
```

A crash, restart, or partial failure at any hop after commit leaves the system
unable to answer "was this effect delivered, still owed, or lost?" — today the
answer after a crash is always "lost, silently." Production (per `ROADMAP.md`
§7.2) requires a durable answer: an accepted effect, once recorded, MUST
survive process death and remain owed until it reaches `succeeded` or
`terminal-failed`; an effect that was in-flight at the crash MUST be treated
as not-yet-confirmed and become eligible for redispatch, never assumed
delivered. "Durable" is scoped to acceptance onward — the post-commit
dual-write gap (crash between commit and `accept`) is narrowed by a durable
store, not closed; CORE-019 already documents this and PROD-002 keeps that
honesty verbatim.

## Current Gap

1. **Ports exist, durable implementation does not.** `EffectStateStore`
   (`accept`/`mark_in_flight`/`mark_succeeded`/`mark_retryable`/`mark_terminal`/
   `claim_due`/`recover_in_flight`) and `EffectDedupStore`
   (`reserve`/`commit_success`/`release`, `DedupOutcome`) in
   `crates/runtime/src/effects/store.rs` are built entirely from public types
   (`AcceptedEffect`, `EffectId`, `StoredEffect`, `TerminalReason`,
   `EffectStoreError`) — implementable from any crate. CORE-019's own
   Non-Goals say: *"Durable delivery store implementation (Postgres outbox) —
   the ports are shaped to enable one, but none ships in this capability."*
   PROD-002 is that shipment.
2. **Recovery affordances are contractual but untested against durability.**
   The spec requirement "Delivery State Is Reconstructable After a Restart"
   and the `DeliveryRunner` reclaim loop (`claim_due`/`recover_in_flight`)
   already assume a durable store; no implementation exercises them across a
   real process boundary.
3. **`claim_due` has no ownership marking.** Nothing in CORE-019 addresses
   two runner instances (or a restarted process racing its predecessor's
   in-flight work) claiming the same effect — the ROADMAP checklist names
   claim ownership and lease/fencing semantics explicitly.
4. **No TestKit double** exists for `EffectStateStore`/`EffectDedupStore`
   fault injection (crash simulation, transient store errors, claim races).
5. **No cleanup/retention precedent** — the in-memory store just grows.

## Scope

### In Scope

Tracks the `ROADMAP.md` §7.2 checklist:

- Durable implementation(s) of **both existing ports** — `EffectStateStore`
  and `EffectDedupStore` — shipped against **two first-class providers from
  day zero**, each satisfying both ports independently (as the spec already
  requires of a durable implementation):
  - **PostgreSQL** — external server, reuses the workspace's existing
    sqlx/Postgres conventions (`crates/persistence/src/postgres/*`).
  - **[Stoolap](https://stoolap.io/)** — an embedded, Rust-native SQL
    database (MVCC, snapshot isolation, in-process, zero external
    dependencies). This is the provider the reference-app dogfoods against in
    full, since it needs no running server.
  - Both conform to the same two ports; a third-party provider is always
    possible against that same contract — this is not a closed list.
- **Stable effect identity**: the runtime-minted `EffectId` (CORE-019,
  unchanged) becomes the durable primary identity across restarts.
- **State model reuse**: pending → in-flight → succeeded | retryable-failed |
  terminal-failed, exactly as defined by CORE-019 — persisted, not
  redesigned. Atomic state transitions in the durable store.
- **Claim ownership and lease/fencing semantics** for concurrent workers and
  crash-restart races. **Decided direction**: durability is the mandatory
  provider contract (every provider MUST make an accepted effect survive
  process death and expose atomic state transitions). Multi-node-safety is a
  separate, *not universally mandated* capability — a provider declares
  whether it offers it, rather than every provider being forced to solve
  cross-node coordination:
  - **PostgreSQL** provides multi-node coordination natively (transactional
    locking / leases / fencing) — `Durable + ConcurrentLocalSafe +
    MultiNodeSafe`.
  - **Stoolap** is a durable *local* state machine — embedded, single-process
    ownership. It does not solve multi-node coordination, and PROD-002 does
    not ask it to — `Durable + ConcurrentLocalSafe (local)`, but
    `MultiNodeSafe: NO` for this change. A distributed deployment that needs
    multiple nodes each running Ego+Stoolap composes an *external*
    coordination layer (e.g. a host application's own consensus/leader-
    election, such as OpenRaft) that decides ownership/fencing across nodes,
    while each node still persists locally to its own Stoolap. That
    coordination layer is explicitly out of scope for PROD-002 (see
    Non-Goals) — Ego is not where clustering lives.
  - Ego does **not** implement Raft, leader election, service discovery, or
    cluster membership. The exact shape of how a provider declares its
    capabilities (e.g. an `EffectStoreCapabilities` descriptor with
    `durable`/`concurrent_local_safe`/`multi_node_safe`/`supports_leases`
    flags, or an equivalent mechanism) and the lease/fencing token shape for
    the providers that do support it are design.md decisions — see Open
    Questions.
- **Retry persistence**: attempt counts, next-due times, and backoff
  bookkeeping survive restarts; the retry policy semantics stay CORE-019's.
- **Idempotency keys**: scoped `(tenant, effect_type, key)` dedup identity
  persisted durably; `DedupOutcome` semantics unchanged.
- **Crash recovery / recovery of abandoned effects**: `claim_due` and
  `recover_in_flight` honored across a real process boundary; stale claims
  recoverable.
- **Cleanup/retention** of succeeded/terminal rows (policy shape is a
  design.md decision).
- **Delivery semantics, stated honestly**: durable at-least-once from
  acceptance onward; composes to logical once-only with a cooperating
  destination; never exactly-once; dual-write gap narrowed, not closed.
- **Integration with the existing runtime**: the `DeliveryRunner`, lifecycle
  (startup/readiness), and graceful shutdown (drain deadline, `Cancelled` →
  pending, `drain_incomplete`) work unchanged against the durable store;
  observability extends the existing `log_*` signal surface in
  `crates/runtime/src/effects/observability.rs`, not a parallel one.
- **Developer-facing API**: registering the durable store where
  `InMemoryEffectStore` is registered today; what an external provider must
  implement is exactly the two existing ports — documented as the provider
  contract.
- **`InMemoryEffectStore` stays**, explicitly labeled non-durable/dev-only
  (already shipped and labeled; docs sharpened where needed).
- **TestKit support**: a fault-injection store double (real trait impl, per
  the `RecordingExecutor` convention in `crates/testkit/src/effects.rs`) for
  retry, recovery, and idempotency scenarios.
- Delta spec work: retire CORE-019's "no durable store ships" non-goal in
  `openspec/specs/external-effects/spec.md` and reconcile the "cross-node
  leasing" non-goal with whatever lease scope design.md settles (flagged here
  so spec/design don't silently ignore either).

### Out of Scope / Non-Goals

- NOT a workflow engine or saga/temporal-style durable execution (CORE-029).
- NOT Saga orchestration or compensation.
- NOT a general scheduler or distributed job system.
- NOT a distributed transaction manager; no transactional enlistment of
  effect acceptance inside the event commit (that remains a separate future
  redesign, per CORE-019 §7).
- NOT a mandatory concrete Kafka/NATS/Redis implementation; PostgreSQL and
  Stoolap are the two first-class durable backends this change ships, not a
  required transport for every deployment. No official broker adapters.
- NOT a universal exactly-once guarantee — the phrase stays banned from the
  public contract.
- NOT clustering, Raft, leader election, service discovery, or cluster
  membership implemented inside Ego itself, and NOT a requirement that every
  provider be multi-node-safe. Multi-node safety is a provider *capability*
  (PostgreSQL has it natively; Stoolap does not by design). Composing an
  embedded Stoolap-per-node deployment with an external multi-node
  coordination layer (e.g. an OpenRaft-based consensus/claims layer in a
  host application such as Bridge) is a valid deployment shape but is that
  host application's architecture to build — not something PROD-002
  designs, ships, or depends on (see Scope).
- NOT CORE-030 Transactional Outbox (see §Boundary).
- No changes to `ExternalEffectDescription`, the executor contract, the
  pipeline shape, or the state model.

## Capabilities

### New Capabilities

- None. PROD-002 introduces no new capability and no new mandatory public
  trait — it implements the two existing ports. It adds two durable provider
  implementations (`PostgresEffectStore` and `StoolapEffectStore`), the
  TestKit `FaultInjectingEffectStore`, and the additive
  `EffectStoreCapabilities` descriptor exposed through a defaulted
  `capabilities()` method on both existing ports (design.md AD-3) — new
  public surfaces, but not a new capability or a new required trait.

### Modified Capabilities

- `external-effects` (extended, not replaced): requirements added for durable
  implementation behavior — durability of accepted effects across restarts,
  claim ownership under concurrency, stale-claim recovery, retry persistence,
  cleanup/retention; the "durable delivery store — none ships" non-goal is
  retired; the cross-node-leasing non-goal is reconciled with the decided
  lease scope.

## Approach (direction, not design)

Implement the two shipped ports against two providers:

- **PostgreSQL**, reusing the workspace's existing sqlx/Postgres conventions
  (`crates/persistence/src/postgres/*`: numbered migrations, `PgPool`,
  DB-error-code → typed `EffectStoreError` mapping).
- **Stoolap** (embedded, in-process, zero external dependencies), which is
  what `examples/reference-app` dogfoods end-to-end since it requires no
  running server.

The existing `DeliveryRunner` reclaim loop drives recovery for both — the
durable store plugs into `claim_due`/`recover_in_flight` rather than growing
a second recovery mechanism.

Crate-placement options were surfaced in exploration, **deliberately not
decided here** — and now also cover how a second provider is organized
(one crate with per-backend feature flags vs. one crate per provider):

1. New crate(s) depending on `ego-runtime` + the backend driver (sqlx for
   Postgres, Stoolap's Rust API for the embedded provider) — keeps today's
   verified dependency graph intact; exploration leans this way.
2. Extend `ego-persistence` with a new `ego-persistence → ego-runtime` edge
   (reuses sqlx/migration plumbing for the Postgres side; inverts a
   documented boundary; still leaves the Stoolap provider's placement open).

This AD, the lease/fencing model for providers that support it, and the
exact shape of the capability-declaration mechanism (`EffectStoreCapabilities`
or equivalent — see Scope) are the first decisions design.md must settle.
Stoolap is not required to satisfy multi-node-safety; it ships as a durable,
single-node provider by design, and that is treated as resolved at proposal
level, not left as an open tension.

## Boundary with CORE-030 (Transactional Outbox)

Textually adjacent, architecturally distinct — do not conflate:

| | PROD-002 | CORE-030 |
|---|---|---|
| Payload | Runtime-accepted `ExternalEffectDescription` delivery state | Application-owned integration events |
| Write point | Post-commit acceptance (inherits narrowed dual-write gap) | Inside the application transaction (closes the dual-write window) |
| Consumer | `DeliveryRunner` → `ExternalEffectExecutor` | Outbox publisher → Messaging SPI (Kafka/NATS) |

Shared vocabulary (claiming, leases, crash recovery, retry, cleanup) is
mirrored deliberately for consistency; no shared implementation is proposed
here.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| New crate `ego-effect-store` (`crates/effect-store/`) | New | Durable PostgreSQL + Stoolap `EffectStateStore` + `EffectDedupStore` implementations, behind `postgres`/`stoolap` Cargo features (design.md AD-1: one new crate depending on `ego-runtime`, no new edge into `ego-persistence`) |
| `crates/runtime/src/effects/store.rs` | Referenced | Ports implemented, not redesigned; possible additive extension is a design AD |
| `crates/runtime/src/effects/runner.rs` | Referenced/Modified | Reclaim loop drives durable recovery; changes only if the lease model requires them |
| `crates/runtime/src/effects/observability.rs` | Modified | Extend existing `log_*` signals for claim/recovery/cleanup events |
| `crates/testkit/src/effects.rs` | Modified | Fault-injection store double (real trait impl convention) |
| SQL migrations (`crates/effect-store/src/postgres/migrations/`) | New | Effect state + dedup tables; own numbered sequence starting at `001` (design.md AD-10), no collision with `ego-persistence`'s 001-006 |
| `openspec/specs/external-effects/spec.md` | Modified | Delta: durable-store requirements; retire/reconcile non-goals |
| `examples/reference-app` | Modified | Full dogfood of the durable store against the embedded **Stoolap** provider — no external server dependency |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Crate placement blocks implementation | High | First design.md AD; both options pre-analyzed in exploration |
| Stale-claim recovery conflated with single-process restart sweep | Med | Named as a distinct open question; distributed lease expiry ≠ `recover_in_flight` sweep |
| "Durable" read as closing the dual-write gap | Med | Honesty clauses carried verbatim from CORE-019 into intent, scope, and delta spec |
| Unbounded table growth without cleanup | Med | Cleanup/retention is an in-scope checklist item with its own design AD |
| Migration-sequence collision if placed in `ego-persistence` (001–006) | Low | Named in open questions; resolved with the placement AD |
| Consumers wrongly assume every provider is multi-node-safe | Med | Capability is queryable/declared per provider (`EffectStoreCapabilities` or equivalent, design.md); Stoolap explicitly declares `MultiNodeSafe: NO` — a misuse the type/API should make hard, not just document |
| Scope creep: PROD-002 pulled into designing Bridge/OpenRaft-style cross-node coordination | Med | Explicit non-goal; that composition (external consensus layer + per-node local Stoolap durability) is a host application's architecture, not this change's |
| Two providers double the conformance-testing surface | Low | Both implement the same two ports; a shared conformance test suite (design.md decision) amortizes this |

## Open Questions for Design

Deferred to design.md — listed here so none is silently resolved:

1. **Crate placement**: new crate vs. extending `ego-persistence` (dependency
   direction, `ARCHITECTURE.md` impact)?
2. **Lease/fencing model for MultiNodeSafe providers**: how does PostgreSQL
   mark claim ownership — lease tokens, fencing counters, transactional
   locking (`SELECT … FOR UPDATE SKIP LOCKED`), or something simpler — and
   what does a fenced-out worker observe? (Stoolap does not need this model
   for PROD-002's scope — see decided direction in Scope.)
3. **Capability-declaration mechanism shape**: how does a provider declare
   `Durable`/`ConcurrentLocalSafe`/`MultiNodeSafe`/`SupportsLeases` —
   an `EffectStoreCapabilities` struct/trait method, associated constants, or
   something else — and where/when is it checked (registration time,
   type-level, or both)? What does registering a `MultiNodeSafe: NO`
   provider in a topology that needs it do — reject at registration, or is
   that the host application's responsibility to avoid?
4. **Stale/abandoned claim recovery**: `recover_in_flight` today assumes a
   single-process restart sweep; is distributed lease expiry in or out, and
   who runs the recovery sweep?
5. **Atomic state transitions**: what Postgres mechanism guarantees
   transition atomicity under concurrent claimers (conditional `UPDATE`,
   `SELECT … FOR UPDATE SKIP LOCKED`, …)?
6. **Port sufficiency**: do `claim_due`/`recover_in_flight` suffice for the
   chosen lease model, or is an additive port extension needed — and how does
   `InMemoryEffectStore` remain conformant if so?
7. **Retry persistence shape**: which retry bookkeeping is durable (attempt
   count, next-due, per-type overrides), and does policy configuration live
   in the store or stay runtime-side?
8. **Dedup durability**: how do `reserve`/`commit_success`/`release` behave
   across a crash mid-reservation, and what is the dedup retention window?
9. **Cleanup/retention policy**: TTL vs. count vs. operator-triggered for
   succeeded/terminal rows, and who executes it?
10. **Migration versioning**: new sequence in a new crate vs. reconciling
    with `ego-persistence`'s existing 001–006 sequence?
11. **Graceful shutdown with held claims**: at drain deadline, are leases
    released explicitly or left to expire, and what does the successor
    observe?
12. **TestKit double shape**: what fault-injection API (crash points,
    transient `EffectStoreError`s, claim races) serves retry/recovery/
    idempotency tests while staying a real trait impl?
13. **Conformance suite across providers**: should PostgreSQL and Stoolap
    implementations be required to pass one shared port-conformance test
    suite (proving both satisfy `EffectStateStore`/`EffectDedupStore`
    identically), and where does that suite live?

## Rollback Plan

Purely additive: the durable store is opt-in at registration, exactly like
any `EffectStateStore`/`EffectDedupStore` implementation. Rollback = stop
registering it (fall back to `InMemoryEffectStore`), drop the new tables via
a down-migration, and revert the crate/module. No changes to ports, pipeline,
domain types, or existing behavior to unwind; delta-spec retirement of the
non-goal reverts with the change folder.

## Dependencies

- CORE-019 (shipped, archived) — ports, state model, runner, observability.
- Existing workspace sqlx 0.8 / Postgres conventions in `crates/persistence`.
- **New external dependency**: [Stoolap](https://stoolap.io/) (embedded
  Rust-native SQL database) — not previously in the workspace; brought in
  specifically for the embedded durable provider and reference-app
  dogfooding.

## Success Criteria

- [ ] Kill-the-process test: an accepted effect survives a real process
      restart and is delivered exactly as the spec's reconstructability
      requirement demands — the inverse of CORE-019's documented in-memory
      loss boundary.
- [ ] Mid-delivery (in-flight) effect at crash time is redispatched after
      restart, never silently treated as delivered.
- [ ] Two concurrent claimers never hold overlapping *valid* claims on the
      same effect: proven cross-process against PostgreSQL (`MultiNodeSafe`,
      under the decided lease model) and locally against Stoolap
      (`ConcurrentLocalSafe` only — not a multi-node claim). After a lease
      expires, redispatch — and therefore possible duplicate external
      execution — is expected and is covered by the at-least-once +
      idempotency contract, not prevented by claim exclusivity.
- [ ] Retry bookkeeping (attempt count, next-due) survives restart; backoff
      resumes, not resets.
- [ ] Scoped dedup identity holds across restart: a replayed scoped key is
      deduplicated, a reused key with different payload/destination is still
      rejected as invalid.
- [ ] Both ports satisfied independently by the durable implementation (no
      composite requirement), per the existing spec scenario.
- [ ] Both the PostgreSQL and Stoolap providers pass the same set of
      durability/recovery/idempotency criteria above — proving the port
      contract, not a single backend's incidental behavior.
- [ ] Graceful shutdown against the durable store drains or emits
      `drain_incomplete`; nothing is lost either way.
- [ ] TestKit double exists and is used by the above tests where a real
      Postgres is not.
- [ ] `openspec/specs/external-effects/spec.md` no longer lists the durable
      store as a non-goal; "exactly once" still appears nowhere in the public
      contract.
- [ ] `cargo test --workspace` green.
