# Tasks: release-tag-changelog — Release Cutting on `main` + Hotfix Backport Integrity

> Canonical / English. Spanish companion: `tasks.es.md` (1:1 numbering).
> Strict TDD is on for this repo (`sdd-init` record). The only branching logic in this change is
> `.github/scripts/backport-drift.sh`; its test suite (`.github/scripts/release-guards.test.sh`) is
> RED-first per Phase 2. YAML wiring and vendor-CLI invocations have no branch to red-test (design.md
> "Testing Strategy"). Design's two proposal slices are kept, with slice 2 split across two PRs for
> the RED/GREEN separation and the 400-line review budget.

## Reconciliation Findings (spec ↔ design) — read before Phase 0

Spec and design were authored independently, each from `proposal.md` only, and design.md's own
report explicitly asked for this check. One genuine mismatch was found; it gates Phase 0.

1. **Genuine mismatch — decision needed.** `design.md` AD-9 introduces a stateless
   `BACKPORT_WINDOW_HOURS` (default 72) committer-age filter: a hotfix on `main` not yet backported
   to `develop` is **not** reported while inside that window. Design's own Testing Strategy table
   confirms this explicitly ("fresh unbackported → empty (in-window)"). But
   `branch-promotion-integrity/spec.md`'s "An un-backported hotfix is reported" scenario carries no
   age qualifier: *"GIVEN a hotfix commit landed on main and its change is absent from develop WHEN
   drift detection next runs THEN that hotfix's change is reported as drift"* — read literally, a
   fresh (1-hour-old) un-backported hotfix must be reported. This is an observable-behavior
   difference, not phrasing. See Phase 0.
2. **Informational — not a mismatch.** The task brief cites "7 requirements / 12 scenarios" for
   `release-automation` and "5 requirements / 8 scenarios" for `branch-promotion-integrity`. The
   spec files as read contain 6 requirements / 10 scenarios and 5 requirements / 7 scenarios,
   respectively. No content gap found against design — treat the brief's counts as approximate, not
   as evidence of missing requirements.
3. **Informational — verification task added, not a mismatch.** `release-automation`'s "grouped by
   Conventional-Commit type" requirement is satisfied by git-cliff's **default** changelog template;
   `design.md` AD-2 only pins `[bump]`/`[git]` keys, not a `[changelog]` template override. This is
   consistent with the design (no override is claimed), but it means the requirement is satisfied by
   an unstated default rather than an explicit config line — Task 1.5 adds a manual check so this
   isn't discovered for the first time on the first real release.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~590 total — PR1 ~180, PR2 ~305, PR3 ~105 |
