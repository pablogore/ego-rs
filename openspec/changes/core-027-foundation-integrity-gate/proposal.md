# Proposal: CORE-027 — Foundation Integrity Gate

## Metadata

| Field | Value |
|-------|-------|
| Change ID | CORE-027 |
| Title | Foundation Integrity Gate |
| Type | Stabilization gate (no new functional capability) |
| Date | 2026-07-16 |
| Related | CORE-011A (archived proposal assumes `scripts/verify-layers.sh` exists — it does not); commit `2d5861d` (prior flaky-test fix in `persistent-entity`) |
| Status | PROPOSING |

## 1. Intent

The architecture is documented as strictly layered (domain → application →
infrastructure/transport, with `ego-runtime`/`ego-scheduler` as foundation),
but **nothing enforces the dependency direction**. This change closes the gap
between documented and enforced architecture. It is a stabilization gate: no
new feature work should land until it closes.

## 2. Current Gap (verified against source)

1. **`layers.toml` maps only 8 of the workspace's 16 crates.** Missing:
   `ego-event-adapter`, `ego-persistence`, `persistent-entity`,
   `ego-service-sdk`, `ego-service-sdk-macros`, `ego-security-sdk`,
   `security-apikey`, `ego-testkit`.
2. **`layers.toml` has a dead entry** `"runtime-slice" = "domain"` — no
   workspace crate is named `runtime-slice` (verified against every
   `crates/*/Cargo.toml` `name` field).
3. **The enforcement script named in `layers.toml`'s own header does not
   exist.** `scripts/verify-layers.sh` is referenced by the file and assumed
   by the archived CORE-011A proposal
   (`openspec/changes/archive/2026-06-23-CORE-011A-key-resolver-architecture/proposal.md`),
   but it does not exist anywhere in the repo and never has.
4. **No general CI exists.** `.github/workflows/claude.yml` only triggers the
   Claude Code bot on `@claude` mentions; nothing runs `cargo build`,
   `cargo test`, or `cargo clippy`. CI/CD is being rebuilt separately as a
   Dagger pipeline (not ready yet) — building that pipeline is explicitly not
   part of this change.
5. **Known flakiness precedent.** Commit `2d5861d` fixed a gate-release race
   in two adversarial `persistent-entity` tests; whether related suspects
   still reproduce must be checked, not assumed.
6. **No architecture-lint tooling of any kind was found** (no `clippy.toml`,
   no custom lint crate). Anything discovered during implementation should be
   consolidated into this gate, not duplicated.

## 3. Scope

### In Scope

1. **Fix `layers.toml`**: remove the dead `runtime-slice` entry; add the 8
   missing crates with correct layer assignments (exact assignments are a
   design decision, argued against the documented layer rules).
2. **Build the layer-check tool** — `scripts/verify-layers.sh` or an
   equivalent Rust binary — that parses `layers.toml` and the workspace
   dependency graph and fails on: (a) a dependency pointing the wrong
   direction per the documented rules, (b) a dependency cycle, (c) a
   workspace crate missing from `layers.toml`. It must be runnable
   locally/manually and ready to plug into the future Dagger pipeline.
3. **Per-crate isolation check**: verify each crate compiles in isolation
   (`cargo check -p <crate> --no-default-features` or equivalent), so
   workspace feature unification cannot hide a violation that would surface
   for a downstream consumer with a narrower feature set.
4. **Consolidate existing architecture-lint mechanisms**: grep the repo first;
   fold anything found into the gate rather than inventing parallel tooling.
5. **Flaky-test triage**: re-run the three suspects — `persistent-entity`
   concurrent spawn, `provider-access` under parallel execution, effects
   deadline/cancellation — enough times to determine whether each still
   reproduces. Fix any that do; explicitly record any that no longer
   reproduce and need no action.
6. **Stale-change hygiene check**: fail when an un-archived duplicate of an
   already-archived change exists under `openspec/changes/`. The one known
   instance (`core-019-reliable-external-effects`) was already removed
   earlier this session; the check exists so future drift gets caught, not to
   re-fix that instance.

