# CORE-018b Spec — Restrict RuntimeInner Construction to RuntimeBuilder

## Context

`crates/service-sdk/src/runtime/runtime_builder.rs` currently exposes
`RuntimeInner::new()` and `impl Default for RuntimeInner` as `pub`. Both
construct a fully-formed `RuntimeInner` outside `RuntimeBuilder`, bypassing
logger wiring, teardown registration, and security-provider setup.

This specification does **not** redesign `RuntimeBuilder` or add new
dependency-injection surface (`.with_adapter()` / `.with_config()` is
issue #120, a separate change). It restricts an existing, over-wide
construction path.

This is a **delta** against `openspec/specs/service-sdk/spec.md`, which
already documents `RuntimeBuilder` requirements (e.g. "RuntimeBuilder
optional security registration"). This change ADDS one requirement to that
spec: `RuntimeInner` is not publicly constructible.

---

# Scope

| Domain | Spec Type | Description |
|--------|-----------|-------------|
| `service-sdk` | Delta — ADDED requirement | `RuntimeInner` construction restricted to `RuntimeBuilder::build()` |

Base spec file: `openspec/specs/service-sdk/spec.md`

---

# Requirements

## Requirement: RuntimeInner Not Publicly Constructible

`RuntimeInner::new()` MUST be `pub(crate)`. Any `Default` implementation for
`RuntimeInner` MUST be either removed or `pub(crate)` — it MUST NOT be
`pub`. No public constructor for `RuntimeInner` may exist outside the
`service-sdk` crate.

The only construction path reachable from outside `crates/service-sdk` is
`RuntimeBuilder::build()` (via `RuntimeInner::new_with_logger`, already
`pub(super)`).

#### Scenario: External crate cannot construct RuntimeInner directly

- GIVEN a crate outside `service-sdk` (e.g. an application or integration
  test crate depending on `service-sdk` as a library)
- WHEN that crate attempts to call `RuntimeInner::new(...)` or
  `RuntimeInner::default()`
- THEN compilation fails with a visibility error

#### Scenario: RuntimeBuilder::build() remains the sole construction path

- GIVEN the `service-sdk` crate after this change
- WHEN `rg "RuntimeInner\s*\{|RuntimeInner::new\(|RuntimeInner::default\(\)" crates/` is run
- THEN every match resolves to `RuntimeBuilder::build()`'s internal call
  chain (`new_with_logger`) or a `#[cfg(test)]` / `pub(crate)` test helper
  inside `service-sdk`
- AND no match originates from a crate other than `service-sdk`

#### Scenario: In-crate test helper stays crate-private

- GIVEN a test inside `crates/service-sdk` needs a `RuntimeInner` state not
  reachable through `RuntimeBuilder::build()`
- WHEN such a helper is added
- THEN it is gated `#[cfg(test)]` and/or `pub(crate)`
- AND it is never re-exposed as `pub`

---

## Requirement: RuntimeBuilder::build() Behavior Is Unchanged

Restricting `RuntimeInner`'s constructors MUST NOT alter the observable
behavior of `RuntimeBuilder::build()` for correctly-built runtimes: logger
wiring, ordered teardown registration, and security-provider installation
behave identically before and after this change.

#### Scenario: Logger wiring unchanged

- GIVEN a `RuntimeBuilder` configured with `.with_logger(logger)`
- WHEN `.build()` is called
- THEN the resulting `Runtime`'s `RuntimeInner::logger()` returns the same
  logger instance as before this change

#### Scenario: Teardown ordering unchanged

- GIVEN a `RuntimeBuilder` with infrastructure registered that pushes
  teardown entries
- WHEN `.build()` is called and the runtime is later shut down
- THEN teardown entries drain in the same reverse-construction order as
  before this change

#### Scenario: Security provider installation unchanged

- GIVEN a `RuntimeBuilder` configured with `.with_security(authn, authz)`
- WHEN `.build()` is called
- THEN `RuntimeInner::authorization_provider()` returns the same provider as
  before this change

#### Scenario: Build without security still succeeds

- GIVEN a `RuntimeBuilder` with no `.with_security(...)` call
- WHEN `.build()` is called
- THEN a valid `Runtime` is returned with `security_providers == None`,
  identical to pre-change behavior

---

## Requirement: Known Call Sites Migrated to RuntimeBuilder

All call sites that construct `RuntimeInner` directly (via `::new()` or
`Default`) MUST be migrated to construct a `Runtime` through
`RuntimeBuilder::build()` instead. A call-site survey MUST re-verify the
full set immediately before implementation; known sites at proposal time:

- test module in `crates/service-sdk/src/runtime/builder.rs`
- `crates/service-sdk/tests/authorization_integration.rs`

#### Scenario: Test suite passes after migration

- GIVEN all known direct-construction call sites are migrated to
  `RuntimeBuilder::build()`
- WHEN `cargo test --workspace` is run
- THEN all tests pass, including `authorization_integration.rs` and the
  `builder.rs` test module

#### Scenario: No direct construction remains outside RuntimeBuilder

- GIVEN the migrated codebase
- WHEN `rg "RuntimeInner::new\(|RuntimeInner::default\(\)|RuntimeInner\s*\{" crates/service-sdk/tests crates/service-sdk/src/runtime/builder.rs` is run
- THEN zero matches reference direct field-literal or constructor
  construction outside `RuntimeBuilder::build()`'s internal chain

---

# Non-Goals

- Do not add `.with_adapter()` / `.with_config()` to `RuntimeBuilder`
  (issue #120).
- Do not touch kit-config wiring or host examples (issue #119 is independent).
- Do not add new authorization or tenant-enforcement logic.
- No behavioral change to correctly-built runtimes — visibility restriction
  and call-site migration only.

---

# Success Criteria

- `RuntimeInner::new()` and any remaining `Default` impl are not `pub`.
- `rg` finds no `RuntimeInner` construction outside `RuntimeBuilder::build()`
  and crate-internal test helpers.
- `cargo build --workspace` and `cargo test --workspace` pass after
  call-site migration.
- `openspec/specs/service-sdk/spec.md` gains this requirement once the
  change is archived.
