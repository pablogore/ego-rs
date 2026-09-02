# Verify Report: PROD-013 — Production Composition Hardening

> Canonical / source of truth. Spanish review companion: `verify-report.es.md` (1:1 identifiers).

**Status**: PASS
**Verified against**: `opsx/prod-013-wu7-architecture-docs` (accumulates all 7 stacked-to-main work units)
**Baseline in design.md**: `develop @ a740d34`

## Executive Summary

All 39 tasks across the 7 work units are functionally complete and verified against the real code (not against prior apply reports). 4 of the 5 gate commands (`cargo check --workspace`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, and the Docker-backed `run-suite`) exit 0; `cargo fmt --check` remains non-zero solely because of a verified pre-existing baseline drift on `develop`, unrelated to this change. Net: **0 CRITICAL, 0 WARNING, 4 non-blocking SUGGESTIONS** (see Issues below). CRITICAL check #10 — whether a `Profile::Production` composition can still silently run on volatile storage — found no bypass: the gate is structurally closed for the reference app (private `profile` field, two constructors) and the one architectural boundary that does exist (`AppBuilder::profile()` does not cascade to already-built entity runtimes) is explicitly documented in code exactly as `design.md` AD-5 requires, and is the same residual risk the proposal (R-1) already named and accepted rather than concealed.

## Task Completeness (tasks.md)

