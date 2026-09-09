# Exploration: release-tag-changelog

## Current State

- Target flow: feature -> develop (PR), develop -> main (PR), hotfix branch off main -> main (PR) + backport to develop. Every merge is PR-gated (1 approval + `production-readiness` check, identical branch protection on `main` and `develop`, confirmed via `gh api repos/pablogore/ego-rs/branches/{main,develop}/protection`). Neither branch requires linear history or signed commits.
- `develop` is routinely ahead of `main` by design (currently 3 commits ahead, clean fast-forward, no divergence) — the hotfix-backport gap is structural, nothing today enforces or detects it.
- No git tags, no GitHub Releases, no `CHANGELOG.md`, no release workflow, and no prior manual release/version-bump commits anywhere in history — this is a true greenfield rollout.
- Only `.github/workflows/production-gate.yml` (triggers on `pull_request` + `push: [develop, main]`, four parallel jobs behind a no-op `production-readiness` gate) and `.github/workflows/shipwright-validation.yml` (workflow_dispatch + push to a feature branch only) exist. No `workflow_dispatch` release trigger, no tag trigger anywhere.
- Commit history is already overwhelmingly Conventional Commits — an existing convention any changelog tool should consume, not introduce.
- Root `Cargo.toml` has no `[workspace.package]` version. All 22 workspace members declare independent `version = "0.1.0"` (verified by grepping every member `Cargo.toml`). Nothing is published to crates.io — this is repo-level tag/release versioning, not per-crate publish/bump.
- `CONTRIBUTING.md` documents production-gate CI steps in detail but has zero content on branching model, release process, or hotfix policy — natural home for that documentation given the repo's own pattern (it was touched in the same commit as the recent CI split).
- `openspec/config.yaml` has no release-related rules in any phase section.
- `shipwright-validation.yml` establishes a repo precedent for vendoring external tooling as a pinned release binary (curl + chmod, version pinned via env var) rather than a marketplace Action — relevant if a Rust-native binary tool is preferred for consistency over a Node Action.

## Affected Areas

- `.github/workflows/production-gate.yml` — already triggers on `push: [main]` (no change needed for gating), but any new release workflow must coexist without an infinite trigger loop if it ever pushes a commit back to `main`.
- `.github/workflows/shipwright-validation.yml` — precedent for how this repo pins/vendors external release tooling.
- `CONTRIBUTING.md` — needs the formalized branching model + hotfix/backport policy; currently silent on both.
- root `Cargo.toml` — confirms repo-level tag semantics, no shared version to bump.
- `openspec/config.yaml` — no existing release-phase rules; a later phase may want to add some.
- No `.github/workflows/release*.yml`, no `CHANGELOG.md` — greenfield additions for whichever approach `sdd-propose` picks.
- GitHub branch protection on `main`/`develop` (external config) — 1 approval + `production-readiness`, no linear-history requirement, no required signed commits — a hard external constraint: neither branch accepts a direct push, so any tool that wants to commit a changelog/version bump back to the branch must go through a real PR or a bypass-privileged token, which doesn't exist today.

## Approaches

1. **release-please** (Node Action, release-PR model) — standing bot PR accumulates conventional-commit changelog entries as PRs merge to `main`; merging that PR creates the tag/release/changelog update.
   - Pros: derives version+changelog+tag+release purely from the existing Conventional Commits convention; the changelog/version-bump commit lands via a normal reviewable PR, fitting the required-approval branch protection without any bypass token; `release-type: simple` avoids touching any of the 22 crate manifests, matching the no-shared-version/no-crates.io state.
   - Cons: Node/npm-based, a different toolchain axis than this repo's Rust/Go/Dagger-first CI; a standing bot PR needs review each release cycle; config files to own.
   - Effort: Medium

2. **git-cliff + a version-bump helper (cocogitto/convco) + scripted tag/release** — a `push: [main]`-triggered workflow computes next semver, tags, generates `CHANGELOG.md` via git-cliff, and calls `gh release create`.
   - Pros: git-cliff is Rust-native, consistent with the Shipwright-pinned-binary precedent; full control, no standing bot PR if a fully automatic (or manual `workflow_dispatch`-gated) flow is preferred.
   - Cons: four separate concerns stitched together as custom glue (bump, tag, changelog, release) instead of one maintained tool; committing `CHANGELOG.md` back into the repo hits the same branch-protection wall as option 4 unless it opens its own follow-up PR (re-inventing release-please's PR step by hand) or skips committing the file into the tree entirely.
   - Effort: Medium-High

3. **cargo-release** — Rust crate version bump + tag + optional crates.io publish.
   - Pros: idiomatic if the workspace ever adopts a shared version / publishes crates.
   - Cons: doesn't match confirmed state (no shared version, nothing published); doesn't generate a changelog or GitHub Release by itself; adopting it nudges the workspace toward a shared-version model, which is scope creep here.
   - Effort: Low (but solves the wrong problem)

4. **semantic-release** (Node, plugin-based) — single CI run computes bump, tags, changelogs, releases, and optionally commits back, no separate release-PR step.
   - Pros: single-pass, no standing bot PR, mature plugin ecosystem.
   - Cons: same Node toolchain mismatch as option 1, worse on branch protection — its default model commits directly back to the release branch, which this repo's protection rejects outright without a bypass token (a bigger structural conflict than option 1's PR-based model).
   - Effort: Medium

## Recommendation

Not selecting an approach — that's `sdd-propose`'s decision. The material finding for that phase: whichever tool is chosen must resolve how the version-bump/changelog write reaches `main` given branch protection allows no direct pushes and no bypass token exists today. Options 1 and 2-with-a-follow-up-PR respect that constraint; option 4's default direct-commit model does not; option 3 doesn't address changelog/release generation at all. Treat "release automation on push to main" and "hotfix-backport drift detection/enforcement" as two related but distinct pieces of scope — the backport gap is currently a documentation-only policy with zero enforcement, and formalizing it in `CONTRIBUTING.md` alone won't close that gap without at least an optional detection mechanism.

## Risks

- Branch protection on both branches rejects direct pushes — any release approach committing changelog/version bump directly to `main`/`develop` fails outright unless routed through a real PR or a new bypass-privileged token/app is introduced (a governance change that belongs in the proposal explicitly, not as an implementation-detail surprise).
- No linear-history or signed-commit requirement today — a release tag can land on an unsigned, non-linear-history commit; minor supply-chain hygiene gap worth naming for propose to accept or address explicitly.
- `production-gate.yml`'s `push: [main]` trigger re-runs the full gate on any commit a release tool pushes to `main` — harmless extra CI cost for PR-merge flows, but a real infinite-loop risk if a workflow both triggers on and pushes back to `push: [main]` without a guard.
- Hotfix-backport policy has no enforcement mechanism as described; a merged hotfix silently not backported to `develop` remains a real correctness gap even after `CONTRIBUTING.md` documents the policy.
- This is a true greenfield release rollout (zero prior tags/releases) — no prior tag/version output exists to validate the chosen tool against; the first cut needs manual verification regardless of tool choice.

## Ready for Proposal

Yes.
