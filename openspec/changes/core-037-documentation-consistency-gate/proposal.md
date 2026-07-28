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
machine-verifiable versus human-review, and runs fully offline — with an
optional GitHub issue-state cross-check that degrades gracefully when the
network or an authenticated `gh` is unavailable.

This proposal defines the **gate**, not any one-off reconciliation. The
concrete archiving of PROD-005, the operation-naming doc fix, and the External
Data Providers doc fix are explicitly out of scope.

## Scope

### In Scope
- Declare the source of truth per state type: change lifecycle status →
  `state.yaml`; task completion → `tasks.md`; issue open/closed → GitHub;
  roadmap → `ROADMAP.md`; canonical behavior → `openspec/specs/`.
- Detect, from local repository files alone, the machine-verifiable
  inconsistencies: (a) stale-unarchived change (`apply.status: complete` AND
  `verify.status: complete` but `archive.status: pending`); (b) un-archived
  change dir duplicating an archived name (the existing hygiene check,
  preserved); (c) `state.yaml` phase status disagreeing with the presence of
  the corresponding artifact.
- Classify every finding as machine-verifiable (fails the gate) versus
  human-review (advisory, never fails the local run).
- Offline-first execution: the full core validation MUST require no network or
  GitHub access.
- Optional GitHub issue-state cross-check as online enrichment that is skipped
  (not failed) when offline or unauthenticated.
- Integrate as one local `xtask` subcommand, consistent with the existing gate
  commands and their exit-code contract.

### Out of Scope (Non-Goals / Follow-ups)
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
  and an optional, gracefully degrading GitHub issue-state cross-check, exposed
  as the `xtask` subcommand `verify-doc-consistency`.

### Modified Capabilities
- None. The existing `foundation-integrity` capability's stale-change hygiene
  requirement (FR-006) is **preserved unchanged**; this gate references and
  reuses that duplication rule rather than modifying it (placement justified in
  design ADR-0).

## Approach

Add a new `xtask` subcommand `verify-doc-consistency` that parses each change's
`state.yaml`, cross-checks phase status against on-disk artifacts and against
the archive directory, and emits a classified, human-readable report with a
non-zero exit on any machine-verifiable inconsistency. The core detection rules
operate purely on repository files and are exercised by offline, fixture-based
tests. A separately gated GitHub issue-state cross-check probes for an
authenticated, reachable `gh`; when present it may surface human-review findings
(e.g. a closed issue linked to a still-un-archived, apply+verify-complete
change), and when absent it is recorded as skipped without failing the run.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `xtask/src/main.rs` | Modify (FUTURE) | Add a `verify-doc-consistency` dispatch arm and usage string alongside the existing three subcommands (`main.rs:9-19`) |
| `xtask/src/doc_consistency.rs` (new) | New (FUTURE) | Offline detection rules, finding classification, human-readable report |
| `xtask/src/state_yaml.rs` (new) | New (FUTURE) | Typed parse of the `state.yaml` schema (`change_id`, `title`, `status`, `phases.*`, `notes`) |
| `xtask/src/doc_consistency/github.rs` (new) | New (FUTURE) | Optional, gracefully degrading GitHub issue-state cross-check |
| `openspec/specs/documentation-consistency/` | New (this change) | New capability spec (created on archive from this change's delta) |

> All `xtask/src/*` rows are FUTURE work specified by this change; no production
> or xtask source is written here.

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Core validation accidentally depends on GitHub/network | Med | Offline-first is normative; the GitHub cross-check is a separate, optional module; offline fixture tests exercise the full core path with no network |
| Over-broad scope creeps into prose/spec-content diffing | Med | Scope is fixed to file-derivable state consistency; prose-vs-prose semantic drift is explicitly human-review, never a gate failure |
| False positives block local work | Med | Only machine-verifiable findings fail the gate; every rule is derivable from repo files and covered by positive + negative fixtures |
| Duplicating/altering `verify-hygiene` semantics | Low | FR-006 duplication rule is preserved unchanged and referenced, not modified (ADR-0) |
| GitHub cross-check flakiness fails local runs | Low | Cross-check unavailability is a skip, never a failure (normative, negative-scenario covered) |

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
- [ ] The full core validation runs offline with no network/GitHub access and
  exits non-zero on any machine-verifiable inconsistency.
- [ ] The GitHub issue-state cross-check is optional and is skipped — not
  failed — when offline or unauthenticated.
- [ ] The gate runs as one local `xtask` subcommand consistent with the
  existing three.
