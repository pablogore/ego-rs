# Design: PROD-002 — Durable External Effect Store

This design answers the 13 Open Questions the proposal deliberately left for
design.md. It does **not** restate the proposal's scope, nor redesign
CORE-019's ports, state model, runner, or observability — those are inputs,
cited per decision. It introduces **no new operation** on either public port's
delivery surface; the only additive surface is a defaulted, declaration-only
capability method (AD-3) added to **both** ports, justified in place.
Retention/cleanup is provider-owned internal maintenance (AD-9), not a port
operation.

The proposal already resolved the central tension: **durability is the
mandatory provider contract; multi-node-safety is a provider-declared
capability, not universally mandated.** This design decides the concrete
mechanism for that declaration (AD-3), the lease/ownership model for the
provider that offers multi-node safety (AD-2/AD-5/AD-6), and the remaining
eleven questions.

## 1. Technical Approach

Ship the two already-shipped CORE-019 ports — `EffectStateStore` and
`EffectDedupStore` (`crates/runtime/src/effects/store.rs`) — against two
durable providers behind a single new crate, `ego-effect-store`
(`crates/effect-store/`), depending on `ego-runtime` plus each backend driver
under a Cargo feature (`postgres` → `sqlx 0.8`, `stoolap` → Stoolap's Rust
API). Both providers implement **both** ports independently (as the spec
already requires of a durable implementation), persist the CORE-019 state
machine unchanged, and are proven correct by a **three-tier** conformance
suite (AD-13): a shared port-conformance tier all three stores pass identically,
a durable-provider tier only the durable providers pass (real close→reopen
across the same backing storage), and a capability-gated multi-node tier only
`PostgresEffectStore` runs.

The durable store plugs into the **existing** `DeliveryRunner`
(`crates/runtime/src/effects/runner.rs`) recovery affordances —
`claim_due`/`recover_in_flight` — rather than growing a second recovery
mechanism. Multi-node claim ownership on PostgreSQL is expressed **entirely
inside the provider's SQL** (owner column + expiring lease, plus a
non-load-bearing `claim_epoch` counter kept only for observability); no lease
token ever crosses the Rust port boundary (AD-6). Stoolap is a
durable *local* state machine — single-host ownership, `MultiNodeSafe: NO` by
design — and needs none of that machinery. `InMemoryEffectStore` stays,
unchanged, declaring the non-durable profile through the AD-3 default.

The runtime never learns which provider it holds beyond the queryable
capability descriptor: it holds `Arc<dyn EffectStateStore>` /
`Arc<dyn EffectDedupStore>` exactly as today.

## 2. Module / Crate Placement

New crate, **not** an extension of `ego-persistence`. One crate holds both
providers behind feature flags — not one crate per provider.

| File | Action | Contents |
|------|--------|----------|
| `crates/effect-store/Cargo.toml` | Create | `ego-effect-store`; deps `ego-runtime`; optional `sqlx = "0.8"` (feature `postgres`), Stoolap driver (feature `stoolap`); `async-trait`, `chrono`, `uuid`, `thiserror`. **No default backend feature** — each deployment opts into exactly the driver it runs |
| `crates/effect-store/src/lib.rs` | Create | Crate root, re-exports, `EffectStoreCapabilities`-based provider docs |
| `crates/effect-store/src/postgres/mod.rs` | Create (feat `postgres`) | `PostgresEffectStore` implementing both ports; owner/epoch/lease SQL |
| `crates/effect-store/src/postgres/migrations.rs` + `migrations/00X_*.sql` | Create (feat `postgres`) | Own numbered sequence starting at `001` (AD-10), hand-rolled `include_str!` runner mirroring `ego-persistence` |
| `crates/effect-store/src/stoolap/mod.rs` | Create (feat `stoolap`) | `StoolapEffectStore` implementing both ports; local ownership, no lease columns |
| `crates/effect-store/tests/conformance.rs` | Create | Three-tier conformance (AD-13): (1) shared **port** harness against Postgres (env-gated), Stoolap, and `InMemoryEffectStore`; (2) **durable-provider** harness driven by a test-only `DurableStoreFactory` (reopen against the same backing location) for Stoolap + Postgres, plus an `InMemoryEffectStore` **negative** non-durability test; (3) **multi-node** harness (Postgres only, `capabilities().multi_node_safe`-gated). `DurableStoreFactory` lives here, never in `crates/runtime/src/effects/store.rs` |
| `crates/runtime/src/effects/store.rs` | Modify | Add `EffectStoreCapabilities` struct + **defaulted** `capabilities()` on **both** `EffectStateStore` and `EffectDedupStore` (AD-3/G6). No other port change |
| `crates/runtime/src/effects/observability.rs` | Modify | Extend the existing `log_*` surface with claim/lease-expiry/cleanup signals (AD-14) — not a parallel surface |
| `crates/testkit/src/effects.rs` | Modify | `FaultInjectingEffectStore` real-trait-impl double (AD-12), beside `RecordingExecutor` |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | Register a durable store where `InMemoryEffectStore` registers today; log **both** registered ports' declared capabilities at startup, so a mixed durable/non-durable registration is observable (G6) |
| `examples/reference-app` | Modify | Full dogfood against the embedded **Stoolap** provider (no external server) |

### 2.1 Dependency-graph impact (`ARCHITECTURE.md`)

`ego-effect-store` adds exactly one new node with edges
`ego-effect-store → ego-runtime` and `ego-effect-store → {sqlx | stoolap}`.
It introduces **no** edge into `ego-persistence` and **does not** invert the
documented `ego-persistence → ego-domain`-only boundary. `ARCHITECTURE.md`'s
verified graph gains one leaf; nothing existing changes direction. This is the
decisive reason for AD-1.

## 3. Architecture Decisions

