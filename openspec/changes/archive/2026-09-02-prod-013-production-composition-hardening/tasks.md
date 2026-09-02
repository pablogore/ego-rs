# Tasks: PROD-013 — Production Composition Hardening

> Canonical / source of truth. Spanish review companion: `tasks.es.md` (1:1 identifiers).
> TDD is strict: every RED task must fail for the right reason before its paired GREEN task starts.
> AD-11 (Approach C) is confirmed in design.md as evaluated-and-deferred — no implementation
> task exists for it in this file (Phase 8.3 is verification-only).

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines (single PR, all work units) | ~800 lines (additions+deletions), ~15 files |
| Largest single work unit (WU4, reference-app wiring) | ~220 lines / 4 files |
| 400-line budget risk (as one PR) | **High** |
| 400-line budget risk (per chained work unit below) | Low (each WU stays ≤ ~220 lines) |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 → PR 2 → PR 3 → PR 4 → PR 5 → PR 6 → PR 7 (PR 7 independently mergeable) |
| Delivery strategy | ask-on-risk |
| Chain strategy | **stacked-to-main** — confirmed by the architect |

Decision needed before apply: Resolved
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High (single PR) — mitigated by chaining into 7 work units

**Why AD-8/AD-9 (WU4) and the Postgres runtime-flavor fix (WU5) are the pressure point**, stated
honestly rather than rounded down: WU4 wires two real `PostgreSQLSnapshotStore` instances into
`EntityEventStores::open`, adds a private `profile` field + accessor, makes `observed_entity_runtime`
fallible, and threads `stores.profile()` through `build_runtime_with`'s `App::builder()` chain — real
implementation across `examples/reference-app/src/lib.rs` plus consequence edits in its callers, not
validation-only code. WU5 is a correctness fix (`block_in_place` panics on a current-thread runtime),
not padding, and cannot be skipped once WU4 ships. Neither is safely compressible without hiding risk.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | `Profile` + `require_durably_configured` + `PersistenceCompositionError` in `persistent-entity`, plus the `is_durable()` capability declaration on `EventStore`/`Snapshot` | PR 1 | `cargo test -p persistent-entity profile::tests` | N/A — pure unit tests, no runtime needed | Delete `profile.rs`, the new error variant, the `is_durable()` trait methods, and the `pub mod profile;` line; nothing depends on it yet |
| 2 | `EntityRuntimeBuilder::try_build()` + profile-gated `build()` | PR 2 | `cargo test -p persistent-entity try_build` | N/A — unit-level, no real runtime | Revert `.profile()`, `validate_persistence()`, `try_build()`, and `build()`'s panic guard; `build()` reverts byte-for-byte, no call site touched |
| 3 | Effect-store gate on `RuntimeBuilder`/`AppBuilder` | PR 3 | `cargo test -p ego-service-sdk validate_persistence_profile` | `cargo test -p ego-service-sdk --test '*'` (CompositionError surfacing) | Revert the new `RuntimeError` variant, `validate_persistence_profile()`, `AppBuilder::profile()`; `effect_store()`'s existing fallback is untouched |
| 4 | Reference-app `EntityEventStores` profile + durable `PostgreSQLSnapshotStore` wiring | PR 4 | `cargo test -p reference-app entity_event_stores` | Docker-backed Postgres suite: `EntityEventStores::open(pool)` then inspect wired stores | Revert `EntityEventStores` fields/constructors and the `observed_entity_runtime`/`build_runtime_with` edits; `in_memory()` path is untouched by a revert |
| 5 | Migrate at-risk Postgres integration tests to `#[tokio::test(flavor = "multi_thread")]` | PR 5 | `cargo test -p integration-tests durable_entity_progress_postgres` (Docker) | Docker-backed Postgres suite — no substitute exists for this risk | Revert the flavor attribute changes; only safe once PR 4 is also reverted (coupled boundary) |
| 6 | AD-10 regression guards (Dev + Production) | PR 6 | `cargo test -p reference-app --test production_profile_guard` | Docker-backed Postgres suite for the `durable_entity_progress_postgres.rs` assertion; N/A for the Dev-side guard | Delete the new test file and the one added assertion; no production code touched |
| 7 | Persistence completeness rule + PROD-005 boundary docs (IS-9/IS-10) | PR 7 | N/A — documentation only | N/A — no executable surface | Revert the doc file |

---

## Phase 1: Foundation — `Profile` and the Shared Predicate (AD-1, AD-2, AD-3)

