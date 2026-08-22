# Tasks: PROD-002 — Durable External Effect Store

Strict TDD is active (`cargo test --workspace`). Every phase writes the RED
test(s) first, then the minimal GREEN implementation. Order follows the
dependency edge `ego-runtime` (capability descriptor, AD-3) → `ego-effect-store`
(providers, AD-1) → `service-sdk`/`reference-app` (wiring). The conformance
suite (AD-13) is a **three-tier** suite — Tier 1 port conformance, Tier 2
durable-provider (real close→reopen) conformance, Tier 3 multi-node
conformance — built and proven against Stoolap **before** Postgres: design.md
§6 names Stoolap SQL-dialect fidelity as the residual risk Tier 2 gates, so
Tier 2 for Stoolap must land in the same phase as the Stoolap provider, not be
deferred to a final catch-all phase.

Each task cites the design.md AD(s)/§ and spec.md requirement it satisfies.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~2050-2250 (~2150 midpoint) — new crate, two full providers, migrations, TestKit double with three distinct crash ops, three-tier conformance suite incl. `DurableStoreFactory` + two factory impls, wiring |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 → PR2 → PR3 → PR4 → PR5 (see Work Units) |
| Delivery strategy | ask-on-risk |
| Chain strategy | `feature-branch-chain` (confirmed) — 5 sequential PRs sharing the new `ego-effect-store` crate; PR1 targets a tracker branch, each child targets the immediately previous PR branch, only the tracker merges to main |

Decision needed before apply: No — chain strategy confirmed
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain
400-line budget risk: High

### Estimated Lines By File/Task Group

| File/area | Task group | Est. lines |
|---|---|---|
| `crates/effect-store/{Cargo.toml,src/lib.rs}` | Phase 0 | ~40 |
| `crates/runtime/src/effects/store.rs` (capabilities) | Phase 1 | ~50 |
| `crates/runtime/src/effects/observability.rs` | Phase 2 | ~120 |
| `crates/effect-store/tests/conformance.rs` (Tier 1 harness + Tier 2/3 scaffolding) | Phase 3 | ~300 |
| `crates/effect-store/src/stoolap/mod.rs` + tests + `StoolapDurableStoreFactory` (Tier 2) | Phase 4 | ~480 |
| `crates/effect-store/src/postgres/{mod.rs,migrations.rs,migrations/*.sql}` + tests + `PostgresDurableStoreFactory` (Tier 2) + Tier 3 run | Phase 5 | ~700 |
| `crates/testkit/src/effects.rs` (three distinct crash ops) | Phase 6 | ~280 |
| `crates/service-sdk/src/runtime/builder.rs` | Phase 7 | ~70 |
| `examples/reference-app` | Phase 8 | ~100 |
| `ARCHITECTURE.md`, spec/proposal finalization | Phase 9 | ~30 |
| **Total** | | **~2170** |

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Crate scaffold + capability descriptor (AD-3) + observability extension (AD-14) — Phases 0-2 | PR1 | `cargo test -p ego-runtime effects::store:: effects::observability::` | N/A — no runnable behavior yet, pure additive types/signals | Delete `crates/effect-store/`, revert `capabilities()` on both ports, revert 4 new `log_*` fns |
| 2 | Tier 1 conformance harness + Stoolap provider + Tier 2 durable-provider conformance (`DurableStoreFactory`, Stoolap reopen) — Phases 3-4 (dialect-fidelity gate) | PR2 | `cargo test -p ego-effect-store --features stoolap` | N/A — no host wiring yet; harness proven against `InMemoryEffectStore` + `StoolapEffectStore` directly | Delete `conformance.rs` + `src/stoolap/`; PR1 untouched |
| 3 | Postgres provider + migrations + Tier 2/3 conformance — Phase 5 | PR3 | `cargo test -p ego-effect-store --features postgres` (env-gated `DATABASE_URL`, skips cleanly otherwise) | N/A — provider-level only | Delete `src/postgres/`; PR1/PR2 unaffected |
| 4 | TestKit fault-injection double, three distinct crash ops — Phase 6 | PR4 | `cargo test -p ego-testkit effects::` | N/A — test-double crate only | Revert `FaultInjectingEffectStore` addition |
| 5 | service-sdk wiring + reference-app dogfood + docs — Phases 7-9 | PR5 | `cargo test -p ego-service-sdk runtime::builder::` | `cargo test -p reference-app` (real `StoolapEffectStore`-backed build) | Revert builder.rs registration/logging, reference-app registration, `ARCHITECTURE.md` node |

