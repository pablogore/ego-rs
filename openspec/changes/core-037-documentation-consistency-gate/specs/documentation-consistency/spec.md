# Documentation Consistency Gate Specification

## Purpose

Defines the observable contract for a local, offline-first consistency gate over
this project's tracked state. Project state is spread across files that are meant
to agree: change lifecycle status in each change's `state.yaml`, task completion
in `tasks.md`, canonical behavior in `openspec/specs/`, the roadmap in
`ROADMAP.md`, and issue open/closed state on GitHub. This spec fixes WHAT the
gate guarantees — a declared source of truth per state type, detection of the
file-derivable inconsistencies, a machine-verifiable/human-review classification
of every finding, offline execution, and an optional GitHub cross-check that
degrades gracefully — not the checker's implementation shape.

Out of scope: any concrete reconciliation (e.g. archiving a specific change,
fixing operation-naming or External Data Providers documentation), prose-versus-
prose semantic diffing, spec-content correctness checking, and any unbounded
"sync all documentation" behavior. This spec defines the GATE, not the fixes.

---

## Requirements

### Requirement: Source Of Truth Is Declared Per State Type

The gate MUST treat exactly one artifact as the source of truth for each state
type: change lifecycle status → the change's `state.yaml`; task completion →
`tasks.md`; issue open/closed state → GitHub; roadmap → `ROADMAP.md`; canonical
behavior → `openspec/specs/`. When a state type has a declared source of truth,
the gate MUST derive that state from that source and MUST NOT infer it from a
different artifact.

#### Scenario: Lifecycle status is read from state.yaml, not tasks.md
- GIVEN a change whose `tasks.md` shows checked-off tasks but whose `state.yaml`
  records `archive.status: pending`
- WHEN the gate evaluates that change's lifecycle status
- THEN it takes the lifecycle status from `state.yaml`, not from the `tasks.md`
  checkboxes

#### Scenario: Issue open/closed state is never inferred from local files
- GIVEN the gate needs a linked issue's open/closed state
- WHEN it evaluates that state
- THEN it treats GitHub as the only source of truth for it and does not derive
  open/closed from any local file

### Requirement: Stale-Unarchived Change Detection From Local Files

The gate MUST detect, using local repository files alone, a change directory
under `openspec/changes/` (other than `archive/`) whose `state.yaml` records
`apply.status: complete` AND `verify.status: complete` AND
`archive.status: pending`. Such a change MUST be reported as a stale-unarchived
finding classified `MachineVerifiable`. A change that does not meet all three
conditions MUST NOT be reported by this rule.

#### Scenario: Apply and verify complete but archive pending is flagged
- GIVEN a change dir under `openspec/changes/` whose `state.yaml` has
  `apply.status: complete`, `verify.status: complete`, and
  `archive.status: pending`
- WHEN the gate runs
- THEN it reports a stale-unarchived finding for that change, classified
  `MachineVerifiable`

#### Scenario: A fully archived change is not flagged as stale
- GIVEN a change whose `state.yaml` has `apply.status: complete`,
  `verify.status: complete`, and `archive.status: complete`
- WHEN the gate runs
- THEN it reports no stale-unarchived finding for that change

#### Scenario: A change still in progress is not flagged as stale
- GIVEN a change whose `state.yaml` has `apply.status: complete` but
  `verify.status: pending` and `archive.status: pending`
- WHEN the gate runs
- THEN it reports no stale-unarchived finding for that change

### Requirement: Archived-Name Duplication Detection Is Preserved

The gate MUST detect an un-archived change directory under `openspec/changes/`
whose name suffix-matches an already-archived `archive/<YYYY-MM-DD>-<name>`
directory, with the same semantics as the existing stale-change hygiene rule.
Such a duplicate MUST be reported and classified `MachineVerifiable`. A change
whose name matches no archived directory MUST NOT be reported by this rule.

#### Scenario: An un-archived duplicate of an archived change is flagged
- GIVEN an archived `archive/2026-07-15-core-019-reliable-external-effects`
  directory and an un-archived `core-019-reliable-external-effects` directory
- WHEN the gate runs
- THEN it reports the un-archived directory as an archived-name duplicate,
  classified `MachineVerifiable`

#### Scenario: A change with no archived counterpart is not flagged as a duplicate
- GIVEN an un-archived `core-020-something-else` directory and no archived
  directory whose name suffix-matches it
- WHEN the gate runs
- THEN it reports no archived-name-duplicate finding for that change

### Requirement: Phase Status Must Agree With Artifact Presence

The gate MUST detect a discrepancy between a `state.yaml` phase's recorded status
and the on-disk presence of that phase's declared artifact. A phase whose
`status` is `complete` and that declares an `artifact` whose file or directory is
absent MUST be reported as a phase/artifact mismatch classified
`MachineVerifiable`. A phase whose `status` is `complete` and whose declared
artifact is present MUST NOT be reported by this rule. A phase legitimately
without an artifact (for example a `skipped` phase carrying only a `note`) MUST
NOT be reported by this rule.