| 400-line budget risk | Low — every PR forecast well under 400 with margin |
| Chained PRs recommended | Yes — 3 PRs, not a single PR |
| Suggested split | PR1 (release-automation, independent) · PR2 (backport-drift.sh + RED-first test suite, independent) · PR3 (backport-drift.yml + branching/hotfix docs, depends on PR2's script; sequence after PR1 merges to avoid both PRs inserting into the same new `CONTRIBUTING.md` heading) |
| Delivery strategy | ask-on-risk |
| Chain strategy | PR1 and PR2 branch independently from `develop`; PR3 chains onto PR2 (code dependency: the script must exist) and should be rebased/opened after PR1 merges (shared-file dependency, not a code dependency) |

Decision needed before apply: **Yes** — for two independent reasons: (a) the Reconciliation Finding
#1 above (AD-9 window vs. the literal spec scenario) must be resolved before Phase 2's "fresh
unbackported" test case can be authored one way or the other; (b) confirm the 3-PR chained split
before `sdd-apply` starts opening branches.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Depends on |
|------|------|-----------|----------------------|------------|
| 0 | Resolve AD-9 vs. spec scenario mismatch | — (decision, no code) | — | none |
| 1 | Release cut on push to `main` + baseline tag + manual first-cut checklist | PR1 | local `git cliff --bumped-version` / `git cliff --unreleased` dry-run (no CI harness — vendor logic, per design) | Unit 0 not required (independent slice) |
| 2 | `backport-drift.sh` + RED-first `release-guards.test.sh` | PR2 | `bash .github/scripts/release-guards.test.sh` | Unit 0 (gates the in-window test case) |
| 3 | `backport-drift.yml` + branching/hotfix/drift-check docs | PR3 | `bash .github/scripts/release-guards.test.sh` (re-run, now covering both workflow files) | Unit 2 (script must exist); sequence after Unit 1 merges (shared `CONTRIBUTING.md` heading) |

## Phase 0: Reconciliation Decision — Blocking, No Code

- [ ] 0.1 Resolve Reconciliation Finding #1: either (a) amend `branch-promotion-integrity/spec.md` +
      `spec.es.md`'s "An un-backported hotfix is reported" scenario to state the 72-hour allowance
      explicitly (add a paired scenario for the in-window case, matching design's own test-case
      language), or (b) redefine/drop AD-9 in `design.md` + `design.es.md` so a fresh un-backported
      hotfix is still reported and the window is removed. Do not start Phase 2's task 2.7 until one
      side is amended and both documents agree.

## Phase 1: Release Automation (Slice 1) — PR1

Covers `release-automation/spec.md`'s six requirements. Independent of Phase 0/2/3.

- [ ] 1.1 Create `cliff.toml` at repo root (AD-2): `[bump] breaking_always_bump_major = false`,
      `features_always_bump_minor = true`, `initial_tag = "v0.1.0"`; `[git] tag_pattern = "v[0-9]*"`.
      Leave `[changelog]` on git-cliff's default template — do not hand-roll a grouping template;
      verify the default in 1.5 (satisfies "Release Body Is Human-Readable Record Grouped By
      Conventional-Commit Type").
- [ ] 1.2 Create `.github/workflows/release.yml`: `on: push: branches: [main]`;
      `concurrency: { group: release, cancel-in-progress: false }`; `permissions: contents: write`;
      checkout with `fetch-depth: 0, fetch-tags: true` (git-cliff needs full history); pin
      `GIT_CLIFF_VERSION` as an env var, install via curl + chmod (AD-1, same pattern as
      `.github/workflows/shipwright-validation.yml`'s `SHIPWRIGHT_VERSION` + curl/chmod install at
      lines 24/50-51). Confirm the pinned git-cliff release tag and its linux asset filename actually
      resolve (design "Open Questions") — bump the pin if the tracker's `2.14.1` guess is stale.
- [ ] 1.3 Same file: `VERSION="$(git cliff --bumped-version)"`; idempotency guard — if a tag named
      `$VERSION` already exists, exit 0 before doing anything else (satisfies "cut only, and exactly
      once, as a direct result of a push landing on main").
- [ ] 1.4 Same file: `git cliff --unreleased --tag "$VERSION" -o "$RUNNER_TEMP/notes.md"` (AD-3);
      `git tag -a "$VERSION" -m "$VERSION"`; `git push origin "$VERSION"` — an explicit tag refspec
      only, never a bare `git push` or any branch ref (AD-4; this is the one line the design
      invariant depends on); `gh release create "$VERSION" --verify-tag --notes-file
      "$RUNNER_TEMP/notes.md"` (AD-5) with `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` supplied via
      `env:`, never interpolated into a `run:` string.
- [ ] 1.5 Manual dry-run, no push: locally run `git cliff --bumped-version` and `git cliff
      --unreleased --tag <that-version>` against the current `main` head; confirm the rendered notes
      are grouped by Conventional-Commit type (closes Reconciliation Finding #3) and record the
      computed version in the PR description (design "Testing Strategy", Integration row; proposal
      risk "Version derivation misbehaves under 0.x semantics").
- [ ] 1.6 `CONTRIBUTING.md`: add `## Branching, Releases, and Hotfixes` after `## CI: Production
      Gate` (confirmed anchor, current line 18) with only the `### Cutting a release` subsection for
      this PR — what triggers a release, that it is fully automatic, and the one-time baseline-tag
      seed commands (`git tag -a v0.1.0 <main-sha> -m v0.1.0 && git push origin v0.1.0`, AD-6). Leave
      `### Branching model` and `### Backport drift check` for PR3 (design "File Changes" table).
- [ ] 1.7 Same subsection: append the first-release-only manual verification checklist, transcribed
      from design's "Testing Strategy" Manual row — seed the baseline tag; watch the first automated
      run; confirm exactly one new tag and one new Release; confirm `production-gate.yml` did **not**
      re-run as a result of the tag push; confirm `git log origin/main` is unchanged by the workflow;
      re-run the workflow against the same `main` SHA and confirm no second tag/Release is created.
      This is the task list's required "manual verification checklist for the first real release"
      and the proposal's top-risk mitigation.
- [ ] 1.8 Execute the baseline tag seed by hand on the current `main` head (design "Migration /
      Rollout" step 1), before or immediately after merging PR1. Record the seeded tag and its SHA in
      the PR description.
- [ ] 1.9 Verification: review confirms no `run:` step in `release.yml` interpolates
      `${{ github.event... }}` (design Threat Matrix, "PR commands" row — values must arrive via
      `env:`), and the only `git push` in the file targets `"$VERSION"`, never a branch ref (design
      invariant fact 1).

## Phase 2: Backport Drift Script + RED-First Test Suite — PR2

Covers `branch-promotion-integrity/spec.md`'s drift-reporting and patch-equivalence requirements at
the script layer. Independent of Phase 1 (no file overlap). Gated by Phase 0 for task 2.7 only.

- [ ] 2.1 RED: create `.github/scripts/release-guards.test.sh`. First case: build a throwaway repo
      under `mktemp -d`, land a hotfix on `main`, merge it into `develop` normally, run
      `.github/scripts/backport-drift.sh` (does not exist yet) against it, assert empty stdout and
      exit code `0`. Confirm the test fails only because the script is missing.
- [ ] 2.2 GREEN: create `.github/scripts/backport-drift.sh`. Signature: optional
      `<upstream-ref> <head-ref>` (default `origin/develop origin/main`); reads
      `BACKPORT_WINDOW_HOURS` (default `72`); never uses `git -C` or `cd`s — operates on the cwd
      repository only (design Threat Matrix, "Git repository selection" row); runs
      `git cherry -v "$upstream" "$head"`, treats `+`-prefixed lines as drift (AD-7); writes a
      markdown report to stdout (empty when clean); always exits `0`. Make 2.1 pass.
- [ ] 2.3 RED: add the cherry-pick case — hotfix on `main`, backported to `develop` via
      `git cherry-pick` (different commit SHA, same patch), assert empty stdout.
- [ ] 2.4 GREEN: confirm 2.3 passes against the unmodified 2.2 implementation (patch-id comparison
      already covers this per AD-7); if it fails, fix the classification logic.
- [ ] 2.5 RED: add the "old unbackported" case — hotfix on `main` with a committer date older than
      `BACKPORT_WINDOW_HOURS` and no equivalent commit on `develop`; assert non-empty stdout that
      names the commit.
- [ ] 2.6 GREEN: implement the committer-age filter using `BACKPORT_WINDOW_HOURS` in
      `backport-drift.sh`; make 2.5 pass.
- [ ] 2.7 RED: add the "fresh unbackported, in-window" case, per Phase 0's resolved decision — assert
      empty stdout if AD-9's window stands, or assert non-empty stdout if Phase 0 removed/shrank it.
      Do not author this case ahead of the Phase 0 decision.
- [ ] 2.8 GREEN: confirm 2.7 passes against the Phase-0-resolved behavior; adjust the age filter if
      Phase 0 changed it.
- [ ] 2.9 RED: repo-selection isolation case (design Threat Matrix) — invoke the script with cwd set
      to a scratch repo distinct from any outer checkout; assert it reports only that scratch repo's
      drift.
- [ ] 2.10 GREEN: confirm 2.9 passes (should already hold given 2.2 never uses `git -C` or absolute
      paths; add a regression fix only if it fails).
- [ ] 2.11 RED+GREEN: static invariant assertions (design "Testing Strategy", loop/injection row) in
      the same test file — grep `release.yml` (already merged from PR1, or present on this branch)
      to assert no `git push` line targets a branch ref, and no `run:` block contains
      `${{ github.event... }}`. Scope this PR's assertion to `release.yml` only —
      `backport-drift.yml` does not exist until PR3; re-run the full assertion in Phase 3.
- [ ] 2.12 Verification: `bash .github/scripts/release-guards.test.sh` green end to end; confirm no
      network call and no `gh` invocation anywhere in the suite (design constraint — pure git/bash).

## Phase 3: Backport Drift Workflow + Branching/Hotfix Docs — PR3

Covers `branch-promotion-integrity/spec.md`'s remaining requirements (documented policy, recurring
schedule, non-blocking end-to-end). Depends on PR2 (script must exist). Sequence after PR1 merges —
not a code dependency, but both PRs insert subsections into the same new `CONTRIBUTING.md` heading.

- [ ] 3.1 Create `.github/workflows/backport-drift.yml`: `on: { schedule: [{ cron: ... }],
      workflow_dispatch: {} }`; `permissions: { contents: read, issues: write }`;
      `git fetch origin main develop`; run `.github/scripts/backport-drift.sh > report.md`.
- [ ] 3.2 Same file: `[ -s report.md ]` decides create/update vs. close — upsert one open issue
      labelled `backport-drift` (`gh issue create` / `gh issue edit --body-file` / `gh issue close`,
      AD-8); the job always exits `0` regardless of report content (satisfies "non-blocking,
      report-only, no automated remediation").
- [ ] 3.3 `CONTRIBUTING.md`: add `### Branching model` under the existing `## Branching, Releases,
      and Hotfixes` heading — feature branches → `develop` via PR; `develop` → `main` via PR; a
      hotfix branches off `main`, lands on `main` via PR, and MUST subsequently be backported to
      `develop` (satisfies "branching model and hotfix obligation are documented").
- [ ] 3.4 Same file: add `### Backport drift check` — what the `backport-drift` issue means, that
      the fix is `git cherry-pick` onto `develop` (patch-equivalent, so the next scheduled run closes
      the issue itself), and that the check never blocks a merge or push.
- [ ] 3.5 Re-run 2.11's static invariant assertions, now scoped to both `release.yml` and
      `backport-drift.yml`; extend `release-guards.test.sh` in place rather than duplicating the
      check in a second file.
- [ ] 3.6 Verification: confirm the merged `CONTRIBUTING.md` heading contains all three subsections
      in the order design specifies — `### Branching model` → `### Cutting a release` → `### Backport
      drift check`. If PR3 lands with PR1 already merged, this is a pure ordering check; if the merge
      order inverted, move PR1's subsection to the correct position in this PR.

## Cross-Cutting Acceptance Criteria (apply to every PR above, not stated once)

- No commit is ever created or pushed to `main` or `develop` by any workflow in this change.
- No `git push` line in any workflow file targets a branch ref — only the explicit tag refspec in
  `release.yml`.
- No `run:` block in either workflow interpolates `${{ github.event... }}` directly; values arrive
  via `env:`.
- No crate manifest (`Cargo.toml`, root or any of the 22 members) is touched anywhere in this change.
- `.github/workflows/production-gate.yml` is not edited by any PR in this change.

## Out of Scope (reaffirmed, not re-litigated)

Per-crate versioning or crates.io publishing; a tracked `CHANGELOG.md` in the repository tree;
auto-opened backport pull requests or any merge-blocking behavior from drift detection; branch
protection changes, a bypass app/PAT, or signed-commit/linear-history requirements; edits to
`production-gate.yml`'s gating behavior or any `.shipwright/workflow.yaml` promotion logic.
