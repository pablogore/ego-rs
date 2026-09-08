# Design: release-tag-changelog — Release Cutting on `main` + Hotfix Backport Integrity

> Canonical / English. Spanish companion: `design.es.md` (1:1 headings).
> Sources: `proposal.md`, `research.md`. `specs/` not yet written (sdd-spec runs in parallel); this
> design is derived from `proposal.md` and must be reconciled against the delta specs before tasks.

## Technical Approach

Four additive files plus one doc section. Nothing in the repo tree is generated, committed, or pushed.

- `release-automation` — `.github/workflows/release.yml` on `push: branches: [main]`. A pinned
  git-cliff binary derives the version and renders notes; an annotated tag is pushed to a **tag** ref;
  `gh release create --verify-tag` publishes the notes as the Release body.
- `branch-promotion-integrity` — `.github/scripts/backport-drift.sh` (pure git, patch-equivalence)
  driven by `.github/workflows/backport-drift.yml` (cron + `workflow_dispatch`), reporting into one
  upserted tracking issue; plus the branching/hotfix section in `CONTRIBUTING.md`.

## Design Invariant: no loop, no bypass

**No step in either workflow pushes to a branch ref.** This is an invariant, not an assumption, and it
holds under three independent facts — any one alone breaks the loop:

1. The only push anywhere is `git push origin "$VERSION"` — an explicit tag refspec.
2. `production-gate.yml`'s `push:` trigger carries a `branches:` filter, which never matches a tag push.
3. Pushes made with the default `GITHUB_TOKEN` do not retrigger workflows (research §5).

Permissions are least-privilege per workflow: release `contents: write`; drift `contents: read` +
`issues: write`. No PAT, no GitHub App, no branch-protection change. `production-gate.yml` is not
edited.

## Architecture Decisions

| # | Choice | Rejected | Rationale |
|---|---|---|---|
| AD-1 | git-cliff as a pinned release binary (curl + chmod, `GIT_CLIFF_VERSION` env) | `orhun/git-cliff-action` | Matches `production-gate.yml` / `shipwright-validation.yml` precedent, and the *same* command runs locally — which is what makes the dry-run verification strategy real rather than CI-only |
| AD-2 | `cliff.toml` at repo root with `[bump] breaking_always_bump_major = false`, `features_always_bump_minor = true`, `initial_tag = "v0.1.0"`; `[git] tag_pattern = "v[0-9]*"` | git-cliff defaults; config under `.github/` | Defaults bump `!`/BREAKING straight to `1.0.0`, violating the "stay in `0.x`" scope line. At root, a bare local `git cliff` reproduces CI byte-for-byte with no `--config` flag to forget |
| AD-3 | Notes rendered to `$RUNNER_TEMP/notes.md`, consumed by `--notes-file` | `content` action output / multiline `$GITHUB_OUTPUT` heredoc | No delimiter escaping, and arbitrary commit text never crosses an interpolation surface. `$RUNNER_TEMP` is outside the worktree, so the notes cannot be committed even by accident |
| AD-4 | `git tag -a` + `git push origin <tag>`, then `gh release create --verify-tag` | Letting `gh release create` create the tag | The Releases API creates a **lightweight** tag; the proposal requires annotated. `--verify-tag` fails loudly instead of silently creating one if the push step ever regresses |
| AD-5 | `gh` CLI for the Release API, `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` | `softprops/action-gh-release` | Preinstalled on runners; auth is identical either way, so a third-party Action would add a supply-chain edge to replace one command |
| AD-6 | Baseline `v0.1.0` seeded once, by hand, documented in `CONTRIBUTING.md`; `initial_tag` set as deterministic fallback | A `workflow_dispatch` bootstrap job | Dead code after one run, and the proposal's top risk explicitly wants the first cut human-verified. The fallback means an unseeded first run still yields `v0.1.0` rather than an error |
| AD-7 | `git cherry -v origin/develop origin/main`; `+` lines are drift | `git log develop..main` | `git cherry` compares by patch-id, so a cherry-picked backport reads as `-`, satisfying the proposal's explicit "cherry-pick is not drift" contract. Commit-identity comparison false-positives on every backport. Merge commits are excluded by construction |
| AD-8 | Upsert one open issue labelled `backport-drift` (create / `gh issue edit --body-file` / close when clean); job always exits `0` | Failing the job; auto-opening a backport PR | A red cron run carries no content and is easy to ignore; an issue is durable, assignable, human-readable, and self-heals to closed. Auto-PR is explicitly out of scope |
| AD-9 | Stateless backport window: `BACKPORT_WINDOW_HOURS: "72"` filter on committer date | A persisted "seen" state file or API history | `develop` is legitimately behind for the length of the window; an age filter removes that whole false-positive class in five lines with no state to corrupt |

## Data Flow

```
merge PR → main
   ├─→ production-gate.yml            (unchanged, push: branches: [main])
   └─→ release.yml                    (push: branches: [main], concurrency: cancel-in-progress false)
         checkout fetch-depth: 0, fetch-tags: true     ← git-cliff needs full history + tags
         VERSION=$(git cliff --bumped-version)
         [ tag $VERSION already exists ? → exit 0 ]    ← re-run idempotency
         git cliff --unreleased --tag "$VERSION" -o "$RUNNER_TEMP/notes.md"
         git tag -a "$VERSION" ; git push origin "$VERSION"   ← TAG REF ONLY
         gh release create "$VERSION" --verify-tag --notes-file "$RUNNER_TEMP/notes.md"
                │
                └── tag push: no `branches:` match → no workflow rerun → no loop

cron (daily) / workflow_dispatch
   └─→ backport-drift.yml
         git fetch origin main develop
         .github/scripts/backport-drift.sh   → markdown on stdout, EMPTY if clean, exit 0
              git cherry origin/develop origin/main │ '^+' │ committer age > window
         gh issue create | edit --body-file | close   (label: backport-drift)
```

