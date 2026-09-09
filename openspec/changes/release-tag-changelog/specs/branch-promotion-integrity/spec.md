# Branch Promotion Integrity Specification

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## Purpose

Defines the documented branch-promotion and hotfix-backport policy, and the observable contract of
the recurring check that detects when a hotfix landing on `main` has not reached `develop`.
Detection and reporting only — no enforcement, blocking, or automated remediation is in scope.

## Requirements

### Requirement: The Branching Model And Hotfix Backport Obligation Are Documented

The contributor-facing documentation MUST state the branching model — feature branches promote to
`develop` via pull request, `develop` promotes to `main` via pull request, and a hotfix branches
off `main`, lands on `main` via pull request, and MUST subsequently be backported to `develop` —
as an explicit, discoverable policy.

#### Scenario: A contributor finds the branching model and hotfix obligation

- GIVEN a contributor reads the contributor documentation
- WHEN they look for the branch-promotion or hotfix policy
- THEN they find the branching model and the explicit obligation to backport a hotfix's change to
  `develop`

### Requirement: Backport Drift Detection Runs On A Recurring Schedule

The drift-detection check MUST run on a recurring schedule, independent of any single push or
merge event, so that a hotfix legitimately not yet backported does not need to trigger it.

#### Scenario: Drift detection runs without a triggering push or merge

- GIVEN no push or merge has just occurred
- WHEN the scheduled interval for drift detection elapses
- THEN the drift-detection check runs

### Requirement: A Main-Only Change Absent From Develop Is Reported As Drift, After A Grace Period

The drift-detection check MUST report when a commit's change exists on `main` but no equivalent
change exists on `develop`, once that commit is older than a fixed grace period. Within the grace
period, an as-yet-unbackported change MUST NOT be reported, so a contributor has a real window to
backport before the check treats the gap as drift.

#### Scenario: An un-backported hotfix older than the grace period is reported

- GIVEN a hotfix commit landed on `main`, its change is absent from `develop`, and the commit is
  older than the grace period
- WHEN drift detection next runs
- THEN that hotfix's change is reported as drift

#### Scenario: An un-backported hotfix within the grace period is not yet reported

- GIVEN a hotfix commit landed on `main`, its change is absent from `develop`, and the commit is
  younger than the grace period
- WHEN drift detection next runs
- THEN that hotfix's change is not reported as drift

### Requirement: Backport Recognition Uses Patch Equivalence, Not Commit Identity

The drift-detection check MUST determine whether a main-only change has reached `develop` by
comparing the effective change content, not by comparing commit SHAs. A change backported via
cherry-pick, which produces a different commit identity than the original, MUST NOT be reported as
drift once its equivalent content is present on `develop`.

#### Scenario: A cherry-picked backport is not reported as drift

- GIVEN a hotfix commit landed on `main` and was later backported to `develop` by cherry-pick,
  producing a different commit identity
- WHEN drift detection next runs
- THEN that hotfix's change is not reported as drift

#### Scenario: A normally merged backport is not reported as drift

- GIVEN a hotfix commit landed on `main` and its change reached `develop` through a regular merge
- WHEN drift detection next runs
- THEN that hotfix's change is not reported as drift

### Requirement: Drift Detection Is Non-Blocking, Report-Only, With No Automated Remediation

The drift-detection check MUST NOT block, fail, or prevent any merge, push, or other repository
operation. It MUST NOT create a pull request, commit, or any other repository-modifying action on
behalf of a contributor.

#### Scenario: Drift detection reports without blocking any operation

- GIVEN drift is present between `main` and `develop`
- WHEN a contributor merges an unrelated pull request or pushes to either branch
- THEN that operation succeeds and is not blocked or delayed by the drift report

#### Scenario: Drift detection takes no repository-modifying action

- GIVEN drift is reported
- WHEN the repository is inspected afterward
- THEN no pull request, commit, or branch was created or modified by the drift-detection check

## Non-Goals

- Automatically opening a backport pull request for a detected drift.
- Blocking a merge, push, or release because of detected drift.
- Any change to branch protection rules, required reviewers, or required status checks.
- Detecting drift on any branch pair other than `main` and `develop`.
