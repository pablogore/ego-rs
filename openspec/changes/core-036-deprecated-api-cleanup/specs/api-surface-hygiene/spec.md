# API Surface Hygiene Specification

## Purpose

Define and enforce the cross-cutting "no shims in pre-stable crates" policy stated in `PRD.md:140`
("**No shims** — when a public API is removed, it is removed. No deprecated aliases in pre-stable
crates"). This capability owns the rules that do not belong to any single feature capability: that a
removed API leaves **zero** references, that removals are total (no deprecated alias/re-export/shim),
and that the policy is enforced by an observable, automated gate rather than prose.

Out of scope: what any particular API *does*, or whether a specific symbol should be removed — those
decisions live in the owning capability's spec (e.g. `persistent-entity`, `service-sdk`). This
capability governs only the *hygiene invariant* and its *verification*.

## Requirements

### Requirement: No Deprecated Aliases In Pre-Stable Crates

Pre-stable crates (version `0.x`) MUST NOT carry any `#[deprecated]` attribute. A public API that is
removed MUST be removed outright — no deprecated alias, no `#[deprecated]` wrapper, and no
compatibility re-export standing in for the removed symbol. The count of `#[deprecated]` attributes
across pre-stable crate sources MUST be zero.

#### Scenario: Zero deprecated attributes remain

- GIVEN the workspace after CORE-036
- WHEN it is scanned with `rg '#\[deprecated' crates/`
- THEN the search returns zero matches

#### Scenario: No lingering deprecation suppressors

- GIVEN the workspace after the deprecated symbols are removed
- WHEN it is scanned with `rg '#\[allow\(deprecated\)\]' crates/`
- THEN the search returns zero matches, because nothing deprecated remains to suppress

#### Scenario: A removed API leaves no alias

- GIVEN a public API removed under this policy
- WHEN the crate is inspected for a replacement symbol of the same name, a `pub use` re-export, or a `#[deprecated]` wrapper
- THEN none exists — the removal is total

### Requirement: Removed APIs Reach Zero References

When an API is removed, the number of references to it MUST reach zero, verified by two independent
observable checks: (1) the workspace MUST compile and test green (`cargo build --workspace`,
`cargo test --workspace`) — a dangling reference to a removed symbol cannot compile; and (2) a
targeted text search for the removed identifier(s) MUST return zero matches outside historical
OpenSpec artifacts.

#### Scenario: Compilation proves no dangling references

- GIVEN every deprecated symbol targeted by the change has been removed and its callers migrated
- WHEN `cargo build --workspace` and `cargo test --workspace` run
- THEN both succeed with no missing-symbol or unresolved-import error

#### Scenario: Text search confirms zero references

- GIVEN the removed identifiers of the change
- WHEN each is searched across `crates/` (and docs where the symbol appeared) with `rg`
- THEN every search returns zero matches (excluding retained distinct symbols such as `_for` variants and historical OpenSpec artifacts)

### Requirement: The No-Shims Policy Is Enforced By A Test Gate

The no-shims policy MUST be enforced by a participant of the project's standard test gate
(`cargo test --workspace`), not by an unenforced external script. A source-scan test
(`no_deprecated_shims_lint`), modeled on the existing `tenant_scoped_lint.rs`, MUST anchor on
`CARGO_MANIFEST_DIR`, ascend to the `[workspace]` root, scan pre-stable crate sources, and fail if
any `#[deprecated]` attribute is present.

The detection MUST be exposed as a callable function over source text, so that its own correctness is
demonstrable by passing fixtures as arguments and asserting the returned count. The gate MUST NOT be
validated by leaving an intentionally failing test in the suite: `cargo test --workspace` MUST be
green on a compliant workspace.

The scan scope MUST match the stated policy exactly. The lint MUST read each crate's own `Cargo.toml`
and apply the zero-`#[deprecated]` rule **only** to crates whose `version` has major `0`. A crate at
`1.0` or above MUST be skipped, because `#[deprecated]` is the correct stability tool there; scanning
it would enforce a stronger policy than `PRD.md:140` states.

#### Scenario: The detector reports a re-introduced shim

- GIVEN a fixture source string containing exactly one `#[deprecated]` attribute
- WHEN the detector function is called with that fixture
- THEN it returns a count of `1`, and the gate would fail were that source in a pre-stable crate

#### Scenario: The detector reports nothing on clean source

- GIVEN a fixture source string containing no `#[deprecated]` attribute
- WHEN the detector function is called with that fixture
- THEN it returns a count of `0`

#### Scenario: The gate passes on a clean workspace

- GIVEN the workspace after all deprecated symbols are removed
- WHEN `cargo test --workspace` runs
- THEN `no_deprecated_shims_lint` passes, because the pre-stable `#[deprecated]` count is zero

#### Scenario: A stable crate is outside the policy's scope

- GIVEN a crate whose `Cargo.toml` declares `version = "1.2.0"` and whose source carries a `#[deprecated]` attribute
- WHEN `no_deprecated_shims_lint` runs
- THEN that crate is classified as not pre-stable, its source is not scanned, and the gate still passes

### Requirement: Intentional Non-Deprecated Surface Is Not A Shim

The policy targets only `#[deprecated]` symbols and dead aliases. Deliberate, non-deprecated surface
MUST NOT be flagged or removed by a no-shims audit, including: `#[doc(hidden)] pub` items required
for proc-macro-generated code to reach otherwise-`pub(crate)` internals; compatibility fields kept
authoritative-by-construction under a newer type (e.g. the legacy `trace_id` mirror under
`TraceContext`); and coverage of an external dependency's back-compat API (e.g. testkit exercising
`kitlogger`'s `log(Severity, &str)` path). Such retentions MUST carry a documented justification.

#### Scenario: Documented retentions are excluded from the audit

- GIVEN the macro-visibility hatches, the legacy `trace_id` mirror, and the testkit external back-compat coverage
- WHEN the no-shims audit runs
- THEN none is flagged, because none carries `#[deprecated]` and each has a recorded justification in its owning capability's spec

#### Scenario: An external-dependency back-compat path is not an ego shim

- GIVEN testkit exercises `kitlogger::KITLogger::log(Severity, &str)`
- WHEN the no-shims policy is applied to ego crates
- THEN this is not counted as an ego deprecated surface, because the symbol is owned by the external `kitlogger` crate, not by any `ego-*` crate