## Phase 0: Crate Scaffolding (AD-1)

- [x] 0.1 Add `crates/effect-store` to root workspace `Cargo.toml` members.
- [x] 0.2 Create `crates/effect-store/Cargo.toml`: `ego-effect-store`, dep `ego-runtime`; optional `sqlx = "0.8"` (feature `postgres`), Stoolap driver (feature `stoolap`); `async-trait`, `chrono`, `uuid`, `thiserror`. No default backend feature.
- [x] 0.3 Create `crates/effect-store/src/lib.rs` — crate root, module doc naming `EffectStoreCapabilities`-based provider docs, feature-gated `mod postgres;`/`mod stoolap;`.
- [x] 0.4 RED (build gate) — `cargo check -p ego-effect-store --no-default-features`, `--features postgres`, `--features stoolap` each succeed independently; no cross-feature leak.

## Phase 1: Capability Descriptor on Shared Ports (AD-3, G6)

Spec: "A provider declares its capabilities honestly."

- [x] 1.1 RED — `crates/runtime/src/effects/store.rs`: test `in_memory_declares_all_false_capabilities_by_default` for both `EffectStateStore::capabilities()` and `EffectDedupStore::capabilities()`.
- [x] 1.2 GREEN — add `EffectStoreCapabilities { durable, concurrent_local_safe, multi_node_safe, supports_leases }` (Debug, Clone, Copy, PartialEq, Eq) + defaulted `fn capabilities(&self) -> EffectStoreCapabilities` (all-false default) on both traits (design §3.2 exact shape).
- [x] 1.3 GREEN — confirm existing `InMemoryEffectStore` and any third-party impl still compile unchanged (defaulted method, no signature break).

## Phase 2: Observability Extension (AD-14)

Spec: signals must exist for claim/lease/recovery/cleanup, same redaction discipline (no payload, hashed idempotency key).

- [x] 2.1 RED — `crates/runtime/src/effects/observability.rs`: test `claim_acquired` emits `effect_id`, `owner`, `expires_at` (in-file `tracing::Subscriber` double, mirrors existing `log_*` tests).
- [x] 2.2 RED — test `claim_reclaimed_after_expiry` emits `effect_id, previous_owner, new_owner, previous_epoch, new_epoch`.
- [x] 2.3 RED — test `recovered_in_flight` emits recovered count/scope.
- [x] 2.4 RED — test `cleanup_deleted` emits deleted-row count + table name.
- [x] 2.5 GREEN — implement `log_claim_acquired`, `log_claim_reclaimed_after_expiry`, `log_recovered_in_flight`, `log_cleanup_deleted`.

## Phase 3: Tier 1 — Port Conformance, Proven Against InMemoryEffectStore First (AD-13)

Spec: "Both day-zero providers pass the same durability criteria"; "A provider
declares its capabilities honestly."

