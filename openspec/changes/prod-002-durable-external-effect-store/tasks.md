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

- [x] 7.1/7.2 (PR5 Phase 4, superseding design) — added
  `RuntimeBuilder::with_effect_store<T: EffectStateStore + EffectDedupStore>(Arc<T>)`
  (`crates/service-sdk/src/runtime/builder.rs`): the single seam this phase
  was missing (the field's own prior doc comment said so explicitly — "this
  builder has no seam yet to register a custom durable effect store at
  all"). Splits ONE registered `Arc<T>` into both the `effect_state_store`
  and `effect_dedup_store` builder fields, mirroring the
  `idempotency_reservation_store`/`build()`'s own `InMemoryEffectStore` split
  idiom already used elsewhere in this file. `build()`'s zero-cost gate now
  three-way selects: no executors → no store/acceptor built at all; executors
  + a registered custom store → that store's two ports are used directly;
  executors + no custom store → `InMemoryEffectStore` is constructed exactly
  as before (byte-identical default-path behavior). A `debug_assert_eq!`
  documents that `effect_state_store`/`effect_dedup_store` can only ever be
  both-`Some` or both-`None`, since the one public setter always sets them
  together.

  This supersedes the original 7.1/7.2 plan (register two independently-typed
  ports + log a possibly-mismatched capability pair at startup): the new API
  accepts only ONE concrete type implementing both `EffectStateStore` and
  `EffectDedupStore`, so a mixed durable/non-durable registration is no
  longer a state this builder can express, rather than one it detects and
  logs. `with_effect_retention_store` is unchanged and composes unchanged —
  `.with_effect_store(store.clone()).with_effect_retention_store(store)`.

  Evidence: `crates/service-sdk/tests/effect_store_composition.rs` (6 tests,
  all passing) — default path still dispatches a real effect through
  `InMemoryEffectStore` end-to-end; a registered custom double's
  `EffectStateStore` calls are exercised; its `EffectDedupStore` calls are
  exercised; both trait-object handles the builder produces trace back to
  the same concrete `Arc` (proven via `Arc::ptr_eq` across matching-type
  coercions plus a stripped-vtable pointer comparison across the two trait
  types); a custom store registered with zero executors builds no pipeline
  and is never called; a store additionally implementing
  `RetentionMaintenance` composes with `with_effect_retention_store` and
  builds. `crates/effect-store/src/postgres/mod.rs` and
  `crates/effect-store/src/stoolap/mod.rs` were not touched — both providers
  already implement the required trait pair, wiring one through
  `with_effect_store` is real-provider follow-up, not blocked by anything
  here. `cargo fmt --check`, `cargo build --workspace --all-features`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` all
  clean.

## Phase 7.5: Provider Composition Validation (real Stoolap + real PostgreSQL)

Scope addition to the Phase 7 PR, approved before merge. Phase 7's own
evidence proved the seam generically, against `RecordingEffectStore` — a
decorator around a real `InMemoryEffectStore`, not a from-scratch
reimplementation, but still an in-process double. This phase closes the one
remaining gap the design.md AD-2/§3.2 table opens: that `with_effect_store`
composes not merely with anything implementing the two traits, but
specifically with EACH first-party provider (`StoolapEffectStore`,
`PostgresEffectStore`), and that a real dispatch actually lands in that
provider's own backing storage — not that the runtime merely believes it
does. Deliberately NOT a re-test of `claim_due`/`mark_succeeded`/lease/
retention/recovery semantics: that is Tier 1/2/3 conformance, already
covered by `ego-effect-store`'s own `tests/conformance.rs` and the G11
Postgres integration tests.

- [x] Stoolap composition — `crates/service-sdk/tests/
  effect_store_composition.rs` gained a 7th test,
  `a_real_stoolap_effect_store_registered_via_with_effect_store_actually_receives_the_dispatch`.
  Constructs a real, embedded `StoolapEffectStore::open` against a
  `tempfile::tempdir()` path (no Docker, no external process), registers it
  via `with_effect_store`, drives one effect end-to-end (accept → dispatch →
  success) through a real spawned entity actor, then independently proves
  the dispatch landed in THAT store: a second, independently-constructed
  `stoolap::Database::open` handle at the identical DSN (Stoolap shares one
  live engine per DSN while any handle for it is still open) reads the raw
  `effect_state` row with plain SQL. `InMemoryEffectStore` has no table this
  could even be asked about — a row showing up here is real evidence of
  where the dispatch landed, not the runtime's own say-so. Runs under plain
  `cargo test --workspace --all-features`, hermetically.

  New dev-dependencies on `crates/service-sdk/Cargo.toml`, dev-only (the
  crate's hexagonal boundary — service-sdk must never depend on a concrete
  effect-store backend as a regular dependency — stays intact): `ego-
  effect-store` (path dependency, `features = ["stoolap"]`), `stoolap`
  (`"0.4"`, for the independent raw-SQL read), `tempfile` (`"3"`, for the
  on-disk path).

- [x] PostgreSQL composition — new file `integration-tests/tests/
  infrastructure/effect_store_composition_postgres.rs`, registered as `mod
  effect_store_composition_postgres;` in `integration-tests/tests/
  infrastructure.rs` and documented in `integration-tests/README.md`'s
  PROD-002 table and budget block (13 infrastructure tests total). One test,
  `a_real_postgres_effect_store_registered_via_with_effect_store_actually_receives_the_dispatch`:
  connects a real `PostgresEffectStore` against this test's
  `isolated_database()`-provisioned, per-test-exclusive PostgreSQL database,
  registers it via `with_effect_store`, drives one effect end-to-end through
  a real spawned entity actor, then independently proves the dispatch landed
  in Postgres: a second, completely separate `sqlx::PgPool` (opened via the
  isolated-database fixture's own `db.pool()`, not through
  `PostgresEffectStore`'s internal pool) polls the raw `effect_state` table
  in the schema the store was constructed with until it reads back a
  `succeeded` row. `integration-tests/Cargo.toml` needed NO new
  dev-dependency — `ego-effect-store` (`features = ["postgres"]`) and
  `ego-service-sdk` were already dev-dependencies there from G11 and the
  PROD-012 scenario suite, respectively; the task brief's assumption that
  `ego-service-sdk` would need adding was checked and found already
  satisfied.

  Run via the suite's own runner (`cargo run --manifest-path
  integration-tests/Cargo.toml --bin run-suite`, `DOCKER_HOST=unix://
  $HOME/.colima/default/docker.sock`): 35 passed, 1 pre-existing ignored
  (the documented Tier 1 vs. Postgres G1 tension in
  `effect_store_postgres_conformance.rs`, unrelated to this change), 0
  failed — including the new test, alongside every pre-existing G11 Postgres
  test with no regression. The suite's own `ledger.rs` self-check (README
  row ↔ module registration ↔ directory agreement) required a matching
  README row and budget-count update, both added.

- [x] Default in-memory path — re-confirmed
  `default_path_without_with_effect_store_dispatches_through_in_memory_effect_store`
  (Phase 7's own test 1: no `with_effect_store` call, dispatch still
  succeeds through the default-constructed `InMemoryEffectStore`) is exactly
  the "InMemory ✅" leg this phase's validation matrix needs. No
  strengthening or duplication added — the existing assertion (the command
  commits and the registered executor is actually invoked) already covers
  the default-path composition claim end-to-end.

- [x] Existing 6 Phase 7 tests — unmodified. No conflict forced an
  adjustment.

  Validation: `cargo fmt --check` clean (both the root workspace and
  `integration-tests/Cargo.toml`); `cargo build --workspace --all-features`
  clean; `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` clean; `cargo test --workspace --all-features` — all crates
  green, the new Stoolap composition test passing among them, zero Docker/
  testcontainer references anywhere in that run; `scripts/
  detect-integration-tests.sh` PASS; `scripts/
  detect-integration-tests-selftest.sh` PASS (all 11 self-test assertions);
  the real Postgres suite via `run-suite` — 35 passed, 1 pre-existing
  ignored, 0 failed, no regression.

## Phase 8: Reference-App Dogfood (Stoolap)

Spec: kill-the-process success criterion.

Phase 8 blocked on AppBuilder facade delegation, resolved by PR6a
(`AppBuilder::effect_store`/`effect_retention_store`,
`crates/service-sdk/src/app/mod.rs`) — `examples/reference-app` builds
through `App::builder()`, not `RuntimeBuilder` directly, and `AppBuilder` had
no delegation for `RuntimeBuilder::with_effect_store`/
`with_effect_retention_store`. Phase 8 itself resumes unstarted below.

- [x] 8.1 RED — reference-app e2e: an accepted effect survives a
  simulated restart against the embedded `StoolapEffectStore` (new
  process/store instance over the same on-disk file).

  `examples/reference-app/tests/stoolap_restart_persistence.rs`
  (`an_effect_accepted_before_a_restart_is_delivered_only_by_the_process_that_restarts`).
  "Process A" and "process B" are each built through
  `reference_app::build_runtime_with` (the real public composition path)
  over the SAME on-disk Stoolap directory (a fresh `tempfile::tempdir()`).
  Process A registers a real user through the real write path
  (`entities.user.entity_ref(...).send_command(UserCommand::Register {..})`
  — the identical `entity_ref`/`send_command` path `RegisterUserImpl`
  itself calls, per `tests/effects_e2e.rs`'s own precedent), asserts the
  welcome-email effect was ACCEPTED (`CommandResult::Events`, never
  `EffectsAcceptanceFailed`), then is dropped WITHOUT ever calling
  `App::start()`. Process B reopens the identical on-disk directory with a
  brand-new `StoolapEffectStore::open` handle, builds through the same
  composition path, and DOES call `App::start()`.

  Confirmed RED for the correct reason before the implementation existed:
  temporarily reverted `lib.rs`/`main.rs`/`Cargo.toml`/`domain/user.rs`
  (via `git stash`) while keeping only this test file — `cargo test -p
  reference-app --test stoolap_restart_persistence` failed to COMPILE
  (`E0061`: `build_runtime_with` takes 4 arguments, 5 were supplied;
  `E0433`/`E0432`: no `ExternalEffectsWiring`, no `reference_app::effects`
  module) — the exact missing-API shape this phase adds. Restoring the
  stashed changes made it compile and pass (see 8.3's evidence below).

  Finding (the actual hard part of this phase, beyond simple config
  wiring): `Runtime::effect_acceptor()` (`service-sdk/src/runtime/
  builder.rs:1300`) deliberately returns `None` until `start_effects()`
  has actually run — by design, so nothing can hold an acceptor whose
  drain loop was never spawned. A naive "build a scratch `RuntimeBuilder`,
  call `start_effects()` on it, take the acceptor" (as first sketched)
  would spawn a REAL, live `DeliveryRunner` against the shared on-disk
  store for the sole purpose of extracting an acceptor — which would then
  itself race to claim and deliver the effect via its `Deferred`-mode
  admission channel almost immediately, defeating the entire "process A
  never delivers" premise this test depends on, and would leak an orphan
  polling task with no lifecycle owner in production. The actual fix
  (`build_runtime_with` in `lib.rs`): construct `ego_runtime::effects::
  RuntimeEffectAcceptor::new(state, dedup, registry, DeliveryConfig::
  default())` DIRECTLY — a type `ego-runtime`'s own doc comment already
  describes as "constructible from any crate depending on ego-runtime",
  bypassing `RuntimeBuilder`/`AppBuilder` entirely for this one narrow
  need. `RuntimeEffectAcceptor::new` never spawns a task and is safe
  outside Tokio (confirmed from its own doc comment and `RuntimeBuilder::
  build`'s "construct only, never `.start()`, here" comment) — `accept()`
  durably writes into the store via `EffectStateStore::accept` regardless
  of whether a runner was ever started; only `.start()` (never called on
  this standalone instance) would spawn one. This is handed to `User`'s
  `EntityRuntimeBuilder::with_effect_acceptor` before that runtime's
  `Arc` is built (required — `EntityRuntime::effect_acceptor` has no
  interior mutability). The REAL, started delivery path is entirely
  separate: the same `store`/`executor` are ALSO registered on the real
  `AppBuilder` chain via `.effect_store()`/`.effect_executor()` (PR6a's
  facade), and only THAT runtime's `DeliveryRunner` — spawned by
  `App::start()` → `Runtime::start_effects()` in process B — ever claims
  and delivers anything. Confirmed by reading `runner.rs`: `run_inner`
  deliberately resets/skips its first reclaim tick
  (`RECLAIM_INTERVAL` = 5s) so a fresh runner never reclaims before
  anything could possibly be due — this is also why the test's polling
  timeout is 8s (mirroring `tests/effects_e2e.rs`'s own margin for the
  identical reason), not a shorter one: `claim_due` matches plain
  `pending` rows too (confirmed against
  `crates/effect-store/src/postgres/mod.rs`'s `claim_due` SQL), so
  process B's runner picks up process A's row on its very first reclaim
  tick, consistently landing at ~5.0-5.2s across repeated runs.

- [x] 8.2 GREEN — wire `examples/reference-app` to register
  `StoolapEffectStore` in place of `InMemoryEffectStore` — no external
  server dependency.

  `ego-effect-store` (features = ["stoolap"]) added as a regular
  (non-dev) dependency of `examples/reference-app` — `main.rs` uses it in
  production, not only in tests. `build_runtime_with` gained a 5th
  parameter, `effects: ExternalEffectsWiring` (`None` | `Stoolap { store,
  executor }` — declared visibly, mirroring `IdempotencyWiring`'s
  "reachable only by explicit choice, never by omission" shape); all 3
  existing call sites (`build_runtime_observed_in_memory`,
  `tests/idempotency_wiring.rs`, and `main.rs`) updated. `main.rs` opens
  `StoolapEffectStore::open(path)` at a new deterministic directory
  (`examples/reference-app/data/effects`, default via
  `concat!(env!("CARGO_MANIFEST_DIR"), "/data/effects")`), overridable via
  `EGO_REFERENCE_APP_EFFECT_STORE_PATH` — read exactly once, at the
  composition root, mirroring the existing `CRASH_FAILPOINT_VAR` idiom
  already documented in `lib.rs`. Directory added to a new
  `examples/reference-app/.gitignore` (never committing runtime DB
  files); README documents the path and reset instructions (delete the
  directory).

  A new `src/effects.rs` module holds `WelcomeEmailExecutor` — a real
  (but log-only, no actual mailer, mirroring the existing "no real
  network call, deliberately in-memory/log-only dogfood" convention
  `providers/pricing_lookup.rs` already sets) `ExternalEffectExecutor`
  that always logs the destination/idempotency key via `println!` (this
  crate has no `tracing`/`log` dependency to reach for instead) and
  returns `AttemptOutcome::Success`. The real, already-existing effect
  this dogfoods is `UserEntity::external_effects`'s "welcome email"
  (`effect_type: "user.welcome_email"`, now named once as
  `domain::user::WELCOME_EMAIL_EFFECT_TYPE` so the description side and
  every registration site can never silently drift apart) — no fake demo
  effect was invented.

  `build_runtime_with`'s internals (composition-root code, the diff
  briefly): computes `user_effect_acceptor` via the directly-constructed
  `RuntimeEffectAcceptor` described in 8.1's finding when `effects` is
  `Stoolap`; builds `org`/`user` `EntityRuntime`s directly via the
  (now 3-arg) private `observed_entity_runtime` helper instead of via
  `compose_entity_runtimes` (whose public 2-arg signature is unchanged —
  it always passes `None` for both aggregates, since
  `TenantOrganization` never describes external effects and nothing else
  in the codebase needs it wired) — `org` always gets `None`, `user`
  gets the computed acceptor; and, on the REAL `AppBuilder` chain that
  actually gets `.build()`'d/`.start()`'d, calls
  `.effect_store(store.clone()).effect_executor([WELCOME_EMAIL_EFFECT_TYPE],
  executor.clone())` (PR6a's facade — this is the concrete production
  dogfood of `AppBuilder::effect_store`/`effect_executor` Phase 8 exists
  to prove).

- [x] 8.3 Run `cargo test -p reference-app` — 0 regressions.

  `cargo test -p reference-app`: every existing test file green
  (guard-chain, dual-write, observability, read-side projection, HTTP
  round trip, `effects_e2e.rs`'s real-actor-spawn path, the
  `external_data_provider_lint` — after rewording one doc comment in
  `effects.rs` that happened to contain the literal substring
  `PricingLookupProvider`, which the lint's line-based scan flags
  wherever it appears — all 4 lint sub-tests pass) plus the new
  `stoolap_restart_persistence.rs`, 1 passed, 0 failed. Zero regressions.

  Full validation, all green: `cargo fmt --check` (workspace); `cargo
  build --workspace --all-features`; `cargo clippy --workspace
  --all-targets --all-features -- -D warnings`; `cargo test --workspace
  --all-features` (every crate, no failures, no Docker/testcontainer
  anywhere in the run); `cargo test -p reference-app`; `bash scripts/
  detect-integration-tests.sh` PASS; `bash scripts/
  detect-integration-tests-selftest.sh` PASS (all 11 self-test
  assertions). The restart test itself re-run 3× standalone, consistently
  ~5.07-5.17s, comfortably inside its 8s timeout.

  Restart guarantee demonstrated, mapped exactly to proposal.md's
  criterion #1 ("an accepted effect survives a real process restart and
  is delivered exactly as the spec's reconstructability requirement
  demands") — and ONLY that criterion. Criterion #2
  (in-flight-at-crash redispatch via `recover_in_flight`) and criterion
  #3 (multi-node claim exclusivity) are deliberately NOT exercised here —
  both already covered by Tier 2/3 conformance (Phases 4.6-4.8,
  5.15-5.18) — and `recover_in_flight` was correctly never needed: the
  effect in this test is accepted but never claimed by process A (no
  runner ever ran there), so process B's ordinary `claim_due` (not
  `recover_in_flight`) is what picks it up, exactly as the brief
  predicted. No recovery/lifecycle gap encountered.

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

## Phase 12: G11 — Postgres Test Relocation onto the Shared Harness (post-freeze execution)

Spec: PROD-012 built a top-level `integration-tests/` workspace (one shared
PostgreSQL per run, one template migrated once, one isolated database per
test) to replace the old container-per-test-file shape. PROD-002's two
real-Postgres test files (`crates/integration-tests/tests/
effect_store_postgres_{unit,conformance}.rs`, each starting its own
`testcontainers` container) predate that harness and were never migrated onto
it. Scope: relocate and adapt those two files only. No change to
`EffectStateStore`/`EffectDedupStore`, `PostgresEffectStore`'s production
code, AD-6, the conformance tier model (Tier 1/2/3, all frozen, §3.6), or
Tier 1/Tier 2-Stoolap's crate-local location
(`crates/effect-store/src/conformance.rs`,
`crates/effect-store/tests/conformance.rs`).

- [x] 12.1 Recovered both files' real content (not a blind `git mv`) from the
  last commit that held the old `crates/integration-tests` crate
  (`c65432f`), since the reconciliation worktree's tree no longer carries
  that crate at all.
- [x] 12.2 Investigated whether the harness's template-DB migration step
  (`ego_persistence::postgres::migrations::run`, `integration-tests/src/
  main.rs`) needed extending to also apply `ego-effect-store`'s migrations.
  Finding: it does not. `PostgresEffectStore::connect` (AD-10) already
  creates its own schema and runs its own migration sequence on every call,
  idempotently — self-contained per schema, independent of whatever the
  template's `public` schema carries. No wiring was added; the two crates'
  migrations coexist in the same physical database (different schemas) with
  no shared version ledger and no double-apply risk.
- [x] 12.3 Moved both files into `integration-tests/tests/infrastructure/`,
  same flat layout and `*_postgres.rs` naming convention as every other file
  there, and registered both as `mod` declarations in
  `integration-tests/tests/infrastructure.rs` (alphabetical, matching the
  existing order).
- [x] 12.4 Adapted both files' setup: `testcontainers`/per-test
  `uuid`-suffixed schema replaced with `ego_integration_tests::
  isolated_database()` (the same fixture every other file in this directory
  uses) plus a fixed schema constant — fixed rather than `uuid`-suffixed
  because each test already owns an exclusive database from the harness, so
  nothing is left for the schema name to disambiguate. `db.close().await` at
  the end of every test, matching the existing convention.
- [x] 12.5 Satisfied `tests/ledger.rs`'s three-way consistency guard (disk ↔
  registration ↔ `README.md`) by adding two rows under a new "PROD-002
  durable effect-store" section, plus updating the suite's test-count
  summary block (10 → 12). Confirmed green:
  `cargo test --manifest-path integration-tests/Cargo.toml --test ledger`.
- [x] 12.6 `crates/effect-store/Cargo.toml` checked for a leftover
  `testcontainers`/`testcontainers-modules` dev-dependency: none present —
  Tier 1/Tier 2-Stoolap conformance (`crates/effect-store/tests/
  conformance.rs`) never depended on it, so there was nothing to remove.
  `crates/effect-store/src/lib.rs`'s doc comment (naming the old
  `crates/integration-tests` as the consumer of the public `conformance`
  module) updated to name the current top-level `integration-tests/`
  workspace instead — comment-only, no code change.
- [x] 12.7 Validation, all green:
  - `bash scripts/detect-integration-tests.sh` and
    `bash scripts/detect-integration-tests-selftest.sh` — PASS.
  - `cargo build --workspace --all-features` and
    `cargo test --workspace --all-features` — 0 failures, no Docker/
    testcontainers reference anywhere in the output (root workspace stays
    hermetic; PROD-012's own invariant, unchanged by this phase).
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
    and the same against `integration-tests/Cargo.toml` — clean.
  - The harness's own runner, `cargo run --manifest-path
    integration-tests/Cargo.toml --bin run-suite` (never `cargo test`
    directly against the `infrastructure` target — the harness's own guard
    rejects that path, see its `README.md`/`src/lib.rs`): the ledger guard
    ran first and passed, then 34 passed / 1 ignored / 0 failed across the
    whole suite in 1.94s (13 of those 34+1 are the relocated effect-store
    tests: 8 unit + 5 conformance, one of the five `#[ignore]`d by the
    same documented Tier-1/G1 tension the pre-relocation file already
    carried). Every pre-existing PROD-012 infrastructure test in the same
    run still passed, confirming no isolation regression between the two
    features' tests sharing one harness.
  - Tier 2 (`postgres_satisfies_durable_conformance`) and Tier 3
    (`postgres_satisfies_multi_node_conformance`) both ran and passed in
    that same run — confirmed by name in the runner's own test-result
    output, not inferred.
