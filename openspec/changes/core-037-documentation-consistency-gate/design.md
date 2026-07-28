# Design: CORE-037 — Documentation Consistency Gate

## Technical Approach

A new `xtask` subcommand `verify-doc-consistency` performs a mechanically
verifiable consistency check over project state. The whole gate is a pure function
of repository files — there are no tiers: for every change directory under `openspec/changes/`
(excluding `archive/`), it parses `state.yaml`, then evaluates three
file-derivable detection rules — stale-unarchived, archived-name duplication,
and phase-status-vs-artifact-presence — collecting a list of `Finding`s. Every
finding is classified `MachineVerifiable` or `HumanReview`. The command exits
non-zero if and only if at least one `MachineVerifiable` finding exists, and
always prints a human-readable, per-change report. This mirrors the existing
gate contract: one local command, non-zero on violation, readable report
(`foundation-integrity` FR-004; `xtask/src/main.rs:38-51`).

There is **no** GitHub issue-state cross-check. An earlier draft specified one as
optional online enrichment, but nothing in this repository declares which issue a
change is linked to: `state.yaml` has no issue field, and scraping a number out of
free-text `notes` is ambiguous parsing rather than a contract, so the rule could
be neither implemented nor verified deterministically. Defining that link is a
prerequisite change, not a side effect of this one — see ADR-1. The gate is
therefore offline in whole, not offline-in-core.

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
| New capability | `foundation-integrity` fixes the contract of the **code/workspace foundation** gate — layer-map completeness, dependency direction, dependency-cycle freedom, per-crate isolation compilation, plus the narrow stale-change *duplication* hygiene rule (`openspec/specs/foundation-integrity/spec.md:1-11`). This change is about **documentation/state consistency** — a distinct concern with its own source-of-truth model and its own subcommand. Distinct concern ⇒ distinct capability. | **Chosen** |
| Modify `foundation-integrity` | Would overload a code-integrity capability with documentation-state semantics and risk re-scoping FR-006. The gate only needs to **reuse** FR-006's duplication rule, not restate or change it. | Rejected |

FR-006 (stale-change hygiene, `foundation-integrity/spec.md:81-91`) is preserved
**unchanged**: `verify-hygiene` keeps its exact suffix-match duplication
semantics. This gate's requirement for the same duplication class is satisfied
by reusing that rule's logic, and the two commands coexist. No delta to
`foundation-integrity` is emitted by this change.

### ADR-1 (DECISION 1): Issue-state cross-check → **Excluded; it has no linkage contract to build on**

**Choice**: ship no GitHub cross-check. The gate is offline in whole.

**Why the earlier two-tier design could not be implemented.** A cross-check needs
to answer "which issue is this change linked to?", and no artifact answers it:

| Candidate source | Why it fails |
|---|---|
| `state.yaml` | Has no issue field. Its schema is `change_id`, `title`, `status`, `artifact_store_mode`, `phases`, `notes` — nothing structured about issues |
| The typed change model | Cannot expose a number that its input never carries |
| `notes` free text | A number like `#237` appears in prose. Extracting it is ambiguous parsing, not a contract: no defined position, no cardinality, no way to distinguish "the linked issue" from an incidentally-mentioned one |
| `proposal` / `design` / `tasks` | None fixes a format or a cardinality either |

Without a contract there is no deterministic rule, so a specified
`IssueClosedButUnarchived` finding would be unimplementable and unverifiable —
worse than absent, because it reads as covered.

**Rejected**: (a) parse `#NNN` out of `notes` — the ambiguity above, and it makes
gate behavior depend on prose editing; (b) add an `issues: [237]` field to
`state.yaml` inside CORE-037 — that grows this change into an SDD-schema change,
and every existing `state.yaml` would need an absence policy; the link deserves
its own change that defines field, location, cardinality (including empty and
absent) and ownership; (c) keep the cross-check "specified but unimplemented" —
exactly the ceremonial-requirement failure this design is trying to avoid.

**Consequence**: GitHub joins `tasks.md`, `ROADMAP.md` and `openspec/specs/` as a
**declared but unvalidated** source of truth. The declaration still binds — no
later rule may invent a second source for issue state — while no rule here reads
it. A follow-up change may define the link and then add the cross-check on top of
the `HumanReview` channel this design already specifies.

### ADR-2 (DECISION 2): Which inconsistencies are mechanical vs human-review

A finding is **`MachineVerifiable`** iff it is a deterministic function of
repository file bytes alone — no semantic prose judgment and no external system.
A finding is **`HumanReview`** iff confirming it requires human semantic judgment
or an external source of truth.

**Rules this change ships** — all `MachineVerifiable`, all gate-failing:

| Inconsistency | Class | Why | Fails gate? |
|---|---|---|---|
| Stale-unarchived: `apply.status: complete` ∧ `verify.status: complete` ∧ `archive.status: pending`, dir still under `openspec/changes/` | `MachineVerifiable` | Pure read of `state.yaml` phase fields + directory location | Yes |
| Archived-name duplication (un-archived dir suffix-matches an `archive/<date>-<name>`) | `MachineVerifiable` | Existing FR-006 rule; directory-name comparison only | Yes |
| Phase-status vs artifact presence, BOTH directions: a phase with `status: complete` whose declared `artifact` is absent, or a declared artifact present while its phase is neither `complete` nor `skipped` | `MachineVerifiable` | Pure `state.yaml` field ↔ filesystem existence check | Yes |

**Classified but NOT shipped** — recorded so the taxonomy is complete and so a
later change inherits the advisory channel rather than inventing one:

| Inconsistency | Would be | Why it is not shipped |
|---|---|---|
| A closed GitHub issue linked to a still-un-archived, apply+verify-complete change | `HumanReview` | No change-to-issue linkage contract exists (ADR-1). Excluded entirely, not specified-and-skipped |
| `ROADMAP.md` prose disagreeing with a shipped/archived change or with `openspec/specs/` canonical behavior | `HumanReview` | Prose-vs-prose semantic comparison; not file-byte-derivable. `ROADMAP.md` and `openspec/specs/` are declared sources with no rule reading them |
| Operation-naming / External Data Providers doc content drift | `HumanReview` | Content-correctness judgment, explicitly out of scope as a *fix* |

Only `MachineVerifiable` findings drive the exit code. Since every shipped rule is
`MachineVerifiable`, **`HumanReview` has no producer in this change** — the class
and the reducer branch are specified and tested anyway so the first rule needing
human judgment has an advisory channel instead of a choice between failing the
gate and staying silent.

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
subcommands convention; (c) adding a CLI-args parser/`clap` — the gate takes no
options, so no new arg surface is needed.

The command runs one unconditional offline pass. Report and exit-code semantics
match the existing gates so it composes with them locally (this repo has no CI —
`foundation-integrity` FR-004; `prod-005-runtime-health-model/state.yaml:31-34`
records gates run locally).

## Data Flow

    verify-doc-consistency
      │
      ├─ THE WHOLE GATE (repository files only; no tiers, no network) ──┐
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
      │                                                                  │
      │    NO network tier: no gh probe, no HTTP client, no issue state   │
      │    (ADR-1 — the change-to-issue link has no contract to read)     │
      │                                                                  │
      └─ REPORT + EXIT ─────────────────────────────────────────────────┘
           print per-change findings, each tagged Machine|Human
           (in this change every finding is Machine)
           exit non-zero iff any MachineVerifiable finding exists

### Sequence: the only run mode there is

    Dev        xtask(verify-doc-consistency)   FS(openspec/changes)
     │─run──────────▶│
     │               │─read state.yaml (each change)─▶│
     │               │◀── phases, artifacts ──────────│
     │               │  evaluate rules a/b/c (pure)
     │               │  no external call of any kind
     │◀─ report + exit(1 iff any MachineVerifiable finding) ─┤

## File Changes

> All rows are FUTURE work specified by this change. No `xtask` or production
> source is written, modified, or deleted here.

| File | Action | Description |
|------|--------|-------------|
| `xtask/Cargo.toml` | Modify (FUTURE) | Add a YAML parser dependency — `serde_yaml = "0.9"` — under `[dependencies]`. **This is a hard prerequisite, not a detail:** `xtask` today depends only on `serde`, `serde_json`, `toml` and `anyhow`, so it can parse JSON and TOML but **not** YAML, and the `serde`-derived `state.yaml` model below cannot be deserialized without it. `serde` with `derive` is already present, so only the format crate is added |
| `xtask/src/main.rs` | Modify (FUTURE) | Add `Some("verify-doc-consistency") => verify_doc_consistency()?` arm to the dispatch (`main.rs:9-19`); extend the usage string; add `fn verify_doc_consistency() -> anyhow::Result<bool>` mirroring the existing `verify_*` fns |
| `xtask/src/state_yaml.rs` | Create (FUTURE) | `serde`-derived typed model of the `state.yaml` schema (`change_id`, `title`, `status`, `artifact_store_mode`, `phases.{proposal,explore,spec,design,tasks,apply,verify,archive}.{status,artifact,note}`, `notes`) + loader |
| `xtask/src/doc_consistency.rs` | Create (FUTURE) | `Finding`, `FindingKind`, `FindingClass { MachineVerifiable, HumanReview }`; the three offline rules; report formatter; `fn check_doc_consistency(changes_dir, workspace_root) -> Vec<Finding>` |