- [x] 3.1 GREEN — create `crates/effect-store/tests/conformance.rs`: `run_state_store_conformance(store: &impl EffectStateStore)` / `run_dedup_conformance(store: &impl EffectDedupStore)` — **Tier 1** (AD-13, design §3.6) port-conformance harness covering only what's provable on ONE live instance: CORE-019 transition scenarios, `DedupOutcome` classification (all six variants, fingerprint-mismatch → `Conflict`), retry bookkeeping *shape* (`mark_retryable` resumes `attempt`, never resets), `rows_affected` atomicity (`InvalidTransition` vs `Conflict`), both ports satisfied independently. Deliberately contains **no restart-survival assertion** — design §3.6: restart survival cannot be a shared Tier 1 assertion since `InMemoryEffectStore` is contractually required to lose state on crash.
- [x] 3.2 RED/GREEN — invoke both harness fns against `InMemoryEffectStore` — proves the Tier 1 harness itself before any durable provider exists, since it needs no durable backend.
- [x] 3.3 GREEN — define `run_multi_node_conformance(factory: &impl DurableStoreFactory)` (**Tier 3**, design §3.6): reuses Tier 2's `DurableStoreFactory` by calling `factory.open()` **twice without dropping either result** (concurrent, not sequential) to obtain two independently-owned live claimers against the same backing storage — no new factory trait, since each `open()` already yields a fresh `worker_id` (§3.1). Asserts cross-process claim exclusivity: two claimers never hold overlapping *valid* claims on the same effect; once one's lease expires, the other may claim/redispatch it, and duplicate execution is expected (idempotency-covered), not prevented. Gated on `capabilities().multi_node_safe` — a no-op when the factory's store declares `multi_node_safe: false` (i.e. never meaningfully invoked against `StoolapDurableStoreFactory`); exercised for real only once `PostgresDurableStoreFactory` exists (Phase 5).
- [x] 3.4 GREEN — **Tier 2 negative test** (AD-13, design §3.6): accept an effect into an `InMemoryEffectStore`, drop it, construct a **new** `InMemoryEffectStore`, assert the effect is **absent**. A passing assertion of documented non-durable behavior (spec: Delivery Guarantee — "the shipped in-memory store, the guarantee MUST be documented as degrading to at-most-once across a crash"), not an omission. `InMemoryEffectStore` deliberately implements no `DurableStoreFactory` (design §3.6).

## Phase 4: Stoolap Provider + Tier 2 Durable-Provider Conformance (AD-1, AD-2 local path, AD-5, AD-8, AD-9, AD-13 Tier 2)

Design §6: this phase is the SQL-dialect-fidelity gate — sequenced before
Postgres, now proven by the real Tier 2 close→reopen test, not Tier 1 alone.

