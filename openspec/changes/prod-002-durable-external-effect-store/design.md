# Design: PROD-002 — Durable External Effect Store

## Technical Approach

A crash-durable adapter in `crates/infrastructure` implements the two existing
CORE-019 ports — `EffectStateStore` (`crates/runtime/src/effects/store.rs:202-249`)
and `EffectDedupStore` (`store.rs:358-385`) — persisting effect lifecycle state
and dedup reservations so a restart reconstructs the in-flight world instead of
losing it (the `InMemoryEffectStore` gap, `store.rs:412-426`,
`openspec/specs/external-effects/spec.md:127-131`). Because the shipped
`claim_due` is deliberately non-atomic and single-consumer (AD-8,
`openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/design.md:68`),
a durable/multi-consumer backend adds an **atomic claim-with-lease** operation
plus **renew** and **expiry reclaim** that the current port does not express. The
adapter is the sole DB-driver consumer; the ports stay vendor-neutral, keeping
the hexagonal boundary (`crates/domain` and the runtime ports never see a DB
type). `Timestamp` is already chrono-backed so `next_at` and lease deadlines
serialize directly (`store.rs:44-70`); `EffectStoreError` already carries the
transient/permanent split a durable backend needs (`store.rs:106-135`). The
in-memory store is retained unchanged as the reference/test implementation.

## Architecture Decisions

### ADR-1 (OPEN FORK): Storage technology → **Verdict: Postgres-outbox via existing `sqlx`**

**Choice**: a Postgres-backed outbox — one `effect_state` table and one
`effect_dedup` table — using the `sqlx` 0.8 postgres/chrono/json/migrate
dependency that already exists but is unused by effects
(`crates/infrastructure/Cargo.toml:14`, `crates/persistence/Cargo.toml:8`).
**Rejected**: Kafka/log-based; Redis; embedded SQLite/sled.
**Rationale (evidence-grounded, decided here — NOT pre-decided in the proposal):**

| Option | Tradeoff | Verdict |
|---|---|---|
| **Postgres-outbox (`sqlx`)** | ACID transaction lets state transition + dedup commit share **one** boundary (ADR-4). `SELECT … FOR UPDATE SKIP LOCKED` gives atomic claim + row-level visibility with no bespoke CAS (ADR-2/ADR-3). chrono-backed `Timestamp` maps straight to `timestamptz` for `next_at`/lease deadline (`store.rs:44-70`). `EffectStoreError` transient/permanent split maps cleanly from SQLSTATE classes (ADR-4). `sqlx` is already a workspace dependency, and `infrastructure` is already the sole `sqlx` consumer — zero new hexagonal boundary crossing. | **Chosen** |
| Kafka / log-based | No single transactional boundary spanning dedup + state; dedup/idempotency needs a compacted-topic side-store, and consumer-group offsets couple ordering into the model that the spec deliberately leaves unordered. Heavy new infra dependency for a store the ports already shape as a two-table outbox. | Rejected |
| Redis | Persistence/durability is weaker (AOF/RDB windows) than an effect-durability store warrants; multi-key transactional dedup+state is awkward (Lua/`MULTI`); reclaim/visibility must be hand-rolled. No existing workspace dependency. | Rejected |
| Embedded SQLite / sled | Single-process/file-local: cannot serve the multi-consumer case ADR-3 enables, and offers no network-shared durability across restarts on different hosts. Fine as a second reference, not as the production backend. | Rejected |

The Verdict is Postgres-outbox. Concrete numeric tuning (pool size, claim
`limit`, lease duration defaults) stays implementation-level, not spec-normative,
exactly as CORE-019 kept its backoff numbers out of the spec (AD-5).

### ADR-2 (OPEN FORK): Atomic-claim/lease shape → **New supplementary port, not an extension of `EffectStateStore`**

**Choice**: add a **new** port trait (working name `LeasedEffectStore`) carrying
atomic-claim, renew, and expiry-reclaim; a durable backend implements
`EffectStateStore` + `EffectDedupStore` + `LeasedEffectStore`. `EffectStateStore`
and its `claim_due` are left **unchanged**.
**Rejected**: adding lease methods directly onto `EffectStateStore` (with or
without defaults).
**Rationale**: `claim_due` is contractually non-atomic and single-consumer by
AD-8 (`.../core-019.../design.md:68`) — a claimant still calls `mark_in_flight`
separately (`store.rs:234-243`). Adding `atomic_claim`/`renew_lease` onto that
same trait would either (a) force every `EffectStateStore` implementor —
including the reference `InMemoryEffectStore` and every test double — to grow
lease semantics it does not need, or (b) require default method bodies that
silently do the wrong thing for a durable multi-consumer store. A separate port
keeps `EffectStateStore`'s single-consumer contract and all its existing tests
intact (backward-compatible), while a durable backend opts into leasing by
implementing the extra trait. Composition over contract-mutation; matches
CORE-019's "one struct MAY implement multiple ports, each independently
satisfiable" rule (`spec.md:63-89`).

