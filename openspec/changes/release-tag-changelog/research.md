# Research: release-tag-changelog

## Current State (evidence check per exploration claim)

### 1. release-please — delivery model vs. branch protection (1 approval + required status check, no bypass)

**Confirmed, with a caveat exploration missed.** release-please genuinely uses a release-PR model: it opens/maintains a standing `chore(main): release X.Y.Z` PR against the target branch, and the actual tag + GitHub Release + CHANGELOG commit are only cut when that PR is merged [S1]. Merging the release-please PR is a normal PR merge — it goes through the exact same branch-protection gate (required review + required status check) as any human PR, **which is the good news for ego-rs's constraint.**

The caveat: the default `GITHUB_TOKEN` release-please uses by default (a) cannot self-approve, so branch protection's "1 required approval" still has to come from a real human or a configured bypass actor, and (b) pushes made with `GITHUB_TOKEN` don't retrigger downstream workflows, meaning the release-please PR itself may not pick up the required status check unless a PAT/GitHub App token is substituted in [S1][S2]. The community-standard workaround for org repos with strict "no bypass app/token" protection (matching ego-rs's stated setup) is either (a) a human reviewer approves the release-please PR like any other PR — no tooling change needed — or (b) an auto-approve bot step, which is optional convenience, not a requirement. **For ego-rs specifically, since no bypass exists, the release-please PR would need a human approval + the status check to pass exactly like every other PR — this is compatible, not blocked, by the described protection.**

`release-type: simple` is confirmed to exist and fit ego-rs's "no `[workspace.package]` version" situation: `simple` tracks a plain `version.txt` + `CHANGELOG.md` rather than any Cargo.toml, so it does not attempt per-crate version bumping — this matches the "repo-level tagging, not per-crate" requirement from exploration [S1][search evidence].

**Maintenance/version status:** `googleapis/release-please-action` shows active issue triage through July 2026 (issues opened Apr/May/Jun/Jul 2026), no archival/deprecation signal. Current major is v4 (v3→v4 changed some input names: `command` was removed in favor of explicit booleans like `skip-github-release`) [S1]. Note there is a separate, explicitly **archived** fork, `google-github-actions/release-please-action` — do not confuse the two; `googleapis/release-please-action` is the maintained one.

### 2. git-cliff — version/status, and the no-commit variant

**Confirmed and exploration's implicit assumption (that a commit-back-to-branch is required) is corrected.** git-cliff is Rust-native, current stable is **2.14.1** (released ~Apr 2026, per npm/mise-tools version tracking) [S5], and actively maintained.

The key finding: `git-cliff-action` exposes a **`content` output** — the generated changelog text as a string, not just a file — specifically for the "Advanced" use case of populating a GitHub Release body directly (e.g., piped into `softprops/action-gh-release` or `svenstaro/upload-release-action`) [S6]. This variant **writes nothing to the repo tree and commits nothing**, sidestepping the branch-protection problem entirely for the changelog step — it only needs write access to create the GitHub Release/tag via the API, not a git push to a protected branch. The action also supports the alternative "commit CHANGELOG.md back to the repo" pattern, but that variant would hit the same protected-branch problem as everything else and would need a PR, same as release-please.

### 3. semantic-release — branch protection limitation

**Confirmed as a hard limitation, exploration was correct.** `semantic-release/github` issue #175 (open since 2019, still the canonical reference) establishes that semantic-release's default model pushes the version bump/tag commit **directly** to the release branch; there is no native "open a PR and wait for review" mode built into the tool [S3]. All documented workarounds are permission-escalation, not workflow-restructuring: exempting the release bot/token from the "require PR" rule, using a GitHub App token with an explicit bypass entry, or granting the automation account admin rights so branch protection doesn't apply to it. None of these are compatible with ego-rs's stated "no bypass app/token configured" constraint — under that constraint, semantic-release's default flow would simply fail with a `GH006` protected-branch rejection. A community pattern exists (`peter-evans/create-pull-request` to turn the version-bump diff into a PR instead of pushing) but that is a workaround built by wiring an unrelated action around semantic-release, not a supported mode of the tool itself.

### 4. cargo-release — no built-in changelog/release generation

**Confirmed exactly as exploration stated.** The official FAQ states cargo-release is "unopinionated" about changelogs by design and does not generate one from git history; it recommends either wiring `git-cliff` in as a `pre-release-hook`, or hand-maintaining a Keep-a-Changelog file with `pre-release-replacements`. It also explicitly recommends a **separate** CI workflow gated on tag-push to create the GitHub Release itself (e.g. via `taiki-e/create-gh-release-action`) — cargo-release does not create GitHub Releases [S4].

### 5. Infinite-loop pitfall for `push: [main]` workflows that push back to `main`

**Confirmed, with the precise mechanism.** The built-in safeguard: a push made using the default `GITHUB_TOKEN` does **not** retrigger other workflow runs, which is GitHub's designed loop-breaker. The loop only manifests when a PAT or GitHub App token is used for the push-back (which release-please and semantic-release setups often need, precisely to work around the same token's inability to trigger required-check workflows) — real documented case: `googleapis/release-please-action` issue #1028 describes exactly this loop with the release-PR being repeatedly re-updated [S1 issue evidence]. Standard mitigations, in order of robustness: (a) `[skip ci]`/`[skip actions]` in the commit message of the automated commit — GitHub natively recognizes this on `push`/`pull_request` triggers [S7]; (b) `paths-ignore` — weaker, because it only looks at the current push's file diff, doesn't inspect commit messages/actors, and — critically — if it causes a workflow to be skipped, a **required status check that workflow would have produced is left in "Pending" state forever**, which can block PR merges rather than help them [S8]; (c) explicit actor/branch-name guards (e.g. `if: github.head_ref != 'release-please--branches--main'`) as release-please's own recipes use [S1]. For ego-rs's model (release-please or the no-commit git-cliff variant), since the release itself lands via a normal PR merge, the loop risk is specifically about the release-please bot's own "update the standing PR" step re-triggering itself, not about tags/releases retriggering `push: [main]`.