- [x] 1.1 RED — `crates/persistent-entity/src/profile.rs`: table-driven test `require_durably_configured_matrix` over {Dev, Production} × {durably configured, not} (4 cases); fails to compile.
- [x] 1.2 GREEN — create `profile.rs`: `Profile` enum (`Dev` default, `Production`), `require_durably_configured(profile, durably_configured, capability, fix)` (AD-1, AD-3). The parameter is named for what it must represent — durability, never mere presence — precisely so a call site computing it from `.is_some()` alone reads as wrong on sight.
- [x] 1.3 RED — `crates/persistent-entity/src/error.rs`: test asserting `PersistenceCompositionError::NotConfigured`'s message names the capability AND the fix call (mirrors PROD-012's `the_refusal_names_the_registration_and_the_opt_out`).
- [x] 1.4 GREEN — add `PersistenceCompositionError` (AD-2, `thiserror`) to `error.rs`.
- [x] 1.5 Wire `pub mod profile;` in `persistent-entity/src/lib.rs`; add `pub use persistent_entity::profile::Profile;` in `service-sdk/src/runtime/mod.rs` (AD-1 re-export).
- [x] 1.6 GREEN — add `fn is_durable(&self) -> bool { false }` as a default method on `EventStore` (`crates/domain/src/persistence/event_store.rs`) and the structurally identical method on `Snapshot` (`crates/domain/src/persistence/snapshot.rs`); override to `true` in `PostgreSQLEventStore`/`PostgreSQLSnapshotStore`. This is the durability signal `require_durably_configured`'s callers compute their boolean from — AD-3's "pass the answer, not the builder" only holds if the answer itself is honest, and a store's own trait method is the one thing that can be asked without downcasting to a concrete type (which would also blind the gate to any third-party durable implementation).
- [x] 1.7 RED — `crates/persistent-entity/src/profile.rs`: `presence_alone_is_not_durability` — pins the exact regression a reviewer flagged before this shipped: `Some(InMemoryEventStore::new()).is_some()` and `Some(PostgreSQLEventStore::open(pool)).is_some()` must not be the two inputs `require_durably_configured` receives.

## Phase 2: `EntityRuntimeBuilder` Gate + `try_build()` (AD-4, AD-6, AD-7)

