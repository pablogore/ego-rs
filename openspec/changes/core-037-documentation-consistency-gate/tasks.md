# Tasks: CORE-037 — Documentation Consistency Gate

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~300-380 (2 new xtask modules + `xtask/Cargo.toml` dependency + `main.rs` dispatch/usage diff, incl. offline fixture tests) |
| 400-line budget risk | Medium |
| Chained PRs recommended | No |
| Chain strategy | single-pr (one focused xtask subcommand, fully offline; no online tier to split off) |
| Delivery strategy | auto-forecast (no explicit label); treated conservatively |

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 0 | Add the YAML parser dependency to `xtask` | PR1 | `cargo build -p xtask` | Revert the `xtask/Cargo.toml` dependency line and the `Cargo.lock` delta |
| 1 | `state_yaml.rs` typed parse + three offline detection rules + classification + `verify-doc-consistency` wiring | PR1 | `cargo test -p xtask doc_consistency:: state_yaml::` | Delete `doc_consistency.rs`/`state_yaml.rs`; revert `main.rs` dispatch/usage diff |

## Phase 0: YAML Parser Dependency (prerequisite)

`xtask` currently depends only on `serde`, `serde_json`, `toml` and `anyhow` — it
can parse JSON and TOML but **not** YAML, so the `serde`-derived `state.yaml`
model in Phase 1 cannot deserialize anything until this lands. This phase must
complete before TASK-001.

- [ ] TASK-000: add `serde_yaml = "0.9"` under `[dependencies]` in `xtask/Cargo.toml` (`serde` with `derive` is already present, so only the format crate is added) and commit the resulting `Cargo.lock` delta. AC: `cargo build -p xtask` succeeds and a trivial `serde_yaml::from_str::<serde_yaml::Value>` call compiles inside `xtask`. Rollback: revert the dependency line and the `Cargo.lock` delta. (Traces: design File Changes `xtask/Cargo.toml`.)

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

- [ ] TASK-011 RED: failing test for direction 1 — a phase `status: complete` with `artifact: design.md` and no `design.md` on disk yields one `PhaseArtifactMismatch` finding classified `MachineVerifiable`.
- [ ] TASK-011b RED: failing test for direction 2 — a phase `status: pending` with `artifact: design.md` and a `design.md` present on disk yields one `PhaseArtifactMismatch` finding classified `MachineVerifiable`.
- [ ] TASK-012 RED: failing negative tests — (a) `status: complete` with `artifact: design.md` present yields NO finding; (b) a `status: skipped` phase declaring no `artifact` (only a `note`) yields NO finding; (c) a `status: skipped` phase with `artifact: design.md` PRESENT on disk yields NO finding; (d) a `status: skipped` phase with `artifact: design.md` ABSENT yields NO finding — `skipped` constrains artifact presence in neither direction, so all three `skipped` combinations are covered.
- [ ] TASK-013 GREEN: implement the bidirectional phase/artifact-presence rule emitting a `MachineVerifiable` `PhaseArtifactMismatch` — direction 1: `complete` + declared-artifact-absent ⇒ finding; direction 2: declared artifact present while `status` is neither `complete` nor `skipped` ⇒ finding; no finding for `complete` + present, for `skipped` regardless of presence, or for a phase declaring no `artifact`. AC: TASK-011, TASK-011b and TASK-012 green. (Traces: spec "Phase Status Must Agree With Artifact Presence"; design ADR-2 rule c, both directions.)

## Phase 5: Finding Classification & Exit Decision (offline)

