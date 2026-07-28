# Proposal: CORE-037 — Documentation Consistency Gate

## Intent

Project state is tracked across several files that are meant to agree with one
another but drift silently: change lifecycle status lives in each change's
`state.yaml`, task completion in `tasks.md`, canonical behavior in
`openspec/specs/`, the roadmap in `ROADMAP.md`, and issue open/closed state on
GitHub. The only mechanical guard today is `verify-hygiene`
(`xtask/src/hygiene.rs:9-43`), which flags an un-archived
`openspec/changes/<name>` **only** when it suffix-matches an existing
`archive/<YYYY-MM-DD>-<name>` — it never consults `state.yaml`, `tasks.md`,
`ROADMAP.md`, or issue state. Concrete drift it misses: a change still under
`openspec/changes/` whose `state.yaml` records `apply: complete` and
`verify: complete` but `archive: pending` (a stale-unarchived change) passes
`verify-hygiene` clean.

This change defines a **Documentation Consistency Gate**: a mechanically
verifiable consistency check over project state, exposed as a new local `xtask`
subcommand (`verify-doc-consistency`), consistent with the existing
`verify-layers` / `verify-isolation` / `verify-hygiene` commands
(`xtask/src/main.rs:9-19`). The gate declares a single source of truth per state
type, detects the file-derivable inconsistencies, classifies each finding as
machine-verifiable versus human-review, and runs fully offline. It is scoped to
what it can actually read: there is no GitHub issue-state cross-check, because no
artifact in this repository declares which issue a change is linked to.

This proposal defines the **gate**, not any one-off reconciliation. The
concrete archiving of PROD-005, the operation-naming doc fix, and the External
Data Providers doc fix are explicitly out of scope.

## Scope

### In Scope
- Declare the source of truth per state type: change lifecycle status →
  `state.yaml`; task completion → `tasks.md`; issue open/closed → GitHub;
  roadmap → `ROADMAP.md`; canonical behavior → `openspec/specs/`. The
  declaration binds all five so no later rule invents a second source; the
  rules shipped here read only `state.yaml` and the filesystem (see the
  explicit non-goal below).
- Detect, from local repository files alone, the machine-verifiable
  inconsistencies: (a) stale-unarchived change (`apply.status: complete` AND
  `verify.status: complete` but `archive.status: pending`); (b) un-archived
  change dir duplicating an archived name (the existing hygiene check,
  preserved); (c) `state.yaml` phase status disagreeing with the presence of
  the corresponding artifact, in **both** directions — a `complete` phase whose
  declared artifact is absent, and a declared artifact present while the phase
  is neither `complete` nor `skipped`.
- Add a YAML parser dependency to `xtask`, which today can parse JSON and TOML
  but not YAML — a hard prerequisite for reading `state.yaml` at all.
- Classify every finding as machine-verifiable (fails the gate) versus
  human-review (advisory, never fails the local run).
- Fully offline execution: the whole gate MUST require no network or GitHub
  access — not merely a "core" subset of it.
- Explicitly NO GitHub issue-state cross-check: no artifact declares which issue
  a change is linked to, so the rule would be neither implementable nor
  verifiable. Excluded until a change defines that link (design ADR-1).
- Integrate as one local `xtask` subcommand, consistent with the existing gate
  commands and their exit-code contract.

### Out of Scope (Non-Goals / Follow-ups)
- **Validating `tasks.md`, `ROADMAP.md` or `openspec/specs/`.** Their sources of
  truth are declared so no later rule invents a competing one, but no rule in
  CORE-037 reads them. The gate validates lifecycle status, phase/artifact
  agreement, and archive-name duplication only, and it must not present the
  three unread sources as covered. Rules that actually consume them are a
  follow-up change.
- The concrete archiving of PROD-005 (or any specific change) — this defines the
  detector, not the reconciliation.
- The operation-naming documentation fix and the External Data Providers
  documentation fix — content reconciliations, not gate logic.
- Any unbounded "sync all documentation" scope, prose-vs-prose semantic
  diffing, or spec-content correctness checking.
- Modifying or extending the existing `verify-hygiene` semantics
  (`hygiene.rs`); the duplication check is referenced and preserved, not
  changed.
- CI wiring (this repo has no CI; gates run locally — issue #229 is the open CI
  track and is referenced as context only).

## Capabilities

### New Capabilities
- `documentation-consistency`: a local, offline-first consistency gate over
  project state — declared sources of truth, machine-verifiable inconsistency
  detection (stale-unarchived, archived-name duplication, phase-status vs
  artifact presence), machine-verifiable/human-review finding classification,
  exposed as the `xtask` subcommand `verify-doc-consistency`.

