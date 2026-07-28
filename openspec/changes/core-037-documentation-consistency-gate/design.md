# Design: CORE-037 — Documentation Consistency Gate

## Technical Approach

A new `xtask` subcommand `verify-doc-consistency` performs a mechanically
verifiable consistency check over project state. Its **core** is a pure function
of repository files: for every change directory under `openspec/changes/`
(excluding `archive/`), it parses `state.yaml`, then evaluates three
file-derivable detection rules — stale-unarchived, archived-name duplication,
and phase-status-vs-artifact-presence — collecting a list of `Finding`s. Every
finding is classified `MachineVerifiable` or `HumanReview`. The command exits
non-zero if and only if at least one `MachineVerifiable` finding exists, and
always prints a human-readable, per-change report. This mirrors the existing
gate contract: one local command, non-zero on violation, readable report
(`foundation-integrity` FR-004; `xtask/src/main.rs:38-51`).

An **optional** GitHub issue-state cross-check runs only as online enrichment.
It probes for a reachable, authenticated `gh`; when present it may add
`HumanReview` findings (e.g. a change linked to a closed issue that is still
un-archived with `apply`+`verify` complete). When `gh` is absent, offline, or
unauthenticated, the cross-check contributes a single "skipped" note and changes
nothing about the exit code. The core validation never calls it and never
depends on it.

The current gate has three subcommands and the stale-unarchived case slips
through: `verify-hygiene` (`xtask/src/hygiene.rs:9-43`) only flags an
un-archived dir that suffix-matches an `archive/<date>-<name>` dir
(`hygiene.rs:37-39`); it never opens `state.yaml`. The live example is this very
repository: `prod-005-runtime-health-model/state.yaml` records `apply: complete`
(`state.yaml:23-24`), `verify: complete` (`state.yaml:29-30`), and
`archive: pending` (`state.yaml:35-36`) while the directory still sits under
`openspec/changes/` — `verify-hygiene` passes it clean. This gate is the
detector for exactly that class of drift (the concrete PROD-005 archiving is out
of scope — see the proposal).

## Architecture Decisions

### ADR-0 (spec placement): New capability `documentation-consistency` → **New capability, not a `foundation-integrity` delta**

**Choice**: create a NEW capability spec `documentation-consistency` rather than
add requirements to `foundation-integrity`.
**Rejected**: a MODIFIED delta extending `foundation-integrity`.
**Rationale**:

| Option | Tradeoff | Verdict |
|---|---|---|
| New capability | `foundation-integrity` fixes the contract of the **code/workspace foundation** gate — layer-map completeness, dependency direction, dependency-cycle freedom, per-crate isolation compilation, plus the narrow stale-change *duplication* hygiene rule (`openspec/specs/foundation-integrity/spec.md:1-11`). This change is about **documentation/state consistency** — a distinct concern with its own source-of-truth model, its own offline/online split, and its own subcommand. Distinct concern ⇒ distinct capability. | **Chosen** |
| Modify `foundation-integrity` | Would overload a code-integrity capability with documentation-state semantics and risk re-scoping FR-006. The gate only needs to **reuse** FR-006's duplication rule, not restate or change it. | Rejected |

FR-006 (stale-change hygiene, `foundation-integrity/spec.md:81-91`) is preserved
**unchanged**: `verify-hygiene` keeps its exact suffix-match duplication
semantics. This gate's requirement for the same duplication class is satisfied
by reusing that rule's logic, and the two commands coexist. No delta to
`foundation-integrity` is emitted by this change.

### ADR-1 (DECISION 1): Offline-first core with optional online enrichment → **Two-tier: pure-file core, gated GitHub cross-check**

**Choice**: split the gate into a mandatory offline core (repository files only)
and an optional online enrichment tier (GitHub issue state). The core computes
every gate-failing (`MachineVerifiable`) finding; the enrichment tier can only
add non-failing (`HumanReview`) findings.
**Rejected**: (a) a single tier that queries GitHub inline — makes the local run
network-dependent and non-deterministic; (b) making the GitHub check mandatory
with a "fail-open on error" flag — conflates absence with success and invites
silent skips of a check that was meant to run.
**Rationale**:

| Aspect | Offline core | Optional GitHub cross-check |
|---|---|---|
| Inputs | `openspec/changes/**`, `openspec/specs/**`, `ROADMAP.md` | GitHub issue open/closed via authenticated `gh` |
| Availability | Always (no network) | Only when `gh` is present, authenticated, and reachable |
| On unavailability | N/A | Recorded as **skipped**, exit code unaffected |
| Finding class | `MachineVerifiable` (fails gate) | `HumanReview` (never fails gate) |
| Determinism | Deterministic from repo bytes | Best-effort, advisory only |

The availability probe is explicit and its result is one of `Available` or
`Unavailable(reason)`; `Unavailable` is a normal, non-error outcome. This keeps
the local developer experience fully offline (constraint: "validating the LOCAL
repository MUST NOT require GitHub API access or network connectivity") while
still allowing a maintainer with an authenticated `gh` to surface issue-linked
drift as advice.

### ADR-2 (DECISION 2): Which inconsistencies are mechanical vs human-review

A finding is **`MachineVerifiable`** iff it is a deterministic function of
repository file bytes alone — no semantic prose judgment and no external system.
A finding is **`HumanReview`** iff confirming it requires human semantic judgment
or an external source of truth (GitHub).

| Inconsistency | Class | Why | Fails gate? |
|---|---|---|---|
| Stale-unarchived: `apply.status: complete` ∧ `verify.status: complete` ∧ `archive.status: pending`, dir still under `openspec/changes/` | `MachineVerifiable` | Pure read of `state.yaml` phase fields + directory location | Yes |
| Archived-name duplication (un-archived dir suffix-matches an `archive/<date>-<name>`) | `MachineVerifiable` | Existing FR-006 rule; directory-name comparison only | Yes |
| Phase-status vs artifact presence: a phase with `status: complete` whose declared `artifact` is absent, or an artifact present while its phase is not `complete`/`skipped` | `MachineVerifiable` | Pure `state.yaml` field ↔ filesystem existence check | Yes |
| A closed GitHub issue linked to a still-un-archived, apply+verify-complete change | `HumanReview` | Depends on GitHub (external), and "closed issue ⇒ should be archived" is a judgment, not a guarantee | No |
| `ROADMAP.md` prose disagreeing with a shipped/archived change or with `openspec/specs/` canonical behavior | `HumanReview` | Prose-vs-prose semantic comparison; not file-byte-derivable | No |
| Operation-naming / External Data Providers doc content drift | `HumanReview` | Content-correctness judgment, explicitly out of scope as a *fix* | No |

The gate reports both classes but only `MachineVerifiable` findings drive the
exit code. `HumanReview` findings are advisory output so a maintainer can act on
them without the gate blocking unrelated local work.

### ADR-3 (DECISION 3): Integration point in xtask — subcommand shape (specified, not implemented)

**Choice**: add exactly one subcommand `verify-doc-consistency` to the existing
`match cmd.as_deref()` dispatch (`xtask/src/main.rs:9-19`), following the same
shape as `verify_layers` / `verify_isolation` / `verify_hygiene`: a
`fn verify_doc_consistency() -> anyhow::Result<bool>` that returns `Ok(true)`
when clean and `Ok(false)` on any `MachineVerifiable` finding, with `main`
mapping `false → exit(1)` via the existing `std::process::exit(if passed {0} else {1})`
(`main.rs:20`).
**Rejected**: (a) a flag on `verify-hygiene` — overloads a preserved,
unchanged check; (b) a separate binary — inconsistent with the one-`xtask`-many-
subcommands convention; (c) adding a CLI-args parser/`clap` — the online cross-
check toggle is an availability *probe*, not a required flag, so no new arg
surface is needed for the core contract.

The command runs the offline core unconditionally, then attempts the GitHub
cross-check as best-effort enrichment. Report and exit-code semantics match the
existing gates so it composes with them locally (this repo has no CI —
`foundation-integrity` FR-004; `prod-005-runtime-health-model/state.yaml:31-34`
records gates run locally).

## Data Flow

    verify-doc-consistency
      │
      ├─ OFFLINE CORE (repository files only; no network) ──────────────┐
      │    for each dir D under openspec/changes/ (skip archive/):       │
      │      parse D/state.yaml (change_id, status, phases.*, notes)     │
      │      RULE a: apply=complete ∧ verify=complete ∧ archive=pending  │
      │              ⇒ Finding(StaleUnarchived, MachineVerifiable)       │
      │      RULE b: name suffix-matches archive/<date>-<name>           │
      │              ⇒ Finding(ArchivedNameDuplicate, MachineVerifiable) │
      │      RULE c: phase.status=complete but declared artifact absent  │
      │              (or artifact present while phase ≠ complete/skipped) │
      │              ⇒ Finding(PhaseArtifactMismatch, MachineVerifiable) │
      │    collect Vec<Finding>                                          │
      │                                                                  │
      ├─ OPTIONAL GITHUB CROSS-CHECK (enrichment) ──────────────────────┤
      │    probe gh availability ─▶ Available | Unavailable(reason)      │
      │      Available:   closed-issue-linked-to-unarchived-change       │
      │                   ⇒ Finding(IssueClosedButUnarchived, HumanReview)│
      │      Unavailable: emit "skipped: <reason>"  (exit unaffected)    │
      │                                                                  │
      └─ REPORT + EXIT ─────────────────────────────────────────────────┘
           print per-change findings, each tagged Machine|Human
           exit non-zero iff any MachineVerifiable finding exists

### Sequence: run with GitHub offline

    Dev        xtask(verify-doc-consistency)   FS(openspec/changes)   gh
     │─run──────────▶│
     │               │─read state.yaml (each change)─▶│
     │               │◀── phases, artifacts ──────────│
     │               │  evaluate rules a/b/c (pure)
     │               │─probe availability──────────────────────────────▶│  (no gh / offline)
     │               │◀── Unavailable(offline/unauthenticated) ─────────│
     │               │  record cross-check = skipped (not a failure)
     │◀─ report + exit(1 iff any MachineVerifiable finding) ─┤

## File Changes

> All rows are FUTURE work specified by this change. No `xtask` or production
> source is written, modified, or deleted here.

| File | Action | Description |
|------|--------|-------------|
| `xtask/src/main.rs` | Modify (FUTURE) | Add `Some("verify-doc-consistency") => verify_doc_consistency()?` arm to the dispatch (`main.rs:9-19`); extend the usage string; add `fn verify_doc_consistency() -> anyhow::Result<bool>` mirroring the existing `verify_*` fns |
| `xtask/src/state_yaml.rs` | Create (FUTURE) | `serde`-derived typed model of the `state.yaml` schema (`change_id`, `title`, `status`, `artifact_store_mode`, `phases.{proposal,explore,spec,design,tasks,apply,verify,archive}.{status,artifact,note}`, `notes`) + loader |
| `xtask/src/doc_consistency.rs` | Create (FUTURE) | `Finding`, `FindingKind`, `FindingClass { MachineVerifiable, HumanReview }`; the three offline rules; report formatter; `fn check_doc_consistency(changes_dir, workspace_root) -> Vec<Finding>` |
| `xtask/src/doc_consistency/github.rs` | Create (FUTURE) | `probe_gh() -> CrossCheckAvailability`; the closed-issue cross-check emitting `HumanReview` findings; graceful `Unavailable(reason)` skip path |

## Interfaces / Contracts

```rust
// xtask/src/doc_consistency.rs — PROPOSED (FUTURE), not implemented here.

/// Whether a finding fails the gate or is advisory only.
pub enum FindingClass { MachineVerifiable, HumanReview }

pub enum FindingKind {
    /// apply=complete ∧ verify=complete ∧ archive=pending, still un-archived.
    StaleUnarchived,
    /// Un-archived dir suffix-matches an archive/<date>-<name> (FR-006, reused).
    ArchivedNameDuplicate,
    /// Phase status disagrees with presence of the declared artifact.
    PhaseArtifactMismatch { phase: String, artifact: String },
    /// Closed GitHub issue linked to a still-un-archived, done change (online only).
    IssueClosedButUnarchived { issue: u64 },
}

pub struct Finding {
    pub change_id: String,
    pub kind: FindingKind,
    pub class: FindingClass,
    pub detail: String, // human-readable; never a raw credential/token
}

/// OFFLINE CORE — pure function of repository files; performs no network I/O.
pub fn check_doc_consistency(changes_dir: &Path, workspace_root: &Path)
    -> anyhow::Result<Vec<Finding>>;

// xtask/src/doc_consistency/github.rs — optional online enrichment.
pub enum CrossCheckAvailability { Available, Unavailable(String) }
pub fn probe_gh() -> CrossCheckAvailability;
/// Only ever appends HumanReview findings; Unavailable ⇒ Ok(empty) + skip note.
pub fn issue_cross_check(changes: &[LoadedChange]) -> anyhow::Result<Vec<Finding>>;

// xtask/src/main.rs — dispatch fn, mirrors verify_hygiene (main.rs:78-92).
fn verify_doc_consistency() -> anyhow::Result<bool>; // Ok(true) clean; Ok(false) any MachineVerifiable
```

## Error Model

- Malformed or unreadable `state.yaml`, and a change directory missing
  `state.yaml`, are themselves `MachineVerifiable` findings (the file is a
  required source of truth), not process aborts — the gate reports them and
  fails, rather than panicking.
- The GitHub cross-check never returns `Err` for absence: `gh` missing, not
  authenticated, offline, or rate-limited all map to
  `CrossCheckAvailability::Unavailable(reason)` and a skip note. Only a
  genuinely unexpected internal fault surfaces as `Err`, and even then it MUST
  NOT change the offline core's exit decision.
- Reports are redaction-safe: no issue-body text, tokens, or `gh` auth material
  is printed — only issue numbers, change ids, phase names, and artifact paths.

## Observability

Single human-readable report to stdout, grouped per change, each finding tagged
`[machine]` or `[human]`, matching the existing gates' `OK (...)` / `FAIL (n
violation(s))` style (`xtask/src/main.rs:38-51`). The GitHub cross-check prints
exactly one line stating `available` or `skipped: <reason>`. No metrics, no
tracing — this is a local developer/maintainer tool.

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit (offline) | `state.yaml` parse: all phase fields, `status`, `notes`, artifact/note presence | `tempfile` fixtures, no network |
| Unit (offline) | Rule a — stale-unarchived: apply+verify complete ∧ archive pending ⇒ `MachineVerifiable`; negative: archive complete ⇒ no finding | fixture dirs with crafted `state.yaml` |
| Unit (offline) | Rule b — archived-name duplication preserved (FR-006 parity, positive + negative) | mirror `hygiene.rs` tests against the new rule |
| Unit (offline) | Rule c — phase-status vs artifact presence, positive (complete + missing artifact) and negative (complete + present) | fixtures with/without artifact files |
| Unit (offline) | Classification — every emitted finding carries a `FindingClass`; the three rules are `MachineVerifiable` | assert on `Finding.class` |
| Unit (offline) | Exit decision — non-zero iff ≥1 `MachineVerifiable`; `HumanReview`-only ⇒ clean exit | drive `check_doc_consistency` + reducer |
| Unit (offline) | GitHub cross-check `Unavailable(reason)` ⇒ empty findings + skip note, exit unaffected | inject an `Unavailable` probe; assert no network call and no failure |
| Unit (online, opt) | When `gh` reports a closed issue for an un-archived done change ⇒ one `HumanReview` finding | injected fake issue-state source; still no real network in tests |

All core-rule tests are offline and fixture-based; no test requires GitHub
access. The online path is exercised through an injected issue-state source so
tests stay hermetic.

## Threat Matrix

Shell/process-integration surface: the optional GitHub cross-check shells out to
`gh`. Mitigations: (1) it is never on the core path and never affects exit code;
(2) absence/failure is a graceful skip, never a failure or panic; (3) reports
redact all issue text and auth material, printing only numbers/ids/paths; (4) no
untrusted input is passed to a shell — change ids and issue numbers are read
from repository files the maintainer already controls. No routing, no
credential handling beyond delegating auth to `gh`, no data-loss surface.

## Migration / Rollout / Compatibility

Purely additive: a new subcommand and new modules. Existing subcommands and
`verify-hygiene`'s FR-006 semantics are unchanged, so no behavior regresses.
Rollback = remove the new arm and modules. No schema, no production crate, no
runtime impact. The `state.yaml` schema is consumed read-only; this change does
not alter it.

## Open Questions

None blocking. The exact enumerated `FindingKind` set and the precise `gh`
invocation for the optional cross-check are implementation details of the FUTURE
xtask work and may be refined during apply without changing the offline-first
contract or the machine/human classification.