- [x] 12.8 Scope guard — confirmed by diff, not just asserted: the only
  files touched are `integration-tests/Cargo.toml`,
  `integration-tests/README.md`, `integration-tests/tests/infrastructure.rs`,
  the two new test files, and one doc-comment line in
  `crates/effect-store/src/lib.rs`. No port, domain, or production-code file
  changed; Tier 1/Tier 2-Stoolap's files untouched. G11 CLOSED.

## Phase 13: G12 — Runtime-Owned Retention Capability (post-audit rewrite)

Spec: AD-9's retention SQL stays provider-owned and unduplicated
(`run_retention` in both providers, unchanged); only the *schedule* moves to
a runtime-owned worker, mirroring PROD-012's reservation-retention shape.
Scope: a new optional `RetentionMaintenance` capability trait, its two
provider implementations (delegating, not reimplementing), a new
`EffectRetentionWorker` in `ego-service-sdk`, and additive `RuntimeBuilder`
wiring. Explicitly does not touch `EffectStateStore`/`EffectDedupStore`'s
trait signatures, `run_retention`'s SQL, or PROD-012's
`RetentionWorker`/`RetentionPolicy`/`start_retention()`.

- [x] 13.1 RED/GREEN — `RetentionMaintenance` trait added to
  `crates/runtime/src/effects/store.rs` (`purge_before(cutoff, batch) ->
  Result<u64, EffectStoreError>`; `oldest_terminal() -> Result<Option
  <Timestamp>, EffectStoreError>` defaulting to `Ok(None)`, mirroring
  `OperationReservationStore::oldest_completed`'s default). Test:
  `oldest_terminal_defaults_to_none_for_a_bare_implementor`. Exported from
  `crates/runtime/src/effects/mod.rs`.
- [x] 13.2 GREEN — `PostgresEffectStore`/`StoolapEffectStore` each implement
  `RetentionMaintenance::purge_before` by calling their existing
  `run_retention(cutoff, Duration::zero(), batch)` and mapping the error to
  its `source` — no SQL duplicated. Tests (Stoolap, in-process, no Docker):
  `retention_maintenance_purge_before_calls_through_to_run_retention`,
  `retention_maintenance_oldest_terminal_defaults_to_none`
  (`crates/effect-store/tests/conformance.rs`). Postgres SQL correctness
  stays covered by `run_retention`'s own existing tests — not re-tested here
  per design.
- [x] 13.3 GREEN — new module `crates/service-sdk/src/runtime/
  effect_retention.rs`: `EffectRetentionPolicy`/`EffectRetentionPolicyError`
  (no `Default`, validated `new`, identical shape to `RetentionPolicy`) and
  `EffectRetentionWorker` (start/stop, `Notify`-based cancellation,
  abort-then-await bounded shutdown, `effect.purge_batch` root span per
  tick, `effect.cleanup.rows`/`effect.cleanup.batch_duration` metrics using
  G13's already-fixed names). Reuses `super::retention::isolate_panics`
  rather than a second copy.
- [x] 13.4 GREEN — `RuntimeBuilder`: `with_effect_retention_store`,
  `with_effect_retention_policy`, `with_effect_retention_clock`; a
  `build()`-time guard refusing a configured policy with no registered
  `RetentionMaintenance` (mirrors the existing reservation-retention guard);
  `Runtime::start_retention_effects()` — named distinctly from PROD-012's
  `start_retention()` so both coexist on the same `Runtime`.
- [x] 13.5 Tests — `crates/service-sdk/tests/
  effect_retention_worker_lifecycle.rs` (mirrors
  `retention_worker_lifecycle.rs`'s style): disabled by default and no
  worker starts without a policy; a degenerate policy is refused at
  construction; a policy with no registered store is refused at `build()`;
  a configured worker purges with the exact cutoff (computed from an
  injected fake clock, never wall time) and batch, then stops on shutdown;
  starting twice starts one worker; an overrunning worker is aborted, not
  detached; every tick opens a root `effect.purge_batch` span.
- [x] 13.6 Scope guard — confirmed by diff:
  `EffectStateStore`/`EffectDedupStore` byte-identical; `run_retention`'s
  SQL in both providers untouched; PROD-012's
  `RetentionWorker`/`RetentionPolicy`/`start_retention()`/
  `retention_worker_lifecycle.rs` untouched (its 22 tests still pass
  unmodified); G10 (Clock injection)/G11 (harness relocation)/G15 (causal
  dedup-release gate) untouched.
- [x] 13.7 Validation, all green: `cargo build --workspace --all-features`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --all-features` — 0 failures, 0 regressions
  (`ego-effect-store`, `ego-runtime`, `ego-service-sdk` — including both
  retention lifecycle test files — and every other workspace crate all
  green). G13 (effects metrics wiring) confirmed still open elsewhere in
  this worktree — this worker uses its two already-fixed cleanup metric
  names, and leaves a `TODO(G13)` comment (not an invented gauge) where a
  settled-backlog-age gauge would go once G13 names one. G12 CLOSED.

## Phase 14: G13 — Effects Observability/Metrics Wiring (AD-14 reconciliation)

Spec: tracing and metrics are complementary, not substitutes — every
existing `log_*` call in `crates/runtime/src/effects/observability.rs`
stays exactly as it was; metrics are added alongside at existing call
sites. Scope: `DeliveryRunner`/`RuntimeEffectAcceptor` wiring +
`EffectRetentionWorker`'s cleanup tick. Explicitly does not touch
`EffectStateStore`/`EffectDedupStore`, the Postgres claim SQL, Stoolap's
ownership model, G10's `Clock` model, G11's integration-test layout, G12's
`RetentionMaintenance` contract, or G15's causal gate.

- [x] 14.1 GREEN — `DeliveryRunner` gains an `Option<Arc<dyn Observability>>`
  field + additive `with_observability` builder step (same idiom G10 used
  for `Clock`; `DeliveryRunner::new`'s signature unchanged).
  `RuntimeEffectAcceptor` gains a sibling `with_observability` constructor
  (and an internal `with_clock_and_observability` composition point both
  named constructors delegate to) so `service-sdk`'s `RuntimeBuilder::
  build()` can pass `self.observability.clone()` through instead of the
  previous unconditional `RuntimeEffectAcceptor::new(...)` call — the same
  `Arc<dyn Observability>` already registered for macro-guard denials now
  also reaches the effects delivery pipeline.
- [x] 14.2 GREEN — `effect.claim.event` (Counter, `event ∈ {"acquired",
  "reclaimed_after_expiry"}`) emitted from `DeliveryRunner::reclaim_due` —
  provider-agnostic, bucketed purely from `StoredEffect::state` as
  `claim_due` already returns it (`Pending`/`RetryableFailed` → acquired,
  `InFlight` → reclaimed). No owner/epoch/lease timestamp crosses this
  boundary — `StoredEffect` carries none for any provider, so exposing them
  here would mean widening the frozen `EffectStateStore` contract, which
  this task does not do. `log_claim_acquired`/`log_claim_reclaimed_after_expiry`
  stay exactly as unwired as before (`#[allow(dead_code)]`), for that same
  data-shape reason, not an oversight. Tests (`runner.rs`, no Docker):
  `a_purely_fresh_batch_emits_only_the_acquired_bucket`,
  `a_purely_reclaimed_batch_emits_only_the_reclaimed_bucket`,
  `a_mixed_batch_reports_both_buckets_with_their_own_counts`,
  `an_empty_batch_emits_nothing`, `no_observability_registered_is_a_silent_no_op`,
  `only_the_closed_event_attribute_ever_appears_never_owner_or_epoch_or_timestamps`,
  `the_reclaim_loop_itself_emits_effect_claim_event_for_a_pending_effect`
  (end-to-end through `run_inner`, proving the wiring reaches the real
  production tick, not just a direct method call).
- [x] 14.3 Found unwireable, reported rather than improvised —
  `effect.recovery.rows`: `EffectStateStore::recover_in_flight` has **no
  production caller anywhere** in `ego-runtime`/`ego-service-sdk` as of
  G12's HEAD (confirmed by exhaustive grep — every call site outside its
  own trait/impl definitions is a test double or a direct unit-test call).
  There is no startup/crash-recovery sweep this metric could be added
  alongside without first inventing one, and inventing that runtime
  behavior (when a sweep runs, at what scope, on what schedule) is a
  functional design decision outside this observability-wiring change's
  charter. `effect.recovery.rows` and `log_recovered_in_flight` remain
  unwired — a pre-existing gap this task surfaces rather than papers over,
  not a new G13 gap. Whoever designs the recovery sweep gets both the
  tracing call and this metric name for free at that call site.
- [x] 14.4 GREEN — cleanup metrics need no new wiring path: G12 already
  threaded `Arc<dyn Observability>` into `EffectRetentionWorker::start`, so
  `effect.cleanup.rows`/`effect.cleanup.batch_duration` (already emitted
  since G12, using these exact fixed names) needed no change here. Added
  `effect.cleanup.oldest_terminal_age` (Gauge) — same `effect.cleanup.*`
  family since it's queried from the same tick — mirroring PROD-012's
  `idempotency.purge.oldest_completed_age` line for line: queried *after*
  the purge (describes the remaining backlog, not what this batch just
  removed), computed from the worker's injected `Clock` (never wall time),
  clamped at zero for cross-replica clock skew, silent on `None`/`Err`
  exactly as `OperationReservationStore::oldest_completed`'s
  `Empty`/`Unsupported` cases already are for reservations. Tests
  (`effect_retention_worker_lifecycle.rs`):
  `a_successful_tick_counts_the_rows_it_removed_and_its_duration`,
  `a_failing_purge_reports_its_duration_and_no_rows`,
  `an_uninstrumented_worker_still_purges_and_counts_nothing`,
  `the_gauge_reports_the_age_of_the_oldest_surviving_settlement`,
  `a_settlement_ahead_of_the_observing_clock_reports_zero_not_a_negative_age`,
  `no_sample_when_the_store_reports_none`, `no_sample_when_oldest_terminal_errors`,
  `each_metric_is_emitted_exactly_once_per_tick_not_duplicated` (provider vs.
  worker double-emission guard).
- [x] 14.5 Scope guard — confirmed by diff: `EffectStateStore`/
  `EffectDedupStore` byte-identical; Postgres `claim_due`'s
  `UPDATE ... RETURNING` untouched (no new returned column, no new branch);
  Stoolap's ownership model, G10's `Clock` model, G11's integration-test
  layout, G12's `RetentionMaintenance` contract/trait shape, and G15's
  causal gate in `abandon_and_release` all untouched.
- [x] 14.6 Cardinality guard — `only_the_closed_event_attribute_ever_appears_never_owner_or_epoch_or_timestamps`
  (14.2) and the cleanup metrics' zero-extra-attribute call sites (14.4)
  jointly confirm owner/previous_owner/new_owner/epoch/expires_at/reason
  never appear as a metric attribute anywhere this task touches.
- [x] 14.7 Validation, all green: `cargo build --workspace --all-features`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --all-features` — 0 failures, 0 regressions.
  PROD-012's own `retention_worker_lifecycle.rs` (22 tests) and observability
  conformance tests re-confirmed green, unmodified. `spec.md` untouched — no
  public-contract contradiction found. G13 CLOSED.

## Phase 15: G14 — Remove `ExternalEffectExecutor::honors_idempotency_key()`

Spec: non-blocking API cleanup, no behavior change.

- [x] 15.1 Re-verified usage exhaustively before touching anything: every
  hit for `honors_idempotency_key` across the workspace (Rust, Markdown,
  OpenSpec) was inside `crates/runtime/src/effects/executor.rs` itself —
  the trait's default method, one test-double override
  (`IdempotentExecutor`), and two unit tests exercising exactly that
  default/override. Zero hits in `runner.rs`, `service-sdk`, `reference-app`,
  `integration-tests`, or `spec.md`.
- [x] 15.2 Intent recovered: CORE-019's own archived design.md
  (`openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/design.md`)
  already carried this exact default (`fn honors_idempotency_key(&self) ->
  bool { false }`) with the identical doc comment — "doc-only signal for
  end-to-end semantics; does not change runtime behavior." It was never
  intended to drive runtime branching; it was always documentation-shaped,
  from the change that introduced it.
- [x] 15.3 Rule of Two — FAILS. Only "use case" found was the trait's own
  default+override pair, tested only against itself (`IdempotentExecutor`
  existed solely to override this one method; its `execute()` returning a
  fixed `RetryableFailure` was never independently asserted for any other
  reason). No second real consumer anywhere in the workspace, no normative
  requirement in `spec.md`. DROP, not retain.
- [x] 15.4 GREEN — removed `honors_idempotency_key()` from
  `ExternalEffectExecutor`, its override in the now-pointless
  `IdempotentExecutor` test double (deleted, no other purpose), and the two
  tests that existed solely to exercise it
  (`default_honors_idempotency_key_is_false`,
  `executor_may_override_honors_idempotency_key`) — both were about this
  method, not incidentally covering something else worth preserving.
  `AlwaysSucceeds` (existed only to host the first of those two tests) was
  removed with it. No replacement capability flag, enum, or generalized
  capability system introduced — deletion only, per the task's own
  constraint.
- [x] 15.5 Compatibility: `crates/runtime` is not published, PROD-002 is
  still unreleased/unmerged — no external consumer exists to break. Not a
  public-API-breaking-change concern in practice.
- [x] 15.6 Validation, all green: `cargo build --workspace --all-features`
  (no unused-import warnings — `IdempotencyKey`/`TenantId` remain used by
  `EffectContext`'s own fields, untouched), `cargo clippy --workspace
  --all-targets --all-features -- -D warnings`, `cargo test --workspace
  --all-features` — 0 failures, 0 regressions anywhere in the workspace.
  `execute()`'s contract and every existing executor's behavior unchanged.
- [x] 15.7 Frozen architecture confirmed untouched:
  `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance` contracts,
  Clock/Observability models, Postgres claim SQL, Stoolap model, G15's
  causal gate — none touched. `design.md`/`spec.md` never documented this
  method as part of the intended architecture, so neither needed updating.
  G14 CLOSED — DROP.

## Threat Matrix

| Case | Covered by |
|---|---|
| Two claimers race the same due row (multi-node) | 5.3, 5.4, 6.4 |
| Superseded worker's stale write lands after reclaim (different worker) | 5.6 for the `EffectStateStore` guard; **10.1-10.3 for the `EffectDedupStore` release it was not gated on (G15)** |
| Dedup crash mid-reservation (partial state) | 4.3, 5.10 |
| Lease expiry enabling duplicate dispatch | 3.3 (`run_multi_node_conformance` defined, reusing `DurableStoreFactory` opened twice concurrently), 5.17, 5.18 (Tier 3 run against two live `PostgresEffectStore` instances — asserted as expected + idempotency-covered, not prevented) |
| Same-`worker_id` reclaim window (G2) | **Known accepted limitation** (design §3.1) — deliberately unfenced; not testable as a guard, only bounded by lease-tuning (design §6); no task closes it, flag at verify/archive time |
| Restart survival is proven only by the durable-provider tier, never by port conformance | 3.4 (`InMemoryEffectStore` negative non-durability test), 4.6-4.8 (Stoolap Tier 2 reopen), 5.15-5.16 (Postgres Tier 2 reopen) — Tier 1 (3.1-3.2, 4.1, 5.14) deliberately asserts no restart-survival claim |
