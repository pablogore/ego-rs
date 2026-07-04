# Apply Progress: CORE-016 — Application Configuration Model

## Batch 1 — Phase 1: Foundation — Library Contract

Status: **done** (Phase 1 only, per work-unit / chained-PR scope)

### Tasks completed

- [x] TASK-001 RED — `crates/domain/src/config.rs` test module: `FixtureConfig` implementing `Validate`, asserting `Ok(())` on valid input, `Err(ConfigError::Invalid { .. })` on invalid input, `ConfigError: std::error::Error`, and `Display` output. Confirmed RED via `cargo test -p ego-domain` (compile failure: `ConfigError`/`Validate` undefined).
- [x] TASK-002 GREEN — implemented `pub trait Validate { fn validate(&self) -> Result<(), ConfigError>; }` and `pub enum ConfigError { Invalid { field: String, reason: String } }` via `thiserror::Error` (already a workspace dep of `ego-domain`) in `crates/domain/src/config.rs`.
- [x] TASK-003 REFACTOR — added module docs to `config.rs`; wired `pub mod config;` + `pub use config::{ConfigError, Validate};` and a module-table row in `crates/domain/src/lib.rs`. `cargo test -p ego-domain` green (180 passed, 4 new).

## Batch 2 (this session) — Phase 2: Existing Domain Validation

Status: **done** (Phase 2 only, per work-unit / chained-PR scope — TASK-004..010)

### Tasks completed

- [x] TASK-004 RED — `crates/persistent-entity/src/runtime.rs`: added `runtime_config_validate_tests` module asserting `RuntimeConfig::default().validate()` is `Ok`, and `mailbox_capacity: 0` / `concurrency_budget: 0` / (`single_tenant_mode: false` + empty `tenant_id`) are each `Err`. Confirmed RED via `cargo test -p persistent-entity` (compile failure: `validate` method not found).
- [x] TASK-005 GREEN — `impl ego_domain::Validate for RuntimeConfig` in `crates/persistent-entity/src/runtime.rs`: non-zero `mailbox_capacity`, non-zero `concurrency_budget`, non-empty `tenant_id` when `single_tenant_mode == false`. `ego-domain` was already a dependency in `crates/persistent-entity/Cargo.toml` — no Cargo.toml change needed. `cargo test -p persistent-entity --lib runtime_config_validate_tests` → 5/5 passed.
- [x] TASK-006 RED — `crates/security-jwt/src/config.rs`: added `jwt_provider_config_validate_tests` module asserting default `JwtProviderConfig::validate()` is `Ok`, `leeway_seconds: Some(301)` (bound `> 300`) is `Err`, and `expected_aud: Some(vec![])` is `Err`. Confirmed RED via `cargo test -p security-jwt` (compile failure: `validate` method not found).
- [x] TASK-007 GREEN — `impl ego_domain::Validate for JwtProviderConfig` in `crates/security-jwt/src/config.rs`: `leeway_seconds` bounded to `MAX_LEEWAY_SECONDS = 300`, `expected_aud` (when `Some`) must be non-empty. `ego-domain` was already a dependency in `crates/security-jwt/Cargo.toml` — no Cargo.toml change needed. `cargo test -p security-jwt --lib jwt_provider_config_validate_tests` → 5/5 passed.
- [x] TASK-008 RED — `crates/ego-scheduler/src/event_bus.rs`: added `event_bus_config_validate_tests` module asserting `EventBusConfig::default().validate()` is `Ok` and `capacity: 0` is `Err`. Confirmed RED via `cargo test -p ego-scheduler` (compile failure: `validate` method not found).
- [x] TASK-009 GREEN — `impl ego_domain::Validate for EventBusConfig` in `crates/ego-scheduler/src/event_bus.rs`: non-zero `capacity`. `ego-domain` was already a dependency in `crates/ego-scheduler/Cargo.toml` — no Cargo.toml change needed. `cargo test -p ego-scheduler --lib event_bus_config_validate_tests` → 2/2 passed.
- [x] TASK-010 REFACTOR — ran `cargo test -p persistent-entity -p security-jwt -p ego-scheduler` (192 total tests in that scope, 0 failed). Evaluated deduping the three `validate()` field-bound checks into a shared helper: each check has different field name, different bound expression (non-zero vs. numeric-bound vs. collection-empty vs. cross-field), and different reason string — a shared helper would need as many parameters as the inline check itself. Left as three independent inline impls (YAGNI), per task guidance. Also fixed a `clippy::items_after_test_module` lint introduced by placing the new `runtime_config_validate_tests` module before `EntityRuntime` — moved the test module to the end of `runtime.rs`, after all production items, matching the pattern already used in `crates/domain/src/config.rs`.

