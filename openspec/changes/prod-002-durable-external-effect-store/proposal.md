# Proposal: PROD-002 — Durable External Effect Store

## Intent

CORE-019 (archived `2026-07-15-core-019-reliable-external-effects`) shipped the
reliable-effects subsystem with **two public ports** — `EffectStateStore` and
`EffectDedupStore` (`crates/runtime/src/effects/store.rs:202-249`, `:358-385`) —
and exactly **one** implementation, `InMemoryEffectStore` (`store.rs:412`),
which is labelled reference/convenience-only and loses every `Pending`/
`InFlight` effect on crash (canonical `openspec/specs/external-effects/spec.md`,
scenario "In-memory store loses undelivered effects on crash", `:127-131`). The
ports were deliberately shaped to enable a durable backend — `Timestamp` is
chrono-backed precisely so `next_at` can be serialized (`store.rs:44-70`),
`EffectStoreError` carries an explicit transient/permanent split for a durable
backend (`store.rs:106-135`), and the non-goals already state "Durable delivery
store implementation (Postgres outbox) — the ports are shaped to enable one, but
none ships" (`spec.md:307-320`). This change delivers that durable backend and
the port extensions a durable, potentially multi-consumer store requires — while
keeping `InMemoryEffectStore` as the reference/test implementation.

## Scope

### In Scope

- A production-grade, crash-durable backend satisfying `EffectStateStore` and
  `EffectDedupStore` so `Pending`/`InFlight` effects survive process crash and
  restart.
- The port extension(s) a durable, multi-consumer-capable store requires that
  the current ports do not express: **atomic claim** (claim-and-transition in
  one step), **lease / visibility-timeout**, **lease renewal**, and
  **lease-expiry reclaim**. `claim_due` today is deliberately non-atomic and
  single-consumer (AD-8,
  `openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/design.md:68`).
- Durable dedup-reservation persistence with the same `(tenant, effect_type,
  key)` scoping and ownership/status semantics the in-memory store already
  implements (`store.rs:358-385`).
- Transactional state transitions with a documented transaction boundary, and
  SQLSTATE-class → `EffectStoreError` transient/permanent mapping.
- Crash-recovery semantics on a durable store (in-flight → reclaimable) that
  preserve the record rather than lose it.
- Retaining `InMemoryEffectStore` unchanged as the reference/test
  implementation; both implementations satisfy each port independently.

### Out of Scope (Non-Goals / Follow-ups)

- Selecting or shipping official adapters for HTTP/Kafka/NATS/SMTP/S3 executors
  (unchanged CORE-019 non-goal).
- CDC/Debezium, distributed scheduling, cross-node cluster coordination beyond
  the leasing needed for multi-consumer claim safety, or sharding.
- Physical exactly-once delivery. **`exactly once` MUST NOT appear** in this
  change's contract or docs (preserves `spec.md:111-131`).
- A workflow/saga/temporal-style durable execution engine.
- Changing the executor registry, acceptor seam, backoff policy, or the
  admission-queue mechanism (all owned by CORE-019, unchanged here).
- Wiring the durable store as the default. It is opt-in; the in-memory store
  remains the default reference implementation.

## Frozen Decisions (decided constraints, not open questions)

1. **At-least-once is preserved; exactly-once is never claimed.** The durable
   store MUST preserve the CORE-019 at-least-once guarantee across crash/restart
   and MUST NOT introduce, imply, or document exactly-once delivery. Idempotency
   remains the handler/executor's responsibility
   (`crates/runtime/src/effects/executor.rs:33,49-53`).
2. **Ordering is NOT guaranteed.** The durable store MUST NOT promise FIFO or
   any cross-effect ordering. Concurrent consumers, retries with backoff, and
   `next_at` scheduling all reorder delivery; the contract stays orderless.
3. **In-memory store is retained as reference.** `InMemoryEffectStore` stays,
   unchanged in behavior, as the reference/test implementation. The durable
   store is a second, independent implementation of the same ports — not a
   replacement of the ports' contract.
4. **Dedup scope is unchanged.** Durable dedup identity stays scoped
   `(tenant, effect_type, key)` with the existing ownership/status outcomes
   (`Fresh`/`OwnedInProgress`/`OwnedSucceeded`/`OtherInProgress`/
   `OtherSucceeded`/`Conflict`); no cross-tenant collision is ever possible
   (`spec.md:269-290`).
5. **Hexagonal layering.** The durable adapter lives in `infrastructure` (the
   sole `sqlx`/DB-driver consumer). `crates/domain` and the effect **ports**
   stay vendor-neutral; no DB type leaks into a port signature.
6. **Bounded metric cardinality.** Durable-store metrics MUST use only
   low-cardinality labels (e.g. `effect_type`, outcome, consumer role). Raw
   `effect_id`, idempotency key, tenant id, destination, or payload MUST NOT
   appear as a metric label.

## Open Fork for DESIGN (do not resolve here)

**The concrete storage technology is NOT selected in this proposal.** The design
MUST weigh a Postgres-outbox (leveraging the `sqlx` 0.8 postgres dependency that
already exists, unused by effects, in `crates/infrastructure/Cargo.toml:14` and
`crates/persistence/Cargo.toml:8`) against alternatives (Kafka/log-based,
Redis, embedded SQLite/sled) and record a **Verdict** in an ADR. Two further
shape decisions are also deferred to design: (a) whether atomic-claim/lease is
added by **extending** `EffectStateStore` or by a **new** supplementary port;
(b) whether the durable store supports single-consumer only or multi-consumer,
and exactly how leasing enables the multi-consumer case.