- [x] 4.1 RED — extend `conformance.rs` (feature `stoolap`) to invoke Phase 3's Tier 1 harness fns against a `StoolapEffectStore` — fails until 4.2 lands.
- [x] 4.2 GREEN — create `crates/effect-store/src/stoolap/mod.rs`: `StoolapEffectStore` implementing both ports — plain conditional `UPDATE … WHERE state IN (...)` under MVCC/snapshot isolation, no owner/epoch/lease columns (Stoolap has no `ANY($array)` operator — adapted to `IN (...)`, a dialect shaping difference, not a semantic one); `reserve` = `INSERT … ON CONFLICT DO NOTHING` + classify (mirrors `InMemoryEffectStore::reserve`); `capabilities()` = `{durable: true, concurrent_local_safe: true, multi_node_safe: false, supports_leases: false}`. Dialect finding: Stoolap only supports a single-column `INTEGER PRIMARY KEY`; a `TEXT` PK (needed for a UUID `effect_id`) is rejected at DDL time, and a table-level composite `PRIMARY KEY (...)` parses but is silently unenforced (confirmed by reading `executor/ddl.rs`) — uniqueness is expressed via `UNIQUE (...)` instead (fully enforced, single- and multi-column, and what `ON CONFLICT` matches against).
- [x] 4.3 RED — dedup crash-mid-reservation test: atomic upsert leaves no partial state; `commit_success` flips `succeeded` in place, never deletes (AD-8).
- [x] 4.4 GREEN — provider-owned TTL retention task (AD-9): batched delete of settled rows, emits `cleanup_deleted` (Phase 2). Dialect finding: `DELETE … WHERE col IN (SELECT … LIMIT n)` — the natural batched-delete shape — silently deletes **zero** rows against Stoolap 0.4.0 even with a literal, unparameterized subquery (confirmed by direct experiment; not a param-binding issue); `DELETE … WHERE col IN (<value list>)` and `DELETE … WHERE col = $1` both work correctly, so retention selects the bounded batch of eligible identifiers first, then deletes each individually.
- [x] 4.5 Run `cargo test -p ego-effect-store --features stoolap` — Tier 1 conformance suite green against `StoolapEffectStore`.
- [x] 4.6 RED — define the test-only `DurableStoreFactory` trait (`crates/effect-store/tests/`, design §3.6: `async fn open(&self) -> Self::Store`, **never** added to the production ports in `crates/runtime/src/effects/store.rs`) plus `StoolapDurableStoreFactory` (owns a `tempfile::TempDir`; `open()` reopens a `StoolapEffectStore` at that fixed path) — `run_durable_conformance` fails until 4.7 lands. (Trait declared at 3.3 since `run_multi_node_conformance`'s signature already needed it; `StoolapDurableStoreFactory` + `run_durable_conformance`'s body land here.)
- [x] 4.7 GREEN — implement `run_durable_conformance(factory: &impl DurableStoreFactory)` (AD-13 Tier 2): accept an effect, drop the store, reopen via the factory, assert the effect survives; an effect left `InFlight` at drop is redispatch-eligible after reopen (via `recover_in_flight`/`claim_due`'s expired-lease path); a scoped dedup reservation survives reopen (`Owned*`/`Other*`, never `Fresh`).
- [x] 4.8 Run `cargo test -p ego-effect-store --features stoolap` — Tier 2 `run_durable_conformance` green against `StoolapDurableStoreFactory` (design §6 dialect-fidelity gate satisfied by a real close→reopen proof, not Tier 1 alone).

## Phase 5: PostgreSQL Provider + Tier 2/3 Conformance (AD-2, AD-4, AD-5, AD-6, AD-8, AD-9, AD-10, AD-11, AD-13)

- [x] 5.1 GREEN — `crates/effect-store/src/postgres/migrations/001_effect_state.sql`, `002_effect_dedup.sql` (own sequence starting `001`, AD-10): `effect_state` with `claim_owner UUID NULL`, `claim_expires_at TIMESTAMPTZ NULL`, `claim_epoch BIGINT NOT NULL DEFAULT 0`; `effect_dedup` PK `(tenant_id, effect_type, idempotency_key)`.
- [x] 5.2 GREEN — `crates/effect-store/src/postgres/migrations.rs`: hand-rolled `include_str!` runner mirroring `ego-persistence`'s pattern (uses `sqlx::raw_sql`, not `sqlx::query`, since each migration file holds more than one `;`-terminated statement — confirmed against real PostgreSQL that `sqlx::query` rejects multi-statement text).
- [x] 5.3 RED — `claim_due` G1 guard test: a second `claim_due` call never re-stamps a row already carrying a live claim (`claim_owner IS NULL OR claim_expires_at < now`). RED→GREEN verified against a real PostgreSQL instance (Docker).
- [x] 5.4 RED — `claim_due` picks up expired-lease `in_flight` rows (AD-4) alongside due `pending`/`retryable_failed`, in one `FOR UPDATE SKIP LOCKED` transaction, without transitioning `state`. RED→GREEN verified against real PostgreSQL.
- [x] 5.5 GREEN — implement `PostgresEffectStore::claim_due` per design §3.1 SQL (plus a NULL-safe `next_at` check, matching accept()'s NULL initial value).
- [x] 5.6 RED — ownership-guarded `mark_*`: a superseded worker's conditional `UPDATE` affects 0 rows → `EffectStoreError::Conflict`; a live worker's transition applies, `rows_affected == 1`. RED→GREEN verified against real PostgreSQL with two live `PostgresEffectStore` instances.
- [x] 5.7 GREEN — implement `mark_in_flight`/`mark_succeeded`/`mark_retryable`/`mark_terminal` (AD-5 conditional-UPDATE classification). `mark_in_flight` uses a self-claiming guard (`claim_owner IS NULL OR claim_owner = $worker_id`) so the shared Tier 1 harness's direct `accept`→`mark_in_flight` call (no `claim_due` in between) still works; `mark_succeeded`/`mark_retryable`/`mark_terminal` use design.md's literal strict guard. `mark_retryable` additionally clears `claim_owner`/`claim_expires_at` on success — discovered necessary during real-Postgres testing (a stale, still-valid lease from the failed attempt would otherwise block the row from being reclaimed on its next `next_at` tick, including by its own worker).
- [x] 5.8 RED — `recover_in_flight(now)` on Postgres scoped to expired-lease rows only — never resets a live peer's in-flight row (AD-4). RED→GREEN verified against real PostgreSQL.
- [x] 5.9 GREEN — implement `recover_in_flight`; `worker_id: Uuid` minted fresh per construction (residual-risk note, design §6).
- [x] 5.10 RED — dedup `reserve`/`commit_success`/`release`: atomic upsert, in-place `succeeded` flip, crash-mid-reservation leaves no partial state (AD-8). RED→GREEN verified against real PostgreSQL.
- [x] 5.11 GREEN — implement `PostgresEffectStore`'s `EffectDedupStore` half.
- [x] 5.12 GREEN — `capabilities()` = `{durable: true, concurrent_local_safe: true, multi_node_safe: true, supports_leases: true}`.
- [x] 5.13 GREEN — provider-owned TTL retention task (AD-9), same shape as 4.4 (single atomic CTE+DELETE per table; the `effect_dedup` delete re-checks `succeeded`/`settled_at < cutoff` at delete time, not just scope-key identity — same TOCTOU discipline as 4.4's fix). RED→GREEN verified against real PostgreSQL, including the TOCTOU-race scenario.
- [x] 5.14 RED — extend `conformance.rs` (feature `postgres`, env-gated on `DATABASE_URL`, skipped with a logged notice when absent) to run Phase 3's Tier 1 harness fns against `PostgresEffectStore`. **Finding**: `run_dedup_conformance` and the capability-independence check pass unmodified against real PostgreSQL; `run_state_store_conformance`'s "claim_due respects limit" sub-assertion is structurally incompatible with G1 (5.3) — it assumes a still-`Pending` row stays repeatably claimable via a second `claim_due` call without an intervening `mark_in_flight`, which is exactly the "rapid repeat" scenario G1 exists to reject (design.md §3.1's own wording). `postgres_satisfies_state_store_conformance` is `#[ignore]`d with a full explanation in `tests/conformance.rs`, not silently deleted — flagged for the maintainer at verify/archive time, same posture as the accepted G2 limitation below. G1 was kept correct (not weakened) and Phase 3's shared harness was not modified, per this phase's explicit constraints.
- [x] 5.15 RED — implement `PostgresDurableStoreFactory` (owns a `DATABASE_URL` + a unique per-test schema; `open()` builds a fresh `PgPool` over those same tables — a genuine second "process") and invoke `run_durable_conformance` (AD-13 Tier 2, Phase 4.7's harness fn) against it. Uses a short test-only lease (50ms) plus a deliberate 150ms sleep in `open()` so a prior instance's lease has genuinely elapsed by the time a new instance checks recoverability — deterministic, not a wall-clock race — modeling that real recovery ticks run well after any short in-flight lease, unlike this test's otherwise-immediate reopen.
- [x] 5.16 GREEN — `run_durable_conformance` passes against `PostgresDurableStoreFactory`: an accepted effect survives drop/reopen; an `InFlight`-at-drop effect becomes redispatch-eligible after reopen; a scoped dedup reservation survives reopen. Verified against real PostgreSQL.
- [x] 5.17 RED — extend `conformance.rs` to invoke Phase 3.3's `run_multi_node_conformance` against `PostgresDurableStoreFactory` (from 5.15) — opening two live `PostgresEffectStore` instances (fresh `worker_id` each) over the same tables via two concurrent `factory.open()` calls, neither dropped (spec: "Claim Ownership Is Exclusive While Leased, Not a Double-Dispatch Guarantee").
- [x] 5.18 GREEN — `run_multi_node_conformance` passes against the two live `PostgresEffectStore` instances: two independent claimers never hold overlapping valid claims for as long as a lease is valid; once a lease expires, redispatch is possible and covered by idempotency, not prevented by claim exclusivity. Verified against real PostgreSQL.
- [x] 5.19 Run `cargo test -p ego-effect-store --features postgres` (env-gated) — Tier 1 + Tier 2 + Tier 3 conformance suite green (7 passed, 1 deliberately `#[ignore]`d with full rationale — see 5.14). Verified against a real, disposable PostgreSQL 16 instance (Docker), run 3x for stability, plus `cargo test --workspace` (default, no-backend build) — 0 regressions — and `cargo clippy -p ego-effect-store --features postgres[,stoolap] --tests --no-deps -- -D warnings` — clean.

## Phase 6: TestKit Fault-Injection Double (AD-12)

Spec: "Retry is exercised without a real durable backend."

- [ ] 6.1 RED — `crates/testkit/src/effects.rs`: scripted per-op transient-error queue (`fail_calls: HashMap<StoreOp, VecDeque<EffectStoreError>>`) drives the retry path identically to a real transient failure.
- [ ] 6.2 RED — `simulate_process_crash()` destroys all volatile state; a subsequent `recover_in_flight`/`claim_due` sees **nothing** recoverable — models `InMemoryEffectStore`'s documented non-durable loss. Not the operation recovery-logic tests use.
- [ ] 6.3 RED — `simulate_runner_crash()` preserves all backing state but abandons in-flight ops (as if the runner holding them died); a subsequent `recover_in_flight`/`claim_due` **does** see pre-crash in-flight effects as recoverable/reclaimable. This is the operation recovery-logic tests use, not `simulate_process_crash()`.
- [ ] 6.4 RED — claim-race interleave hook: two `claim_due` → `mark_in_flight` sequences never both hold a valid claim; once a lease "expires" (scripted), redispatch is possible.
- [ ] 6.5 RED — `crash_after(op)`: the write for `op` lands but its `Ok` never reaches the caller (ambiguity-window / idempotency-on-retry test), backed by the `FaultPlan::crash_after` field.
- [ ] 6.6 GREEN — implement `FaultInjectingEffectStore` wrapping a real `InMemoryEffectStore`, both ports, `FaultPlan`/`StoreOp` (design §3.5), and all three crash operations — `simulate_process_crash()`, `simulate_runner_crash()`, `crash_after(op)` — no randomness (determinism axiom).
- [ ] 6.7 Add `FaultInjectingEffectStore` as an additional Phase 3 Tier 1 harness subject — proves it stays a genuine `EffectStateStore + EffectDedupStore`.

## Phase 7: service-sdk Registration + Capability Logging (design §3.2, §2 row)

Spec: "A mixed durable/non-durable registration is not silently treated as durable."

- [ ] 7.1 RED — `crates/service-sdk/src/runtime/builder.rs`: registering a durable `EffectStateStore` + a non-durable `EffectDedupStore` logs both capability profiles independently at startup.
- [ ] 7.2 GREEN — register the durable store where `InMemoryEffectStore` registers today; log both ports' `capabilities()` at startup via existing tracing conventions.

## Phase 8: Reference-App Dogfood (Stoolap)

Spec: kill-the-process success criterion.

- [ ] 8.1 RED — reference-app e2e: an accepted effect survives a simulated restart against the embedded `StoolapEffectStore` (new process/store instance over the same on-disk file).
- [ ] 8.2 GREEN — wire `examples/reference-app` to register `StoolapEffectStore` in place of `InMemoryEffectStore` — no external server dependency.
- [ ] 8.3 Run `cargo test -p reference-app` — 0 regressions.

## Phase 9: Docs + Spec Finalization

- [ ] 9.1 Confirm `specs/external-effects/spec.md` (already written) merge-readiness: verify ADDED/MODIFIED requirements match the shipped `capabilities()` shape, claim/lease semantics, cleanup, and TestKit double — do not re-author.
- [ ] 9.2 Update `ARCHITECTURE.md` §2.1: add `ego-effect-store` node, edges `ego-effect-store → ego-runtime` and `ego-effect-store → {sqlx | stoolap}`; confirm no new edge into `ego-persistence`.
- [ ] 9.3 Update `proposal.md` Success Criteria checkboxes once demonstrably true.
- [ ] 9.4 Run `cargo test --workspace` (default no-backend-feature build included) — 0 failures.

## Phase 10: G15 — Causal Gating of Destructive Dedup Release (post-audit fix)

Spec: preserve AD-8's "a different later submission still finds
`OtherSucceeded`, never `Fresh`" invariant across a superseded abandonment.
Scope: `DeliveryRunner::abandon_and_release` only. Explicitly does not touch
`EffectDedupStore`'s trait signature, schema, AD-6, or the accepted
same-`worker_id` G2 window's semantics.

- [x] 10.1 RED — regression test proves a stale terminal abandonment can
  delete a dedup reservation another attempt already succeeded
  (`crates/runtime/src/effects/runner.rs`,
  `stale_terminal_abandonment_does_not_release_a_dedup_reservation_another_attempt_already_succeeded`).
  Confirmed failing for the correct reason: expected `OtherSucceeded`, got
  `Fresh`. Finding: the in-memory double never produces `Conflict` from
  `mark_terminal` (only `accept()` does) — the same bug is exercised via the
  naturally-occurring `InvalidTransition` it does produce, confirming the fix
  must gate on *any* `mark_terminal` error, not `Conflict` specifically.
- [x] 10.2 GREEN — change `DeliveryRunner::abandon_and_release` so
  `dedup.release()` runs only when the preceding `mark_terminal()` call
  returns `Ok`.
- [x] 10.3 Regression — re-run 10.1's test, confirm green. `cargo test
  --package ego-runtime stale_terminal_abandonment`: 1 passed. Full
  `cargo test --package ego-runtime effects::`: 126 passed, 0 failed —
  no regressions.
- [x] 10.4 Scope guard — confirmed: no change to `EffectDedupStore`'s trait
  signature, no schema change, no change to AD-6, no change to the
  same-`worker_id` G2 window's accepted semantics. Verified explicitly: a
  same-`worker_id` reclaim leaves `claim_owner` unchanged, so a stale
  same-worker `mark_terminal` call still returns `Ok` and `release()` still
  runs exactly as before this fix — G2 confirmed unchanged, not improved,
  not regressed.
- [x] 10.5 Re-ran the Fault/Crash Semantics audit's 11 scenarios end-to-end
  against the fixed code. Scenarios 5 and 6 (previously UNSAFE) now SAFE —
  AD-8 confirmed holding for the cross-worker case, traced with file:line
  evidence against the real Postgres `mark_terminal` guard
  (`crates/effect-store/src/postgres/mod.rs:527-533`) and the in-memory
  regression test. Sanity pass confirmed the other 9 scenarios unaffected
  (grep confirms `dedup.release()` has exactly one production call site,
  `runner.rs:851`, inside `abandon_and_release`; retention deletes via its
  own raw SQL, independent of this port). All 11 scenarios SAFE, bounded
  only by the already-accepted G2/at-least-once residual. G15 CLOSED.

## Phase 11: G10 — Clock Reconciliation (post-freeze execution)

Spec: single wall-clock authority for every load-bearing lease decision,
reusing `ego_domain::Clock` exactly as it already exists — no redesign, no
second time trait, no feature flags.

- [x] 11.1 Inject `Arc<dyn Clock>` into `PostgresEffectStore` (new field +
  `connect()` parameter) and `DeliveryRunner` (new field/constructor param +
  a `now(&self) -> Timestamp` helper), mirroring the existing
  `security-jwt`/`security-apikey` injection idiom. `RuntimeEffectAcceptor`
  gained an additive `with_clock(...)` seam (defaulting to `SystemClock`)
  instead of changing `new`'s signature, to avoid touching ~20 existing test
  call sites.
- [x] 11.2 GREEN — `mark_in_flight`'s `Utc::now() + self.lease` →
  `self.clock.now() + self.lease`; all four `mark_*` guards' SQL-side
  `claim_expires_at > now()` → a bound parameter carrying the same
  `self.clock.now()` instant used to compute the claim — single authority for
  lease computation and validation. `claim_due`/`recover_in_flight` were
  already deterministic (caller-supplied `Timestamp`) — untouched. Non-load-
  bearing SQL `now()` (`settled_at`) deliberately left unchanged.
- [x] 11.3 `runner.rs`'s `reclaim_due`/`requeue_without_charging_attempt`/
  `finish_success`/`retry_or_give_up` now source `now` from `self.clock.now()`
  instead of `Timestamp::now()`/`Utc::now()`.
- [x] 11.4 Tests: added a `FixedClock` double (per-crate idiom, matching
  security-jwt/security-apikey) and one new deterministic test,
  `mark_in_flight_computes_claim_expires_at_from_the_injected_clock_not_wall_clock`,
  proving `mark_in_flight` uses the injected clock, not wall time — no
  `sleep`. Existing integration tests threaded `Arc::new(SystemClock)`
  through `connect()` calls, behavior-preserving.
- [x] 11.5 Scope guard — confirmed: `Clock` trait, `Timestamp` type,
  `EffectStateStore`/`EffectDedupStore` signatures, and `DeliveryRunner`'s
  public shape all unchanged (diff-checked, not just asserted).
- [x] 11.6 Surprise, noted not silently absorbed: **Stoolap has no lease
  concept at all** (no `claim_owner`/`claim_expires_at` columns or guard) —
  nothing to fix there. **`RuntimeBuilder` only ever constructs
  `InMemoryEffectStore`** today — no durable-backend builder wiring exists
  yet, so no `RuntimeBuilder::with_clock()` was added (YAGNI; trivial to add
  once something needs it).
- [x] 11.7 Validation green: `cargo build --workspace --all-features`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --all-features` (Postgres suites against real
  Docker) — zero failures across `ego-effect-store`, `ego-integration-tests`,
  `ego-runtime` (166 tests), `ego-service-sdk`, `security-jwt` (24,
  unmodified), `security-apikey` (42, unmodified). G10 CLOSED.

## Threat Matrix

| Case | Covered by |
|---|---|
| Two claimers race the same due row (multi-node) | 5.3, 5.4, 6.4 |
| Superseded worker's stale write lands after reclaim (different worker) | 5.6 for the `EffectStateStore` guard; **10.1-10.3 for the `EffectDedupStore` release it was not gated on (G15)** |
| Dedup crash mid-reservation (partial state) | 4.3, 5.10 |
| Lease expiry enabling duplicate dispatch | 3.3 (`run_multi_node_conformance` defined, reusing `DurableStoreFactory` opened twice concurrently), 5.17, 5.18 (Tier 3 run against two live `PostgresEffectStore` instances — asserted as expected + idempotency-covered, not prevented) |
| Same-`worker_id` reclaim window (G2) | **Known accepted limitation** (design §3.1) — deliberately unfenced; not testable as a guard, only bounded by lease-tuning (design §6); no task closes it, flag at verify/archive time |
| Restart survival is proven only by the durable-provider tier, never by port conformance | 3.4 (`InMemoryEffectStore` negative non-durability test), 4.6-4.8 (Stoolap Tier 2 reopen), 5.15-5.16 (Postgres Tier 2 reopen) — Tier 1 (3.1-3.2, 4.1, 5.14) deliberately asserts no restart-survival claim |
