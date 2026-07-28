# Tasks: PROD-002 — Durable External Effect Store

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1100-1400 (1 new lease port + DTOs in `store.rs`, 1 new durable adapter, 1 migration, shared contract test-suite, integration tests) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 lease port + DTOs (`store.rs`) → PR2 shared port contract test-suite → PR3 durable adapter + migration + integration tests |
| Delivery strategy | auto-forecast (not a recognized ask-on-risk/auto-chain/single-pr/exception-ok label — treated conservatively) |
| Chain strategy | feature-branch-chain (PR1→PR2→PR3); only the tracker merged to develop |

Decision needed before apply: Yes (storage tech / port shape resolved in design ADR-1/ADR-2/ADR-3)
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain (PR1→PR2→PR3)
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | `LeasedEffectStore` port + `LeasedEffect`/`LeaseToken` DTOs in `store.rs` (additive) | PR1 | `cargo test -p ego-runtime effects::store::` | N/A — trait + DTOs, no DB | Revert the additive `store.rs`/`mod.rs` lines |
| 2 | Shared port contract test-suite runnable against any impl | PR2 | `cargo test -p ego-runtime effects::store_contract::` | in-memory impl under the shared suite | Delete the shared test module |
| 3 | `PostgresEffectStore` adapter + migration + integration tests | PR3 | `cargo test -p ego-infrastructure effects::durable_store::` | Postgres-backed `#[tokio::test]` | Delete `infrastructure/src/effects/`, the migration, revert `Cargo.toml`/`lib.rs` |

## Phase 1: Lease Port + DTOs (additive, no DB)

- [ ] TASK-001 RED: failing test in `crates/runtime/src/effects/store.rs` proving `LeaseToken` and `LeasedEffect { effect: StoredEffect, lease_token: LeaseToken, leased_until: Timestamp }` exist and `LeasedEffect.effect` reuses the existing `StoredEffect` DTO (assert construction from a `StoredEffect` compiles and fields round-trip). AC: named types are constructible.
- [ ] TASK-002 GREEN: add `LeaseToken`, `LeasedEffect` DTOs to `store.rs` (ADR-2). AC: TASK-001 green; `deny(missing_docs)` satisfied.
- [ ] TASK-003 RED: failing test proving `LeasedEffectStore` is object-safe — build a `Vec<Arc<dyn LeasedEffectStore>>` from a local trivial stub with `atomic_claim`/`renew_lease` — AND that `EffectStateStore` is UNCHANGED: `claim_due`'s signature and non-atomic contract still exist and `InMemoryEffectStore` still implements `EffectStateStore` without implementing `LeasedEffectStore`.
- [ ] TASK-004 GREEN: add the `#[async_trait] LeasedEffectStore { async fn atomic_claim(&self, now, limit, lease) -> Result<Vec<LeasedEffect>, EffectStoreError>; async fn renew_lease(&self, id, token, now, lease) -> Result<(), EffectStoreError>; }` port as a SEPARATE trait (ADR-2); do NOT modify `EffectStateStore` or `claim_due`. AC: TASK-003 green; existing `store.rs` tests pass unchanged.
- [ ] TASK-005: `pub use` `LeasedEffectStore`, `LeasedEffect`, `LeaseToken` from `crates/runtime/src/effects/mod.rs`. AC: importable via `ego_runtime::effects::{...}`; `cargo build -p ego-runtime` succeeds.

## Phase 2: Shared Port Contract Test-Suite

- [ ] TASK-006 RED: add a reusable contract test module `effects::store_contract` parameterized over a store factory, asserting the `EffectStateStore`/`EffectDedupStore` scenarios (accept/mark_*/claim_due/recover_in_flight, reserve/commit_success/release, tenant isolation). Run it against `InMemoryEffectStore` first — it MUST pass, proving the suite matches shipped behavior. AC: suite compiles and passes for the in-memory store.
- [ ] TASK-007 GREEN: factor the shared suite so a second implementation can be plugged in by supplying an `async` store factory; no behavior change to `InMemoryEffectStore`. AC: TASK-006 green; in-memory tests unmodified in behavior.

## Phase 3: Schema + Migration

