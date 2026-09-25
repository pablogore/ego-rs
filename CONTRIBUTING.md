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
`needs:` five independent jobs that run in parallel — `lint`, `check`,
`test`, `architecture`, `integration` — so wall-clock is bounded by the
slowest of them, not the sum of all steps. `check` and `test` used to be one
`build-test` job; they were split because `cargo test --workspace` alone
(~520s) dwarfed every other job, and giving `cargo check` its own runner
means a compile error fails fast instead of queuing behind the full test run:

```bash
# lint
dagger run ./shipwright --workflow .shipwright/workflow.yaml -step workspace-lint  # cargo clippy --workspace --all-targets --all-features -- -D warnings

# check
cargo check --workspace --all-targets

# test
cargo nextest run --workspace
cargo test --doc --workspace  # nextest does not run doctests

# architecture
cargo run -p xtask -- verify-layers
cargo run -p xtask -- verify-isolation
cargo run -p xtask -- verify-hygiene

# integration
cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite
```

`test` uses `cargo-nextest` (`cargo install cargo-nextest --locked` locally,
or `cargo binstall cargo-nextest`) instead of plain `cargo test` — it
parallelizes test-binary execution across cores. `cargo test --doc` still
covers doctests since nextest cannot run those.

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

### Compiler profile and linker (shared by CI and developers)

CI jobs no longer pass their own `RUSTFLAGS`; every job and every checkout
compiles with the same settings, so `check`, `test` and `architecture` restore
one cache (`shared-key: production-gate`, saved only by `test` on `develop`,
read-only for PRs).

- **Profile** — the root `Cargo.toml` (and, as a separate workspace,
  `integration-tests/Cargo.toml`) sets `debug = "line-tables-only"` for
  workspace crates and `debug = false` for dependencies. Backtraces keep
  file:line for our code; the linker stops moving full debuginfo around. If
  you need full debuginfo to step through a dependency, override it locally:
  `cargo build --config 'profile.dev.package."*".debug=true'`.
- **Linker** — `.cargo/config.toml` routes `x86_64-unknown-linux-gnu` links
  through `scripts/cargo-linker.sh`: mold when it is on `PATH` (via clang,
  else cc), the default linker otherwise. Installing mold locally
  (`apt install mold`, `brew install mold`, …) is optional and only speeds up
  linking; without it nothing breaks, which is also what keeps the Dagger
  `lint` step working in the stock `rust:<version>` image. macOS and other
  targets are untouched. CI sets `EGO_REQUIRE_MOLD=1` so a missing mold fails
  the job instead of silently linking without it. The wrapper's own test is
  `scripts/tests/test-cargo-linker.sh`.

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
