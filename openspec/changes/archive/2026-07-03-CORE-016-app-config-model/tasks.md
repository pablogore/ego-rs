# Tasks: CORE-016 — Application Configuration Model

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~420-480 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (Foundation: Validate trait) → PR 2 (existing-domain impls) → PR 3 (new config crates) → PR 4 (reference example) |
| Delivery strategy | ask-on-risk (default, not yet decided) |
| Chain strategy | pending |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | `Validate`/`ConfigError` contract in `ego-domain` | PR 1 | ~60 lines; base = main/tracker; no other crate depends on it yet, ships alone |
| 2 | `impl Validate` for `RuntimeConfig`, `JwtProviderConfig`, `EventBusConfig` | PR 2 | ~120 lines; depends on PR 1 merged (or PR 1 branch if chained) |
| 3 | New `DatabaseConfig` + `GrpcServerConfig` crates config modules | PR 3 | ~150 lines; depends on PR 1 only, independent of PR 2 |
| 4 | `examples/reference-app/` composition + pipeline test | PR 4 | ~150 lines; depends on PR 1-3 merged |

## Phase 1: Foundation — Library Contract

- [x] TASK-001 RED: add `crates/domain/src/config.rs` test module asserting a fixture type implementing `Validate` returns `Ok(())` when valid and `Err(ConfigError::Invalid { .. })` when invalid; assert `ConfigError` implements `std::error::Error` + `Display`. (Spec: Validation; Design: Library Contract)
- [x] TASK-002 GREEN: implement `pub trait Validate { fn validate(&self) -> Result<(), ConfigError>; }` and `ConfigError` (via `thiserror`, already a dep) in `crates/domain/src/config.rs` to pass TASK-001.
- [x] TASK-003 REFACTOR: add module docs to `config.rs`; add `pub mod config;` and re-export `pub use config::{Validate, ConfigError};` in `crates/domain/src/lib.rs`. Run `cargo test -p ego-domain`.

## Phase 2: Existing Domain Validation

- [x] TASK-004 RED: in `crates/persistent-entity/src/runtime.rs` tests, assert `RuntimeConfig::default().validate()` is `Ok`, and a config with `mailbox_capacity: 0` or `concurrency_budget: 0` is `Err`.
- [x] TASK-005 GREEN: `impl Validate for RuntimeConfig` in `crates/persistent-entity/src/runtime.rs` enforcing non-zero `mailbox_capacity`/`concurrency_budget` and non-empty `tenant_id` when `single_tenant_mode == false`. Add `ego-domain` dep to `crates/persistent-entity/Cargo.toml` if not already present.
- [x] TASK-006 RED: in `crates/security-jwt/src/config.rs` tests, assert default `JwtProviderConfig::validate()` is `Ok`, and `leeway_seconds` above a defined bound (e.g. `> 300`) or `expected_aud: Some(vec![])` is `Err`.
- [x] TASK-007 GREEN: `impl Validate for JwtProviderConfig` in `crates/security-jwt/src/config.rs` per TASK-006 rules. Add `ego-domain` dep to `crates/security-jwt/Cargo.toml` if not already present.
- [x] TASK-008 RED: in `crates/ego-scheduler/src/event_bus.rs` tests, assert `EventBusConfig::default().validate()` is `Ok`, and `capacity: 0` is `Err`.
- [x] TASK-009 GREEN: `impl Validate for EventBusConfig` in `crates/ego-scheduler/src/event_bus.rs` enforcing non-zero `capacity`. Add `ego-domain` dep to `crates/ego-scheduler/Cargo.toml` if not already present.
- [x] TASK-010 REFACTOR: run `cargo test -p ego-persistent-entity -p security-jwt -p ego-scheduler`, dedupe any repeated bound-check helper across the three `validate()` impls only if trivial (else leave — YAGNI).

## Phase 3: New Config Domains

- [x] TASK-011 RED: add `crates/persistence/src/config.rs` test module asserting `DatabaseConfig::default().validate()` is `Ok`, and empty `url` or `max_connections: 0` is `Err`.
- [x] TASK-012 GREEN: create `crates/persistence/src/config.rs` with `DatabaseConfig` (`Deserialize + Default`) fields `url: String`, `max_connections: u32`, plus `impl Validate for DatabaseConfig`. Wire `pub mod config;` + re-export in `crates/persistence/src/lib.rs`. Add `ego-domain` dep to `crates/persistence/Cargo.toml`.
- [x] TASK-013 RED: add `crates/transport/src/config.rs` test module asserting `GrpcServerConfig::default().validate()` is `Ok`, and `port: 0` or empty `bind_address` is `Err`.
- [x] TASK-014 GREEN: create `crates/transport/src/config.rs` with `GrpcServerConfig` (`Deserialize + Default`) fields `bind_address: String`, `port: u16`, plus `impl Validate for GrpcServerConfig`. Wire `pub mod config;` + re-export in `crates/transport/src/lib.rs`. Add `ego-domain` dep to `crates/transport/Cargo.toml`.
- [x] TASK-015 REFACTOR: run `cargo test -p persistence -p transport`; confirm no `kit-config` dependency was introduced in either crate's `Cargo.toml`.

## Phase 4: Reference Example (docs/example-only, no TDD cycle)

- [x] TASK-016 Add `examples/reference-app/` crate (`Cargo.toml` + `src/main.rs`) as a new workspace member in root `Cargo.toml`, depending on `ego-domain`, `persistent-entity`, `security-jwt`, `ego-scheduler`, `persistence`, `transport`.
- [x] TASK-017 In `examples/reference-app/src/main.rs`, define illustrative `AppConfig { runtime, jwt, scheduler, database }` composing the four domains, with `impl Validate for AppConfig` calling each subtree's `validate()` plus one cross-domain rule (per design.md Interfaces section).
- [x] TASK-018 In `examples/reference-app/src/main.rs`, wire the Host → `AppConfig` → service construction → `RuntimeBuilder` pipeline per design.md Data Flow (construct `AppConfig` directly in-process, bypassing kit-config — no new dependency added).

## Phase 5: Integration Verification

- [x] TASK-019 Integration test: `examples/reference-app/tests/pipeline.rs` asserting a valid in-memory `AppConfig` passes `validate()` and each constructed service accepts its typed subtree config; asserting an invalid subtree config causes `AppConfig::validate()` to return `Err` before any service is constructed.
- [x] TASK-020 Run full workspace check: `cargo test --workspace` and `cargo build --workspace`; confirm `RuntimeBuilder` call sites in `crates/service-sdk` are unchanged (no diff).

## Acceptance Criteria (per task)

Each TASK above is verifiable by: the RED sub-step fails before its GREEN counterpart, `cargo test -p <crate>` passes after GREEN, and the crate's `Cargo.toml` gains no `kit-config` dependency.