- [ ] TASK-008 RED: failing `#[tokio::test]` (Postgres-backed, `#[ignore]`-gated by env if no DB) asserting the migration creates `effect_state` (columns for id, tenant, effect_type, description/json, state, attempt, next_at, leased_until, lease_token) and `effect_dedup` (tenant, effect_type, key, fingerprint, owner effect_id, succeeded) with a UNIQUE constraint on `(tenant, effect_type, key)` and indexes on `(state, next_at)` and `(state, leased_until)`. AC: schema introspection matches.
- [ ] TASK-009 GREEN: add `crates/infrastructure/migrations/NNNN_effect_outbox.sql` and apply via `sqlx` migrate in test setup. AC: TASK-008 green.

## Phase 4: Durable State Store — Persistence + Transactions

- [ ] TASK-010 RED: failing `#[tokio::test]` — `PostgresEffectStore::accept` then reopen a fresh store instance against the same DB and confirm the effect is still `Pending` (durability across "restart"); a `Succeeded` effect is NOT returned by the due query after reopen. Covers spec "Durable Effect State Survives Crash and Restart".
- [ ] TASK-011 GREEN: implement `PostgresEffectStore` impl `EffectStateStore` (accept/mark_in_flight/mark_succeeded/mark_retryable/mark_terminal/claim_due/recover_in_flight) over `sqlx`, each transition in one transaction (ADR-4). AC: TASK-010 green.
- [ ] TASK-012 RED: failing `#[tokio::test]` — a transition forced to roll back leaves NO partial state row change and NO partial dedup write; a `mark_succeeded`+`commit_success` pair is committed atomically together (ADR-4). Covers "State Transitions Are Transactional With a Documented Boundary".
- [ ] TASK-013 GREEN: implement the shared-transaction path for the succeed+commit_success pair and confirm rollback atomicity. AC: TASK-012 green.
- [ ] TASK-014 RED: failing test — SQLSTATE-class mapping: serialization-failure/deadlock/pool-timeout ⇒ `EffectStoreError::TemporarilyUnavailable`; a permanent fault ⇒ `Backend`; a dedup unique violation ⇒ `Conflict`. Covers the transactional requirement's transient/permanent scenarios.
- [ ] TASK-015 GREEN: implement the SQLSTATE → `EffectStoreError` mapping (reusing the existing variants in `store.rs:106-135`; no new variants). AC: TASK-014 green.

## Phase 5: Atomic Claim + Lease

- [ ] TASK-016 RED: failing concurrent `#[tokio::test]` — two `atomic_claim` calls over the same due set return DISJOINT `LeasedEffect` sets (no effect in both); each claimed effect is transitioned to `InFlight` with a `leased_until` in the future. Covers "Atomic Claim With Lease Prevents Concurrent Double Delivery".
- [ ] TASK-017 GREEN: implement `atomic_claim` as `SELECT … FOR UPDATE SKIP LOCKED LIMIT limit` over `Pending` ∪ due `RetryableFailed` ∪ expired-lease `InFlight`, transitioning claimed rows to `InFlight` and stamping `leased_until = now + lease` and a fresh `lease_token`, all in one transaction (ADR-2/ADR-3/ADR-5). AC: TASK-016 green.
- [ ] TASK-018 RED: failing `#[tokio::test]` — a leased effect with a live lease is NOT returned to a second consumer's `atomic_claim`; the existing non-atomic `claim_due` on `InMemoryEffectStore` is unaffected (compat). Covers "A leased effect is invisible" + "Existing non-atomic claim_due is preserved".
- [ ] TASK-019 GREEN: ensure the claim query excludes live-lease `InFlight` rows; add a compat assertion that `InMemoryEffectStore` still exposes `claim_due` unchanged. AC: TASK-018 green.

## Phase 6: Lease Renewal + Expiry Reclaim + Crash Recovery