### Out of Scope / Non-Goals

- No new functional capability of any kind.
- No GitHub Actions workflow, no Dagger pipeline wiring — CI/CD is tracked
  separately; this change only produces the tool the pipeline will call.
- No unrelated refactors, even where the layer check reveals ugliness that is
  not a violation.

## 4. Capabilities

### New Capabilities

- `foundation-integrity`: the enforcement contract — layer-map completeness,
  dependency-direction and cycle verification, per-crate isolation
  compilation, and stale-change hygiene.

### Modified Capabilities

- None. Flaky-test fixes (if any reproduce) are implementation-level; if a
  fix turns out to change spec-level behavior of `persistent-entity` or
  `external-effects`, the spec phase must add the corresponding delta.

## 5. Approach

- Correct the layer map first (data), then build the checker (enforcement)
  against it, so the tool is green on day one and every subsequent failure is
  a real regression.
- Prefer parsing `cargo metadata` output over hand-rolling Cargo.toml parsing.
- Whether the checker is a shell script or a small Rust binary is a design
  decision; the contract (three failure classes above, non-zero exit,
  human-readable report) is fixed here.
- Flaky-test triage is evidence-driven: N repeated runs per suspect (N chosen
  in design), verdict recorded per suspect.

## 6. Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `layers.toml` | Modified | Dead entry removed; 8 crates added; header updated to name the real tool |
| `scripts/verify-layers.sh` (or Rust equivalent) | New | Layer/cycle/completeness checker |
| Per-crate isolation check (script or checker subcommand) | New | `cargo check -p` isolation sweep |
| `crates/persistent-entity` | Conditional | Only if the concurrent-spawn suspect still reproduces |
| `crates/runtime` (effects) | Conditional | Only if deadline/cancellation suspect still reproduces |
| Provider-access tests | Conditional | Only if the parallel-execution suspect still reproduces |
| `openspec/specs/foundation-integrity/` | New | Canonical spec for this capability |

## 7. Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Correct layer assignment for the 8 unmapped crates is contested (e.g., `ego-testkit`, `ego-service-sdk-macros` don't fit the four documented layers cleanly) | Med | Design records each assignment with rationale; the checker supports whatever layer vocabulary the design settles, no silent special cases |
| Checker turns up real existing violations, expanding scope | Med | Violations are fixed only if mechanical; anything structural becomes an explicit follow-up change, and the checker gains a documented, temporary allowlist entry rather than a silent pass |
| Flaky suspects reproduce only under CI-like load, not locally | Med | Run suspects with high iteration counts and parallelism locally; record confidence level per verdict rather than claiming certainty |
| Tool bit-rots without CI to run it | Med | Keep the tool trivially runnable (one command, no setup) and hand it to the Dagger pipeline work as an explicit input |

## 8. Rollback Plan

Everything is additive tooling plus a data-file fix: revert the commit range
to restore the old `layers.toml` and delete the checker. Flaky-test fixes (if
any) are independent commits, revertible individually. No runtime behavior,
public API, or persisted data is touched.

## 9. Dependencies

- None technical. The Dagger CI/CD effort consumes this change's tool later;
  nothing here waits on it.

## 10. Success Criteria

- [ ] `layers.toml` maps all 16 workspace crates; the dead `runtime-slice`
      entry is gone.
- [ ] The checker exists, runs locally with one command, and fails on
      wrong-direction dependencies, cycles, and unmapped crates — verified by
      deliberately introducing each failure class.
- [ ] The checker passes on the current (fixed) workspace.
- [ ] Every crate passes the per-crate isolation check.
- [ ] Each of the three flaky suspects has a recorded verdict: fixed, or
      demonstrated non-reproducing with the run evidence noted.
- [ ] The stale-change hygiene check exists and passes (no un-archived
      duplicates of archived changes).
- [ ] No GitHub Actions or Dagger files were added or modified.
