# CORE-018a Spec — Real kit-config Host Example in reference-app

## Context

The CORE-016 frozen constraint — "RuntimeBuilder MUST NOT receive raw
configuration values"
(`openspec/changes/archive/2026-07-03-CORE-016-app-config-model/spec.md:148`)
— has only ever been demonstrated with hand-simulated JSON
(`crates/service-sdk/examples/logging_bootstrap.rs`). No code in ego-rs
proves the contract end-to-end with a real config source. `examples/reference-app`
is the composition root the CORE-016 audit (`docs/core-016-config-audit.md`)
says kit-config should land at, yet its `build_runtime()` wires only security
providers and its doc comment still claims kit-config is "intentionally out
of scope".

This specification does **not** redesign `ConfigurationProvider`,
`build_logger`, or `RuntimeBuilder` — those are already correct per
CORE-016/017. It adds a requirement that a real host example proves the
existing contract.

This is a **delta** against `openspec/specs/service-sdk/spec.md`. It ADDS
requirements to that spec; it does not modify existing ones.

---

# Scope

| Domain | Spec Type | Description |
|--------|-----------|-------------|
| `service-sdk` | Delta — ADDED requirements | Reference host example proves the kit-config-to-`RuntimeBuilder` materialization contract with a real config source |

Base spec file: `openspec/specs/service-sdk/spec.md`

---

# Requirements

## ADDED Requirements

### Requirement: Reference Host Example Materializes Configuration Through kit-config

The reference host example (`examples/reference-app`) MUST materialize
application configuration through `kit-config` at its composition root,
before any `RuntimeBuilder` construction begins. It MUST hand `RuntimeBuilder`
only materialized configuration, delivered through `ConfigurationProvider` —
never a raw configuration source (unparsed file, raw environment map, or
config-loading intermediate).

This confirms, with a real example, the frozen constraint already established
in `openspec/changes/archive/2026-07-03-CORE-016-app-config-model/spec.md:148`.
It does not redefine that constraint.

#### Scenario: build_runtime wires real kit-config output

- GIVEN `examples/reference-app` depends on `kit-config` as a git dependency
- WHEN `build_runtime()` executes
- THEN configuration is materialized via `kit-config`, delivered to
  `RuntimeBuilder` through `ConfigurationProvider`, and a logger derived from
  it is installed via `.with_logger(...)`

#### Scenario: No raw configuration source reaches RuntimeBuilder

- GIVEN the reference-app composition root after this change
- WHEN every value passed into `RuntimeBuilder`'s builder methods is reviewed
- THEN none of them is an unparsed config source — only materialized
  configuration delivered via `ConfigurationProvider` reaches it

#### Scenario: Existing framework contract remains untouched

- GIVEN `crates/service-sdk`'s `ConfigurationProvider`, `build_logger`, and
  `RuntimeBuilder` implementations
- WHEN this change is applied
- THEN `crates/service-sdk` and `crates/service-sdk/examples/logging_bootstrap.rs`
  show zero diff

---

## Non-Normative Notes

The doc-comment rewrite (removing the stale "kit-config is intentionally out
of scope" claim) and the precedence-limitation documentation are change-level
deliverables, not formal service-sdk contract requirements — they mandate
prose inside one example file, not framework behavior. They are tracked as
Proposal Success Criteria (`proposal.md`) and verified at PR review, not
merged into this domain spec as ADDED Requirements. Only the materialization
contract above (the negative constraint: `RuntimeBuilder` never receives raw
configuration) is a framework-level guarantee worth freezing here.

---

# Non-Goals

- Do not change `crates/service-sdk` (`ConfigurationProvider`, `build_logger`,
  `RuntimeBuilder` are already correct per CORE-016/017) or `kit-config`
  itself.
- `crates/service-sdk/examples/logging_bootstrap.rs` stays exactly as-is.
- No new example crate or bin — extend `reference-app` only.
- Logging config only: no DB config stub, no other typed config view, no
  custom configuration source built to override kit-config's precedence.

---

# Success Criteria

- `examples/reference-app` builds and its tests pass with real kit-config
  loading.
- `build_runtime()` demonstrates kit-config → materialized configuration →
  `ConfigurationProvider` → `build_logger()` → `RuntimeBuilder`.
- No unresolved/raw configuration source reaches `RuntimeBuilder`.
- The stale "out of scope" doc comment is gone; the precedence limitation is
  documented as observed kit-config behavior.
- `crates/service-sdk` and `logging_bootstrap.rs` show zero diff.
- `openspec/specs/service-sdk/spec.md` gains this requirement once the
  change is archived.