No `doc_consistency/github.rs` and no network module: per ADR-1 the issue-state
cross-check is excluded until a change defines the change-to-issue link.

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
    /// Phase status disagrees with presence of the declared artifact,
    /// in either direction (complete+absent, or present+not-complete/skipped).
    PhaseArtifactMismatch { phase: String, artifact: String },
    // No IssueClosedButUnarchived variant: ADR-1 excludes the cross-check until
    // a change defines the change-to-issue link as structured data.
}

pub struct Finding {
    pub change_id: String,
    pub kind: FindingKind,
    pub class: FindingClass,
    pub detail: String, // human-readable; never a raw credential/token
}

/// The whole gate — a pure function of repository files; performs no network I/O.
pub fn check_doc_consistency(changes_dir: &Path, workspace_root: &Path)
    -> anyhow::Result<Vec<Finding>>;

/// Exit reducer. Specified with both branches even though no shipped rule emits
/// HumanReview, so a later advisory rule inherits the channel.
pub fn any_gate_failing(findings: &[Finding]) -> bool; // true iff any MachineVerifiable

// xtask/src/main.rs — dispatch fn, mirrors verify_hygiene (main.rs:78-92).
fn verify_doc_consistency() -> anyhow::Result<bool>; // Ok(true) clean; Ok(false) any MachineVerifiable
```

## Error Model

- Malformed or unreadable `state.yaml`, and a change directory missing
  `state.yaml`, are themselves `MachineVerifiable` findings (the file is a
  required source of truth), not process aborts — the gate reports them and
  fails, rather than panicking.
- There is no external-availability error class to model, because there is no
  external call: no `gh` probe, no HTTP client, no rate-limit or auth path. Every
  error the gate can produce comes from reading local files.
- Reports print only change ids, phase names and artifact paths. With no external
  source consulted, no credential, token or issue text can reach the output.

## Observability

Single human-readable report to stdout, grouped per change, each finding tagged
`[machine]` or `[human]`, matching the existing gates' `OK (...)` / `FAIL (n
violation(s))` style (`xtask/src/main.rs:38-51`). Every finding this change emits
is tagged `[machine]`; the `[human]` tag is part of the report format so a later
advisory rule renders consistently. No metrics, no tracing — this is a local
developer/maintainer tool.

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit (offline) | `state.yaml` parse: all phase fields, `status`, `notes`, artifact/note presence | `tempfile` fixtures, no network |
| Unit (offline) | Rule a — stale-unarchived: apply+verify complete ∧ archive pending ⇒ `MachineVerifiable`; negative: archive complete ⇒ no finding | fixture dirs with crafted `state.yaml` |
| Unit (offline) | Rule b — archived-name duplication preserved (FR-006 parity, positive + negative) | mirror `hygiene.rs` tests against the new rule |
| Unit (offline) | Rule c — phase-status vs artifact presence, BOTH directions. Positives: (1) `complete` + artifact missing; (2) `pending` + artifact present. Negatives: (3) `complete` + present; (4) `skipped` + present; (5) `skipped` + absent; (6) no `artifact` declared. Cases 4-6 are the three `skipped` combinations, all unflagged because `skipped` constrains presence in neither direction | fixtures with/without artifact files |
| Unit (offline) | Classification — every emitted finding carries a `FindingClass`; the three rules are `MachineVerifiable` | assert on `Finding.class` |
| Unit (offline) | Exit decision — non-zero iff ≥1 `MachineVerifiable`; `HumanReview`-only ⇒ clean exit | drive `check_doc_consistency` + reducer |
| Unit (offline) | No network reachability — no HTTP client and no `gh` invocation is reachable from any gate code path | grep/dependency assertion over the gate module |

Every test is offline and fixture-based; no test requires GitHub access, and there
is no online path to exercise. The reducer's `HumanReview` branch is covered by
constructing a `HumanReview` finding directly in the test, since no shipped rule
emits one.

## Threat Matrix

**No shell, process, or network surface at all.** Excluding the cross-check
(ADR-1) removes the only one this design ever had: the gate spawns no subprocess,
opens no socket, and handles no credential. It reads files under
`openspec/changes/` and `openspec/` that the maintainer already controls, and
prints change ids, phase names and artifact paths. No routing, no auth
delegation, no data-loss surface. Reintroducing a cross-check reintroduces this
surface, and the follow-up change that does so owns re-assessing it.

## Migration / Rollout / Compatibility

Purely additive: a new subcommand and new modules. Existing subcommands and
`verify-hygiene`'s FR-006 semantics are unchanged, so no behavior regresses.
Rollback = remove the new arm and modules. No schema, no production crate, no
runtime impact. The `state.yaml` schema is consumed read-only; this change does
not alter it.

## Open Questions

None blocking. The exact enumerated `FindingKind` set is an implementation detail
of the FUTURE xtask work and may be refined during apply without changing the
fully-offline contract or the machine/human classification.
