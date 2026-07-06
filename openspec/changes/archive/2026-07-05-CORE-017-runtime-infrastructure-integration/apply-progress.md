# Apply Progress: CORE-017 — Runtime Infrastructure Integration

Status: **all 24 tasks complete** (`crates/service-sdk` only). Applied across two
runs: Phases 1-3 (TASK-001 through TASK-017) in a prior session, Phases 4-5
(TASK-018 through TASK-024) in this session.

## Phase 1 — Foundation: Dependencies + ConfigurationProvider (TASK-001..004)

- `crates/service-sdk/Cargo.toml`: `serde` moved to `[dependencies]`;
  `serde_json` and `kitlogger-formatter` (git, same repo/branch as `kitlogger`)
  added as real dependencies. Also added `console-exporter` and
  `kitlogger-log-domain` as `[dev-dependencies]` (same git repo/branch) — not
  in design.md's File Changes table, but required to construct
  `Arc<ConsoleExporterImpl>` and `Severity` for the capture-buffer tests
  TASK-008/TASK-022 call for (kitlogger's crate root re-exports neither type).
- `crates/service-sdk/src/runtime/config_provider.rs` created:
  `ConfigurationProvider { root: serde_json::Value }`, `from_value`,
  `logging()`, `LoggingSettings` (`#[serde(default)]`), `LogFormatSetting`
  (`#[serde(rename_all = "lowercase")]`), `RuntimeInfraError` (thiserror:
  `ConfigInvalid`, `LoggerInit`, `Teardown`) — per design.md's Interfaces
  block verbatim.
- **Discovery** (documented in tasks.md TASK-004, carried forward here):
  `#[serde(default)]` on `LoggingSettings` only fills missing *fields* within
  an existing map. It does not rescue deserialization when the top-level
  `logging` key is entirely absent — `.get("logging").cloned().unwrap_or_default()`
  yields `Value::Null`, and a struct cannot deserialize from `null`. So an
  absent `logging` subtree is `ConfigInvalid`, not "all defaults" — hosts must
  supply at least `"logging": {}` to get field-level defaults. This is the
  literal, frozen design.md code, not something this change altered.

## Phase 2 — Logger Boundary Adapter (TASK-005..009)

- `crates/service-sdk/src/runtime/logger.rs` created: `build_logger(&LoggingSettings) -> Result<Option<Arc<KITLogger>>, RuntimeInfraError>`
  mapping each `LogFormatSetting` to `kitlogger_formatter::LogFormat`,
  returning `Ok(None)` when `enabled == false` with no `KITLogger`
  constructed; `logger.init()` failure maps to `LoggerInit { reason: format!("{e:?}") }`.
- `TeardownStack { entries: Vec<Arc<KITLogger>> }` (private, `pub(super)`):
  `new()`, `push()`, `drain()` — LIFO pop, collects the first error as
  `Teardown`, idempotent on an already-drained stack.
- **Documented deviation carried forward — TASK-008 (LIFO-ordering test)**:
  design.md's Testing Strategy row 6 suggested "stub/verify order with a
  capture logger." The real `ConsoleExporterImpl::flush()` is a stub that
  always returns `Ok(())` without touching writers, and `shutdown()` exposes
  no externally observable side channel to time-order two independent
  shutdowns from the outside. This is not black-box observable against the
  real kitlogger API as design.md anticipated. The test instead verifies the
  actual guarantee: `push` preserves insertion order in `entries` (asserted
  directly in the same module, which may access the private field), and
  `Vec::pop` is LIFO by language definition — so popping after two pushes
  always yields the second-pushed entry first — then confirms `drain()`
  performs real teardown (not a no-op) by asserting both loggers are
  unusable (`log()` returns `Err`) afterward.
- Also added (TASK-007 refinement): `logger_init_failure_maps_to_logger_init_error`,
  which drives a *real*, non-fabricated `AdapterError` (initializing the same
  shared `ConsoleExporterImpl` twice — an invalid `Running -> Running`
  lifecycle transition) through the identical `map_err` expression
  `build_logger` uses, since `build_logger` itself has no seam to trigger its
  own `init()` failure (it always constructs a fresh, `Uninitialized`
  exporter internally).

## Phase 3 — RuntimeBuilder + RuntimeInner Integration (TASK-010..017)

- `crates/service-sdk/src/runtime/builder.rs`: `RuntimeBuilder` gained
  `logger: Option<Arc<KITLogger>>` and `with_logger(mut self, logger: Arc<KITLogger>) -> Self`,
  mirroring `.with_security(..)` exactly. `build()` stays `pub fn build(self) -> Runtime`
  (infallible); it pushes the logger (if any) onto a `TeardownStack` and
  constructs `RuntimeInner` via `new_with_logger(..)`. `Runtime::shutdown(&self) -> Result<(), RuntimeInfraError>`
  drains `self.inner.teardown.lock().expect(..)`.
- `crates/service-sdk/src/runtime/runtime_builder.rs`: `RuntimeInner` gained
  `logger: Option<Arc<KITLogger>>` and `teardown: Mutex<TeardownStack>`
  fields; `new_with_logger(..)` constructor; `#[doc(hidden)] pub fn logger(&self) -> Option<&Arc<KITLogger>>`
  accessor (macro-facing, per the existing AD-7 pattern on
  `authorization_provider()`); `Default for RuntimeInner` sets
  `logger: None` and an empty `Mutex<TeardownStack>`.
- **Documented deviation carried forward — TASK-016 (ownership test)**:
  tasks.md's literal wording asked to assert `Arc::strong_count(&logger) == 1`
  after `rt.shutdown()` ("back to only the test's reference"). Tracing
  design.md's own frozen `build()`/`new_with_logger` code shows this is
  unreachable as specified: `RuntimeInner` retains its own **permanent**
  `logger: Option<Arc<KITLogger>>` field (so `.logger()` keeps working after
  shutdown) *in addition to* the separate clone `TeardownStack` holds for
  ordered teardown — two independent long-lived owners by design (File
  Changes table: "RuntimeInner gains `logger: Option<Arc<KITLogger>>` +
  `Mutex<TeardownStack>`"). `shutdown()` only drains the stack's clone; the
  true post-shutdown count is 2 (test's + `RuntimeInner.logger`), not 1. The
  test instead asserts the part of the contract that actually holds
  unconditionally: `Arc::strong_count(&logger) > 1` after `.build()`, and
  `Arc::strong_count(&logger)` strictly *decreases* after `.shutdown()` —
  this still catches a real leak (the property TASK-016 exists to verify)
  without asserting an unreachable exact value. (Previously also saved to
  engram as a design/reality mismatch.)
- TASK-017: `cargo test -p ego-service-sdk runtime` confirmed passing; no
  existing `RuntimeBuilder::new().build()` call site (including
  `examples/reference-app`) needed a change.

## Phase 4 — ServiceContext Access (TASK-018/019, this session)

- `crates/service-sdk/src/context/mod.rs`: added `logger: Option<Arc<KITLogger>>`
  field, `with_logger(mut self, logger: Arc<KITLogger>) -> Self`, and
  `logger(&self) -> Option<&KITLogger>`, mirroring the existing
  `security`/`with_security`/`security()` triplet exactly. `ServiceContext::new()`
  defaults `logger: None`.
- Tests added: `logger_is_none_by_default`, `with_logger_sets_logger`.
- **Side effect discovered and fixed**: `KITLogger` does not implement
  `Debug`. `ServiceContext` previously derived `#[derive(Debug, Clone)]`;
  adding the `logger` field broke that derive. Replaced it with a hand-rolled
  `Debug` impl (mirroring `RuntimeInner`'s own hand-rolled `Debug` in
  `runtime_builder.rs`, which already works around the identical constraint
  for the same field type) that reports `logger` as `self.logger.is_some()`
  rather than attempting to print the value.

## Phase 5 — Public Surface + Integration Verification (TASK-020..024, this session)

- **TASK-020**: `crates/service-sdk/src/runtime/mod.rs` already had
  `mod config_provider;`, `mod logger;`, and the re-exports of
  `ConfigurationProvider`, `LoggingSettings`, `LogFormatSetting`,
  `RuntimeInfraError`, `build_logger` from the prior (interrupted) apply run.
  Verified present; no duplicate `mod`/`pub use` added.
- **TASK-021** (bootstrap-path integration test) and **TASK-022**
  (shutdown-path integration test): added as an `integration_tests` module
  inside `crates/service-sdk/src/runtime/mod.rs`. Per
  `skills/testing/SKILL.md`'s Decision Gates table, both exercise only
  in-memory state (kitlogger's capture-buffer exporter, `serde_json::json!`
  values) — no real DB/broker/HTTP — so they are ordinary `#[cfg(test)]`
  modules, not files under `crates/integration-tests/` (which does not exist
  in this workspace and was not created).
  - `bootstrap_path_wires_logger_from_config_to_service_context`: drives
    `ConfigurationProvider::from_value(json!({"logging": {"enabled": true, "format": "json"}}))`
    → `.logging()` → `build_logger(&settings)` →
    `RuntimeBuilder::new().with_logger(logger).build()` →
    `ServiceContext::new().with_logger(rt.logger().unwrap().clone())` (updated
    post-review to use the public `Runtime::logger()` facade added after this
    task shipped, replacing the hidden `RuntimeInner::logger()` call it
    originally used);
    asserts `ctx.logger()` is `Some` and `Arc::ptr_eq` confirms it's the same
    logger the runtime holds. Manual `ServiceContext::new().with_logger(..)`
    construction is intentional (matches the existing manual
    `.with_security(..)` pattern used elsewhere in this codebase) — not a gap.
  - `shutdown_path_flushes_capture_buffer_with_no_lost_records`: builds a
    `KITLogger::with_exporter_and_format` wired to a `CaptureBuffer` writer,
    logs three records, wires it into a `Runtime` via `.with_logger(..)`,
    calls `rt.shutdown()`, and asserts all three records are present in the
    buffer and `shutdown()` returned `Ok(())`. **Grounding note** (same
    real-API constraint already documented for TASK-008):
    `ConsoleExporterImpl::export()` writes to the router synchronously on
    every `log()` call, and `shutdown()`'s flush is a documented no-op stub —
    so "no lost records" is verified as "already-written records survive
    shutdown intact and shutdown itself succeeds," not via an external flush
    side-channel kitlogger's stub doesn't provide.
- **TASK-023**: ran `cargo test --workspace` and `cargo build --workspace`
  for real (results below). Confirmed no `kit-config` crate exists in this
  workspace (nothing to check there beyond its absence — `fd -t d "kit-config" crates`
  returns nothing) and `kitlogger` itself is untouched (only consumed as a
  git dependency, no vendored/patched copy in-tree). `rg -l "kitlogger" --glob "**/Cargo.toml"`
  across the repo returns only `crates/service-sdk/Cargo.toml` — no other
  crate gained a new kitlogger-family git dependency.
- **TASK-024**: grep-based architectural conformance check. Searched
  `KITLogger::(new|with_format|with_config|with_exporter_and_format|default)\(`
  across `examples/` and `crates/` excluding `crates/service-sdk/` — zero
  matches: no application/service code constructs a `KITLogger` directly.
  Within `service-sdk` itself, direct construction exists only inside
  `#[cfg(test)]` fixtures (`context/mod.rs`, `runtime/builder.rs`,
  `runtime/mod.rs`'s `integration_tests`, `runtime/logger.rs`'s own test
  module) — consistent with the already-accepted TASK-010/014/016 test
  pattern — plus inside `build_logger` in `runtime/logger.rs` itself, which
  is the canonical, designated construction point. No production code
  outside `logger.rs` constructs a `KITLogger`.

## Final Verification Results (this session)

```
cargo build --workspace
  -> Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.80s
  -> 0 errors, 0 warnings surfaced

cargo test -p ego-service-sdk --lib
  -> test result: ok. 57 passed; 0 failed; 0 ignored
     (53 from Phases 1-3 + 4 new: logger_is_none_by_default,
      with_logger_sets_logger, bootstrap_path_wires_logger_from_config_to_service_context,
      shutdown_path_flushes_capture_buffer_with_no_lost_records)

cargo test --workspace
  -> every reported "test result:" line is "ok. ... 0 failed"
     across all crates and all doc-tests (ego-service-sdk lib: 57 passed;
     workspace total across all crates/doc-tests: 0 failed, 0 unexpected
     ignores beyond the 3 pre-existing #[ignore]'d doc examples)
```

No `kit-config`/`kitlogger` source modification. Only files under
`crates/service-sdk/` changed:

- `crates/service-sdk/Cargo.toml` (Phase 1)
- `crates/service-sdk/src/runtime/config_provider.rs` (Phase 1, new)
- `crates/service-sdk/src/runtime/logger.rs` (Phase 2, new)
- `crates/service-sdk/src/runtime/builder.rs` (Phase 3)
- `crates/service-sdk/src/runtime/runtime_builder.rs` (Phase 3)
- `crates/service-sdk/src/runtime/mod.rs` (Phase 3 re-exports; Phase 5 integration tests)
- `crates/service-sdk/src/context/mod.rs` (Phase 4)

## Scope Discipline Confirmed

No `with_configuration_provider`, no fallible `build()`, no `OutputInit`
variant, no tracing/metrics/otel/authz/service-discovery/hot-reload were
introduced — matches the proposal's Non Goals and the design's frozen
`Interfaces/Contracts` block.

## Post-Verify Fix

`sdd-verify`'s one SUGGESTION (`RuntimeInfraError` living in
`config_provider.rs` rather than a dedicated module) was addressed:
extracted to `crates/service-sdk/src/runtime/error.rs`, re-exported
unchanged from `runtime/mod.rs`. Not moved into the crate-wide
`crates/service-sdk/src/error/` module — that holds the unrelated
`ServiceError` business-error taxonomy, and `RuntimeInfraError` models
infrastructure lifecycle failures, a different concern. Re-verified:
`cargo build --workspace` clean, `cargo test -p ego-service-sdk --lib`
57/57 passing, no import or call-site changes needed elsewhere.
