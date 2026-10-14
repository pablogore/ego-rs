# Proposal: release-tag-changelog — Release Cutting on `main` + Hotfix Backport Integrity

> Canonical / English. Spanish companion: `proposal.es.md` (1:1 headings).
> ATOMICITY: PASS (two independently revertible slices). BILINGUAL_SYNC: PASS.

## Intent

The repo has never cut a release: zero tags, zero GitHub Releases, no `CHANGELOG.md`, no release
workflow. Merging `develop` → `main` produces no durable, human-readable record of what shipped, even
though commit history is already near-100% Conventional Commits — the input a changelog needs is
present and unused.

Separately, `develop` is ahead of `main` by design, so a hotfix landing on `main` can silently never
reach `develop`. Nothing today enforces or **detects** that. `CONTRIBUTING.md` documents CI in detail
but is silent on the branching model and hotfix policy.

## Scope

### In Scope

- A release is cut **only** when a merge lands on `main` — never on `develop`, never per-PR. One
  release per promotion: an annotated git tag plus a GitHub Release whose body is the
  conventional-commit changelog for the range since the previous tag.
- Version is computed from the conventional commits in that range. Repo-level semver, seeded at
  `v0.1.0` on the current `main` head, staying in `0.x` until an explicit 1.0 decision.
- Release notes are published to the GitHub Release body only. Nothing is written to the repo tree,
  so no automated commit ever targets a protected branch and no bypass token is introduced.
- `CONTRIBUTING.md` gains the branching model (feature → `develop` → `main`; hotfix off `main` →
  `main` → backport to `develop`) and the release process, matching how it already documents CI.
- **Backport drift detection**: a recurring, non-blocking check that reports when `main` holds a
  commit whose change is absent from `develop`. A backport performed by cherry-pick MUST NOT be
  reported as drift.

### Out of Scope

- Per-crate versioning and crates.io publishing. The 22 members keep their independent `0.1.0`; repo
  tag version is independent of crate versions and no manifest is touched.
- `CHANGELOG.md` in the repo tree. The notes live in Releases and are regenerable from git history on
  demand; a tracked file only buys an in-tree copy at the cost of a bot PR per release.
- Blocking or auto-remediating a missed backport (auto-opened backport PRs, merge blocking). Detection
  and reporting only.
- Changing branch protection, adding a bypass app/PAT, or requiring signed commits / linear history.
- Changes to `production-gate.yml`'s gating behavior, and any `.shipwright/workflow.yaml` promotion.

## Capabilities

### New Capabilities

- `release-automation`: what constitutes a release, when exactly one is cut, how its version is
  derived from conventional commits, and what the published tag and Release body must contain.
- `branch-promotion-integrity`: the documented promotion/hotfix policy, and the observable contract of
  drift detection — including that a cherry-picked backport is not drift.

### Modified Capabilities

- None. No existing spec in `openspec/specs/` covers CI, release, or branching.

## Approach

Use **git-cliff** in its no-commit form, driven by a workflow triggered on push to `main`. git-cliff
computes the next version and renders the notes from the same conventional commits already in
history; the tag and Release are created through the GitHub API with the default `GITHUB_TOKEN`.

This is chosen over release-please because it *removes* the branch-protection problem instead of
negotiating with it: nothing is ever pushed to a protected branch, so there is no standing bot PR to
approve each release, no required-check-never-fires stall, and no infinite-loop guard to maintain.
It also matches the repo's existing precedent of pinning an external CLI binary
(`shipwright-validation.yml`) over adopting a marketplace Action. semantic-release is disqualified —
research confirmed its direct-push model cannot pass this protection. cargo-release solves per-crate
publishing, which is explicitly not the problem.

Drift detection compares the two branches by patch equivalence rather than commit identity, so a
cherry-picked hotfix reads as backported. It runs on a schedule rather than immediately after a
hotfix merge, because `develop` is legitimately behind for the length of the backport window.

Delivery is two slices: (1) release workflow + baseline tag, (2) branching/hotfix docs + drift check.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `.github/workflows/` | New | Release workflow on push to `main`; scheduled drift check |
| `CONTRIBUTING.md` | Modified | Branching model, hotfix/backport policy, release process |
| Git tags / GitHub Releases | New | Baseline `v0.1.0`, then one per promotion to `main` |
| `.github/workflows/production-gate.yml` | Unchanged | Still runs on push to `main`; nothing is pushed back, so no loop |
| Root `Cargo.toml`, all 22 member manifests | Unchanged | No version bump; repo tag is independent |
| `openspec/config.yaml` | Unchanged | No release-phase rule needed for this change |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Greenfield: no prior tag to validate output against | High | Seed the baseline tag manually and verify the first cut by hand before trusting the workflow |
| Version derivation misbehaves under `0.x` semantics | Med | Pin the tool version; assert the computed version in the first cut before publishing |
| Drift check false-positives on cherry-picked backports, gets ignored | Med | Patch-equivalence contract is a spec requirement, not an implementation detail |
| Release notes are the only changelog; a consumer wants an in-tree file | Low | Notes are regenerable from history at any time into a normal reviewed PR |
| Tag lands on an unsigned, non-linear-history commit | Low | Accepted explicitly; supply-chain hardening is a separate change |

## Rollback Plan

Both slices are additive files. Delete the workflow file to stop all automation immediately. Published
artifacts are removable without touching source: delete the GitHub Release, then delete the tag
locally and remotely. Revert the `CONTRIBUTING.md` section with a normal PR. No source file, manifest,
build, or existing gate is modified, so a full revert restores the exact current state.

## Dependencies

- Repo `contents: write` permission for the default `GITHUB_TOKEN` (tag + Release creation via API).
  No new app, PAT, or branch-protection exemption.
- Conventional Commits discipline on `develop` — already the de facto convention, now load-bearing.

## Success Criteria

- [ ] A merge to `main` produces exactly one tag and one GitHub Release; a merge to `develop` and an
      open PR produce none.
- [ ] The Release body lists the conventional commits in the range since the previous tag, grouped by
      type, and is readable by a human without consulting git.
- [ ] The computed version follows semver from those commits and stays within `0.x`.
- [ ] No automated commit is pushed to `main` or `develop`, and no branch protection rule is changed,
      relaxed, or bypassed.
- [ ] `CONTRIBUTING.md` states the branching model, the hotfix backport obligation, and the release
      process.
- [ ] Drift detection reports a hotfix on `main` that is absent from `develop`, and reports nothing
      once that hotfix is backported — whether by merge or by cherry-pick.
- [ ] No crate manifest version changes; nothing is published to crates.io.
