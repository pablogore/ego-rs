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
`develop`, job `production-readiness`. It must be required by branch
protection on `develop` (not yet configured — the workflow existing does not
make it mandatory on its own). It runs, in order:

```bash
cargo check --workspace --all-targets
cargo test --workspace
cargo run -p xtask -- verify-layers
cargo run -p xtask -- verify-isolation
cargo run -p xtask -- verify-hygiene
cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite
```

`run-suite` is the canonical entrypoint for the integration suite: it
provisions real PostgreSQL 16 and 14 via testcontainers, migrates them, runs
the tests, and reclaims the containers itself — Docker is the only local
requirement (`colima start` or Docker Desktop; see `integration-tests/README.md`).
No secrets are involved: production-profile tests supply deterministic
non-dev test keys in code, never via environment variables.

`cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D
warnings` are not part of this gate yet — the tree currently has pre-existing
violations unrelated to production readiness. Wiring them in is follow-up
work, not part of this gate.