Each AD maps to one proposal Open Question (Q#). Detailed subsections follow the
table for the decisions that need more than one row.

| AD | Q | Decision | Rejected | Rationale |
|----|---|----------|----------|-----------|
| **AD-1** Crate placement | Q1 | One new crate `ego-effect-store` depending on `ego-runtime` + backend drivers; both providers behind Cargo features (`postgres`, `stoolap`) in that **one** crate | (a) Extend `ego-persistence` with a `→ ego-runtime` edge; (b) one crate per provider | (a) inverts a documented boundary and still doesn't home Stoolap; (b) duplicates the port-mapping, capability descriptor, and conformance harness for zero boundary benefit — Rule of Two not met. One crate keeps today's verified graph intact and lets features exclude an unused driver (reference-app compiles Stoolap only, no sqlx/server). See §2.1 |
| **AD-2** Lease/ownership (multi-node path) | Q2 | PostgreSQL only: `SELECT … FOR UPDATE SKIP LOCKED` to claim due rows that are unclaimed or whose lease has expired, stamping `claim_owner` (per-process UUID) + `claim_expires_at` lease (+ a `claim_epoch` counter kept for **observability only**, not for fencing); every later transition is conditional on still-valid ownership. A worker whose row was reclaimed by a peer sees `rows_affected == 0` → `EffectStoreError::Conflict` and drops the attempt without confirming | Advisory locks; bare `FOR UPDATE SKIP LOCKED` without owner+lease; threading a fencing token through the port | Advisory locks are process-scoped, not durable ownership; a bare row lock releases at txn commit, so ownership must live in a durable column + lease. Ownership+lease guarantees claim exclusivity **only while the lease is valid** — once it expires the row is reclaimable and duplicate external execution becomes possible, covered by at-least-once + destination idempotency, **not** prevented by claim exclusivity (matches the corrected spec). A same-`worker_id` re-claim after lease lapse is a known, deliberately-unfenced window (§3.1); `claim_epoch` is retained so the reclaim itself is observable (AD-14), not because it can detect a subsequent stale write. See §3.1 |
| **AD-3** Capability declaration | Q3 | `EffectStoreCapabilities { durable, concurrent_local_safe, multi_node_safe, supports_leases }` value returned by a new **defaulted** `capabilities(&self)` on **both** `EffectStateStore` and `EffectDedupStore` (G6); default = the in-memory (all-false) profile. Declaration only — queryable at registration, logged at startup; PROD-002 does **not** reject a `MultiNodeSafe: NO` provider (Ego has no topology model) | Associated const / type-level marker; a separate mandatory trait; declaring on only one port | The runtime holds `Arc<dyn EffectStateStore>` / `Arc<dyn EffectDedupStore>`, so an associated const/type-level marker is unreadable through the trait object — a method is the only object-safe option. A separate mandatory trait would break "minimal, universally-implementable port" and force every existing/third-party impl to add it. Declaring on only one port would let a mixed durable/non-durable registration look durable — both halves must report their own truthful profile. See §3.2 |
| **AD-4** Stale/abandoned claim recovery | Q4 | Two distinct, non-conflated mechanisms: (1) lease expiry **in scope** for the Postgres path, expressed as an expiry predicate *inside* `claim_due` (`state=in_flight AND claim_expires_at < now`), driven by the runner's existing periodic tick — **no new sweep component**; (2) `recover_in_flight(now)` stays the startup single-owner sweep, and on Postgres is scoped to expired-lease rows so it can never steal a live peer's in-flight work | A separate distributed-recovery daemon; blanket "reset all `InFlight` → `Pending`" on Postgres | A blanket reset in a multi-node world steals a live peer's effect. The existing `now` parameter is exactly enough to scope recovery to expired leases — no port change, no new daemon. See §3.3 |
| **AD-5** Atomic state transitions | Q5 | Each transition is one conditional `UPDATE … WHERE id=$1 AND state = ANY($allowed_from) AND <ownership-valid>`; `claim_due` stamps ownership under `FOR UPDATE SKIP LOCKED` in one transaction. `rows_affected == 0` maps deterministically to `InvalidTransition` (row in another state) or `Conflict` (ownership lost) | Table-level lock; `SERIALIZABLE` + optimistic version without `SKIP LOCKED` | Row-level conditional updates give atomicity without throughput collapse or serialization-failure retry storms; `SKIP LOCKED` lets concurrent claimers skip rather than collide-and-retry. See §3.1 |
| **AD-6** Port sufficiency | Q6 | `claim_due`/`recover_in_flight` + the existing `mark_*` verbs **suffice**. Leasing/ownership is an internal Postgres implementation detail keyed off the provider instance's own `worker_id`; no lease token, epoch, or claim method is added to either port. The **only** additive surface is AD-3's defaulted declaration method (on both ports). `InMemoryEffectStore` stays conformant unchanged | Add `claim_with_lease`/`renew_lease`/`fence_token` to the port | Adding lease/fencing methods would force every impl (in-memory, Stoolap, third-party) to model coordination they don't need and leak a Postgres-specific mechanism into the universal contract. Because one `DeliveryRunner` per process owns the full claim→dispatch→mark lifecycle, the provider validates ownership against its own `worker_id` internally — the token never needs to cross the port. The accepted cost of this minimalism is a bounded limitation: a superseded *same-`worker_id`* claim generation can still land a transition (§3.1), absorbed by at-least-once + idempotency and bounded by lease tuning — not fenced by the port. See §3.1 |
| **AD-7** Retry persistence shape | Q7 | Durable state = `attempt` + `next_at` + `state`, written through the **existing** `mark_retryable(id, attempt, next_at)` verb (no new column semantics). Retry **policy** (max attempts, backoff base/max/jitter, per-`effect_type` overrides) stays runtime-side in `DeliveryRunner`'s `RetryPolicies` (CORE-019 `policy.rs`), **not** in the store | Store retry policy config in the DB | The store records the *outcome* the runner computed; policy is a runner behavior. Keeping policy out of the schema means tuning backoff is a config change, not a migration, and a third-party store never has to understand policy |
| **AD-8** Dedup durability | Q8 | Reservation is a durable row keyed by `(tenant, effect_type, key)` with `effect_id` owner, `fingerprint BYTEA(32)`, `succeeded BOOL`. `reserve` = atomic `INSERT … ON CONFLICT DO NOTHING` + classify → `Fresh`/`Owned*`/`Other*`/`Conflict`; `commit_success` flips `succeeded` in place; `release` deletes. A crash mid-reservation leaves no partial state (single atomic upsert). Retention: succeeded/released dedup rows fall under the AD-9 TTL; a reservation is never deleted while its effect is non-terminal | Delete the reservation on success | Deleting on success makes a same-key crash-recovery re-attempt see `Fresh` and re-execute — the exact silent-loss bug CORE-019's round-4 in-place `succeeded` flag fixed; the durable store must mirror that in-place flip. See §3.4 |
| **AD-9** Cleanup/retention policy | Q9 | TTL-based, operator-tunable (default e.g. 7 days — a runtime constant, **not** spec-normative, same posture as CORE-019 AD-5 backoff numbers). Only terminal-resting rows eligible (`Succeeded`/`TerminalFailed` effect rows + settled dedup rows, `settled_at < now - ttl`), deleted in bounded `DELETE … LIMIT batch` batches. **G12 rewrite:** the SQL stays provider-owned (`PostgresEffectStore`/`StoolapEffectStore::run_retention`, unchanged), but the *schedule* is not — no background task/scheduler lives inside either provider. Both implement the new optional capability trait `RetentionMaintenance` (`purge_before`/`oldest_terminal`), and a **runtime-owned** worker (`ego-service-sdk`'s `EffectRetentionWorker`, sibling of PROD-012's reservation `RetentionWorker`) drives it on a configured schedule — off unless a policy and a `RetentionMaintenance` store are both registered. See §3.7 | Count-based cap; operator-triggered only; a `cleanup`/`purge` verb on the universal port; runner-driven cleanup; a provider-owned background scheduler (Phase 4/5's original shape) | The delivery ports expose **no** purge verb, so the runner has nothing to call, and adding one would force every impl (in-memory, third-party) to implement retention they don't need — violating AD-6. A provider-owned *scheduler* (as opposed to provider-owned *SQL*) duplicates the lifecycle machinery (`Notify`-based cancellation, bounded shutdown, panic isolation) PROD-012 already built once for reservation retention, and gives every provider its own ad hoc on/off switch instead of one runtime-level "off unless asked for" posture. Keeping the SQL in the provider and moving only the schedule to the runtime gets both: no duplicated SQL, and one lifecycle owner. Count caps can evict a still-relevant dedup key; manual-only grows unbounded between runs (the named risk); redundant multi-node deletes are harmless (idempotent `DELETE`) |
| **AD-10** Migration versioning | Q10 | New crate ⇒ **own** sequence starting at `001` in `crates/effect-store/src/postgres/migrations/`, run by the crate's own hand-rolled `include_str!` runner (mirroring `ego-persistence`'s pattern). Tables prefixed `effect_` (`effect_state`, `effect_dedup`). No collision with `ego-persistence`'s 001–006 — different crates, different tables, no shared version ledger | Continue `ego-persistence`'s 007+ sequence | Continuing 007+ only makes sense under the rejected AD-1 option-2, and would entangle two crates' schema lifecycles. `ego-persistence` uses a hand-rolled runner (not sqlx's `migrate!`/`_sqlx_migrations`), so there is no shared ledger to collide on — "collision" is moot once the tables live in a separate crate |
| **AD-11** Graceful shutdown with held claims | Q11 | Lease expiry is the **sole, authoritative** release mechanism. On drain, still-`InFlight` effects flow through CORE-019's existing shutdown path; for the durable store, their lease lapses and a successor re-claims them via `claim_due`'s expired-lease predicate (AD-4), redispatching, never assuming delivered. No proactive/explicit release is performed | Mandatory explicit release as the *only* mechanism; a best-effort proactive release on drain | A hard crash (`kill -9`, OOM) can't run an explicit release; correctness must rest on lease expiry alone, or durability is a lie. A proactive release would also need a port method that does not exist (the delivery ports expose no `release_claims`) and only shortens successor latency without changing correctness — so it is deliberately omitted; the successor picks up the lapsed lease on its next `claim_due` tick |
| **AD-12** TestKit double shape | Q12 | `FaultInjectingEffectStore` — a real impl of **both** ports (like the in-memory composite) wrapping a real `InMemoryEffectStore`, adding a **scripted, deterministic** `FaultPlan`: per-method transient-error queues (`TemporarilyUnavailable`/`Backend` on the Nth call) and three **distinct, non-contradictory** crash operations — `simulate_process_crash()` (destroys volatile state, models non-durable loss), `simulate_runner_crash()` (preserves backing state but abandons in-flight ops so `recover_in_flight`/`claim_due` see them recoverable — the one recovery-logic tests use), and `crash_after(op)` (write landed, response lost — ambiguity/idempotency window) — plus a claim-race interleave hook. No randomness (determinism axiom). Real close→reopen durability is **out of scope** for this double (that is AD-13's durable-provider tier against real Stoolap/Postgres) | A mock-object framework; fault hooks baked into the production durable store; a single `simulate_crash()` doing the job of both loss and recovery | The repo's convention is "real trait impl, not a look-alike" (`RecordingExecutor`; `detect-mock-only-tests.sh` exists). A fault plan on a real store stays usable anywhere the real store is; test concerns never leak into prod code. A single `simulate_crash()` was self-contradictory — dropping state cannot also leave in-flight effects recoverable — so the loss and recovery cases are split. See §3.5 |
| **AD-13** Conformance suite | Q13 | **Three tiers** in `crates/effect-store/tests/`. **(1) Port** — `run_state_store_conformance(&impl EffectStateStore)` / `run_dedup_conformance(&impl EffectDedupStore)`: everything provable on ONE live instance (transitions, `DedupOutcome` classification, retry bookkeeping shape, `rows_affected` atomicity); runs against `InMemoryEffectStore`, Stoolap, **and** Postgres (env-gated) identically. **(2) Durable-provider** — `run_durable_conformance(&impl DurableStoreFactory)`: a test-only factory opens a store bound to a fixed backing location, `accept`s, is dropped, a **new** instance reopens the **same** location and must observe survival + in-flight-at-crash redispatch eligibility; runs against Stoolap (reopen same file) and Postgres (second pool, same tables) **only** — plus an `InMemoryEffectStore` **negative** test proving it does NOT survive drop/reconstruct. **(3) Multi-node** — `run_multi_node_conformance(&impl DurableStoreFactory)`: reuses Tier 2's factory, calling `open()` **twice without dropping either result** (concurrent, not sequential) — each `PostgresEffectStore` mints its own `worker_id` at construction (§3.1), so two live instances against the same tables are already two independently-owned claimers with no new trait needed; `capabilities().multi_node_safe`-gated, Postgres only | Duplicate tests per provider; put the harness in `ego-testkit`; run "restart survival" against `InMemoryEffectStore`; model restart with a single live reference; a second, separate multi-node factory trait | Per-provider duplication is the named "double the surface" risk for the shared port tier. `ego-testkit` can't own it: instantiating providers needs the backend drivers, so the harness lives in the crate that owns them. Restart survival CANNOT be a shared assertion: `InMemoryEffectStore` is contractually required to LOSE `Pending`/`InFlight` on crash, so asserting survival against it contradicts its documented behavior; and a single live `&impl` reference can never cross a real restart boundary (destroy instance → new instance on same storage) — hence the separate factory-driven tier. A second multi-node-specific factory trait was rejected: `DurableStoreFactory::open()` already yields an independently-owned instance per call (fresh `worker_id`), so Tier 3 differs from Tier 2 only in usage pattern (concurrent vs. sequential), not in the abstraction needed. See §3.6 |
| **AD-14** Observability extension | (Scope) | Extend the existing `log_*` surface in `observability.rs` with `claim_acquired`, `claim_reclaimed_after_expiry` (`effect_id`, `previous_owner`, `new_owner`, `previous_epoch`, `new_epoch` — emitted by `claim_due` itself when it takes over a row whose lease had expired, since `claim_due`'s own `UPDATE` sees the prior owner/epoch before overwriting them), `recovered_in_flight`, and `cleanup_deleted` — same field-shape/redaction discipline (no payload, hashed idempotency key) | A parallel signal surface; a `superseded_claim_observed` signal fired from the `mark_*` transitions | A `mark_*`-side "superseded claim" signal is **not implementable**: the transition guard (§3.1) never receives or compares an epoch — it only checks `claim_owner`/lease validity — so no code path at `mark_*` time can know which epoch generation a landing write conceptually belongs to. `claim_reclaimed_after_expiry`, fired from `claim_due` where the previous/new owner and epoch are genuinely both in hand, is what the mechanism can actually observe: a reclaim happened. It cannot and does not claim to detect a stale write landing afterward — that residual risk stays inferred (bounded by lease-tuning, §6), not directly counted |

### 3.1 Lease, ownership, atomicity, and why the port stays unchanged (AD-2 / AD-5 / AD-6)

The three questions Q2/Q5/Q6 share one answer: **ownership lives in the SQL, not
in the Rust port.**

The `PostgresEffectStore` is a per-process instance holding a `worker_id: Uuid`
minted at construction. The `effect_state` table carries three ownership
columns: `claim_owner UUID NULL`, `claim_expires_at TIMESTAMPTZ NULL`, and
`claim_epoch BIGINT NOT NULL DEFAULT 0`. Ownership and lease validity are what
guard transitions; `claim_epoch` is **observability only** (it is stamped and
logged, never checked in a guard — see the known-limitation note below).

**Claim (`claim_due`, honoring CORE-019 AD-8 "does not transition state").**
A durable `claim_due` selects due rows and stamps ownership *without changing
`state`*, in one transaction:

```sql
UPDATE effect_state
SET claim_owner = $worker_id,
    claim_epoch = claim_epoch + 1,
    claim_expires_at = now() + $lease
WHERE effect_id IN (
    SELECT effect_id FROM effect_state
    WHERE (
        state IN ('pending', 'retryable_failed')
        AND next_at <= $now
        AND (claim_owner IS NULL OR claim_expires_at < $now)  -- G1: skip live claims
      )
       OR (
        state = 'in_flight'
        AND claim_expires_at < $now                           -- AD-4 lease expiry
      )
    ORDER BY next_at
    FOR UPDATE SKIP LOCKED
    LIMIT $limit)
RETURNING effect_id, tenant_id, effect_type, destination, idempotency_key,
          payload, attempt, state, next_at;
```

`FOR UPDATE SKIP LOCKED` guards only *concurrent* transactions: once a claim
commits, the row is unlocked again. So the predicate itself must **also** exclude
rows that already carry a live claim — the `claim_owner IS NULL OR
claim_expires_at < $now` guard (G1). Without it, a `pending` row stays
state-neutral after being claimed (by design, `claim_due` does not transition
state), so a *second* `claim_due` — another node, or a rapid repeat — would match
and re-stamp the same still-owned row before its first claimant ever calls
`mark_in_flight`, breaking claim exclusivity at its source. With both guards a
claim is exclusive while its lease holds. `state` stays `Pending`/
`RetryableFailed` (or `InFlight` for a reclaimed expired lease), so the contract
"`claim_due` does not itself transition state" holds verbatim — only ownership
metadata moved, and `RETURNING` yields exactly the rows this call newly owns
(the `UPDATE` touches only rows the inner `SELECT` claimed).

**Every subsequent transition is conditional on still-valid ownership.**
`mark_in_flight`/`mark_succeeded`/`mark_retryable`/`mark_terminal` run:

```sql
UPDATE effect_state SET state = $to, ...
WHERE effect_id = $1
  AND state = ANY($allowed_from)
  AND claim_owner = $worker_id
  AND claim_expires_at > now();      -- my lease is still valid
```

The provider validates ownership against **its own `worker_id`** — the runner
never passes a token, so no port signature changes (AD-6). `rows_affected`
resolves the outcome deterministically (AD-5): `1` = applied;
`0` + row still exists in a different state = `InvalidTransition`;
`0` + `claim_owner`/lease no longer mine = `Conflict`.

A reclaim by a **different** worker is already safe: the winning `claim_due`
overwrites `claim_owner` with the peer's id, so the superseded worker's
`claim_owner = $worker_id` guard matches nothing and its stale write affects 0
rows → `Conflict`. This is the ordinary multi-node case and it is fully fenced.

**Known limitation — `claim_epoch` is *not* load-bearing (G2).** The guard
checks `claim_owner` and lease validity but **not** `claim_epoch`, because the
port carries no epoch/token to check it against (AD-6). One window therefore
stays open: a worker whose own execution stalls past its lease, whose row is
then reclaimed *by that same `worker_id`* (fresh `claim_epoch`, `claim_owner`
unchanged), can have its stale execution later land a transition — the guard
sees the same owner and a valid (new) lease and applies the write. We accept
this **explicitly** rather than fence it: closing it would require threading an
epoch/claim token through `mark_in_flight`/`mark_succeeded`/`mark_retryable`/
`mark_terminal`, forcing every impl (in-memory, Stoolap, third-party) to model
coordination they do not need — the exact port-surface bloat AD-6 exists to
avoid. The residual duplicate external execution this can cause is the *known*
at-least-once possibility the corrected spec covers via destination
idempotency, **not** something claim exclusivity promises to prevent. The window
is bounded by choosing `lease` comfortably longer than one dispatch's
worst-case duration (§6). `claim_epoch` is stamped and logged so **the reclaim
itself** is observable (AD-14 `claim_reclaimed_after_expiry`, fired from
`claim_due` where the previous/new owner and epoch are both in hand) — but
this does not and cannot detect a stale write from the superseded generation
actually landing afterward, since the `mark_*` guard never receives or checks
an epoch. That residual risk stays inferred and bounded by lease tuning
(§6), not directly counted.

**What a reclaimed-out worker observes:** its conditional `UPDATE` affects 0
rows → `EffectStoreError::Conflict`. The `DeliveryRunner` treats "not mine
anymore" exactly like CORE-019's `OtherInProgress` handling — drop the attempt,
charge no retry, release no dedup, leave it reclaim-eligible. No double-confirm;
any duplicate execution is bounded by the destination's own idempotency, not
promised away by the store.

**Stoolap needs none of this.** It is single-host, single-owner: ownership is
implicit in "this process owns the embedded DB." Its transitions are plain
conditional `UPDATE … WHERE state = ANY($allowed_from)` under its MVCC/snapshot
isolation; no owner/epoch/lease columns exist in its schema. That is precisely
what `MultiNodeSafe: NO` / `supports_leases: false` *means* for it.

### 3.2 Capability descriptor (AD-3)

```rust
// crates/runtime/src/effects/store.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectStoreCapabilities {
    /// An accepted effect survives process death.
    pub durable: bool,
    /// Two concurrent workers sharing this store *within a single process or
    /// host ownership domain* never both hold a valid claim on the same
    /// effect. Says nothing about safety across independent processes/nodes —
    /// hence "local", not "process": conventional "process-safe" wording would
    /// wrongly imply cross-process safety.
    pub concurrent_local_safe: bool,
    /// The above holds across independent nodes/hosts.
    pub multi_node_safe: bool,
    /// Claims carry an expiring lease reclaimable after owner death without a
    /// manual sweep.
    pub supports_leases: bool,
}
```

Field meanings are stated as **observable guarantees**, not mechanisms.
Declared profiles:

| Provider | durable | concurrent_local_safe | multi_node_safe | supports_leases |
|----------|:---:|:---:|:---:|:---:|
| `InMemoryEffectStore` (default) | ✗ | ✗ | ✗ | ✗ |
| `StoolapEffectStore` | ✓ | ✓ (local) | ✗ | ✗ |
| `PostgresEffectStore` | ✓ | ✓ | ✓ | ✓ |

Added as a defaulted method — the **identical** signature on **both**
`EffectStateStore` and `EffectDedupStore` (G6) — so no existing or third-party
impl breaks and each half declares its own truthful profile:

```rust
fn capabilities(&self) -> EffectStoreCapabilities {
    EffectStoreCapabilities { durable: false, concurrent_local_safe: false,
                              multi_node_safe: false, supports_leases: false }
}
```

It is synchronous (a static declaration, no I/O), so it does not disturb the
`#[async_trait]` / `Send + Sync` shape.

**Where it is checked, and where it is *not*.** The service-sdk registration
path logs **both** registered stores' capabilities at startup — the
`EffectStateStore` and the `EffectDedupStore` independently (G6). Because the two
ports are registered independently, a deployment can pair, say, a durable
Postgres `EffectStateStore` with a non-durable in-memory `EffectDedupStore`;
logging both makes such a mixed registration **observable** (its
idempotency-across-restart gap visible at startup) rather than silently reported
as "durable" off the state store alone. Consistent with the AD-3 rationale, Ego
still does **not** hard-reject the mix — it only makes the mismatch legible.
Ego does **not** reject a `MultiNodeSafe: NO` provider either — it has no concept
of "a topology that needs multi-node," and inventing one would drag clustering
into the framework (an explicit non-goal). Instead the descriptor is *queryable*,
so
a host application that genuinely runs multiple nodes (e.g. a Bridge-style
deployment composing per-node Stoolap durability under an external OpenRaft
coordination layer) asserts `store.capabilities().multi_node_safe` itself and
fails closed at **its own** composition boundary. Enforcement of topology fit is
the host's responsibility; Ego only makes the fact declarable and hard to
ignore.

### 3.3 Stale/abandoned claim recovery, un-conflated (AD-4)

The proposal's risk table flags conflating two different things. They stay
separate:

- **Distributed lease expiry (Postgres)** is *in scope* but is **not** a new
  component or port method. It is the `state = 'in_flight' AND
  claim_expires_at < now` disjunct already shown in §3.1's `claim_due`. The
  runner's existing periodic `claim_due` tick *is* the recovery sweep — a dead
  node's leases lapse and a live node re-claims them on the next tick, taking
  over `claim_owner` so the dead node's zombie writes (if it revives) hit the
  ownership guard and affect 0 rows. (This cross-node case is fenced by owner
  takeover; only a same-`worker_id` reclaim is the accepted §3.1 window.)
- **`recover_in_flight(now)` startup sweep** stays the single-owner restart
  affordance. On Stoolap/in-memory (single owner) it safely resets all
  `InFlight` → claimable. On Postgres it MUST be scoped to expired-lease rows
  (`claim_expires_at < now`) so a restarting node cannot steal an effect a live
  peer currently owns. The existing `now` parameter is exactly sufficient — no
  signature change.

Who runs it: the same one `DeliveryRunner` per process that exists today. No
new daemon, no second consumer.

### 3.4 Dedup durability and the crash-mid-reservation case (AD-8)

`effect_dedup` primary key `(tenant_id, effect_type, idempotency_key)`; columns
`effect_id UUID`, `fingerprint BYTEA`, `succeeded BOOL`, `reserved_at`,
`settled_at`.

`reserve` is one atomic statement plus a classifying read:

```sql
INSERT INTO effect_dedup (tenant_id, effect_type, idempotency_key,
                          effect_id, fingerprint)
VALUES ($t, $ty, $k, $eid, $fp)
ON CONFLICT (tenant_id, effect_type, idempotency_key) DO NOTHING;
-- then read the (existing or just-inserted) row and map to DedupOutcome
```

The mapping mirrors `InMemoryEffectStore::reserve` exactly (the six `DedupOutcome`
variants, fingerprint-mismatch → `Conflict`, owner+`succeeded` →
`Owned*`/`Other*`). Because the upsert is atomic, a crash mid-`reserve` leaves
**no partial state**: either the row is committed (a later same-`effect_id`
retry sees `OwnedInProgress` and re-executes safely) or it is not (retry sees
`Fresh`). `commit_success` flips `succeeded` in place — never deletes — so a
crash-recovery re-attempt by the same effect still finds `OwnedSucceeded`, and a
different later submission still finds `OtherSucceeded`, never `Fresh` (the
CORE-019 round-4/round-5 invariant, preserved durably). `release` deletes,
freeing the scope for a genuinely unrelated future key.

Retention window: a reservation is retained while its effect is non-terminal
and, once settled, falls under the AD-9 TTL alongside terminal effect rows.

**G15 reconciliation (post-implementation fault-semantics audit).** A
dedicated model-checking audit found that the in-place-flip/delete split
above is necessary but not sufficient: the *caller* orchestrating abandonment
(`DeliveryRunner::abandon_and_release`) called `dedup.release()`
unconditionally, regardless of whether the corresponding
`EffectStateStore::mark_terminal()` transition was accepted. A superseded
attempt whose claim had already been reclaimed by another worker could still
reach this path, receive an error from `mark_terminal` (`Conflict`,
`InvalidTransition`, or a storage failure — the in-memory/Stoolap/Postgres
backends do not all surface the same variant for "someone else already
resolved this," so the rule below is stated in terms of *any* error, not
`Conflict` specifically), and still delete the reservation the new owner had
already flipped to `succeeded` — turning a future, unrelated submission's
`OtherSucceeded` into `Fresh`, in direct violation of the invariant stated
above.

*Resolution — causal gating of destructive dedup release:* a destructive
`EffectDedupStore::release()` call MUST only execute after the corresponding
`EffectStateStore::mark_terminal()` transition has succeeded. If
`mark_terminal()` returns any error, the caller MUST NOT release the dedup
reservation — `mark_terminal()` is the authority check for whether the
current attempt is still entitled to perform the terminal transition at all;
if that check rejects the attempt, the attempt has no standing to perform the
destructive mutation abandonment implies either.

This gate applies specifically to the destructive path. `commit_success()`
does not require the same ordering constraint: it is monotonic and
idempotent (`succeeded = true`, never reset), so a stale `commit_success()`
call can at most redundantly reconfirm success — it cannot turn a completed
`OtherSucceeded` reservation back into `Fresh`. If the `mark_succeeded()`
that follows it is itself rejected, the existing crash-recovery semantics
(this section, and §7's reclaim path) reconcile the resulting partial state
on the next reclaim tick, exactly as they already do for an ordinary
post-success bookkeeping failure.

This is a runner-level fix (`DeliveryRunner::abandon_and_release`), not a
port change: it does not add ownership, epoch, or fencing information to
`EffectDedupStore`'s signature, and does not reopen AD-6 or the
deliberately-accepted same-`worker_id` reclaim window (G2, §3.1/§6) — a
same-`worker_id` reclaim leaves `claim_owner` unchanged, so `mark_terminal`
may still accept a superseded same-worker attempt; that residual is
unchanged by this fix and remains bounded by lease tuning as before.

### 3.5 Fault-injection double (AD-12)

`FaultInjectingEffectStore` wraps a real `InMemoryEffectStore` and implements
both ports, delegating by default:

```rust
pub enum StoreOp { Accept, MarkInFlight, MarkSucceeded, MarkRetryable,
                   MarkTerminal, ClaimDue, RecoverInFlight,
                   Reserve, CommitSuccess, Release }

pub struct FaultPlan {
    /// Scripted transient/permanent errors per op, consumed in order
    /// (mirrors RecordingExecutor::with_outcomes' "repeat last / then pass").
    fail_calls: HashMap<StoreOp, VecDeque<EffectStoreError>>,
    /// Named crash point: after this op's write lands, before its Ok returns
    /// (drives `crash_after` — write landed, caller's response lost).
    crash_after: Option<StoreOp>,
}
```

**Three distinct crash operations, not one.** A single `simulate_crash()` is
self-contradictory: a recovery test needs pre-crash in-flight effects to still
be recoverable *after* the call, which is impossible if the state was dropped.
So the double exposes three non-overlapping operations, each with one job:

1. **`simulate_process_crash()`** — destroys all volatile state, mirroring what
   really happens to `InMemoryEffectStore` on a real crash. Used to exercise the
   non-durable store's documented *loss* behavior. There is nothing to recover
   afterward, by design — it is **not** the operation for recovery-logic tests.
2. **`simulate_runner_crash()`** — deletes nothing; preserves all backing state
   but marks in-flight operations as abandoned (as if the runner that held them
   died), so a subsequent `recover_in_flight`/`claim_due` sees them as
   recoverable/reclaimable. **This** is the operation the retry/recovery
   scenarios use (the ones previously described as depending on
   `simulate_crash()`).
3. **`crash_after(op)`** — the write for `op` landed but its `Ok` never reaches
   the caller, for ambiguity-window / idempotency-on-retry tests (backed by the
   `FaultPlan::crash_after` field above).

Scripted `TemporarilyUnavailable`/`Backend` returns drive the
AD-9-classification and bookkeeping-retry paths; a claim-race hook lets a test
interleave two `claim_due` → `mark_in_flight` sequences to assert two workers
never hold overlapping *valid* claims — and, in the lease-expiry case, that
redispatch is possible and idempotency-covered rather than prevented. Everything
is scripted and deterministic — no randomness, per the determinism axiom — and
it remains a genuine `EffectStateStore + EffectDedupStore`, usable anywhere the
real store is, including as an input to the AD-13 port-tier harness's negative
paths.

**Out of scope for this double: real close→reopen durability.** The fault double
stays in-memory-backed (repo convention: a real trait impl, not a mock) with
corrected, non-contradictory crash semantics; it must not pretend to be durable.
Proving that state actually survives destroying a store instance and rebuilding
it against the same storage is exactly the job of AD-13's durable-provider tier
against real Stoolap/Postgres (§3.6), not this double.

### 3.6 Conformance suite (AD-13), three tiers

A single undifferentiated suite cannot express PROD-002's criteria honestly:
"survives restart" is *false by contract* for `InMemoryEffectStore`, and a
harness holding one live `&impl EffectStateStore` can never model a restart at
all (a real restart destroys the instance and rebuilds a new one over the *same*
backing storage). So the criteria split into three tiers by what they can
actually be run against.

**Tier 1 — Port conformance (all three stores, identically).**
`run_state_store_conformance(store: &impl EffectStateStore)` /
`run_dedup_conformance(store: &impl EffectDedupStore)`: everything provable on
ONE live instance without crossing a restart boundary — CORE-019's transition
scenarios, `DedupOutcome` classification (all six variants,
fingerprint-mismatch → `Conflict`), retry bookkeeping *shape*
(`mark_retryable` resumes `attempt`, does not reset), `rows_affected` atomicity
(`InvalidTransition` vs `Conflict`), and both ports satisfied independently. Runs
against:

- `InMemoryEffectStore` — always (cheap, proves the harness itself).
- `StoolapEffectStore` — always (embedded, no server).
- `PostgresEffectStore` — `#[cfg(feature = "postgres")]`, env-gated on a real
  `DATABASE_URL`; skipped with a logged notice when absent, so
  `cargo test --workspace` stays green without Postgres.

**Tier 2 — Durable-provider conformance (durable providers only).** Restart
survival needs a factory that can build a store, tear it down, and rebuild a new
instance over the *same* storage. The concrete shape chosen is a **test-only
trait** (in `crates/effect-store/tests/`, **not** a production port method):

```rust
// crates/effect-store/tests/ — test-only. NEVER added to EffectStateStore/
// EffectDedupStore in crates/runtime/src/effects/store.rs.
#[async_trait]
trait DurableStoreFactory {
    type Store: EffectStateStore + EffectDedupStore;
    /// Open a store bound to THIS factory's fixed backing location. Called more
    /// than once on the same factory to model close→reopen against the SAME
    /// storage: a first `open()` accepts effects, the returned store is dropped
    /// (process death), a second `open()` must observe the prior state.
    async fn open(&self) -> Self::Store;
}

async fn run_durable_conformance(factory: &impl DurableStoreFactory) { /* … */ }
```

Each durable provider supplies a factory pinned to a fixed location at
construction, so `open()` is genuinely a reopen, not a fresh store:

- **Stoolap** — the factory owns a `tempfile::TempDir`; `open()` opens a Stoolap
  store at that path. Dropping the store closes the file; the second `open()`
  re-reads the same file.
- **Postgres** — the factory owns a `DATABASE_URL` plus a unique per-test schema
  (or table prefix); `open()` builds a fresh `PgPool` against those same tables.
  Dropping the store closes that pool; the second `open()` is a second pool over
  the same rows — a genuine second "process."

`run_durable_conformance` asserts: an `accept`ed effect's state survives the
drop/reopen; an effect left `InFlight` at the drop becomes eligible for
redispatch after reopen (via `recover_in_flight`/`claim_due`'s expired-lease
path); scoped dedup reservations survive the reopen (a same-key re-attempt sees
`Owned*`/`Other*`, never `Fresh`). It runs against Stoolap and Postgres **only**.

`InMemoryEffectStore` deliberately implements **no** `DurableStoreFactory` (a
fresh in-memory instance shares no backing with a dropped one — there is nothing
to reopen). Its honest non-durability is instead pinned by an explicit
**negative** test: accept an effect, drop the store, construct a new
`InMemoryEffectStore`, assert the effect is **absent**. This makes the
documented loss behavior a *passing* assertion rather than a silent omission.

**Tier 3 — Multi-node conformance (Postgres only).** The cross-process
claim-exclusivity criteria (two independent claimers never hold overlapping
*valid* claims; redispatch becomes possible only once a lease expires, covered
by idempotency) need two independently-owned live store instances sharing the
*same* backing storage — a single `&impl EffectStateStore` reference (Tier 1's
signature) cannot represent two claimers any more than it could represent a
restart (Tier 2's reason for needing `DurableStoreFactory`). This tier reuses
`DurableStoreFactory` rather than inventing a second factory trait: where Tier 2
calls `open()`, drops the result, then calls `open()` again (sequential —
models a restart), Tier 3 calls `open()` **twice without dropping either
result** (concurrent — models two live nodes). Because `PostgresEffectStore`
mints a fresh `worker_id: Uuid` at construction (§3.1), each `open()` call
already yields an independently-owned claimer against the same tables with no
further mechanism needed:

```rust
// crates/effect-store/tests/ — reuses DurableStoreFactory (Tier 2), no new trait.
async fn run_multi_node_conformance(factory: &impl DurableStoreFactory) {
    let node_a = factory.open().await; // fresh worker_id A, same backing tables
    let node_b = factory.open().await; // fresh worker_id B, same backing tables — both live at once
    // assert: overlapping claim_due calls from A and B never both claim the
    // same due effect while either's lease is valid; once one's lease expires,
    // the other may claim/redispatch it, and duplicate execution is expected,
    // not prevented (per the corrected spec's Claim Ownership requirement).
}
```

Run only where `capabilities().multi_node_safe` is true — the descriptor
itself selects which assertions apply, so Stoolap (whose `DurableStoreFactory`
impl exists for Tier 2 but which declares `multi_node_safe: false`) is never
asked to prove a guarantee it explicitly declares it does not offer, and
`run_multi_node_conformance` is simply never invoked against
`StoolapDurableStoreFactory`.

**G11 reconciliation (post-freeze harness consolidation).** The Tier 1
bullet above, written at Phase 5, described `PostgresEffectStore` as
"env-gated on a real `DATABASE_URL`; skipped with a logged notice when
absent" — accurate for how Phase 5 first landed it, but superseded before
this change closed: those real-Postgres tests (Tier 1's Postgres row, and
the whole of Tier 2/3) never lived in a root-workspace crate at all by the
time this reconciliation happened. They lived in a separate,
non-root-member `crates/integration-tests` crate (one `testcontainers`
container per test file), which PROD-012 subsequently replaced everywhere
else in the repository with a single top-level `integration-tests/`
workspace — one shared container per run, one template database migrated
once, one isolated database per test. PROD-002's two files were the one
place that replacement had not yet reached.

*Resolution:* both files (`effect_store_postgres_unit.rs`,
`effect_store_postgres_conformance.rs`) moved into
`integration-tests/tests/infrastructure/`, using that harness's
`isolated_database()` fixture in place of their own `testcontainers` calls.
No migration wiring was needed to make this work: `PostgresEffectStore::
connect` (AD-10) already creates and migrates its own schema on every call,
independently of whatever the harness's template pre-migrates into `public`
for `ego-persistence`'s own tables — the two crates' tables coexist in the
same physical database under different schemas, with no shared version
ledger. `cargo test --workspace` at the root was never involved either way:
it does not build this workspace before G11 and still does not after. The
only thing that changed is which harness the real-Postgres run uses, and
that harness happens to be the one PROD-012 built for its own tests.

### 3.7 Runtime-owned retention capability (AD-9 rewrite, G12)

**G12 reconciliation (post-freeze runtime-owned-capability rewrite).** AD-9,
as written at Phase 4/5, described retention as "a low-frequency retention
task each durable provider owns" — accurate for what Phases 4 and 5 actually
shipped (`PostgresEffectStore::run_retention`/`StoolapEffectStore::
run_retention`, each fully implemented and conformance-tested), but never
wired to run in production: nothing ever called either method outside a
test. A prior architectural audit flagged the *shape* of that phrase itself
as the defect worth fixing before wiring it up — "provider-owned internal
maintenance" reads as license for each provider to also own a background
task/scheduler, which is exactly the shape PROD-012's reservation-retention
worker (`ego-service-sdk::runtime::retention`) had already rejected for the
identical problem one subsystem over: a runtime-level `Notify`-based
cancellation loop, bounded shutdown, an `AtomicBool` double-start guard, and
an explicit two-phase construct-then-start API, all owned by `Runtime`, none
of it duplicated per provider.

*Resolution:* the SQL stays exactly where AD-9 always put it —
`run_retention`'s two-table CTE+DELETE per provider is unchanged, still the
only place either provider's retention SQL is written. What moved is the
*schedule*. A new optional capability trait,
[`RetentionMaintenance`](../../../crates/runtime/src/effects/store.rs) (two
methods — `purge_before(cutoff, batch)`, `oldest_terminal()` defaulting to
`Ok(None)`), lives alongside `EffectStateStore`/`EffectDedupStore` in
`crates/runtime/src/effects/store.rs`, the same "optional capability, not a
mandatory port method" shape `EffectStoreCapabilities` (AD-3) already
established. `PostgresEffectStore` and `StoolapEffectStore` each implement it
by calling straight through to their existing `run_retention` — `purge_before`
supplies a zero `ttl` against an already-computed `cutoff`, so no SQL is
duplicated or reimplemented.

A new runtime-owned worker, `ego-service-sdk::runtime::effect_retention::
EffectRetentionWorker`, is the sibling — not a replacement — of PROD-012's
`RetentionWorker`: same lifecycle shape (`Notify` cooperative cancellation,
abort-then-await bounded shutdown, panic isolation via the exact same
`isolate_panics` helper, an `EffectRetentionPolicy` with no `Default` impl,
mirroring `RetentionPolicy`'s "off unless asked for" posture and its
`ZeroRetention`/`ZeroInterval`/`ZeroBatch` validation). It is deliberately a
second, independent worker type rather than a generalization of
`RetentionWorker` over both subsystems: the two purge different rows through
different capability traits, and a runtime may configure either, both, or
neither. `RuntimeBuilder` gained `with_effect_retention_store(Arc<dyn
RetentionMaintenance>)`, `with_effect_retention_policy(...)`, and
`with_effect_retention_clock(...)` — the same "register one concrete
instance under an additional trait object" idiom the effects acceptor's
dual `EffectStateStore`/`EffectDedupStore` registration and the
reservation/`RetentionPolicy` pairing already use — plus a `build()`-time
guard refusing a configured policy with no registered store, mirroring the
existing reservation-retention guard's shape and wording. The entry point on
`Runtime` is `start_retention_effects()`, not `start_retention()`: that name
already belongs to PROD-012's operation-reservation retention, a distinct
subsystem this runtime configures independently, so the two coexist under
two names rather than one overloaded one.

**What did not change:** `EffectStateStore`/`EffectDedupStore`'s trait
signatures (byte-identical); `run_retention`'s SQL in either provider;
PROD-012's `RetentionWorker`/`RetentionPolicy`/`start_retention()`; G10's
Clock-injection work; G11's harness relocation; G15's causal dedup-release
gate. `RuntimeBuilder` still constructs `InMemoryEffectStore` internally
when an executor is registered and still has no seam to register a *custom*
`EffectStateStore`/`EffectDedupStore` (tasks.md Phase 7, still open,
separate from this change) — `with_effect_retention_store` registers the
`RetentionMaintenance` capability independently of that still-missing
wiring, the same way a caller who runs a durable store outside this
builder's construction path can register its `OperationReservationStore`
side today.

## 4. Data flow (durable path)

```
command commit ─▶ RuntimeEffectAcceptor.accept()
                    └─ EffectStateStore.accept()  → INSERT effect_state (Pending)   [durable]
                    └─ (dedup reserve happens at dispatch, attempt 0)

DeliveryRunner periodic tick:
  claim_due(now, limit)  ─▶ SKIP LOCKED claim of unclaimed/expired + stamp owner/lease [exclusive while leased]
  per claimed effect:
    mark_in_flight(id)   ─▶ conditional UPDATE (owner+lease valid)                [ownership-guarded]
    reserve(scope, id, fp) at attempt 0 ─▶ ON CONFLICT DO NOTHING + classify      [durable dedup]
    executor.execute(...)
      Success  ─▶ commit_success(scope); mark_succeeded(id)                       [in-place succeeded flip]
      Retry    ─▶ mark_retryable(id, attempt, next_at)                            [durable backoff bookkeeping]
      Terminal ─▶ mark_terminal(id, reason); release(scope)

restart / peer death:
  recover_in_flight(now)  ─▶ reset expired-lease in_flight → claimable  (Postgres: expired only)
  claim_due  ─▶ picks up expired-lease rows on the next tick                      [no new component]

provider-owned retention (AD-9, not a runner/port step):
  DELETE settled effect_state + effect_dedup older than TTL (batched)             [provider-internal, no port verb]
```

## 5. Spec-delta implications (for spec.md, not written here)

design.md flags — it does not author — what the delta spec must reconcile:

- Retire CORE-019's non-goal *"Durable delivery store implementation (Postgres
  outbox) — the ports are shaped to enable one, but none ships in this
  capability."* PROD-002 ships two.
- Reconcile the cross-node-leasing non-goal with the decided scope: cross-node
  leasing **is** delivered for the `MultiNodeSafe` (PostgreSQL) provider (AD-2);
  it is **not** universally mandated (Stoolap declares `MultiNodeSafe: NO`), and
  composing per-node Stoolap durability under an external consensus layer
  remains out of scope (host-application architecture).
- Add durable-behavior requirements: durability across restart, claim ownership
  under concurrency, expired-lease recovery, retry persistence, dedup
  persistence, cleanup/retention.
- "Exactly once" stays banned from the public contract; delivery remains durable
  at-least-once from acceptance onward, dual-write gap narrowed not closed.

## 6. Residual risks / assumptions requiring validation

- **Stoolap dialect fidelity.** The design assumes Stoolap's MVCC/snapshot
  isolation supports the conditional-`UPDATE`-with-`rows_affected` and
  `INSERT … ON CONFLICT DO NOTHING` semantics the ports need. If its SQL
  dialect diverges, the Stoolap provider's transition primitives may need a
  different (still single-owner) shaping — the conformance suite (AD-13) is the
  gate that catches this. No shared SQL-abstraction layer is built over the two
  dialects (Rule of Two: the dialects differ enough that a premature abstraction
  would cost more than the duplicated per-provider SQL); shared code is the
  ports, the capability descriptor, and the conformance harness only.
- **Lease duration tuning (also bounds the §3.1 accepted window).** `lease` and
  the `claim_due` tick interval interact: the lease must comfortably exceed one
  dispatch's worst-case duration, or a slow-but-alive worker gets reclaimed
  mid-attempt. This same margin bounds the deliberately-unfenced same-`worker_id`
  reclaim window (G2/§3.1): as long as the lease exceeds worst-case dispatch, a
  worker does not reclaim its own still-running effect, so no superseded-claim
  transition lands. These are runtime constants (AD-5/AD-7 posture — not
  spec-normative), defaulted conservatively and operator-tunable; validate
  against real executor latencies in the reference app. The AD-14
  `claim_reclaimed_after_expiry` signal confirms whether same-`worker_id`
  reclaims happen at all in practice (a precondition for the window), but
  cannot itself confirm a stale write never lands afterward — that stays an
  inference from lease tuning, not a directly monitored count.
- **`worker_id` uniqueness across restarts.** A per-process UUID minted at
  construction is assumed unique; a restarted process gets a fresh `worker_id`,
  so it cannot masquerade as its dead predecessor — correct by construction, but
  worth an explicit conformance assertion.
- **Cleanup vs. late duplicates.** The TTL must exceed the longest window in
  which a legitimate duplicate submission for a settled idempotency key could
  arrive, or cleanup could delete a dedup row still needed to reject a replay.
  Default is conservative; flagged for operator awareness in the spec delta.
```