- [ ] TASK-014 RED: failing test — every finding returned by `check_doc_consistency` carries a `FindingClass`, and each of the three shipped rules is `MachineVerifiable`. Assertion: no finding has an unset class; all three `FindingKind`s classify `MachineVerifiable`, so the gate emits no `HumanReview` finding.
- [ ] TASK-015 RED: failing test — the exit reducer returns failing iff at least one `MachineVerifiable` finding exists; a set of only `HumanReview` findings reduces to a clean result. Because no shipped rule emits `HumanReview`, construct those findings directly in the test rather than driving a rule — this branch is specified for the first future advisory rule and must be covered now so it cannot rot.
- [ ] TASK-016 GREEN: implement `FindingClass { MachineVerifiable, HumanReview }`, attach a class to every finding, and implement the exit reducer (`any MachineVerifiable ⇒ Ok(false)` else `Ok(true)`). AC: TASK-014 and TASK-015 green. (Traces: spec "Every Finding Is Classified Machine-Verifiable Or Human-Review".)

## Phase 6: Report & Command Result (offline)

- [ ] TASK-017 RED: failing test — `check_doc_consistency(changes_dir, workspace_root)` runs to completion against a fixture tree with an injected panic-on-network guard in scope, proving the gate performs no network call; a fixture with a stale-unarchived change reduces to a non-zero (failing) result and a fixture with none reduces to a zero (clean) result.
- [ ] TASK-018 GREEN: implement the per-change report formatter (each finding tagged `[machine]` / `[human]`, `OK`/`FAIL (n)` summary consistent with the existing gates) and the orchestration of the three rules. AC: TASK-017 green. (Traces: spec "Validation Runs Fully Offline"; design Observability.)

## Phase 7: No Issue-State Cross-Check (boundary is asserted, not assumed)

Per design ADR-1 there is no GitHub cross-check: no artifact declares which issue
a change is linked to, so the rule would be unimplementable and unverifiable. This
phase asserts the absence mechanically so it cannot drift back in unnoticed.

- [ ] TASK-019 RED: failing test — no HTTP client and no `gh` invocation is reachable from any gate code path. Assertion: a source scan over the gate module finds zero `reqwest`/`ureq`/`hyper`/`Command::new("gh")` references, and `xtask/Cargo.toml` declares no HTTP or GitHub client dependency.
- [ ] TASK-020 GREEN: keep the gate free of any network module — no `doc_consistency/github.rs`, no `probe_gh`, no `IssueClosedButUnarchived` variant. AC: TASK-019 green. (Traces: spec "No Issue-State Cross-Check Without A Declared change-to-issue Link"; design ADR-1, Threat Matrix.)

## Phase 8: xtask Subcommand Wiring (FUTURE integration)

- [ ] TASK-021 RED: failing test asserting the dispatch contract — `verify_doc_consistency()` returns `Ok(true)` for a clean fixture tree and `Ok(false)` when a `MachineVerifiable` finding exists, mirroring `verify_hygiene`'s `Ok(bool)` shape (`xtask/src/main.rs:78-92`).
- [ ] TASK-022 GREEN: add `Some("verify-doc-consistency") => verify_doc_consistency()?` to the dispatch (`xtask/src/main.rs:9-19`), extend the usage string to include `verify-doc-consistency`, and implement `fn verify_doc_consistency() -> anyhow::Result<bool>` that runs the three rules, prints the report, and returns `Ok(true)` clean / `Ok(false)` on any `MachineVerifiable` finding — reusing the existing `std::process::exit(if passed {0} else {1})` mapping (`main.rs:20`). AC: TASK-021 green. (Traces: spec "Integrated As An xtask Subcommand Consistent With Existing Gates"; design ADR-3. This is the FUTURE xtask work this change specifies.)

## Phase 9: Cross-Cutting Guarantees & Verification

- [ ] TASK-023: confirm no existing subcommand's behavior changed — `verify-layers`, `verify-isolation`, and `verify-hygiene` (and `hygiene.rs` FR-006 semantics) are untouched. AC: their tests pass unmodified; `hygiene.rs` has no diff.
- [ ] TASK-024: confirm the gate makes no network call anywhere — no `reqwest`/HTTP/`gh`-shell reference reachable from `check_doc_consistency` or from the dispatch fn, and no such dependency in `xtask/Cargo.toml`. AC: grep clean over the whole gate, not just a core subset.
- [ ] TASK-025: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions.