## Findings

| # | Claim | Status | Confidence |
|---|---|---|---|
| 1 | release-please's release-PR model is compatible with a strict branch-protection target (1 approval + 1 required check, no bypass) because merging the release PR is just a normal PR merge | Confirmed | High |
| 2 | `release-type: simple` exists and fits ego-rs (no per-crate Cargo.toml version bump; tracks version.txt + CHANGELOG.md) | Confirmed | High |
| 3 | `googleapis/release-please-action` is actively maintained, currently major v4; do not confuse with the archived `google-github-actions/release-please-action` fork | Confirmed | Medium-High |
| 4 | git-cliff is actively maintained, current version 2.14.1 | Confirmed | Medium (version numbers from secondary trackers, not the primary GitHub Releases page directly) |
| 5 | git-cliff has a genuine no-commit variant: `content` action output → GitHub Release body directly, sidestepping branch protection entirely | Confirmed | High |
| 6 | semantic-release has no supported branch-protection-with-PR-review mode; direct-push is a hard architectural assumption | Confirmed | High |
| 7 | cargo-release generates neither changelog nor GitHub Release; recommends git-cliff hook + separate tag-triggered release workflow | Confirmed | High |
| 8 | GITHUB_TOKEN pushes don't retrigger workflows (loop-breaker); PAT/App-token pushes do, which is the actual loop cause in release-bot setups; `[skip ci]` is the standard mitigation; `paths-ignore` is weaker and can strand required checks in Pending | Confirmed | Medium-High (well-documented pattern, standard official + community sources agree) |

## Risks

- **release-please + strict protection interaction is untested for ego-rs specifically.** The evidence confirms the *model* is compatible in principle, but ego-rs's exact protection config (no bypass app/token at all) means a human must manually approve the release-please PR every time, and the required status check must be a normal workflow trigger — this needs a real dry-run against ego-rs's protection rules before committing to it, not just doc-reading.
- **git-cliff's no-commit-to-tree variant still requires a `contents: write` token to create the tag + GitHub Release via API.** That's an API call, not a git push, so it should not trip branch protection — but this wasn't independently confirmed against a real GitHub API behavior test in this research pass, only inferred from the action's documented output pattern.
- **Version-number citations for release-please-action and git-cliff came from secondary sources** (npm mirrors, third-party version trackers) rather than the GitHub Releases page's raw content, which did not return clean tag data via WebFetch. Low-severity since exact patch version doesn't affect the architectural decision.
- **semantic-release's disqualification is strong but the "PR-mode" community plugins were not exhaustively surveyed** — only the canonical `semantic-release/github` issue was checked, not the wider plugin ecosystem. Given the other three candidates already satisfy the constraint natively, this gap is low-priority to close.

## Ready for Proposal: yes

The evidence resolves the deciding constraint from exploration for 3 of 4 candidates (release-please: compatible via normal PR merge; git-cliff: compatible, with a bonus no-commit variant; cargo-release: confirmed changelog/release gap, would need pairing) and confirms semantic-release is disqualified by the branch-protection constraint as exploration suspected. No blocking gaps remain that would prevent sdd-propose from weighing these four candidates.

---

Sources:
- [S1] googleapis/release-please-action — https://github.com/googleapis/release-please-action (accessed 2026-09-08)
- [S2] Sergio Carracedo, "Automating package version bump with Release Please" — https://sergiocarracedo.es/release-please/ (accessed 2026-09-08)
- [S3] semantic-release/github issue #175, "Protected branch with PR requirement prevents release" — https://github.com/semantic-release/github/issues/175 (accessed 2026-09-08)
- [S4] crate-ci/cargo-release FAQ — https://github.com/crate-ci/cargo-release/blob/master/docs/faq.md (accessed 2026-09-08)
- [S5] git-cliff version history (mise-tools tracker / npm) — http://mise-tools.jdx.dev/tools/git-cliff, https://www.npmjs.com/package/git-cliff (accessed 2026-09-08)
- [S6] orhun/git-cliff-action — https://github.com/orhun/git-cliff-action (accessed 2026-09-08)
- [S7] GitHub Changelog, "GitHub Actions: Skip pull request and push workflows with [skip ci]" — https://github.blog/changelog/2021-02-08-github-actions-skip-pull-request-and-push-workflows-with-skip-ci/ (accessed 2026-09-08)
- [S8] Shounak Mulay, "Avoid workflow loops on GitHub Actions when committing to a protected branch" — https://blog.shounakmulay.dev/avoid-workflow-loops-on-github-actions-when-committing-to-a-protected-branch (accessed 2026-09-08)