### Tasks NOT started at end of Batch 2 (later phases)

- [ ] Phase 3 (TASK-011..015) — new `DatabaseConfig` / `GrpcServerConfig` crates
- [ ] Phase 4 (TASK-016..018) — `examples/reference-app/`
- [ ] Phase 5 (TASK-019..020) — integration verification / full workspace confirmation of unchanged `RuntimeBuilder` call sites

## Batch 3 (this session) — Phase 3: New Config Domains

Status: **done** (Phase 3 only, per work-unit / chained-PR scope — TASK-011..015)

### Tasks completed

- [x] TASK-011 RED — `crates/persistence/src/config.rs`: added `DatabaseConfig { url: String, max_connections: u32 }` (hand-written `Default`: `url: "postgres://localhost:5432/ego"`, `max_connections: 10` — a derived `Default` would give an empty `url` and `max_connections: 0`, both invalid per spec, so `Default::default().validate()` could never be `Ok`) plus `database_config_validate_tests` module (no `impl Validate` yet). Confirmed RED via `cargo test -p ego-persistence --lib database_config_validate_tests` (compile failure: `no method named validate found for struct DatabaseConfig`, 3 errors).
- [x] TASK-012 GREEN — `impl ego_domain::Validate for DatabaseConfig` in `crates/persistence/src/config.rs`: non-empty `url`, non-zero `max_connections`. Wired `pub mod config;` + `pub use config::DatabaseConfig;` in `crates/persistence/src/lib.rs`. `ego-domain` was already a dependency in `crates/persistence/Cargo.toml` (added in an earlier, unrelated change) — no Cargo.toml change needed. `cargo test -p ego-persistence --lib database_config_validate_tests` → 3/3 passed.
- [x] TASK-013 RED — `crates/transport/src/config.rs`: added `GrpcServerConfig { bind_address: String, port: u16 }` (hand-written `Default`: `bind_address: "0.0.0.0"`, `port: 50051` — same rationale as TASK-011, a derived `Default` would be invalid by construction) plus `grpc_server_config_validate_tests` module (no `impl Validate` yet). Confirmed RED via `cargo test -p ego-transport --lib grpc_server_config_validate_tests` (compile failure: `no method named validate found for struct GrpcServerConfig`, 3 errors).
- [x] TASK-014 GREEN — `impl ego_domain::Validate for GrpcServerConfig` in `crates/transport/src/config.rs`: non-empty `bind_address`, non-zero `port`. Wired `pub mod config;` + `pub use config::GrpcServerConfig;` in `crates/transport/src/lib.rs`. `ego-domain` was already a dependency in `crates/transport/Cargo.toml` — no Cargo.toml change needed. `cargo test -p ego-transport --lib grpc_server_config_validate_tests` → 3/3 passed.
- [x] TASK-015 REFACTOR — ran `cargo test -p ego-persistence -p ego-transport` (6 new tests + full existing suites in both crates, 0 failed). No shared bound-check helper extracted between `DatabaseConfig::validate` and `GrpcServerConfig::validate` — each checks a different field name/type/reason string, matching the same YAGNI call made for the Batch 2 impls. Confirmed via `rg -i "kit-config" crates/persistence/Cargo.toml crates/transport/Cargo.toml` (no match) that neither `Cargo.toml` gained a `kit-config` dependency.

### Tasks NOT started (later phases, out of scope for this batch)

