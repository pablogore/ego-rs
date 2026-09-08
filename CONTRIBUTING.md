# Contributor Checklist

Before submitting any future OpenSpec change, verify SPEC-000 compliance:

- [ ] The change starts from an OpenSpec proposal before implementation.
- [ ] Implementation tasks are traceable to proposal, design, and spec artifacts.
- [ ] Domain and application behavior remains deterministic by default.
- [ ] Validation, authorization, parsing, and governance decisions fail closed.
- [ ] State changes are represented through explicit inputs, outputs, events, or ports.
- [ ] Specs, decisions, events, and migrations preserve append-only lineage.
- [ ] Architecture work complies with `architecture-governance`.
- [ ] Testable code complies with `testing-governance`, including mock-first tests and minimum coverage.
- [ ] New production workflows include structured observability.
- [ ] Breaking changes document compatibility, migration, and rollback impact.
- [ ] Constitution changes are proposed as dedicated OpenSpec amendments.
- [ ] Contract tests are defined and pass.

## CI: Production Gate

`.github/workflows/production-gate.yml` runs on every PR and every push to
`develop`. The required check on `develop` branch protection is
`production-readiness` (not yet configured — the workflow existing does not
make it mandatory on its own); that job does no work itself, it only
`needs:` four independent jobs that run in parallel — `lint`, `build-test`,
`architecture`, `integration` — so wall-clock is bounded by the slowest of
them, not the sum of all steps:

```bash
# lint
dagger run ./shipwright --workflow .shipwright/workflow.yaml -step workspace-lint  # cargo clippy --workspace --all-targets --all-features -- -D warnings

# build-test
cargo check --workspace --all-targets
cargo test --workspace

# architecture
cargo run -p xtask -- verify-layers
cargo run -p xtask -- verify-isolation
cargo run -p xtask -- verify-hygiene

# integration
cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite
```

Lint (clippy) already runs through Shipwright's Dagger-backed `workspace-lint`
step (`rust-command` provider, running the literal `--workspace --all-targets
--all-features -- -D warnings` contract — not the dedicated `clippy` provider,
which cannot express `--all-features`), not a raw `cargo clippy` invocation.
`cargo fmt --all -- --check` also has a Shipwright equivalent now
(`workspace-format`, in `.shipwright/workflow.yaml`) — neither step is wired
into this native gate yet; both remain candidate-only until the full manifest
is promoted (see below).

`run-suite` is the canonical entrypoint for the integration suite: it
provisions real PostgreSQL 16 and 14 via testcontainers, migrates them, runs
the tests, and reclaims the containers itself — Docker is the only local
requirement (`colima start` or Docker Desktop; see `integration-tests/README.md`).
No secrets are involved: production-profile tests supply deterministic
non-dev test keys in code, never via environment variables.

### `.shipwright/workflow.yaml`: canonical candidate

`.shipwright/workflow.yaml` defines the full gate above as a single
Shipwright workflow (`workspace-format`, `workspace-check`, `workspace-tests`,
`workspace-lint`, `architecture-layers`, `architecture-isolation`,
`repository-hygiene`, `production-integration`), runnable with one
invocation:

```bash
dagger run ./shipwright --workflow .shipwright/workflow.yaml
```

It is currently a **canonical candidate**, validated in
`.github/workflows/shipwright-validation.yml` for semantic equivalence with
the native commands above, but not yet the enforced gate — `production-gate.yml`'s
native commands remain load-bearing until a follow-up PR removes them in
favor of this single invocation.

## Branching, Releases, and Hotfixes

### Branching model

- Feature work branches off `develop` and lands back on `develop` via PR.
- `develop` promotes to `main` via PR when it is release-ready.
- A hotfix branches off `main` directly and lands on `main` via PR — never
  by routing an urgent fix through `develop` first, which would gate it
  behind whatever else is in flight there.
- Every hotfix that lands on `main` **must** subsequently be backported to
  `develop`, by `git cherry-pick`, so `develop` never permanently diverges
  from a fix that shipped. See "Backport drift check" below for how this is
  monitored.

### Cutting a release

`.github/workflows/release.yml` cuts exactly one release as a direct result
of a merge landing on `main` — never on `develop`, never per-PR. It is fully
automatic: no manual trigger, no approval step, nothing to run by hand for
an ordinary release.

What it does, in order:

1. Computes the next version from the Conventional Commits since the
   previous release tag (`git cliff --bumped-version`, config in
   `cliff.toml` at the repo root).
2. Renders the release notes for that range, grouped by commit type
   (`git cliff --unreleased`).
3. Pushes an annotated tag for that version — an explicit tag refspec only,
   never a commit or a branch push, so nothing is ever written back to
   `main` or `develop` and no branch-protection rule is touched.
4. Publishes a GitHub Release from that tag with the rendered notes as its
   body (`gh release create --verify-tag`).

The same `git cliff` commands run identically on your machine — no
`--config` flag to remember, because `cliff.toml` lives at the repo root:

```bash
git cliff --bumped-version
git cliff --unreleased --tag "$(git cliff --bumped-version)"
```

Version derivation and changelog grouping are git-cliff's logic, not
custom code in this repo — there is nothing here to unit test beyond the
workflow wiring itself.

#### One-time baseline tag

The very first release needs a starting point. Seed it once, by hand, on
the current `main` head:

```bash
git tag -a v0.1.0 <main-sha> -m v0.1.0
git push origin v0.1.0
```

`cliff.toml`'s `initial_tag = "v0.1.0"` is a deterministic fallback only —
if the workflow ever runs before this seed exists, it still computes
`v0.1.0` rather than failing, but seeding it explicitly keeps the first
automated cut a normal increment instead of a special case.

#### First-release verification checklist

Because there is no prior tag to validate output against, verify the first
automated cut by hand before trusting the workflow going forward:

- [ ] Seed the baseline tag (above) on the current `main` head.
- [ ] Watch the next merge to `main` trigger `release.yml`.
- [ ] Confirm exactly one new tag and one new GitHub Release were created.
- [ ] Confirm `production-gate.yml` did **not** re-run as a result of the
      tag push (it triggers on `push: branches: [develop, main]`, and a
      tag push matches neither).
- [ ] Confirm `git log origin/main` is unchanged — the release process adds
      no commit.
- [ ] Re-run the completed `release.yml` job against that same `main` SHA
      from the Actions UI ("Re-run jobs") and confirm it creates no second
      tag or Release — the idempotency guard should exit cleanly instead.

### Backport drift check

`.github/workflows/backport-drift.yml` runs daily (and on demand via
`workflow_dispatch`) and compares `main` against `develop` by patch content
(`git cherry -v`, so a cherry-picked backport is recognized even though its
commit SHA differs). A hotfix on `main` with no patch-equivalent commit on
`develop`, older than the `BACKPORT_WINDOW_HOURS` grace period (72 hours by
default — see `.github/scripts/backport-drift.sh`), is reported.

A single open GitHub issue labelled `backport-drift` tracks this: it is
created the first time drift is found, its body is refreshed on every
subsequent run while drift remains, and it is closed automatically once
the report goes clean. If you see this issue, the fix is a plain
`git cherry-pick <sha>` of the listed commit(s) onto `develop` — once that
lands, the next scheduled run recognizes the patch match and closes the
issue itself, no manual close needed.

This check never blocks a merge or a push. It is a report, not a gate —
`backport-drift.sh` always exits `0`, and nothing in this workflow can fail
a PR or branch protection check.