### Modified Capabilities
- None. The existing `foundation-integrity` capability's stale-change hygiene
  requirement (FR-006) is **preserved unchanged**; this gate references and
  reuses that duplication rule rather than modifying it (placement justified in
  design ADR-0).

## Approach

Add a new `xtask` subcommand `verify-doc-consistency` that parses each change's
`state.yaml`, cross-checks phase status against on-disk artifacts and against
the archive directory, and emits a classified, human-readable report with a
non-zero exit on any machine-verifiable inconsistency. Every detection rule
operates purely on repository files and is exercised by offline, fixture-based
tests. There is no online tier: the issue-state cross-check an earlier draft
proposed is excluded, because the change-to-issue link it would need is nowhere
declared as structured data (design ADR-1). A follow-up change may define that
link and then add the cross-check onto the human-review channel this gate already
specifies.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `xtask/Cargo.toml` | Modify (FUTURE) | Add a YAML parser dependency (`serde_yaml`). `xtask` today has `serde`, `serde_json`, `toml` and `anyhow` — no YAML parser — so `state.yaml` cannot be read at all without it |
| `xtask/src/main.rs` | Modify (FUTURE) | Add a `verify-doc-consistency` dispatch arm and usage string alongside the existing three subcommands (`main.rs:9-19`) |
| `xtask/src/doc_consistency.rs` (new) | New (FUTURE) | Offline detection rules, finding classification, human-readable report |
| `xtask/src/state_yaml.rs` (new) | New (FUTURE) | Typed parse of the `state.yaml` schema (`change_id`, `title`, `status`, `phases.*`, `notes`) |
| `openspec/specs/documentation-consistency/` | New (this change) | New capability spec (created on archive from this change's delta) |

> All `xtask/src/*` rows are FUTURE work specified by this change; no production
> or xtask source is written here.

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| The gate accidentally acquires a network dependency | Low | The gate has no online tier at all; a task asserts no HTTP client and no `gh` invocation is reachable from any code path, and that `xtask/Cargo.toml` declares no such dependency |
| Over-broad scope creeps into prose/spec-content diffing | Med | Scope is fixed to file-derivable state consistency; prose-vs-prose semantic drift is explicitly human-review, never a gate failure |
| False positives block local work | Med | Only machine-verifiable findings fail the gate; every rule is derivable from repo files and covered by positive + negative fixtures |
| Duplicating/altering `verify-hygiene` semantics | Low | FR-006 duplication rule is preserved unchanged and referenced, not modified (ADR-0) |
| A future cross-check is bolted on without a linkage contract | Med | The exclusion and its precondition (structured change-to-issue link with defined cardinality) are normative in the spec, so the follow-up change must define the link first |

## Rollback Plan

Purely additive and inert until invoked: the gate is a new subcommand with new
files. If unwanted, remove the `verify-doc-consistency` dispatch arm and the new
`xtask/src/doc_consistency*.rs` / `state_yaml.rs` modules; the existing three
subcommands and `verify-hygiene` semantics are untouched, so removal is
behavior-neutral. No production crate, schema, or runtime behavior depends on
this gate.

## Dependencies

- The existing `xtask` gate mechanism and dispatch shape (`xtask/src/main.rs:9-19`).
- The existing stale-change hygiene rule preserved from `xtask/src/hygiene.rs:9-43`
  (`foundation-integrity` FR-006).
- The `state.yaml` schema as used across `openspec/changes/*` (`change_id`,
  `title`, `status`, `artifact_store_mode`, `phases.{proposal,explore,spec,
  design,tasks,apply,verify,archive}.status`, `notes`).

## Success Criteria

- [ ] The gate detects, from local files alone, a change with
  `apply.status: complete` AND `verify.status: complete` but
  `archive.status: pending` (stale-unarchived).
- [ ] The gate preserves the archived-name duplication detection (FR-006) with
  identical semantics.
- [ ] The gate detects a `state.yaml` phase status that disagrees with the
  presence of the corresponding artifact.
- [ ] Every finding is classified as machine-verifiable or human-review.
- [ ] The full gate runs offline with no network/GitHub access and
  exits non-zero on any machine-verifiable inconsistency.
- [ ] No GitHub issue-state cross-check exists, and no HTTP client or `gh`
  invocation is reachable from any gate code path.
- [ ] The gate runs as one local `xtask` subcommand consistent with the
  existing three.