- [ ] Phase 4 (TASK-016..018) — `examples/reference-app/`
- [ ] Phase 5 (TASK-019..020) — integration verification / full workspace confirmation of unchanged `RuntimeBuilder` call sites

## TDD Cycle Evidence (Batch 2 — Strict TDD Mode active)

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| TASK-004/005 | `crates/persistent-entity/src/runtime.rs` (`runtime_config_validate_tests`) | Unit | N/A (new module; file's existing tests untouched) | ✅ Written | ✅ Passed (5/5) | ✅ 4 cases (mailbox=0, concurrency=0, tenant_id empty+multi-tenant, tenant_id set+multi-tenant) | ✅ Moved test module below production code to satisfy `items_after_test_module` |
| TASK-006/007 | `crates/security-jwt/src/config.rs` (`jwt_provider_config_validate_tests`) | Unit | N/A (new module) | ✅ Written | ✅ Passed (5/5) | ✅ 4 cases (leeway in-bound, leeway over-bound, aud empty, aud non-empty) | ➖ None needed — impl already minimal |
| TASK-008/009 | `crates/ego-scheduler/src/event_bus.rs` (`event_bus_config_validate_tests`) | Unit | N/A (new module) | ✅ Written | ✅ Passed (2/2) | ➖ Single invariant (capacity) — spec only defines one rule for this domain | ➖ None needed |

### Test Summary (Batch 2)

- **Total tests written**: 12 (5 + 5 + 2)
- **Total tests passing**: 12/12
- **Layers used**: Unit (12), Integration (0), E2E (0)
- **Approval tests** (refactoring): None — no refactoring tasks, only additive `impl Validate` blocks
- **Pure functions created**: 3 (`RuntimeConfig::validate`, `JwtProviderConfig::validate`, `EventBusConfig::validate` — all pure, no side effects, deterministic on `&self`)

## TDD Cycle Evidence (Batch 3 — Strict TDD Mode active)

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| TASK-011/012 | `crates/persistence/src/config.rs` (`database_config_validate_tests`) | Unit | N/A (new file, new crate module) | ✅ Written — confirmed 3 compile errors (`validate` not found) | ✅ Passed (3/3) | ✅ 3 cases (default valid, empty `url` invalid, `max_connections: 0` invalid) | ➖ None needed — impl already minimal |
| TASK-013/014 | `crates/transport/src/config.rs` (`grpc_server_config_validate_tests`) | Unit | N/A (new file, new crate module) | ✅ Written — confirmed 3 compile errors (`validate` not found) | ✅ Passed (3/3) | ✅ 3 cases (default valid, empty `bind_address` invalid, `port: 0` invalid) | ➖ None needed — impl already minimal |

### Test Summary (Batch 3)

- **Total tests written**: 6 (3 + 3)
- **Total tests passing**: 6/6
- **Layers used**: Unit (6), Integration (0), E2E (0)
- **Approval tests** (refactoring): None — no refactoring tasks, only additive `impl Validate` blocks
- **Pure functions created**: 2 (`DatabaseConfig::validate`, `GrpcServerConfig::validate` — both pure, no side effects, deterministic on `&self`)

## Files changed

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/config.rs` | Created (Batch 1) | `Validate` trait + `ConfigError` enum (`thiserror`), with unit tests (RED→GREEN fixture-based) |
| `crates/domain/src/lib.rs` | Modified (Batch 1) | Added `pub mod config;`, module-table row, and `pub use config::{ConfigError, Validate};` re-export |
| `crates/persistent-entity/src/runtime.rs` | Modified (Batch 2) | `impl Validate for RuntimeConfig` (non-zero capacities, non-empty `tenant_id` when multi-tenant) + `runtime_config_validate_tests` (5 tests) |
| `crates/security-jwt/src/config.rs` | Modified (Batch 2) | `impl Validate for JwtProviderConfig` (`leeway_seconds` ≤ 300s bound, non-empty `expected_aud` when `Some`) + `jwt_provider_config_validate_tests` (5 tests) |
| `crates/ego-scheduler/src/event_bus.rs` | Modified (Batch 2) | `impl Validate for EventBusConfig` (non-zero `capacity`) + `event_bus_config_validate_tests` (2 tests) |
| `openspec/changes/CORE-016-app-config-model/tasks.md` | Modified | Marked TASK-001..010 `[x]` (Batch 2), then TASK-011..015 `[x]` (Batch 3) |
| `crates/persistence/src/config.rs` | Created (Batch 3) | `DatabaseConfig` (`Deserialize`, hand-written `Default`) + `impl Validate for DatabaseConfig` (non-empty `url`, non-zero `max_connections`) + `database_config_validate_tests` (3 tests) |
| `crates/persistence/src/lib.rs` | Modified (Batch 3) | Added `pub mod config;` + `pub use config::DatabaseConfig;` |
| `crates/transport/src/config.rs` | Created (Batch 3) | `GrpcServerConfig` (`Deserialize`, hand-written `Default`) + `impl Validate for GrpcServerConfig` (non-empty `bind_address`, non-zero `port`) + `grpc_server_config_validate_tests` (3 tests) |
| `crates/transport/src/lib.rs` | Modified (Batch 3) | Added `pub mod config;` + `pub use config::GrpcServerConfig;` |

No `Cargo.toml` changes in Batch 2 — `ego-domain` was already a dependency in `persistent-entity`, `security-jwt`, and `ego-scheduler`. No `Cargo.toml` changes in Batch 3 either — `ego-domain` was already a dependency in `persistence` and `transport`. No `kit-config` dependency introduced anywhere. No other crate touched.

## Verification (Batch 2)

- `cargo test -p persistent-entity --lib runtime_config_validate_tests` → 5 passed, 0 failed.
- `cargo test -p security-jwt --lib jwt_provider_config_validate_tests` → 5 passed, 0 failed.
- `cargo test -p ego-scheduler --lib event_bus_config_validate_tests` → 2 passed, 0 failed.
- `cargo test -p persistent-entity -p security-jwt -p ego-scheduler` → all suites green (192 unit tests + integration/doc tests across the three crates, 0 failed).
- `cargo test --workspace` → **all suites green, 0 failed across the entire workspace** (every `test result: ok` line, no `FAILED`).
- `cargo clippy -p persistent-entity -p security-jwt -p ego-scheduler -p ego-domain --all-targets -- -D warnings` → **0 warnings/errors attributable to files touched in this batch** (`runtime.rs`, `config.rs`, `event_bus.rs`, `domain/src/config.rs`). One real lint (`clippy::items_after_test_module`) was caught and fixed by reordering the test module in `runtime.rs`.

## Verification (Batch 3)

- `cargo test -p ego-persistence --lib database_config_validate_tests` → 3 passed, 0 failed.
- `cargo test -p ego-transport --lib grpc_server_config_validate_tests` → 3 passed, 0 failed.
- `cargo test -p ego-persistence -p ego-transport` → all suites green (6 new unit tests + existing suites in both crates, 0 failed, including doc-tests).
- `cargo test --workspace` → **all suites green, 0 failed across the entire workspace** (every `test result: ok` line, no `FAILED`; e.g. `ego-domain` 180/180, `security-jwt` 113/113, `persistent-entity`+`event_bus` combined suite 192/192, `ego-persistence` 3/3, `ego-transport` 3/3).
- `cargo clippy -p ego-persistence -p ego-transport -p ego-domain --all-targets -- -D warnings` → **0 warnings/errors** (clean compile, no lints raised in `crates/persistence/src/config.rs`, `crates/transport/src/config.rs`, or either crate's `lib.rs`).
- `rg -i "kit-config" crates/persistence/Cargo.toml crates/transport/Cargo.toml` → no match (exit 1) — confirms TASK-015's acceptance criterion.
- `git status --porcelain crates/persistence crates/transport crates/domain` → only `crates/persistence/src/lib.rs` (M), `crates/transport/src/lib.rs` (M), `crates/persistence/src/config.rs` (new), `crates/transport/src/config.rs` (new) are this batch's diff; `crates/domain/src/lib.rs` (M) is pre-existing from an earlier session, untouched here.
- `cargo clippy --workspace -- -D warnings` (repo's `Makefile` convention) and `cargo clippy --workspace --all-targets -- -D warnings` both **fail before reaching this batch's crates**, due to a **pre-existing** `clippy::collapsible_match` error in `crates/service-sdk-macros/src/lib.rs:557` (untouched by CORE-016) and pre-existing `too_many_arguments` / `new_ret_no_self` warnings in `crates/persistent-entity/src/testing.rs` and `entity_ref_tokio.rs` (also untouched by this batch). **Verified via `git stash` of this batch's 4 changed files**: the identical `service-sdk-macros` failure reproduces on the pre-existing tree, confirming it predates and is unrelated to this change. Full unscoped clippy output is not achievable on this branch until those unrelated pre-existing lints are fixed elsewhere (tracked outside CORE-016 scope).

## Risks / Notes

- Working tree also shows `openspec/specs/domain/auth.md` as modified — this predates this session (not touched by this batch or Batch 1, unrelated CORE-011 doc work). Flagging for orchestrator awareness before commit staging.
- Per tasks.md's Review Workload Forecast, this change is planned as chained PRs (PR 1 = Foundation, PR 2 = existing-domain impls = this batch). This batch stayed strictly within PR 2 scope (~118 lines diff across 3 files), well under the 400-line budget risk flagged for the full change.
- Repo-wide `cargo clippy -- -D warnings` cannot currently pass end-to-end because of pre-existing lint debt in `service-sdk-macros`, `persistent-entity/testing.rs`, and `persistent-entity/entity_ref_tokio.rs` — none introduced by CORE-016 Phase 1 or Phase 2. This is a pre-existing repo condition, not a regression from this batch; recommend a separate hygiene ticket if a fully green `cargo clippy --workspace -- -D warnings` is required for CI.
- Phase 3 (Batch 3: `DatabaseConfig` / `GrpcServerConfig`) was independent of the Phase 2 batch (both depend only on PR 1/Phase 1) and is now also done — this batch's diff is ~90 lines across 4 files (2 new, 2 modified), well under the 400-line budget risk.
- `DatabaseConfig` and `GrpcServerConfig` both needed a **hand-written `Default`** (not `#[derive(Default)]`) because their invalid states (empty `String`, `0` numeric) coincide exactly with Rust's derived defaults for those types — a derived `Default` would make `Default::default().validate()` always `Err`, contradicting TASK-011/013's acceptance test. This mirrors the same pattern already used for `RuntimeConfig` and `EventBusConfig` in Batch 2.
- Next work units after this batch: Phase 4 (TASK-016..018, `examples/reference-app/` — PR 3/4 per the Suggested Work Units table, depends on PR 1-3/Phases 1-3 all being merged) and Phase 5 (TASK-019..020, integration verification). Both remain unstarted.

## Batch 4 (this session) — Phase 4 + Phase 5: Reference Example + Integration Verification

Status: **done** — TASK-016..020, closing out all 20 CORE-016 tasks.

### Tasks completed

- [x] TASK-016 — New workspace member `examples/reference-app/` (`Cargo.toml` + `src/main.rs` + `src/lib.rs`), added to root `Cargo.toml`'s `[workspace] members`. Verified real Cargo package names before wiring dependencies (the tasks.md text used approximate names): `persistent-entity` (not `persistence`), `security-jwt`, `ego-scheduler`, `ego-persistence` (not `persistence`), `ego-transport` (not `transport`), `ego-domain`, `ego-security-sdk`, `ego-service-sdk`. **Deviation from the literal task list**: `ego-transport` was intentionally NOT added as a dependency — TASK-017 explicitly permits skipping `GrpcServerConfig`/transport in the composed struct "if it complicates the example," and adding an unused dependency would be dead weight (YAGNI) with no illustrative value. Final deps: `ego-domain`, `persistent-entity`, `security-jwt`, `ego-security-sdk` (with `test-helpers` feature, for `AllowAllAuthorizationProvider`), `ego-scheduler`, `ego-persistence`, `ego-service-sdk`.
- [x] TASK-017 — `AppConfig { runtime: RuntimeConfig, jwt: JwtProviderConfig, scheduler: EventBusConfig, database: DatabaseConfig }` in `examples/reference-app/src/lib.rs`, with `impl ego_domain::Validate for AppConfig` calling all four subtrees' `.validate()` then one illustrative cross-domain rule: **"a multi-tenant runtime (`!runtime.single_tenant_mode`) requires `database.max_connections >= 5`"** — chosen over the prompt's literal suggestion ("scheduler capacity > 0 implies database configured") because `EventBusConfig::validate()` already guarantees `capacity > 0` whenever the subtree itself validates, making that specific example degenerate (always true post-validation, no real cross-domain signal). The tenant/connections rule genuinely spans two domains (`runtime` × `database`) and cannot be expressed by either subtree's own `validate()`.
- [x] TASK-018 — `pub fn build_runtime(config: &AppConfig) -> Result<Runtime, ConfigError>` in `lib.rs`: calls `config.validate()?` first (structural/domain/cross-domain gate before any service exists), then constructs a real `Arc<dyn AuthenticationProvider>` (`security_jwt::Hs256AuthenticationProvider::new(config.jwt.clone(), resolver, clock)` with a real `LocalKeyResolver`/`SystemClock`) and a real `Arc<dyn AuthorizationProvider>` (`ego_security_sdk::AllowAllAuthorizationProvider`, gated behind the crate's own `test-helpers` feature — explicitly a dev/test stand-in per that crate's doc comment, which is exactly the right fit for an illustrative example), then `RuntimeBuilder::new().with_security(authn, authz).build()`. **`RuntimeBuilder` receives only constructed services, never raw config** — matches spec.md's "Runtime Integration" requirement verbatim. `main()` is a thin 8-line wrapper calling `build_runtime(&AppConfig::default())`.
- [x] TASK-019 RED → GREEN — `examples/reference-app/tests/pipeline.rs`, 3 tests: (1) valid `AppConfig::default()` passes `.validate()` and `build_runtime()` succeeds; (2) an invalid subtree (`scheduler.capacity = 0`) fails `.validate()` and `build_runtime()` returns `Err` before any service is constructed; (3) the cross-domain rule (multi-tenant + `max_connections: 1`) fails `AppConfig::validate()` even though each subtree individually validates `Ok` in isolation. **Confirmed genuine RED**: temporarily deleted the cross-domain check from `lib.rs`, re-ran `cargo test -p reference-app --test pipeline` — test 3 failed as expected (`invalid_cross_domain_rule_fails_validate ... FAILED`), then restored the check and re-ran — all 3 green. This is the one true RED→GREEN cycle in this batch; TASK-016..018 are docs/example-only per tasks.md's Phase 4 header ("no TDD cycle").
- [x] TASK-020 — `cargo build --workspace` and `cargo test --workspace` both pass; `crates/service-sdk` shows **zero diff** (`git status --porcelain crates/service-sdk` → empty output), confirming design.md's decision that `RuntimeBuilder` needed no changes.

### Real API discovery (why `main.rs`/`lib.rs` don't match design.md's pseudocode literally)

design.md's Interfaces/Data-Flow snippets (`Hs256AuthenticationProvider::new(config.jwt)`, `Scheduler::new(config.scheduler)`, `Database::new(config.database)`) are illustrative pseudocode, not literal signatures — verified by reading the real code before writing any example code:

- `RuntimeBuilder` (`crates/service-sdk/src/runtime/builder.rs`): `RuntimeBuilder::new().with_security(Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>).build() -> Runtime` — confirmed it never takes raw config, exactly as design.md's "RuntimeBuilder left unchanged" decision states.
- `Hs256AuthenticationProvider::new` actually takes 3 args: `(config: JwtProviderConfig, resolver: Arc<dyn KeyResolver>, clock: Arc<dyn Clock>)`, not just config — used the real `LocalKeyResolver::new(JwtAlgorithm::Hs256, VerificationKey::Hmac(..))` and `ego_domain::SystemClock` (both real, already-exported types) to satisfy it.
- Neither `ego-scheduler` nor `ego-persistence` expose a `Scheduler`/`Database` type with a `new(config)` constructor at all — `ego-scheduler` only has `EventBusConfig` + bus plumbing (`SchedulerEventSender`/`Receiver`), `ego-persistence` only has `DatabaseConfig` + a `postgres` module (event store/snapshot/migrations, no generic `Database` handle). Used minimal `SchedulerService<'a>(&'a EventBusConfig)` / `DatabaseService<'a>(&'a DatabaseConfig)` stand-ins (marked with a `ponytail:` comment) that demonstrate "a service receives its typed subtree config" without inventing new public API inside those crates — out of scope for CORE-016, which only requires each domain to satisfy the `Validate` contract, not to gain a new service constructor.
- `AllowAllAuthorizationProvider` lives behind `ego-security-sdk`'s `test-helpers` Cargo feature (aliases `dev-providers`) — confirmed via `crates/security-sdk/Cargo.toml` and the type's own doc comment ("only compiled when the `dev-providers` feature..."). Enabled that feature on `reference-app`'s dependency edge only; no other crate's feature set changed.
- `RuntimeConfig` implements `Deserialize` only (no `Clone`/`Debug`) — `AppConfig` derives only `Default` (all four field types implement `Default`), not `Clone`/`Debug`, to match.
- `EventBusConfig` implements `Deserialize` only (no `Clone`) — `SchedulerService` holds `&EventBusConfig` by reference instead of cloning, since cloning was never actually required (`build_runtime` already borrows `config: &AppConfig` for its whole lifetime).

### Split into `lib.rs` + `main.rs`

Rust cannot import items from a bin-only crate in an integration test (`tests/pipeline.rs` needs `use reference_app::{AppConfig, build_runtime}`). Moved all illustrative logic (`AppConfig`, `Validate` impl, `SchedulerService`/`DatabaseService`, `build_runtime`) into `src/lib.rs`; `src/main.rs` is now an 8-line thin wrapper that just calls into the lib. This is a standard Rust pattern (lib + thin bin), not scope creep.

### TDD Cycle Evidence (Batch 4 — Strict TDD Mode active, Phase 5 only)

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| TASK-019 | `examples/reference-app/tests/pipeline.rs` | Integration | N/A (new crate, new test file) | ✅ Confirmed by deliberately removing the cross-domain check and re-running — `invalid_cross_domain_rule_fails_validate` failed as expected | ✅ Restored the check — all 3 tests passed | ✅ 3 cases (valid config, invalid single subtree, invalid cross-domain combination where each subtree alone is valid) | ➖ None needed — 3 tests, each asserting a distinct scenario, no duplication to extract |

### Test Summary (Batch 4)

- **Total tests written**: 3 (integration layer)
- **Total tests passing**: 3/3
- **Layers used**: Unit (0 new — Phase 4 is docs/example-only per tasks.md), Integration (3), E2E (0)
- **Pure functions created**: 1 (`AppConfig::validate`, pure, no side effects, deterministic on `&self`); `build_runtime` is impure by design (constructs services) but its error path (`config.validate()?`) is exercised without any side effect occurring first.

### Files changed (Batch 4)

| File | Action | Description |
|------|--------|-------------|
| `Cargo.toml` (workspace root) | Modified | Added `"examples/reference-app"` to `[workspace] members` |
| `examples/reference-app/Cargo.toml` | Created | New example crate; deps: `ego-domain`, `persistent-entity`, `security-jwt`, `ego-security-sdk` (`test-helpers` feature), `ego-scheduler`, `ego-persistence`, `ego-service-sdk`. No `ego-transport`, no `kit-config`. |
| `examples/reference-app/src/lib.rs` | Created | `AppConfig` composition + `impl Validate for AppConfig` (4 subtree checks + 1 cross-domain rule) + `SchedulerService`/`DatabaseService` stand-ins + `build_runtime()` (Host → AppConfig → services → RuntimeBuilder pipeline) |
| `examples/reference-app/src/main.rs` | Created | Thin entry point calling `reference_app::build_runtime(&AppConfig::default())` |
| `examples/reference-app/tests/pipeline.rs` | Created | 3 integration tests (valid path, invalid-subtree path, invalid-cross-domain path) |
| `openspec/changes/CORE-016-app-config-model/tasks.md` | Modified | Marked TASK-016..020 `[x]` — all 20 CORE-016 tasks now complete |

### Verification (Batch 4)

- `cargo build -p reference-app` → builds clean.
- `cargo run -p reference-app` → prints `reference-app: runtime constructed from AppConfig`.
- `cargo test -p reference-app` → 3/3 integration tests passed, 0 unit tests (none needed), doc-tests 0.
- RED confirmed: temporarily removed the cross-domain `if` block from `lib.rs`'s `Validate for AppConfig` impl, ran `cargo test -p reference-app --test pipeline` → `invalid_cross_domain_rule_fails_validate` FAILED (1 failed, 2 passed) as expected; restored the block, re-ran → 3/3 passed.
- `cargo clippy -p reference-app --all-targets -- -D warnings` → **fails**, but only due to pre-existing lint debt inside `persistent-entity` (`crates/persistent-entity/src/entity_ref_tokio.rs:76` `too_many_arguments`, `crates/persistent-entity/src/testing.rs:30` `new_ret_no_self`, `crates/persistent-entity/src/testing.rs:170` `too_many_arguments`) — `cargo clippy` lints the full local dependency graph, not just the target package, so `persistent-entity`'s pre-existing debt (already flagged in Batch 2/3 verification notes, unrelated to CORE-016) surfaces here too. **Confirmed zero new lints from this batch**: `cargo clippy -p reference-app --all-targets` (no `-D warnings`) shows `Checking reference-app ... Finished` with no warnings attributed to any `examples/reference-app/*` path. The only other warnings in the full non-`-D` run are 3 pre-existing `security-jwt` warnings in `authenticator.rs`/`validation.rs` (files untouched by this batch or any CORE-016 batch).
- `cargo build --workspace` → clean, 0 errors.
- `cargo test --workspace` → **all suites green, 0 failed** (every `test result: ok` line across ~65 test binaries/doc-test groups, roughly 777 tests total including the 3 new `reference-app` integration tests; no `FAILED` anywhere).
- `git status --porcelain crates/service-sdk` → **empty output** — confirms `RuntimeBuilder` call sites and all of `crates/service-sdk` are byte-for-byte unchanged, per design.md's "RuntimeBuilder left unchanged" decision and TASK-020's acceptance criterion.

### Risks / Notes (Batch 4)

- **All 20 CORE-016 tasks are now complete** (TASK-001 through TASK-020, Phases 1-5).
- `ego-transport`/`GrpcServerConfig` deliberately excluded from `examples/reference-app` — a documented, permitted deviation from TASK-016's literal dependency list (the task itself allows this via TASK-017's "skip if it complicates the example" clause). `GrpcServerConfig` still has its own `impl Validate` (Batch 3) and its own unit tests; it is simply not composed into the illustrative `AppConfig` in this example.
- `AllowAllAuthorizationProvider` requires the `test-helpers` (alias `dev-providers`) feature on `ego-security-sdk` — this is a new feature *activation* on an example-only dependency edge, not a new Cargo dependency and not a change to any shipped crate's default feature set. No production crate's `Cargo.toml` changed its own dependency features.
- Repo-wide `cargo clippy --workspace -- -D warnings` still cannot pass end-to-end due to the same pre-existing lint debt flagged in Batches 2/3 (`persistent-entity/testing.rs`, `persistent-entity/entity_ref_tokio.rs`) plus pre-existing `security-jwt` warnings — none introduced by any CORE-016 batch, including this one. Recommend a separate hygiene ticket if a fully green `cargo clippy --workspace -- -D warnings` is required for CI.
- This batch's diff: 1 line in root `Cargo.toml` (+ `Cargo.lock` regeneration), 4 new files in `examples/reference-app/` (~140 lines total), 2 lines flipped `[ ]` → `[x]` five times in `tasks.md`. Well under the 400-line budget risk.
