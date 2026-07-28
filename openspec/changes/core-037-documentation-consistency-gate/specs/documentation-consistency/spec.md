# Documentation Consistency Gate Specification

## Purpose

Defines the observable contract for a local, fully offline consistency gate over
this project's tracked state. Project state is spread across files that are meant
to agree: change lifecycle status in each change's `state.yaml`, task completion
in `tasks.md`, canonical behavior in `openspec/specs/`, the roadmap in
`ROADMAP.md`, and issue open/closed state on GitHub. This spec fixes WHAT the
gate guarantees — a declared source of truth per state type, detection of the
file-derivable inconsistencies, a machine-verifiable/human-review classification
of every finding, and fully offline execution — not the checker's implementation
shape.

**Declared sources and validated sources are not the same set.** All five state
types get a declared source of truth so no later rule invents a competing one,
but this gate reads only `state.yaml` and the filesystem. `tasks.md`,
`ROADMAP.md`, `openspec/specs/` and GitHub are declared and unvalidated here, and
the gate must not present them as covered.

Out of scope: any concrete reconciliation (e.g. archiving a specific change,
fixing operation-naming or External Data Providers documentation), prose-versus-
prose semantic diffing, spec-content correctness checking, any unbounded
"sync all documentation" behavior, and any GitHub issue-state cross-check — the
last because no artifact declares which issue a change is linked to. This spec
defines the GATE, not the fixes.

---

## Requirements

### Requirement: Source Of Truth Is Declared Per State Type

The gate MUST treat exactly one artifact as the source of truth for each state
type: change lifecycle status → the change's `state.yaml`; task completion →
`tasks.md`; issue open/closed state → GitHub; roadmap → `ROADMAP.md`; canonical
behavior → `openspec/specs/`. When a state type has a declared source of truth,
the gate MUST derive that state from that source and MUST NOT infer it from a
different artifact.

**Validation coverage is narrower than this declaration, deliberately.** The
declaration above binds every state type so that no future rule invents a
second source, but CORE-037 ships rules over only one of them — the change's
`state.yaml`, read together with the filesystem — producing exactly three rules:
stale-unarchived detection, phase/artifact agreement, and archive-name
duplication. Task completion (`tasks.md`), roadmap (`ROADMAP.md`), canonical
behavior (`openspec/specs/`) and issue state (GitHub) are declared here and
**not** validated by this change; rules consuming them are future extensions.
The gate MUST NOT report findings about, or claim coverage of, a state type it
does not actually read — an unvalidated declaration is a binding for later work,
never an implied guarantee.

#### Scenario: A declared-but-unvalidated source produces no findings and no coverage claim
- GIVEN a repository whose `tasks.md`, `ROADMAP.md` and `openspec/specs/`
  contain drift relative to a change's `state.yaml`, and a change whose linked
  issue is closed on GitHub
- WHEN the gate runs
- THEN it reports no finding derived from any of those four sources, and its
  report does not present them as validated

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
and the on-disk presence of that phase's declared artifact, in **both**
directions. Each direction is reported as a phase/artifact mismatch classified
`MachineVerifiable`:

- **Direction 1 — status claims more than the disk shows.** A phase whose
  `status` is `complete` and that declares an `artifact` whose file or directory
  is absent MUST be reported.
- **Direction 2 — the disk shows more than the status claims.** A phase that
  declares an `artifact` which IS present on disk while that phase's `status` is
  neither `complete` nor `skipped` MUST be reported. This catches the common
  drift where a phase's artifact was written but `state.yaml` was never advanced
  past `pending`.

The following MUST NOT be reported by this rule:

- `complete` with its declared artifact present — the agreeing case.
- `skipped` with an artifact present. `skipped` is an explicit maintainer
  decision to bypass a phase, and it is legitimate for a stub or a
  previously-written artifact to remain on disk; a skipped phase therefore
  places no constraint on artifact presence in either direction.
- A phase that declares no `artifact` at all (for example a `skipped` phase
  carrying only a `note`) — with nothing declared, there is nothing to compare.

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

#### Scenario: Pending phase with its declared artifact present is flagged
- GIVEN a `state.yaml` phase with `status: pending` and `artifact: design.md`,
  and a `design.md` present in the change directory
- WHEN the gate runs
- THEN it reports a phase/artifact mismatch for that phase, classified
  `MachineVerifiable`, because the artifact exists while the phase is neither
  `complete` nor `skipped`

#### Scenario: Skipped phase with an artifact present is not flagged
- GIVEN a `state.yaml` phase with `status: skipped` and `artifact: design.md`,
  and a `design.md` present in the change directory