### ADR-3 (OPEN FORK): Consumer model → **Support both single- and multi-consumer; leasing enables multi**

**Choice**: the durable store supports a single consumer (the existing
`DeliveryRunner`, unchanged) AND, safely, multiple concurrent consumers; it does
not *mandate* multi-consumer deployment.
**Rejected**: single-consumer-only (wastes the durable store's cross-host
potential); mandatory-multi-consumer (forces coordination cost on deployments
that don't need it).
**Rationale + mechanism**: multi-consumer safety comes entirely from **leasing
as a visibility-timeout**. An atomic claim (`FOR UPDATE SKIP LOCKED` + transition
to `InFlight` + stamp `leased_until = now + lease`) makes the claimed row
**invisible** to any other consumer's claim query until the lease expires: two
consumers claiming concurrently get disjoint row sets, so the same effect is
never delivered by two consumers at once. A live consumer **renews** (extends
`leased_until`) while still working; a consumer that dies stops renewing, its
lease **expires**, and the effect becomes claimable again (ADR-5). Single
-consumer deployments simply run one claimant with a lease long enough to never
self-expire — the same code path, no special case. AD-8's "atomic claiming would
add compare-and-swap + lease timeouts with no current driver; deferred" is now
driven: this change is that driver.

### ADR-4: Transaction boundary + error mapping

Each state transition (`accept`, `mark_in_flight`, `mark_succeeded`,
`mark_retryable`, `mark_terminal`, atomic-claim) runs in **one** DB transaction;
a dedup `commit_success` that must be atomic with `mark_succeeded` shares that
single transaction so a partial write can never leave a dedup reservation
committed while its state row is not (or vice versa). The boundary is documented:
the transaction commits before the port method returns `Ok`; a rollback surfaces
as an `EffectStoreError`. SQLSTATE classes map to the existing split
(`store.rs:106-135`): serialization failure / deadlock / connection-pool timeout
/ lock timeout → `TemporarilyUnavailable` (retryable); `NotFound` for a missing
row; unique/dedup violation → `Conflict`; check-constraint / corruption / schema
mismatch → `Backend` (permanent). This is exactly the classification AD-7's
delivery runner relies on to decide retryable-vs-terminal.

### ADR-5: Crash recovery = lease-expiry reclaim (durable analogue of `recover_in_flight`)

On the in-memory store, `recover_in_flight` (`store.rs:244-248`, `:574-585`)
sweeps every `InFlight` row back to `Pending` at startup — correct only because a
single process owns all state. On a durable multi-consumer store a blanket sweep
would steal effects still being delivered by a *live* peer consumer. Instead,
recovery is **lease-driven**: an `InFlight` row whose `leased_until <= now`
(its consumer died or stalled past the lease) is reclaimable — the atomic-claim
query treats `Pending`, due `RetryableFailed`, AND expired-lease `InFlight` rows
as claimable, transitioning the reclaimed row back into a fresh lease. The record
is **preserved**, never dropped. A startup `recover_in_flight` on the durable
store therefore reclaims only expired-lease in-flight rows, leaving live-lease
rows to their current consumer — so at-least-once holds across crash without
double-delivering an effect a live peer is mid-delivering.

### ADR-6: Bounded metric cardinality

Durable-store signals reuse the CORE-019 observability contract
(`spec.md:254-268`) and add store-level counters/histograms (claim batch size,
lease renewals, lease expiries/reclaims, transaction retries). Labels are
restricted to a **closed, low-cardinality** set: `effect_type`, outcome/state,
and consumer role. `effect_id`, idempotency key, `tenant` id, `destination`, and
`payload` MUST NOT be metric labels (they may appear only in the already-required
redacted/hashed structured logs, never as unbounded label dimensions). This keeps
time-series cardinality bounded regardless of tenant/effect volume.

### ADR-7: Schema/migration ownership lives in `infrastructure`

The two tables (`effect_state`, `effect_dedup`) and their migrations live under
`crates/infrastructure/migrations/`, applied via `sqlx` `migrate`. The runtime
ports know nothing of the schema; the domain knows nothing of a database. This
keeps the DB entirely behind the adapter boundary (frozen decision 5) and lets
the migration evolve without touching a port signature.

## Data Flow

    accept(AcceptedEffect) ──▶ INSERT effect_state(state=Pending, next_at=NULL)   [txn]
                                   └─ ON CONFLICT(id) ─▶ idempotent no-op / Conflict (ADR-4)

    atomic_claim(now, limit, lease):                         [ONE txn — atomic vs concurrent consumers]
      SELECT … FROM effect_state
        WHERE state=Pending
           OR (state=RetryableFailed AND next_at<=now)
           OR (state=InFlight       AND leased_until<=now)   ← expired-lease reclaim (ADR-5)
        FOR UPDATE SKIP LOCKED  LIMIT limit
      UPDATE claimed rows SET state=InFlight, leased_until = now + lease
      ──▶ returns Vec<LeasedEffect{ StoredEffect, lease_token, leased_until }>

    renew_lease(id, token, now, lease) ─▶ UPDATE … SET leased_until=now+lease WHERE lease_token=token   [txn]
    expiry:  a lease not renewed by leased_until makes the row claimable again (no separate sweep needed)

    reserve(scope, effect_id, fp) ──▶ INSERT effect_dedup … ON CONFLICT ─▶ DedupOutcome (ownership/status)   [txn]
    commit_success(scope) shares the mark_succeeded txn (ADR-4)

### Sequence: two concurrent consumers claim the same due set (no double-delivery)

    ConsumerA        Postgres(effect_state)        ConsumerB
       │─atomic_claim(now,limit,lease)─▶│                    
       │              ├ SELECT … FOR UPDATE SKIP LOCKED (locks rows R1,R2)
       │◀── [R1,R2] leased_until=T ─────┤                    
       │                                │◀─atomic_claim(now,limit,lease)─│
       │                                ├ SKIP LOCKED skips R1,R2 ; locks R3
       │                                ├──────── [R3] leased_until=T ──▶│  (disjoint set — never R1/R2)
       │─deliver R1─ renew_lease(R1)──▶ │  (extends leased_until while working)
       │  ✗ ConsumerA crashes before R2 succeeds                        
       │                                ├ R2.leased_until <= now  ⇒ reclaimable (ADR-5)
       │                                │◀─atomic_claim(now,…)───────────│
       │                                ├──────── [R2] re-leased ───────▶│  (redelivered, at-least-once)

## File Changes

All rows are **FUTURE** production work — none is created by this planning
change.

| File | Action | Description |
|------|--------|-------------|
| `crates/runtime/src/effects/store.rs` | Modify (future) | Add `LeasedEffectStore` port trait + `LeasedEffect`/`LeaseToken` DTOs (ADR-2); `EffectStateStore`/`claim_due` unchanged |
| `crates/runtime/src/effects/mod.rs` | Modify (future) | `pub use` the new lease port + DTOs |
| `crates/infrastructure/src/effects/mod.rs` | Create (future) | Module wiring for the durable adapter |
| `crates/infrastructure/src/effects/durable_store.rs` | Create (future) | `PostgresEffectStore` impl `EffectStateStore` + `EffectDedupStore` + `LeasedEffectStore`; sole `sqlx` consumer |
| `crates/infrastructure/migrations/NNNN_effect_outbox.sql` | Create (future) | `effect_state` + `effect_dedup` tables, indexes on `(state,next_at)` and `(state,leased_until)`, unique on dedup scope |
| `crates/infrastructure/Cargo.toml` | Modify (future) | Depend on `ego-runtime` for the ports; `sqlx` already present (`:14`) |
| `crates/infrastructure/src/lib.rs` | Modify (future) | `pub mod effects;` re-export |

## Interfaces / Contracts

```rust
// crates/runtime/src/effects/store.rs — NEW supplementary port (ADR-2).
// EffectStateStore / EffectDedupStore / claim_due are UNCHANGED.

/// Opaque proof that a caller currently holds a lease on a claimed effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseToken(/* uuid — matched on renew/complete, never a metric label */);

/// A claimed effect plus its lease deadline and token.
pub struct LeasedEffect {
    pub effect: StoredEffect,      // reuses the existing StoredEffect DTO
    pub lease_token: LeaseToken,
    pub leased_until: Timestamp,   // chrono-backed; serializes directly (store.rs:44-70)
}

/// Atomic claim + lease for a durable, possibly multi-consumer backend.
/// Implemented ALONGSIDE EffectStateStore/EffectDedupStore by the durable store.
#[async_trait]
pub trait LeasedEffectStore: Send + Sync {
    /// Atomically claim up to `limit` due-or-expired-lease effects AND transition
    /// them to InFlight under a lease of `lease` duration — in one step, so a
    /// concurrently-claiming consumer can never receive the same rows.
    async fn atomic_claim(
        &self, now: Timestamp, limit: usize, lease: std::time::Duration,
    ) -> Result<Vec<LeasedEffect>, EffectStoreError>;

    /// Extend the lease on a still-in-progress claimed effect. MUST fail (or
    /// no-op with a distinguishable result) if the lease already expired and the
    /// effect was reclaimed by another consumer.
    async fn renew_lease(
        &self, id: EffectId, token: &LeaseToken, now: Timestamp, lease: std::time::Duration,
    ) -> Result<(), EffectStoreError>;
}
```

## Error Model

Reuses `EffectStoreError` (`store.rs:106-135`) with no new variants. SQLSTATE →
variant mapping is ADR-4. A `renew_lease` whose lease already expired resolves to
a distinguishable non-success (`Conflict`), so a consumer that lost its lease
does not later mark an effect it no longer owns.

## Observability

Per ADR-6: bounded-cardinality store metrics (`effect_type`, state/outcome,
consumer role) for claim batches, lease renewals, lease expiries/reclaims, and
transaction retries; redacted/hashed idempotency key and never `payload` in logs
(inherits `spec.md:254-268`). No raw id/key/tenant/destination as a metric label.

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Contract | The **same** port test-suite runs against `InMemoryEffectStore` and the durable store; both satisfy each port independently | shared `#[tokio::test]` matrix |
| Integration | `Pending`/`InFlight` effects survive a simulated crash+restart (state read back after reopening the store) | Postgres-backed `#[tokio::test]` |
| Integration | Two concurrent `atomic_claim` calls return **disjoint** row sets (no double-claim); delivered effect never claimed twice while lease live | concurrent `#[tokio::test]` |
| Integration | `renew_lease` extends visibility; an un-renewed lease expires and the effect becomes claimable; expired-lease `renew` fails `Conflict` | timed `#[tokio::test]` |
| Integration | Crash recovery reclaims only expired-lease in-flight rows, leaving live-lease rows untouched (ADR-5) | Postgres-backed `#[tokio::test]` |
| Integration | Transaction boundary: a forced rollback leaves no partial state/dedup write; SQLSTATE classes map to transient vs permanent `EffectStoreError` | fault-injected `#[tokio::test]` |
| Integration | Durable dedup persists `(tenant, effect_type, key)` scope + ownership status across reopen; cross-tenant never collides | Postgres-backed `#[tokio::test]` |
| Static | No `sqlx`/DB type appears in any port signature in `crates/runtime`/`crates/domain` | source-scan test |
| Doc/static | `exactly once` appears nowhere in the durable store's code, docs, or spec delta | source-scan test |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, or executable-file
classification. The durable store is an outbound DB egress; its data-exposure
risk (leaking payload/key/tenant) is closed structurally by the CORE-019
redaction contract (`spec.md:254-268`) and ADR-6's bounded-label rule, not the
process-integration matrix. SQL-injection risk is closed by `sqlx`
parameterized/compile-checked queries; `destination`/`payload` are stored as
opaque bytes, never interpolated.

## Migration / Rollout / Compatibility

Additive and opt-in. `InMemoryEffectStore` stays the default reference
implementation; the durable store is selected by wiring only. No existing port
contract changes (the lease port is additive — ADR-2), so every existing
implementor and test compiles unchanged. Rolling out = create the two tables via
migration and wire the durable store; rolling back = wire the in-memory store and
drop the tables. No change to `ExternalEffectDescription`, the executor registry,
the acceptor seam, or backoff policy.

## Open Questions

None blocking. The three proposal forks (storage tech, extend-vs-new-port,
consumer model) are resolved in ADR-1/ADR-2/ADR-3. Concrete lease-duration and
claim-`limit` defaults are implementation-level tuning, refinable during apply
without changing any spec-normative contract.
