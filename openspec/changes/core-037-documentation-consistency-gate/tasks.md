# Tasks: CORE-037 — Documentation Consistency Gate

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~350-450 (2 new xtask modules + 1 optional github submodule + `main.rs` dispatch/usage diff, incl. offline fixture tests) |
| 400-line budget risk | Medium |
| Chained PRs recommended | No |
| Chain strategy | single-pr (one focused xtask subcommand; split only if the offline core + optional github tiers exceed the budget, in which case PR1 = state.yaml parse + three offline rules + wiring, PR2 = optional github cross-check) |
| Delivery strategy | auto-forecast (no explicit label); treated conservatively |

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 1 | `state_yaml.rs` typed parse + three offline detection rules + classification + `verify-doc-consistency` wiring | PR1 | `cargo test -p xtask doc_consistency:: state_yaml::` | Delete `doc_consistency.rs`/`state_yaml.rs`; revert `main.rs` dispatch/usage diff |
| 2 | Optional GitHub issue-state cross-check with graceful skip | PR1 (or PR2 if split) | `cargo test -p xtask doc_consistency::github::` | Delete `doc_consistency/github.rs`; remove its call from `verify_doc_consistency` |

## Phase 1: state.yaml Typed Parse (offline)

- [ ] TASK-001 RED: failing test in new `xtask/src/state_yaml.rs` — parse a fixture `state.yaml` (via `tempfile`) and assert the loaded model exposes `change_id`, `status`, and each `phases.{apply,verify,archive}.status` plus a phase's optional `artifact` and `note`. Assertion: `load_state_yaml(path).unwrap().phases.archive.status == "pending"` and `.phases.design.artifact == Some("design.md")`. Offline — no network.
- [ ] TASK-002 GREEN: implement a `serde`-derived model of the `state.yaml` schema (`change_id`, `title`, `status`, `artifact_store_mode`, `phases.{proposal,explore,spec,design,tasks,apply,verify,archive}.{status,artifact,note}`, `notes`) and `load_state_yaml(path)`. AC: TASK-001 green. (Traces: spec "Source Of Truth Is Declared Per State Type"; design File Changes `state_yaml.rs`.)
- [ ] TASK-003 RED: failing test — a change directory missing `state.yaml`, and a malformed `state.yaml`, each produce a `MachineVerifiable` finding rather than a panic. Assertion: `check_doc_consistency` returns a finding tagged `MachineVerifiable` for the missing/malformed case; no `unwrap` panic.
- [ ] TASK-004 GREEN: implement missing/malformed `state.yaml` as a `MachineVerifiable` finding in `check_doc_consistency`. AC: TASK-003 green. (Traces: design Error Model.)

## Phase 2: Stale-Unarchived Detection (offline)

- [ ] TASK-005 RED: failing test in new `xtask/src/doc_consistency.rs` — a fixture change dir with `apply.status: complete`, `verify.status: complete`, `archive.status: pending` yields exactly one `StaleUnarchived` finding classified `MachineVerifiable`. Assertion asserts on `Finding.kind == StaleUnarchived` and `Finding.class == MachineVerifiable`.
- [ ] TASK-006 RED: failing negative tests — (a) a fixture with `archive.status: complete` yields NO `StaleUnarchived` finding; (b) a fixture with `apply.status: complete`, `verify.status: pending`, `archive.status: pending` yields NO `StaleUnarchived` finding.
- [ ] TASK-007 GREEN: implement the stale-unarchived rule (`apply == complete ∧ verify == complete ∧ archive == pending`, dir under `openspec/changes/` other than `archive/`) emitting a `MachineVerifiable` `StaleUnarchived` finding. AC: TASK-005 and TASK-006 green. (Traces: spec "Stale-Unarchived Change Detection From Local Files"; design ADR-2.)

## Phase 3: Archived-Name Duplication Preserved (offline)

- [ ] TASK-008 RED: failing test — a fixture with `archive/2026-07-15-core-019-reliable-external-effects` and an un-archived `core-019-reliable-external-effects` yields one `ArchivedNameDuplicate` finding classified `MachineVerifiable` (mirrors `xtask/src/hygiene.rs` FR-006 semantics).
- [ ] TASK-009 RED: failing negative test — a fixture with an un-archived `core-020-something-else` and no suffix-matching archived dir yields NO `ArchivedNameDuplicate` finding.
- [ ] TASK-010 GREEN: implement the archived-name-duplication rule with the same suffix-match semantics as the existing `verify-hygiene` check (date-prefix strip + case-insensitive suffix match), emitting a `MachineVerifiable` `ArchivedNameDuplicate` finding. AC: TASK-008 and TASK-009 green. (Traces: spec "Archived-Name Duplication Detection Is Preserved"; design ADR-0/ADR-2; `foundation-integrity` FR-006 preserved unchanged.)

## Phase 4: Phase-Status vs Artifact Presence (offline)

- [ ] TASK-011 RED: failing test — a phase `status: complete` with `artifact: design.md` and no `design.md` on disk yields one `PhaseArtifactMismatch` finding classified `MachineVerifiable`.
- [ ] TASK-012 RED: failing negative tests — (a) `status: complete` with `artifact: design.md` present yields NO finding; (b) a `status: skipped` phase declaring no `artifact` (only a `note`) yields NO finding.
- [ ] TASK-013 GREEN: implement the phase/artifact-presence rule (complete + declared-artifact-absent ⇒ finding; complete + present ⇒ none; skipped-without-artifact ⇒ none) emitting a `MachineVerifiable` `PhaseArtifactMismatch`. AC: TASK-011 and TASK-012 green. (Traces: spec "Phase Status Must Agree With Artifact Presence".)