| Phase | Tasks | State |
|---|---|---|
| 1 — `Profile` + shared predicate | 1.1–1.5 | [x] all, verified against `profile.rs`, `error.rs`, `lib.rs` re-export |
| 2 — `EntityRuntimeBuilder` gate + `try_build()` | 2.1–2.7 | [x] all, verified against `builder.rs` (tests + panic guard) |
| 3 — Effect-store gate | 3.1–3.7 | [x] all, verified against `service-sdk/runtime/builder.rs`, `app/mod.rs` |
| 4 — `EntityEventStores` wiring | 4.1–4.10 | [x] all, verified against `reference-app/src/lib.rs` |
| 5 — Postgres runtime-flavor fix | 5.1–5.3 | [x] all, verified: 7 test fns across 4 files migrated to `flavor = "multi_thread"`; migration 012 confirmed present on the `develop` baseline (inherited via WU1, not part of this WU's diff) |
| 6 — AD-10 regression guards | 6.1–6.2 | [x] all, verified: `production_profile_guard.rs` exists; Production assertion present in `durable_entity_progress_postgres.rs:102` |
| 7 — Documentation | 7.1–7.2 | [x] all, verified in `ARCHITECTURE.md` and `ROADMAP.md` |
| 8 — Final verification | 8.1, 8.2 unchecked in tasks.md; 8.3 [x] | **Ran in this verify pass** — see Gate Evidence below. Not a blocker: these two tasks are literally "run the gate commands," which is this phase's own job. Recommend the orchestrator/apply mark them `[x]` post-verify, but this is administrative, not a functional gap. |

No CRITICAL from unchecked tasks: 8.1/8.2 are command-execution tasks whose content this verify pass itself performed and confirmed green.

## Gate Evidence

| Command | Exit | Result |
|---|---|---|
| `cargo fmt --check` | non-zero | 1 diff, in `crates/service-sdk/src/app/mod.rs:275` (`record_app_started` line-wrap). **Confirmed pre-existing on `develop` baseline** (`git show develop:crates/service-sdk/src/app/mod.rs` has the same unwrapped line) — not introduced by any PROD-013 commit (`git diff develop..HEAD` for that file does not touch this line). Not a PROD-013 regression; recorded as a SUGGESTION, out of this change's scope. |
| `cargo check --workspace` | 0 | Clean. |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | Clean, zero warnings. |
| `cargo test --workspace` | 0 | 0 failed across every unit/integration/doc-test binary in the workspace (spot totals include 365, 248, 198, 180, 117, 85, 52, 42, 29, 28, 26×2, 22, 16, 15×2, 13×2, 12×2, 9×3, 8×2, 7×3, 6×5, 5×7, 4×8, 3×many, 2×many, 1×many — all `0 failed`). Confirms SC-7. |
| Docker `run-suite` (`DOCKER_HOST=unix:///Users/pablogore/.colima/default/docker.sock`) | 0 | 43 passed, 0 failed, 1 ignored (the pre-existing/documented `postgres_satisfies_state_store_conformance` Tier-1 exclusion) — matches design.md task 5.3's own reported stabilized outcome exactly. Includes `entity_event_stores_wiring_postgres::{a_written_snapshot_survives_a_fresh_open_against_the_same_pool, opened_stores_declare_profile_production}` and all 3 `durable_entity_progress_postgres` tests (with the AD-10 Production assertion inline) — all `ok`. |

## Spec Compliance Matrix

| Spec | Requirement | Code evidence | Test evidence | Status |
|---|---|---|---|---|
| production-composition-hardening | Explicit Profile Declaration | `profile.rs` `Profile` enum, `Default = Dev` | `require_configured_matrix` | PASS |
| production-composition-hardening | Event Store Gate | `builder.rs:284-297` `validate_persistence` | `try_build_rejects_missing_event_store_under_production` | PASS |
| production-composition-hardening | Snapshot Store Gate | same | `try_build_rejects_missing_snapshot_store_under_production` | PASS |
| production-composition-hardening | Effect Store Gate, conditional on executor | `service-sdk/runtime/builder.rs:777-794` `validate_persistence_profile` | `validate_persistence_profile_rejects_missing_effect_store_when_executor_registered`, `validate_persistence_profile_ok_when_no_executor_registered` | PASS |
| production-composition-hardening | Partial config covered by per-capability gates | AD-7 fold-in, no separate check exists | `try_build_rejects_partial_configuration_under_production` | PASS |
| production-composition-hardening | One validator, single source of truth | `require_configured` (profile.rs:28), called from both crates, no duplicate found by grep | `build_and_try_build_agree_on_persistence_profile_validation` | PASS |
| production-composition-hardening | Rejections actionable | `PersistenceCompositionError::NotConfigured{capability,fix}` | asserted in the same tests above | PASS |
| production-composition-hardening | Non-Production compositions unaffected | `Profile::Dev` default, `build()` unwrap_or_else fallbacks unchanged when not Production | `dev_profile_builds_on_nothing_configured`; `cargo test --workspace` 0 new failures | PASS |
| production-composition-hardening | Persistence completeness rule documented | `ARCHITECTURE.md:192` new subsection | readback | PASS |
| production-composition-hardening | PROD-005 boundary documented | `ARCHITECTURE.md:452-456`, `ROADMAP.md:675-677` | readback | PASS |
| production-composition-hardening | Reference app declares profile via `EntityEventStores` | `lib.rs:345-440` private field, two constructors | `EntityEventStores::in_memory().profile() == Dev` (`production_profile_guard.rs`), `opened_stores_declare_profile_production` (Docker) | PASS |
| production-composition-hardening | Reference app snapshot store is durable | `lib.rs:407-418` two `PostgreSQLSnapshotStore` instances | `a_written_snapshot_survives_a_fresh_open_against_the_same_pool` (Docker) | PASS |
| production-composition-hardening | Regression check guards reference declaration | `production_profile_guard.rs` (Dev half) + `durable_entity_progress_postgres.rs:102` (Production half) | both ran green | PASS |
| persistent-entity | Gates in-memory fallback by Profile | `builder.rs:323-326` `build()` panics before delegating | `build_panics_on_same_condition_try_build_refuses` | PASS |
| persistent-entity | Partial config covered by per-capability gates | same as above | same | PASS |
| persistent-entity | Existing 67 call sites unaffected | `Profile::Dev` default, zero source edits at call sites | `cargo build --workspace` + `cargo test --workspace` green | PASS |
| application-composition | Profile declaration at composition root | `RuntimeBuilder::profile()` (builder.rs:282), `AppBuilder::profile()` (app/mod.rs:494) | `profile_...` unit tests | PASS |
| application-composition | Effect store gate through `CompositionError::Validation` | `RuntimeError::PersistenceNotConfigured(#[from] ...)` (runtime_builder.rs:1510+) → `CompositionError::Validation` (pre-existing path, unmodified) | integration test in `crates/service-sdk/tests/` (task 3.7) | PASS |
| application-composition | Reference app propagates profile, guarded | `lib.rs:735-739` captures `stores.profile()` before move; `App::builder()...profile(profile)` at the chain | Docker Production assertion + Dev guard | PASS |

## Deep Verification of the 10 Explicit Checks

1. **`profile.rs`** — `Profile{Dev(default), Production}` + `require_configured` exist exactly as designed (AD-1, AD-3). Confirmed by direct read.
2. **`builder.rs` gate** — `.profile()`, `try_build()`, `build()` panics under `Profile::Production` with missing capability. Confirmed `cargo build --workspace` (via `cargo check --workspace`, equivalent for this purpose) is green with all 67 `EntityRuntimeBuilder::new()` call sites untouched (verified via `git diff develop..HEAD` — no call-site files outside profile-aware ones were edited beyond what design.md itself lists).
3. **`service-sdk/runtime/builder.rs`** — `validate_persistence_profile`, conditional on `effect_executors.is_empty()`, called from both `build()` (builder.rs:820) and `try_build()` (builder.rs:1144). Confirmed.
4. **`examples/reference-app/src/lib.rs`** — `EntityEventStores::open()` wires two real `PostgreSQLSnapshotStore` instances + `Profile::Production`; `in_memory()` wires `InMemorySnapshotStore` + `Profile::Dev`. Confirmed byte-for-byte against AD-8/AD-9.
5. **`crates/persistence/src/postgres/snapshot.rs` + migration 012** — both exist on the `develop` baseline, inherited from WU1/PR1 and not part of PROD-013 WU5's diff; `IS NOT DISTINCT FROM` used in both `SELECT`s, tenant/systemwide-branched `INSERT ... ON CONFLICT` each targeting its own partial index (`ux_snapshots_identity_tenant`, `ux_snapshots_identity_systemwide`). Confirmed as a verified precondition WU5 builds on, not something WU5 introduces.
6. **Postgres tests migrated to `flavor = "multi_thread"`** — confirmed via grep: exactly the 7 test functions across the 4 files task 5.1/5.2 name (`durable_entity_progress_postgres.rs` ×3, `dual_aggregate_crash_recovery_postgres.rs` ×2, `concurrent_replicas_postgres.rs` ×1, `entity_event_stores_wiring_postgres.rs` ×1).
7. **`production_profile_guard.rs` + Production assertion** — both exist and both pass (verified in gate runs above).
8. **`ARCHITECTURE.md` / `ROADMAP.md`** — both carry the new PROD-013 sections (IS-9/IS-10). Confirmed.
9. **No read-side gate, real or pseudo** — grepped `AppBuilder::projection`, `SharedReadSideStore`, `ReadSideSink` against every `profile`-touching file: zero intersection. `AppBuilder::projection()` remains untouched DI, `SharedReadSideStore`/`ReadSideSink` remain reference-app-local wiring with no `Profile` awareness at all. D-4/OOS-2 fully respected.
10. **The motivating bypass** — for the reference app (the concrete host this change targets), the bypass is closed **structurally**, not conventionally: `EntityEventStores.profile` is a private field, and its only two producers (`open()`, `in_memory()`) always pair the profile with matching real/volatile stores — there is no constructible `EntityEventStores` value that mismatches the two. For the framework in general, one architectural boundary remains and is intentional: `AppBuilder::profile(Production)` does not retroactively validate an `EntityRuntime` that was already built (with `Profile::Dev` defaults) before being registered via `.entity()`. This is not a silent gap — `AppBuilder::profile()`'s doc comment states it explicitly (verified verbatim against AD-5's mandated wording), and the proposal's own R-1 names this exact residual risk and accepts it rather than claims to close it. **Not a blocking finding** — it is the documented boundary of an opt-in convention, not an unacknowledged hole.