- [ ] TASK-020 RED: failing timed `#[tokio::test]` — `renew_lease` extends `leased_until` and keeps the effect invisible to peers; an un-renewed lease expires and the effect becomes claimable again with its record preserved. Covers "Lease Renewal and Expiry Reclaim" (positive scenarios).
- [ ] TASK-021 GREEN: implement `renew_lease` (UPDATE `leased_until = now + lease` WHERE `lease_token` matches and lease still live). AC: TASK-020 green.
- [ ] TASK-022 RED: failing `#[tokio::test]` — renewing an ALREADY-EXPIRED lease whose effect was reclaimed by another consumer returns a distinguishable non-success (`Conflict`), never silent `Ok`. Covers the negative renewal scenario.
- [ ] TASK-023 GREEN: implement expired-lease renewal failure (row no longer matches the token/lease predicate ⇒ `Conflict`). AC: TASK-022 green.
- [ ] TASK-024 RED: failing `#[tokio::test]` — durable crash recovery reclaims ONLY expired-lease `InFlight` rows and leaves live-lease `InFlight` rows to their consumer; every reclaimed record is preserved. Covers "Durable Crash Recovery Reclaims Only Expired-Lease In-Flight Effects".
- [ ] TASK-025 GREEN: implement durable `recover_in_flight` as expired-lease-only reclaim (ADR-5); keep `InMemoryEffectStore`'s blanket recovery unchanged. AC: TASK-024 green; in-memory recovery test still passes.

## Phase 7: Durable Dedup Persistence

- [ ] TASK-026 RED: failing `#[tokio::test]` — `PostgresEffectStore` impl `EffectDedupStore`: a committed `OwnedSucceeded` reservation, read back after reopening a fresh store instance, still returns `OwnedSucceeded` (not `Fresh`); a different fingerprint under the same scope ⇒ `Conflict`; two tenants with identical `effect_type`+key never collide. Covers "Durable Dedup Reservations Persist With Unchanged Scope".
- [ ] TASK-027 GREEN: implement `EffectDedupStore` over the `effect_dedup` table with a UNIQUE `(tenant, effect_type, key)` constraint and the ownership/status outcomes (`Fresh`/`OwnedInProgress`/`OwnedSucceeded`/`OtherInProgress`/`OtherSucceeded`/`Conflict`). AC: TASK-026 green.

## Phase 8: Guarantee Invariants — At-Least-Once, No Exactly-Once, Unordered

- [ ] TASK-028 RED: failing `#[tokio::test]` — an accepted, undelivered effect present before a simulated crash is attempted at least once after reopen; a consumer that crashed after dispatch but before recording success has its effect reclaimed on lease expiry and redelivered (destination may see it more than once). Covers "The Durable Store Preserves At-Least-Once…".
- [ ] TASK-029 GREEN: wire redelivery-after-reclaim through the atomic-claim path so at-least-once holds across crash. AC: TASK-028 green.
- [ ] TASK-030: static source-scan test asserting the string `exactly once` (case-insensitive) appears NOWHERE in `crates/infrastructure/src/effects/` or `crates/runtime/src/effects/store.rs` code/docs. AC: scan clean.
- [ ] TASK-031 RED: failing `#[tokio::test]` — two effects accepted in a known order may be delivered in a different order under concurrent consumers / `next_at` reschedule; the test asserts the store makes NO ordering promise (does not assert a specific order, asserts the absence of a FIFO guarantee via a reordering case). Covers "Durable Delivery Ordering Is Not Guaranteed".
- [ ] TASK-032 GREEN: confirm no code path imposes acceptance-order delivery; document the unordered contract in the adapter docs. AC: TASK-031 green.

## Phase 9: Observability — Bounded Cardinality

- [ ] TASK-033 RED: failing test asserting durable-store metrics use only the closed label set (`effect_type`, state/outcome, consumer role) and that no metric label carries `effect_id`, idempotency key, `tenant` id, `destination`, or `payload`. Covers ADR-6 / bounded-cardinality frozen decision.
- [ ] TASK-034 GREEN: emit claim-batch / lease-renewal / lease-expiry-reclaim / txn-retry signals with bounded labels only; keep redacted/hashed key and no `payload` in logs. AC: TASK-033 green.

## Phase 10: Hexagonal Boundary + Cross-Cutting Verification

- [ ] TASK-035: static source-scan test asserting no `sqlx`/DB type appears in any port signature in `crates/runtime/src/effects/` or `crates/domain/` (the durable adapter is the sole `sqlx` consumer, confined to `crates/infrastructure`). AC: scan clean.
- [ ] TASK-036: confirm acyclic layering — `ego-infrastructure` depends on `ego-runtime` for the ports; `ego-runtime` gains no dependency on `ego-infrastructure` (inspect `Cargo.toml`). AC: dependency direction confirmed, no new cyclic edge.
- [ ] TASK-037: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions; `InMemoryEffectStore` reference tests pass unmodified.