- [x] 2.1 RED — `builder.rs`: `try_build_rejects_missing_event_store_under_production` names event store + `.with_event_store()` (SC-1).
- [x] 2.2 RED — `try_build_rejects_missing_snapshot_store_under_production` (SC-2).
- [x] 2.3 RED — `try_build_rejects_partial_configuration_under_production` — event set / snapshot missing AND the reverse, identifying whichever is missing (SC-6, AD-7 subsumption, EC-1's asymmetric site 15).
- [x] 2.4 RED — `dev_profile_builds_on_nothing_configured` — `Profile::Dev`, nothing configured, still succeeds on in-memory (SC-5).
- [x] 2.5 RED — `build_panics_on_same_condition_try_build_refuses` — `Profile::Production`, missing capability, `build()` panics with the refusal's message.
- [x] 2.6 GREEN — add `.profile()`, `validate_persistence()` (event store checked before snapshot store, per AD-3's ordering), `try_build()` (validate-before-delegate); `build()` calls `validate_persistence()` and panics on `Err` (AD-4/AD-6). No `From<PersistenceCompositionError>` bridge added — the event/snapshot refusal returns to the host only (AD-6). Computes its two `durably_configured` booleans from `self.event_store.as_ref().is_some_and(|s| s.is_durable())` / the snapshot equivalent — never `.is_some()` (AD-3, 1.6).
- [x] 2.7 SC-7 migration check — run `cargo build --workspace` and `cargo test --workspace`; confirm all 67 existing `EntityRuntimeBuilder::new()` call sites (25 files, re-verified in design.md) compile and pass with **zero source edits**. `Profile::Dev` defaulting is what makes this true; any failure here is a design deviation to flag, not to silently patch.
- [x] 2.8 RED — `try_build_rejects_explicit_in_memory_event_store_under_production` and `try_build_rejects_explicit_in_memory_snapshot_store_under_production`: `Profile::Production` with an *explicitly wired* `InMemoryEventStore`/`InMemorySnapshotStore` must be refused, not just a missing one — `is_some()` cannot tell it apart from a durable store, only `is_durable()` can, and this proves the gate actually calls it. Each test isolates one capability with a `DurableStubEventStore`/`DurableStubSnapshotStore` (declares `is_durable() -> true`, every other method `unreachable!()`) standing in for the *other* capability, so an `InMemory*` there would not silently mask which check the test is about.
- [x] 2.9 GREEN — the 2.6 implementation already satisfies 2.8; no further code change.
- [x] 2.10 Deviation found and fixed — `try_build_rejects_missing_snapshot_store_under_production` and the event-only half of `try_build_rejects_partial_configuration_under_production` (2.2/2.3) each wired an explicit `InMemoryEventStore` incidentally, to isolate the snapshot-store check. Under the durability rule, that in-memory event store is itself refused (checked first, per AD-3), which would have flipped both tests' expected error message from "snapshot store" to "event store". Fixed by wiring the 2.8 `DurableStubEventStore` in their setup instead, preserving each test's original isolation intent unchanged; no assertion altered.

## Phase 3: Effect-Store Gate on `RuntimeBuilder`/`AppBuilder` (AD-5)

- [x] 3.1 RED — `service-sdk/src/runtime/builder.rs`: `validate_persistence_profile_rejects_missing_effect_store_when_executor_registered` (SC-3).
- [x] 3.2 RED — `validate_persistence_profile_ok_when_no_executor_registered` (EC-2's conditional gate — no executor means nothing constructed, nothing volatile).
- [x] 3.3 RED — `build_and_try_build_agree_on_persistence_profile_validation`, mirroring the existing idempotency-agreement test.
- [x] 3.4 GREEN — add `profile` field + `.profile()` to `RuntimeBuilder`; add `validate_persistence_profile()` (AD-5), called from `build()` and `try_build()` in the same slot as `validate_idempotency()`; checks `effect_state_store` only (not `effect_dedup_store` — both are always set together by `with_effect_store`, per the existing `debug_assert_eq!`). Computes its `durably_configured` boolean from `self.effect_state_store.as_ref().is_some_and(|s| s.capabilities().durable)` — reuses PROD-002's existing `EffectStoreCapabilities` (already `durable: false` by default, already `true` on every Postgres-backed store) rather than adding a new trait method; never `.is_some()` (AD-3).
- [x] 3.5 GREEN — add `RuntimeError::PersistenceNotConfigured(#[from] PersistenceCompositionError)` in `runtime_builder.rs`.
- [x] 3.6 GREEN — add thin `AppBuilder::profile()` in `app/mod.rs`, mirroring `effect_store()`'s delegation shape; doc comment MUST state it does not propagate to already-built entity runtimes (`AppBuilder::entity()` receives a finished `Arc<EntityRuntime<E>>` — that gate already ran).
- [x] 3.7 RED+GREEN — integration test in `crates/service-sdk/tests/`: the effect-store refusal surfaces as `CompositionError::Validation` through `AppBuilder::build()`.
- [x] 3.8 RED+GREEN — `try_build_rejects_an_explicit_in_memory_effect_store_under_production`: `Profile::Production` with a registered executor and an *explicitly wired* `InMemoryEffectStore` must be refused, not just a missing one — the effect-store counterpart of 2.8, proving `validate_persistence_profile()` reads `capabilities().durable` and not presence.

## Phase 4: Reference App — `EntityEventStores` Profile + Durable Snapshot Wiring (AD-8, AD-9)

Current baseline (verified in `examples/reference-app/src/lib.rs`): `EntityEventStores` has only
`org`/`user` event-store fields; `observed_entity_runtime` (line 488) takes no snapshot store or
profile and calls `.build()`; `compose_entity_runtimes` (line 452) and `build_runtime_with` (line
567, calling `observed_entity_runtime` directly at lines 649/654) both need updating.

- [x] 4.1 RED — test: `EntityEventStores::in_memory().profile() == Profile::Dev`.
- [x] 4.2 RED (integration, Docker) — test: `EntityEventStores::open(pool).await?.profile() == Profile::Production`.
- [x] 4.3 GREEN — add private `profile: Profile` field + `pub fn profile(&self) -> Profile` to `EntityEventStores`; `in_memory()` sets `Profile::Dev`, `open()` sets `Profile::Production` (AD-8).
- [x] 4.4 RED — test: `EntityEventStores::open(pool)`'s snapshot stores are backed by `PostgreSQLSnapshotStore`, asserted behaviorally (e.g. a written snapshot survives a fresh read against the same pool), not by type-check alone.
- [x] 4.5 GREEN — add `org_snapshot`/`user_snapshot: Arc<Mutex<dyn Snapshot + Send>>` fields; `open(pool)` constructs **two typed** `PostgreSQLSnapshotStore` instances over the shared pool (not one shared `Arc` — mirrors the existing per-aggregate-typed-instance rationale on `EntityEventStores`); `in_memory()` constructs two `InMemorySnapshotStore`s (IS-13).
- [x] 4.6 GREEN — `observed_entity_runtime` (line 488) gains a snapshot-store parameter and calls `EntityRuntimeBuilder::try_build()`, returning `Result` (AD-8 consequence).
- [x] 4.7 GREEN — update both call sites: `compose_entity_runtimes` (line 452, calls at 464/469) and `build_runtime_with` (line 567, calls at 649/654) to pass the matching snapshot store and propagate the `Result`.
- [x] 4.8 Decision task — `compose_entity_runtimes` stays on `.build()` (infallible), since the profile field is private and `open()` always supplies every store, so no constructible input can make it refuse (AD-8's "tasks should pick one and say which"); document this choice in a code comment citing AD-8.
- [x] 4.9 GREEN — in `build_runtime_with`, capture `let profile = stores.profile();` **before** `stores.org`/`stores.user`/snapshot fields are moved into the `observed_entity_runtime` calls (lines 649/654), then call `.profile(profile)` on the `App::builder()` chain (line 683) instead of a hardcoded literal.
- [x] 4.10 Fix consequence call sites — update `metrics_reach_one_backend.rs:209` and any other caller of `compose_entity_runtimes`/`observed_entity_runtime` whose signature changed, with zero behavior change on the `Profile::Dev` path.

## Phase 5: Postgres `block_in_place` / Runtime-Flavor Risk (AD-9 landmine)

- [x] 5.1 Audit — corrected from the ≥100-events threshold premise (falsified: `PersistenceFacade::load_for_recovery` calls `load_snapshot` unconditionally on every entity activation, no threshold gate). Re-ran `run-suite` fresh on this branch and confirmed the exact failing set by name: `durable_entity_progress_postgres::{an_organization_receipt_outlives_the_runtime_that_confirmed_it, a_user_receipt_outlives_the_runtime_that_confirmed_it, each_aggregate_keeps_its_own_receipt_under_one_operation_key}`, `concurrent_replicas_postgres::two_replicas_racing_one_key_yield_exactly_one_execution`, `dual_aggregate_crash_recovery_postgres::a_crash_between_the_aggregates_is_recovered_by_takeover`, `entity_event_stores_wiring_postgres::a_written_snapshot_survives_a_fresh_open_against_the_same_pool` — 6 failing, 37 passing, matching WU4's report exactly. Grepped every Postgres integration test file for `EntityEventStores::open`: only 4 files use it (`durable_entity_progress_postgres.rs`, `dual_aggregate_crash_recovery_postgres.rs`, `concurrent_replicas_postgres.rs`, `entity_event_stores_wiring_postgres.rs`) — no other file is on the vulnerable path. Preventively also migrated `dual_aggregate_crash_recovery_postgres::child_crashes_after_the_org_receipt_is_confirmed` (passed in this run in isolation, but panics when spawned as the parent test's child subprocess with the crash env var set — same current-thread runtime, same `block_in_place` call, just gated by a code path the direct run doesn't take). `single_aggregate_crash_recovery_postgres.rs` audited and confirmed NOT on the vulnerable path: it builds its runtime via `EntityRuntimeBuilder::new()` directly, never through `EntityEventStores`/`build_runtime_with`, so it never constructs a `PostgreSQLSnapshotStore` — migrating it would be a structural rewrite, not a trivial one, so left as-is. `durable_entity_progress_postgres::the_instant_an_event_happened_survives_append_and_load` and `entity_event_stores_wiring_postgres::opened_stores_declare_profile_production` use `EntityEventStores::open` but never activate an entity (raw store/profile access only) — not migrated, no `load_for_recovery` call in their path.
- [x] 5.2 GREEN — migrated all 7 affected test functions across the 4 files above to `#[tokio::test(flavor = "multi_thread")]`. `rt-multi-thread` was already enabled on `integration-tests/Cargo.toml`'s `tokio` dependency (both `[dependencies]` and `[dev-dependencies]`); no `Cargo.toml` change needed.
- [x] 5.3 Verify (Docker-required) — first `run-suite` after the attribute migration surfaced a second, independent, pre-existing bug unmasked by removing the panic: `PostgreSQLSnapshotStore`'s SQL used `tenant_id = $2` (never true when `$2` is NULL) and a single `UNIQUE (aggregate_id, tenant_id)` index (Postgres never treats two NULL tenants as conflicting), so the systemwide-scope snapshot a test just wrote could never be found again — root-caused and fixed following the codebase's own established AD-1 pattern (two partial unique indexes over complementary NULL predicates, already used by `events` and `operation_receipts`): new migration `012_fix_snapshots_tenant_null_uniqueness.sql`, `IS NOT DISTINCT FROM` in both SELECTs, and a tenant/systemwide-branched `INSERT ... ON CONFLICT` each targeting its own partial index. The migration also runs a `ROW_NUMBER() OVER (PARTITION BY aggregate_id ORDER BY version DESC, created_at DESC, id DESC)` de-duplication delete over `tenant_id IS NULL` rows before creating the new unique indexes — only that partition can hold duplicates (the pre-existing index already enforced uniqueness for non-null tenants), and `CREATE UNIQUE INDEX` would otherwise fail outright on any deployment old enough to have hit this bug; keeps the highest-version row per `aggregate_id`, matching what `load_snapshot` already treats as current, so this is behavior-preserving for a deployment that reads through that method. Re-ran `run-suite` three times after both fixes: 43 passed / 0 failed / 1 ignored (pre-existing, documented), stable across runs. `cargo test --workspace` (in-memory) unaffected at 0 failures throughout.

## Phase 6: AD-10 Regression Guards

- [x] 6.1 RED+GREEN — new `examples/reference-app/tests/production_profile_guard.rs`: asserts `EntityEventStores::in_memory().profile() == Profile::Dev` AND that `build_runtime_with` over in-memory stores still builds (Dev-path guard, SC-5 at the composition root).
- [x] 6.2 RED+GREEN — add one assertion in the existing `integration-tests/tests/infrastructure/durable_entity_progress_postgres.rs`, immediately after its existing `EntityEventStores::open()` call (`:94`): `assert_eq!(stores.profile(), Profile::Production);` (Production-path guard).

## Phase 7: Documentation (IS-9, IS-10)

- [x] 7.1 Document the persistence completeness rule verbatim (proposal's Architecture Principle section) as forward-looking guidance; explicitly state PostgreSQL is not in violation today (SC-10).
- [x] 7.2 Document the PROD-005 boundary: this spec rejects the bootstrap itself before anything starts; PROD-005 signals the health of an application that already started (SC-10).

## Phase 8: Final Verification

- [x] 8.1 Run `cargo test --workspace`; confirm zero new failures (SC-7); confirm all 67 `EntityRuntimeBuilder::new()` call sites across 25 files compile unmodified.
- [x] 8.2 Run `cargo clippy --workspace -- -D warnings`; confirm no function introduced exceeds cyclomatic complexity 10.
- [x] 8.3 Verification only, no implementation — confirm AD-11 (Approach C) remains recorded as evaluated-and-deferred in design.md; nothing to implement here.

## Phase 9: Final Reconciliation

**Reconciliation note**: this phase originally implemented the configuration-vs-durability
fix (`is_durable()`, the `require_configured` → `require_durably_configured` rename, the two
`DurableStub*` test stubs, and the migration de-duplication step) as a WU8 addendum, discovered
by `/code-review` after `sdd-verify` had already returned PASS on WU1–WU7. The architect's
explicit instruction was not to ship the fix as a later patch: `require_durably_configured` and
`is_durable()` had to be the design Phases 1–3 shipped from the start, not something bolted on
six PRs later. The fix was relocated there (now 1.6, 1.7, 2.6, 2.8–2.10, 3.4, 3.8, and the
migration de-duplication step folded into 5.3) via `git merge` propagated forward through every
downstream work unit, so no PR in the final stack ever contains the weaker, presence-only rule
— not even transiently. What remains here is re-verification only.

- [x] 9.1 Confirm every test the relocated fix depends on passes with the mechanism now
  living at its origin: `cargo test -p persistent-entity` and `cargo test -p ego-service-sdk`.
- [x] 9.2 Verify (Docker-required) — re-run `run-suite`; confirm the migration still applies
  cleanly and all counts hold (43 passed / 0 failed / 1 ignored).
- [x] 9.3 Final gates — `cargo fmt --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, Docker `run-suite`.