## Design Coherence

All 11 architecture decisions (AD-1 through AD-11) verified against the actual code, not merely design intent:
- AD-1 (Profile location + re-export): confirmed.
- AD-2 (`PersistenceCompositionError` shape): confirmed, single `NotConfigured{capability,fix}` variant, no per-capability variant sprawl.
- AD-3 (`require_configured` as the one predicate): confirmed, called from both crates, no second parallel check found anywhere in the composition path (SC-8 satisfied).
- AD-4 (`try_build()` mirrors PROD-012 shape): confirmed shape-for-shape, `self` not `mut self` (correctly narrower, per design's stated difference).
- AD-5 (effect-store gate, conditional, `AppBuilder::profile` non-propagation documented): confirmed.
- AD-6 (no cross-layer bridge for event/snapshot refusal): confirmed — `observed_entity_runtime` returns `Result<_, PersistenceCompositionError>` absorbed by `?` in `build_runtime_with`'s `Box<dyn Error>` return, no `From` impl added.
- AD-7 (partial-config folded into Production gate, no separate check): confirmed, spec text matches (`production-composition-hardening/spec.md` "Partial Event/Snapshot Configuration..." requirement).
- AD-8 (profile travels on `EntityEventStores`): confirmed, private field, two constructors only.
- AD-9 (durable snapshot wiring, two typed instances not one shared): confirmed, `org_snapshot`/`user_snapshot` each their own `PostgreSQLSnapshotStore::new(pool.clone())`.
- AD-10 (two test assertions, not an `xtask` lint): confirmed, `production_profile_guard.rs` + one line in `durable_entity_progress_postgres.rs`.
- AD-11 (Approach C evaluated and deferred, not implemented): confirmed — no default-flip code exists anywhere, `Profile::Dev` remains `#[default]`.

## Issues

**CRITICAL**: None.

**WARNING**: None.

**SUGGESTION** (non-blocking, informational — flagged per the orchestrator's explicit request to surface known debt honestly):

1. `cargo fmt --check` reports one diff in `crates/service-sdk/src/app/mod.rs:275` (`record_app_started` call). Confirmed pre-existing on `develop` before PROD-013 branched (present in `git show develop:...`, untouched by any PROD-013 commit's diff). Environment/rustfmt-version drift unrelated to this change; not this change's responsibility to fix, but flagged so it does not get silently attributed to PROD-013 later.
2. `integration-tests/tests/infrastructure/schema_index_assertion.rs`'s `EXPECTED_PAIRS` table (covering `events`, `operation_reservations`, `operation_receipts`) does **not** include the `snapshots` table, even though migration 012 (already on the `develop` baseline, inherited from WU1 — not part of this change) applies the identical dual-partial-unique-index pattern to it. This is a real coverage gap in that assertion test, already known and documented as accepted debt per the orchestrator's brief — not introduced as a surprise by this verification, and explicitly not a blocker for this change.
3. `crates/persistence/src/postgres/repository.rs` (the `aggregates` table) still uses the non-null-safe `tenant_id = $2` pattern rather than `IS NOT DISTINCT FROM` — the same class of defect migration 012 fixed for `snapshots` on the `develop` baseline. Pre-existing, out of PROD-013's scope (its capability set is fixed at exactly three: event/snapshot/effect store — D-3), and already documented as accepted follow-up debt per the orchestrator's brief.
4. AD-11 (Approach C, the default-flip alternative) remains evaluated-and-deferred per D-7/OOS-5, as design.md records; no implementation task exists for it and none was expected. Not a gap — a deliberate, documented deferral.

## Verdict

**PASS.** All 39 tasks are functionally complete and match the code exactly as described in specs and design (including all evidence corrections EC-1/EC-2 and confirmed decisions AD-7/AD-8/AD-9). 4 of the 5 requested gates pass with exit 0; `cargo fmt --check` remains non-zero solely because of a verified pre-existing baseline drift on `develop`, unrelated to this change. No read-side gate exists, real or pseudo (D-4/OOS-2 hard constraint honored). The motivating bypass is closed for the reference app structurally, and the one remaining architectural boundary in the general framework is intentional, documented exactly as design.md mandated, and matches the residual risk the proposal itself already named and accepted (R-1) rather than silently left open.

**Next recommended phase**: `sdd-archive`.

## Key Learnings

1. `EntityEventStores`'s private `profile` field with exactly two constructors closes R-1 structurally for the reference app — stronger than the "one call plus one regression check" guarantee the proposal originally asked for.
2. The Docker-backed suite's exact result (43 passed / 0 failed / 1 ignored) matches design.md task 5.3's own reported stabilization number precisely, confirming the `flavor = "multi_thread"` migration holds under repeated runs against migration 012's null-safety fix, which is a `develop` baseline precondition (inherited from WU1), not something this WU introduces.
3. `AppBuilder::profile()` deliberately does not cascade to already-built entity runtimes registered via `.entity()` — this is a documented architectural boundary (AD-5), not a silent gap, and the code's doc comment states it verbatim as design.md mandated.
4. `schema_index_assertion.rs`'s `EXPECTED_PAIRS` table was not extended to cover the new `snapshots` dual-partial-unique-index pattern from migration 012, leaving a real (but pre-known, non-blocking) coverage gap in that regression test.
5. The single `cargo fmt --check` diff found in `app/mod.rs` predates PROD-013 entirely (confirmed present on the `develop` baseline commit), so it must not be misattributed to this change during archive.

---

## AD-12 Closure Verification (re-verify, WU8, `opsx/prod-013-wu8-durability-capability-check`)

**Context**: this section supplements the PASS above. It does not re-run the 10 checks already
confirmed there — none of that code changed. It verifies only the new WU8 work: the fix for
AD-12, the configuration-vs-durability gap `/code-review` found after the prior `sdd-verify`
PASS, plus the migration 012 dedup companion fix. Baseline for this pass: `opsx/prod-013-wu8-durability-capability-check`, which stacks all 8 work units (7 prior + WU8) on top of `develop @ a740d34`. Commits inspected: `e0fa699`..`057470e` (12 commits, including the WU8 chain `e503a30`→`057470e`).

### 1. The gap is real and is closed — confirmed by direct source read, not by trusting the apply report

- `crates/domain/src/persistence/event_store.rs:112-124` — `EventStore::is_durable(&self) -> bool { false }`, a default trait method with a doc comment naming AD-12. Confirmed verbatim.
- `crates/domain/src/persistence/snapshot.rs:39-49` — `Snapshot::is_durable(&self) -> bool { false }`, same pattern, same AD-12 reference. Confirmed verbatim.
- `crates/persistence/src/postgres/event_store.rs:422-425` — overrides to `true` ("A committed append survives process death"). Confirmed.
- `crates/persistence/src/postgres/snapshot.rs:152-155` — overrides to `true` ("A committed snapshot survives process death"). Confirmed.
- `crates/persistent-entity/src/builder.rs::validate_persistence()` (lines 290-305) — both `require_configured` calls now pass `self.event_store.as_ref().is_some_and(|s| s.is_durable())` and `self.snapshot_store.as_ref().is_some_and(|s| s.lock().is_durable())`, not `.is_some()`. Confirmed byte-for-byte against design.md AD-12's prescribed diff.
- `crates/service-sdk/src/runtime/builder.rs::validate_persistence_profile()` (lines 777-802) — checks `self.effect_state_store.as_ref().is_some_and(|s| s.capabilities().durable)`, reusing PROD-002's existing `EffectStoreCapabilities` rather than adding a new trait method, exactly as AD-12 specifies. Confirmed.

**The exact motivating scenario is now refused, exercised by a real test, not a mock that simulates the gate:**

```rust
EntityRuntimeBuilder::<TestEvent>::new()
    .profile(Profile::Production)
    .with_event_store(Arc::new(InMemoryEventStore::new()))
    .with_snapshot_store(Arc::new(Mutex::new(DurableStubSnapshotStore)))
    .try_build()
```

is asserted `Err` by `try_build_rejects_explicit_in_memory_event_store_under_production`
(`crates/persistent-entity/src/builder.rs:763-782`), and the symmetric case for the snapshot
store by `try_build_rejects_explicit_in_memory_snapshot_store_under_production` (lines 784-806).
Both call the real `EntityRuntimeBuilder`/`require_configured`/`is_durable` chain end to end —
no test double stands in for the gate itself, only for the store implementations under test
(`DurableStubEventStore`/`DurableStubSnapshotStore`, which exist solely to isolate one field at a
time, per design.md's own instruction). Ran both explicitly:

```
cargo test -p ego-persistent-entity try_build_rejects_explicit_in_memory_event_store_under_production
cargo test -p ego-persistent-entity try_build_rejects_explicit_in_memory_snapshot_store_under_production
```
→ both `ok`.

The effect-store equivalent is `validate_persistence_profile_rejects_explicit_in_memory_effect_store_when_executor_registered`
(`crates/service-sdk/src/runtime/builder.rs:3705-3729`), asserting `RuntimeError::PersistenceNotConfigured`
for `Profile::Production` + a registered executor + an explicit `InMemoryEffectStore`. Confirmed the
same way: real `RuntimeBuilder::try_build()` call, no mocked validator.

**Regression the fix itself required and disclosed** (task 9.6): two of the 39 prior tests
(`try_build_rejects_missing_snapshot_store_under_production` and the event-only half of
`try_build_rejects_partial_configuration_under_production`) had incidentally wired an explicit
`InMemoryEventStore` to isolate the snapshot-store assertion; under the new durability rule that
in-memory event store is itself refused first (event store is checked before snapshot store, per
AD-3's ordering), which would have flipped the expected error message. Confirmed the fix: both
now use a test-local `DurableStubEventStore` instead, and the original assertion in each test is
unchanged. This is an honest, disclosed side effect of tightening the rule, not a silent
weakening of test coverage.

### 2. Migration 012 dedup step — read and sound

`crates/persistence/src/postgres/migrations/012_fix_snapshots_tenant_null_uniqueness.sql` now opens
with a `DELETE FROM snapshots ... USING (SELECT id, ROW_NUMBER() OVER (PARTITION BY aggregate_id
ORDER BY version DESC, created_at DESC, id DESC) ...) WHERE tenant_id IS NULL AND rank > 1` block,
**before** `DROP INDEX IF EXISTS idx_snapshots_aggregate` and the two new
`CREATE UNIQUE INDEX ... ux_snapshots_identity_{tenant,systemwide}` statements. Ordering is correct:
dedup runs first, so a deployment with pre-existing duplicate NULL-tenant rows (the exact defect
this migration fixes) no longer fails outright on `CREATE UNIQUE INDEX`. The dedup keeps the
highest-`version` row per `aggregate_id` (ties broken by `created_at DESC, id DESC`), which matches
`load_snapshot`'s own `ORDER BY version DESC LIMIT 1` — so no observable behavior changes for any
caller going through `Snapshot::load_snapshot`. Scope is correctly restricted to `tenant_id IS NULL`
rows only: the old index already enforced uniqueness for non-null tenants, so no non-null row is at
risk of being a duplicate. This matches the design.md AD-12 companion-fix description exactly.

### 3. Gates re-run on this final branch state (all 8 work units)

| Command | Exit | Result |
|---|---|---|
| `cargo fmt --check` | 1 | Same single pre-existing diff at `crates/service-sdk/src/app/mod.rs:275` (`record_app_started` line-wrap) as the prior verify pass — confirmed still present on `develop` baseline, still untouched by any PROD-013 commit including the WU8 chain. Unchanged SUGGESTION, not a regression. |
| `cargo check --workspace` | 0 | Clean. |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | Clean, zero warnings. |
| `cargo test --workspace` | 0 | 137 test-result blocks (unit + doc-tests across every crate), every one `0 failed`. No regression from the WU8 diff. |
| Docker `run-suite` (`DOCKER_HOST=unix:///Users/pablogore/.colima/default/docker.sock`) | 0 | 43 passed, 0 failed, 1 ignored (the same pre-existing/documented Tier-1 exclusion) — byte-identical count to the prior verify pass, confirming migration 012's dedup addition changed nothing observable for a fresh database and that WU8 introduced no infrastructure regression. |

### 4. Phases 1-8 unaffected by WU8

Re-confirmed by full-suite pass above (0 failures anywhere) plus direct diff inspection: WU8's
commits (`e503a30`..`057470e`) touch only `crates/domain/src/persistence/{event_store,snapshot}.rs`
(new default method), `crates/persistence/src/postgres/{event_store,snapshot}.rs` (override),
`crates/persistent-entity/src/builder.rs` (call-site + new/adjusted tests),
`crates/service-sdk/src/runtime/builder.rs` (call-site + new tests), and the migration SQL file.
No file from Phases 1-8's diff (`e0fa699`..`3b1da9c`) was touched beyond these same files at the
same call sites the design already described changing. The 39 previously-green tasks' tests all
still pass (confirmed via the full-suite run above and task 9.6's explicit accounting of the two
tests that needed internal adjustment without weakening their assertions).

### Verdict for this re-verify

**PASS.** AD-12 is genuinely closed: the gate now checks `is_durable()`/`capabilities().durable`,
not mere presence, at all three call sites, verified against real source and exercised by tests
that call the actual builder chain end to end rather than a stand-in for the gate. Migration 012's
dedup step is correctly ordered and behavior-preserving. 4 of 5 gates exit 0 on the final
8-work-unit branch state; `cargo fmt` remains non-zero solely because of the verified pre-existing
baseline drift. Results are identical to the prior verify pass except for the new WU8 tests
themselves. WU8 introduces no new CRITICAL/WARNING/SUGGESTION; the previously recorded baseline
suggestions remain unchanged.

**Next recommended phase**: `sdd-archive` — once the architect merges the PR chain, the full
8-work-unit change is ready to close.

## Key Learnings (WU8 re-verify)

1. AD-12's fix pattern reused PROD-002's existing `EffectStoreCapabilities.durable` for the effect store and added a matching minimal `is_durable()` default method to `EventStore`/`Snapshot` rather than inventing a new capability struct, keeping the fix proportional to a single boolean per trait.
2. Tightening `require_configured`'s boolean argument from presence to durability silently flipped two pre-existing tests' expected error message, because they had incidentally wired an in-memory event store to isolate a snapshot-store assertion — disclosed and fixed with a durable test stub rather than weakening the assertion.
3. Migration 012's de-duplication step must run before `DROP INDEX`/`CREATE UNIQUE INDEX`, and must be scoped to `tenant_id IS NULL` rows only, since the pre-existing non-null-tenant index already guaranteed no duplicates there.
4. The Docker-backed suite's exact pass/fail/ignore counts staying identical across the WU7 and WU8 verify passes (43/0/1) is itself evidence that WU8 changed no observable runtime behavior for a fresh database.