- WHEN the gate runs
- THEN it reports no phase/artifact mismatch for that phase, because `skipped`
  places no constraint on artifact presence in either direction

#### Scenario: Skipped phase with a declared artifact absent is not flagged
- GIVEN a `state.yaml` phase with `status: skipped` and `artifact: design.md`,
  and NO `design.md` in the change directory
- WHEN the gate runs
- THEN it reports no phase/artifact mismatch for that phase — direction 1 applies
  only to `complete`, and `skipped` is excluded from direction 2, so all three
  `skipped` combinations are unflagged

#### Scenario: A skipped phase without an artifact is not flagged
- GIVEN a `state.yaml` phase with `status: skipped` that declares no `artifact`
  and carries only a `note`
- WHEN the gate runs
- THEN it reports no phase/artifact mismatch for that phase

### Requirement: Every Finding Is Classified Machine-Verifiable Or Human-Review

The gate MUST classify every finding it reports as exactly one of
`MachineVerifiable` or `HumanReview`. A finding derivable purely from repository
file bytes MUST be classified `MachineVerifiable`. A finding requiring human
semantic judgment or an external source of truth MUST be classified
`HumanReview`. The gate's exit status MUST be driven only by `MachineVerifiable`
findings; a `HumanReview` finding MUST NOT by itself cause a non-zero exit.

Every rule CORE-037 ships is a pure read of repository bytes, so **every finding
this change can emit is `MachineVerifiable`, and `HumanReview` currently has no
producer.** The class and the exit reducer are specified anyway, so that the
first rule needing human judgment — an issue-state cross-check, a roadmap check —
inherits an advisory channel instead of being forced to either fail the gate or
stay silent. The reducer's `HumanReview` branch MUST therefore be specified and
tested even though no shipped rule reaches it.

#### Scenario: A machine-verifiable finding fails the gate
- GIVEN at least one `MachineVerifiable` finding (e.g. a stale-unarchived change)
- WHEN the gate finishes
- THEN it exits non-zero and reports that finding tagged as machine-verifiable

#### Scenario: A human-review finding does not fail the gate
- GIVEN a finding set containing only `HumanReview` findings and no
  `MachineVerifiable` finding — constructible directly in tests, since no
  shipped rule emits `HumanReview`
- WHEN the exit reducer is applied
- THEN it yields the clean/zero result and still reports each human-review
  finding tagged as such

### Requirement: Validation Runs Fully Offline

The gate MUST complete using only local repository files, with no network access
and no GitHub API call — this covers the whole gate, not a subset of it, because
every rule it ships is a pure read of repository bytes. The gate MUST run as one
local command and MUST NOT require any CI system. On any `MachineVerifiable`
finding it MUST exit non-zero and print a human-readable report of every finding.

#### Scenario: Validation completes with no network available
- GIVEN a repository checkout with no network connectivity and no GitHub access
- WHEN a developer runs the gate locally
- THEN the gate completes in full and reports every machine-verifiable finding,
  having made no network or GitHub call

#### Scenario: No network client is reachable from the gate at all
- GIVEN the gate's implementation
- WHEN its call graph is inspected
- THEN no HTTP client and no `gh` invocation is reachable from any code path,
  since no rule consults an external source

#### Scenario: A clean repository exits zero locally
- GIVEN a repository with no machine-verifiable inconsistency, run offline
- WHEN the gate runs locally
- THEN it exits zero and prints a report indicating no machine-verifiable
  findings, with no CI system invoked

### Requirement: No Issue-State Cross-Check Without A Declared change-to-issue Link

The gate MUST NOT attempt any GitHub issue-state cross-check, because no artifact
in this repository declares which issue a change is linked to. `state.yaml` has no
issue field, and scraping an issue number out of free-text `notes` is ambiguous
parsing, not a contract — a rule built on it could be neither implemented nor
verified deterministically.

A cross-check MAY be introduced only by a later change that FIRST defines the
link as structured data: the field and its location, its cardinality (including
the empty and absent cases), and which side owns it. Until such a contract
exists, GitHub remains a declared source of truth with no rule reading it, exactly
like `tasks.md`, `ROADMAP.md` and `openspec/specs/`.

#### Scenario: The gate performs no issue-state lookup
- GIVEN a change whose `notes` mention an issue number in free text
- WHEN the gate runs
- THEN it performs no GitHub lookup, derives no issue state, and reports no
  finding about issue open/closed state

#### Scenario: The result depends only on repository files
- GIVEN identical local repository files, and any state of network or GitHub
  availability
- WHEN the gate runs
- THEN the set of `MachineVerifiable` findings and the resulting exit status are
  identical, because no external source is consulted in any run

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