## Capabilities

### New Capabilities

- None. This change adds a durable backend and port extensions to an existing
  capability; no new capability folder is introduced.

### Modified Capabilities

- `external-effects`: adds durable-persistence, atomic-claim/lease, lease
  renewal/expiry, transactional-boundary, and durable-dedup requirements; the
  durable delivery store the CORE-019 non-goals said "none ships" now ships as a
  second implementation. `InMemoryEffectStore` retained as reference.

## Approach

Implement a crash-durable backend in `infrastructure` that satisfies
`EffectStateStore` and `EffectDedupStore`, backed by the storage technology the
design ADR selects. Effect state (`Pending`/`InFlight`/`Succeeded`/
`RetryableFailed`/`TerminalFailed`, `attempt`, `next_at`, `tenant`,
`description`) and dedup reservations are persisted so a restart reconstructs the
in-flight world. A durable/multi-consumer claim is made **atomic** (claim and
transition to in-flight in one step) and protected by a **lease**
(visibility-timeout): a claimed effect is invisible to other consumers until the
lease is renewed or expires, and an expired lease is **reclaimed** (in-flight →
reclaimable) so a dead consumer's effect is redelivered, never lost or
double-delivered concurrently. State transitions run inside a transaction with a
documented boundary; store errors map to the existing transient/permanent
`EffectStoreError` split. The in-memory store stays as the reference
implementation; the durable store is opt-in.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/runtime/src/effects/store.rs` | Modified (future) | Port extension for atomic-claim/lease/renew/expiry (extend vs new port decided in design) |
| `crates/infrastructure/src/effects/` (new) | New (future) | Durable adapter implementing both ports; sole DB-driver consumer |
| `crates/infrastructure/migrations/` (new) | New (future) | Schema/migration for effect state + dedup tables |
| `crates/infrastructure/Cargo.toml` | Modified (future) | Depend on `ego-runtime` for the ports; `sqlx` already present |
| `crates/runtime/src/effects/mod.rs` | Modified (future) | Re-export any new lease port |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Durable claim allows two consumers to deliver the same effect concurrently | High if unaddressed | Atomic claim + lease/visibility-timeout is a normative requirement; a claimed effect is invisible to other consumers until renew/expiry |
| A dead consumer's leased effect is lost (never reclaimed) | Med | Lease-expiry reclaim requirement returns an expired-lease in-flight effect to claimable, preserving the record |
| Exactly-once creeps into the contract via "durable" framing | Med | Frozen decision + preserved requirement: at-least-once only; `exactly once` MUST NOT appear |
| DB technology leaks into a port signature (breaks hexagonal) | Med | Adapter confined to `infrastructure`; ports stay vendor-neutral; boundary check in tasks |
| Metric cardinality explosion from raw ids/keys as labels | Med | Frozen bounded-cardinality decision + negative scenario in the spec |
| Port extension breaks the in-memory single-consumer contract | Med | Design ADR-2 weighs extend-vs-new-port; in-memory `claim_due` semantics preserved |

## Rollback Plan

The durable store is opt-in and additive; nothing is wired to it by default.
Rollback = drop the durable adapter, its migration, and any new lease port, and
fall back to `InMemoryEffectStore` (the unchanged default). If the port was
**extended** rather than added as a new trait, rollback also reverts the added
method(s); default in-memory behavior is preserved by keeping the existing
`claim_due` path intact. No committed application data model changes outside the
effect-store's own tables; dropping those tables is the only schema revert.

## Dependencies

- Builds on the archived CORE-019 reliable-effects lineage
  (`openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/`):
  the ports, `EffectStoreError` transient/permanent split, chrono-backed
  `Timestamp`, and the AD-8 non-atomic-`claim_due` single-consumer invariant
  this change extends. There is **no dedicated open issue**; CORE-019 is the
  related, archived lineage.
- `sqlx` 0.8 (postgres, chrono, json, migrate) already in
  `crates/infrastructure/Cargo.toml:14` and `crates/persistence/Cargo.toml:8`,
  currently unused by the effects subsystem.
- No hard dependency on any other active change; independent.

## Success Criteria

- [ ] A durable backend satisfies both `EffectStateStore` and `EffectDedupStore`
      such that `Pending`/`InFlight` effects survive a process crash and restart.
- [ ] Atomic claim + lease/visibility-timeout provably prevent two concurrent
      consumers from delivering the same claimed effect.
- [ ] Lease renewal extends the claim; lease expiry reclaims the effect
      (in-flight → reclaimable) without losing the record.
- [ ] State transitions are transactional with a documented boundary; store
      errors map to the existing transient/permanent `EffectStoreError` split.
- [ ] Durable dedup persists `(tenant, effect_type, key)` scope and ownership
      status; cross-tenant collision remains impossible.
- [ ] At-least-once is preserved and `exactly once` appears nowhere; ordering is
      documented as not guaranteed.
- [ ] `InMemoryEffectStore` is retained unchanged as the reference
      implementation; `cargo test --workspace` green.