## Phase 5: Finding Classification & Exit Decision (offline)

- [ ] TASK-014 RED: failing test — every finding returned by `check_doc_consistency` carries a `FindingClass`, and each of the three offline rules is `MachineVerifiable`. Assertion: no finding has an unset class; the three offline `FindingKind`s classify `MachineVerifiable`.
- [ ] TASK-015 RED: failing test — the exit reducer returns non-zero iff at least one `MachineVerifiable` finding exists; a set of only `HumanReview` findings reduces to a zero (clean) exit.
- [ ] TASK-016 GREEN: implement `FindingClass { MachineVerifiable, HumanReview }`, attach a class to every finding, and implement the exit reducer (`any MachineVerifiable ⇒ Ok(false)` else `Ok(true)`). AC: TASK-014 and TASK-015 green. (Traces: spec "Every Finding Is Classified Machine-Verifiable Or Human-Review".)

## Phase 6: Offline-First Report & Command Result (offline)

- [ ] TASK-017 RED: failing test — `check_doc_consistency(changes_dir, workspace_root)` runs to completion against a fixture tree with an injected panic-on-network guard in scope, proving the core path performs no network call; a fixture with a stale-unarchived change reduces to a non-zero (failing) result and a fixture with none reduces to a zero (clean) result.
- [ ] TASK-018 GREEN: implement the per-change report formatter (each finding tagged `[machine]` / `[human]`, `OK`/`FAIL (n)` summary consistent with the existing gates) and the offline orchestration of the three rules. AC: TASK-017 green. (Traces: spec "Core Validation Runs Fully Offline"; design Observability.)

## Phase 7: Optional GitHub Issue-State Cross-Check (graceful degradation)

- [ ] TASK-019 RED: failing test in new `xtask/src/doc_consistency/github.rs` — with an injected `CrossCheckAvailability::Unavailable(reason)` probe, `issue_cross_check` returns zero findings plus a recorded skip note, makes no network call, and does not change the offline exit decision. Negative-path is the primary contract.
- [ ] TASK-020 RED: failing test — with an injected available issue-state source reporting a CLOSED issue linked to a still-un-archived, apply+verify-complete change, `issue_cross_check` returns exactly one `IssueClosedButUnarchived` finding classified `HumanReview`, and that finding alone does not flip the exit decision to failing.
- [ ] TASK-021 GREEN: implement `probe_gh() -> CrossCheckAvailability` (missing/unauthenticated/offline/rate-limited ⇒ `Unavailable(reason)`), and `issue_cross_check` emitting only `HumanReview` findings, wired so `Unavailable` is a skip note and never a failure. AC: TASK-019 and TASK-020 green. (Traces: spec "GitHub Issue-State Cross-Check Is Optional And Degrades Gracefully"; design ADR-1, Threat Matrix.)
- [ ] TASK-022 RED: failing test — the set of `MachineVerifiable` findings and the reduced exit status are identical for the same fixture tree evaluated once with an available and once with an unavailable cross-check.
- [ ] TASK-023 GREEN: ensure the cross-check only ever appends `HumanReview` findings and never mutates the offline core result. AC: TASK-022 green. (Traces: spec scenario "The cross-check never changes the offline result".)

## Phase 8: xtask Subcommand Wiring (FUTURE integration)

- [ ] TASK-024 RED: failing test asserting the dispatch contract — `verify_doc_consistency()` returns `Ok(true)` for a clean fixture tree and `Ok(false)` when a `MachineVerifiable` finding exists, mirroring `verify_hygiene`'s `Ok(bool)` shape (`xtask/src/main.rs:78-92`).
- [ ] TASK-025 GREEN: add `Some("verify-doc-consistency") => verify_doc_consistency()?` to the dispatch (`xtask/src/main.rs:9-19`), extend the usage string to include `verify-doc-consistency`, and implement `fn verify_doc_consistency() -> anyhow::Result<bool>` that runs the offline core, attempts the optional cross-check, prints the report, and returns `Ok(true)` clean / `Ok(false)` on any `MachineVerifiable` finding — reusing the existing `std::process::exit(if passed {0} else {1})` mapping (`main.rs:20`). AC: TASK-024 green. (Traces: spec "Integrated As An xtask Subcommand Consistent With Existing Gates"; design ADR-3. This is the FUTURE xtask work this change specifies.)

## Phase 9: Cross-Cutting Guarantees & Verification

- [ ] TASK-026: confirm no existing subcommand's behavior changed — `verify-layers`, `verify-isolation`, and `verify-hygiene` (and `hygiene.rs` FR-006 semantics) are untouched. AC: their tests pass unmodified; `hygiene.rs` has no diff.
- [ ] TASK-027: confirm the offline core makes no network call — no `reqwest`/HTTP/`gh`-shell reference reachable from `check_doc_consistency`; the only `gh` use is inside the optional `doc_consistency/github.rs` cross-check. AC: grep clean over the core path.
- [ ] TASK-028: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions.
