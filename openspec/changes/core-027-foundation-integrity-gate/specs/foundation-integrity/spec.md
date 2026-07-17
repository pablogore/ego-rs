# Foundation Integrity Gate Specification

## Purpose

Defines the observable contract for a local integrity gate over this
workspace's foundation: layer-map completeness and accuracy, dependency
direction, dependency-cycle freedom, per-crate isolation compilation,
stale-change hygiene, and resolved flaky-test verdicts. This spec fixes
WHAT the gate guarantees; the checker's implementation shape is a design
decision.

---

## Requirements

### FR-001 — Complete And Accurate Layer Map

The layer map MUST assign exactly one layer to every crate in the
workspace. The layer map MUST NOT contain an entry naming a crate that
does not exist in the workspace.

#### Scenario: Unmapped crate fails the gate
- GIVEN a workspace crate has no entry in the layer map
- WHEN the gate runs
- THEN it fails with a report identifying the unmapped crate

#### Scenario: Dead layer-map entry fails the gate
- GIVEN the layer map contains an entry naming a crate absent from the
  workspace
- WHEN the gate runs
- THEN it fails with a report identifying the dead entry

### FR-002 — Dependency Direction Enforcement

A crate's dependencies MUST NOT violate the documented layer direction:
transport/infrastructure MAY depend on domain and application only;
application MAY depend on domain only; domain MUST NOT depend on any
other layer.

#### Scenario: Wrong-direction dependency fails the gate
- GIVEN a crate in one layer depends on a crate in a layer the direction
  rules forbid
- WHEN the gate runs
- THEN it fails, identifying the offending crate and the violated
  direction rule

### FR-003 — Dependency Cycle Freedom

The workspace dependency graph MUST NOT contain a cycle among crates.

#### Scenario: Cyclic dependency fails the gate
- GIVEN two or more crates form a dependency cycle
- WHEN the gate runs
- THEN it fails, identifying the crates forming the cycle

### FR-004 — Single Local, Reportable Command

The gate MUST run as one local command, without any CI system. On any
violation it MUST exit non-zero and print a human-readable report of
every violation found.

#### Scenario: Local run reports failure without CI
- GIVEN a workspace with at least one violation
- WHEN a developer runs the gate command locally
- THEN it exits non-zero and prints a human-readable report, with no CI
  system invoked

### FR-005 — Per-Crate Isolation Compilation

Every workspace crate MUST compile under its own narrowest supported
feature set, independent of workspace-wide feature unification. A crate
that compiles only because another crate's features are unified in MUST
be reported as a failure.

#### Scenario: Isolation-only failure surfaces despite workspace build passing
- GIVEN a crate that compiles under full workspace feature unification
  but not under its own narrowest feature set
- WHEN the gate checks that crate in isolation
- THEN it reports a failure for that crate

### FR-006 — Stale-Change Hygiene

An un-archived change under `openspec/changes/` that duplicates a change
already present under `openspec/changes/archive/` MUST fail the hygiene
check.

#### Scenario: Un-archived duplicate of an archived change fails
- GIVEN an archived change exists and an un-archived duplicate of it also
  exists under `openspec/changes/`
- WHEN the hygiene check runs
- THEN it fails, identifying the duplicate

### FR-007 — Resolved Flaky-Test Verdicts

Each of the three flaky-test suspects — `persistent-entity` concurrent
spawn, provider-access under parallel execution, and effects
deadline/cancellation — MUST have a recorded, evidence-backed verdict of
either "still reproduces, now fixed" or "no longer reproduces." No
suspect may remain unresolved or unrecorded.

#### Scenario: Every suspect has a recorded verdict
- GIVEN the three flaky-test suspects
- WHEN triage completes
- THEN each has a recorded verdict backed by run evidence, and none is
  left unresolved