#### Scenario: Complete phase with a missing artifact is flagged
- GIVEN a `state.yaml` phase with `status: complete` and `artifact: design.md`,
  and no `design.md` present in the change directory
- WHEN the gate runs
- THEN it reports a phase/artifact mismatch for that phase, classified
  `MachineVerifiable`

#### Scenario: Complete phase with a present artifact is not flagged
- GIVEN a `state.yaml` phase with `status: complete` and `artifact: design.md`,
  and a `design.md` present in the change directory
- WHEN the gate runs
- THEN it reports no phase/artifact mismatch for that phase

#### Scenario: A skipped phase without an artifact is not flagged
- GIVEN a `state.yaml` phase with `status: skipped` that declares no `artifact`
  and carries only a `note`
- WHEN the gate runs
- THEN it reports no phase/artifact mismatch for that phase

### Requirement: Every Finding Is Classified Machine-Verifiable Or Human-Review

The gate MUST classify every finding it reports as exactly one of
`MachineVerifiable` or `HumanReview`. A finding derivable purely from repository
file bytes MUST be classified `MachineVerifiable`. A finding requiring human
semantic judgment or an external source of truth (such as GitHub issue state)
MUST be classified `HumanReview`. The gate's exit status MUST be driven only by
`MachineVerifiable` findings; a `HumanReview` finding MUST NOT by itself cause a
non-zero exit.

#### Scenario: A machine-verifiable finding fails the gate
- GIVEN at least one `MachineVerifiable` finding (e.g. a stale-unarchived change)
- WHEN the gate finishes
- THEN it exits non-zero and reports that finding tagged as machine-verifiable

#### Scenario: A human-review finding does not fail the gate
- GIVEN a run whose only findings are classified `HumanReview` and no
  `MachineVerifiable` finding exists
- WHEN the gate finishes
- THEN it exits zero and still reports each human-review finding tagged as such

### Requirement: Core Validation Runs Fully Offline

The gate's core validation — the source-of-truth reads and the stale-unarchived,
archived-name-duplication, and phase/artifact detection rules — MUST complete
using only local repository files, with no network access and no GitHub API call.
The gate MUST run as one local command and MUST NOT require any CI system. On any
`MachineVerifiable` finding it MUST exit non-zero and print a human-readable
report of every finding.

#### Scenario: Core validation completes with no network available
- GIVEN a repository checkout with no network connectivity and no GitHub access
- WHEN a developer runs the gate locally
- THEN the core validation completes and reports every machine-verifiable
  finding, having made no network or GitHub call

#### Scenario: A clean repository exits zero locally
- GIVEN a repository with no machine-verifiable inconsistency, run offline
- WHEN the gate runs locally
- THEN it exits zero and prints a report indicating no machine-verifiable
  findings, with no CI system invoked

### Requirement: GitHub Issue-State Cross-Check Is Optional And Degrades Gracefully

The GitHub issue-state cross-check MUST be optional online enrichment only. When
an authenticated, reachable GitHub client is available, the cross-check MAY add
findings (for example, a closed issue linked to a change that is still
un-archived with `apply` and `verify` complete); every such finding MUST be
classified `HumanReview`. When GitHub is unavailable — offline, no client
present, unauthenticated, or rate-limited — the cross-check MUST be skipped and
recorded as skipped, and it MUST NOT cause the gate to fail. The cross-check MUST
NOT be required for, and MUST NOT alter, the offline core validation's result.

#### Scenario: Unavailable GitHub is skipped, not failed
- GIVEN the gate runs with no authenticated, reachable GitHub client
- WHEN the cross-check is attempted
- THEN it is recorded as skipped with a reason, the exit status is unchanged,
  and the offline core validation still produces its full result

#### Scenario: Available GitHub surfaces a closed-issue finding as human-review
- GIVEN an authenticated, reachable GitHub client and a closed issue linked to a
  change that is still un-archived with `apply` and `verify` complete
- WHEN the cross-check runs
- THEN it reports a finding for that change classified `HumanReview`, and this
  finding alone does not cause a non-zero exit

#### Scenario: The cross-check never changes the offline result
- GIVEN identical local repository files evaluated once with GitHub available and
  once with GitHub unavailable
- WHEN the gate runs in each condition
- THEN the set of `MachineVerifiable` findings and the resulting exit status are
  identical in both runs

### Requirement: Integrated As An xtask Subcommand Consistent With Existing Gates

The gate MUST be invocable as a single local `xtask` subcommand, consistent with
the existing `verify-layers`, `verify-isolation`, and `verify-hygiene`
subcommands: it MUST return success/failure through the same exit-code contract
(zero when clean, non-zero on any `MachineVerifiable` finding) and print a
human-readable report. Invoking the gate MUST NOT require modifying the semantics
of any existing subcommand.

#### Scenario: The gate is one subcommand alongside the existing ones
- GIVEN the existing `xtask` gate subcommands
- WHEN the documentation-consistency gate is invoked
- THEN it runs as one additional subcommand, exits zero when clean and non-zero
  on any machine-verifiable finding, and leaves the existing subcommands'
  behavior unchanged