## File Changes

| File | Action | Description |
|---|---|---|
| `.github/workflows/release.yml` | Create | Release cut on push to `main` |
| `.github/workflows/backport-drift.yml` | Create | Scheduled + dispatchable drift report, non-blocking |
| `.github/scripts/backport-drift.sh` | Create | The only real logic; testable in isolation |
| `.github/scripts/release-guards.test.sh` | Create | Drift-script cases + static invariant assertions |
| `cliff.toml` | Create | Bump semantics and changelog grouping |
| `CONTRIBUTING.md` | Modify | New `## Branching, Releases, and Hotfixes` section after `## CI: Production Gate` |
| `.github/workflows/production-gate.yml` | Unchanged | Triggers and jobs untouched |
| Root + 22 member `Cargo.toml` | Unchanged | No version field touched |

`CONTRIBUTING.md` section structure (matching the file's existing command-block style):
`### Branching model` (feature → `develop` → `main`; hotfix off `main` → `main` → mandatory backport)
· `### Cutting a release` (automatic on merge to `main`; what to expect; the one-time baseline-tag seed
commands) · `### Backport drift check` (what the `backport-drift` issue means and that the fix is
`git cherry-pick` onto `develop` — patch-equivalent, so the next run closes the issue itself).

## Interfaces / Contracts

```bash
# Version + notes — identical locally and in CI (AD-1, AD-2)
VERSION="$(git cliff --bumped-version)"
git cliff --unreleased --tag "$VERSION" -o "$RUNNER_TEMP/notes.md"

# Drift — '+' = no patch-equivalent commit on develop, '-' = already backported (AD-7)
git cherry -v origin/develop origin/main
```

`backport-drift.sh` contract: takes optional `<upstream-ref> <head-ref>` (default
`origin/develop origin/main`), reads `BACKPORT_WINDOW_HOURS` (default `72`), operates on the **cwd**
repository only, writes a markdown report to stdout, and always exits `0`. **Stdout is empty when
there is no drift** — the caller's entire decision is `[ -s report.md ]`.

## Testing Strategy

| Layer | What to Test | Approach |
|---|---|---|
| Unit (RED first) | `backport-drift.sh` classification | `.github/scripts/release-guards.test.sh` builds throwaway repos under `mktemp -d` and asserts four cases: merged-normally → empty, cherry-picked → empty, old unbackported → reported, fresh unbackported → empty (in-window). Pure git + bash, no network, no `gh` |
| Unit (RED first) | The loop/injection invariants | Same script, static assertions over both workflow files: no `git push` line targets a branch ref, and no `run:` block contains a `${{ github.event… }}` interpolation |
| Integration | git-cliff version + notes | Documented local dry-run of the two commands above against a real clone; assert the computed version, eyeball the notes. No test authored — that logic is git-cliff's, not ours |
| Manual (first cut only) | End-to-end release | Checklist: seed baseline tag → watch the first automated run → exactly one tag + one Release → `production-gate` did **not** re-run from the tag push → `git log origin/main` unchanged by the workflow → re-run the same SHA and confirm no second tag/Release |

`act` was considered and rejected: it cannot faithfully execute `gh` API calls or tag pushes, so a
green `act` run proves nothing the bash test does not already prove. Strict TDD binds the one
component with branching logic (`backport-drift.sh`) and the invariant assertions; YAML wiring and
vendor invocations have no branch to red-test.

## Threat Matrix

| Boundary | Minimum adversarial cases | Applicability | Design response | Planned RED tests |
|---|---|---|---|---|
| Documentation-like paths | `requirements.txt`, executable Markdown, `README.sh` | N/A — no file is classified or executed by content; the only files written are a temp notes file and a stdout report | — | — |
| Git repository selection | `git -C`, relative paths, absolute paths | Applicable | The cwd checkout is the sole repository authority. `backport-drift.sh` accepts **refs only**, never paths, and never `cd`s or uses `git -C` | Run the script with cwd set to a scratch repo; assert it reports that repo's drift, not the outer repo's |
| Commit state | staged, `commit -a`, empty index | N/A — no step creates a commit anywhere; the worktree is never modified | — | — |
| Push state | tracking branch, first push, explicit refspec | Applicable | The only push is an explicit tag refspec `git push origin "$VERSION"`; no bare `git push`, no branch ref, and the idempotency guard exits before pushing an existing tag | Static assertion that no `git push` in either workflow targets a branch ref; manual re-run-same-SHA check for the guard |
| PR commands | explicit `--head`, environment prefix, composed commands | Applicable (issue commands) | Every `gh` call is an explicit subcommand with `--label`/`--body-file`; the issue body comes from a file, never from command-line interpolation, and `${{ }}` never appears inside a `run:` body (values arrive via `env:`) | Static assertion that no `run:` block contains `${{ github.event… }}` |

## Migration / Rollout

Ordered: (1) seed the baseline tag by hand on the current `main` head and hand-verify a local
`git cliff` dry-run, (2) merge slice 1 — the next promotion cuts the first automated release under
observation, (3) slice 2 is independent and may land in either order. Rollback is file deletion; the
tag and Release are removable without touching source.

## Open Questions

- [ ] git-cliff `2.14.1` and its linux asset filename come from a secondary tracker (research S5).
      Apply MUST confirm the release tag and asset name resolve, and bump the pin if not.
- [ ] Tag protection rules on `v*` (who may delete/overwrite a published tag) — noted, out of scope.
