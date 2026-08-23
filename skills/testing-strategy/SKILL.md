---
name: ego-rs-testing-strategy
description: "Trigger: test strategy, coverage, testing pyramid, test level, where does this test belong, quality gate, tarpaulin, complexity. Define the mandatory testing standards and the test level for a change."
license: Apache-2.0
metadata:
  author: "pablogore"
  version: "1.0"
  ported-from: "syntegrity-platform docs/en/development/testing.md"
---

<!--
  STAGED RULES — sections requiring infrastructure that does not exist yet are
  commented out and marked `PENDING [<id>]` with their activation condition.

  Pending: [INFRA-CRATE], [PACT]

  Do not follow a commented rule. Do not uncomment on your own initiative.
-->


## Activation Contract

Load this skill when:
- Deciding what kind of test a change needs (unit / integration / end-to-end)
- Reviewing whether a PR meets the coverage and complexity gates
- Adding tests to a crate that has none
- Arguing about test strategy or test placement

Pair with `ego-rs-testing-tdd` for the concrete RED→GREEN workflow, and with
`ego-rs-testing` for the hard placement rules.

## Scope

These standards govern **new tests and tests you modify**. The existing suite is
not migrated retroactively.

- New test → full compliance.
- Test you are already editing → bring it into compliance, without expanding the
  diff beyond what you touched.
- Untouched test → leave it. Cleanup is separate, deliberate work.

The coverage and complexity gates are workspace-wide and always apply — they are
properties of the build, not of an individual test.

## Quality Standards (Mandatory)

### Test-Driven Development

- Write the test BEFORE the implementation. The test is the specification.
- Implementation exists to make the test pass. Refactor with the test as the net.
- A task is not started until its RED test exists and fails for the right reason.

### Coverage

- Gate: `make test-cov` → `cargo tarpaulin --workspace --out Html --fail-under 95`.
- Coverage is a floor, not a target. A 95% line number over untested error
  paths is a failure dressed as a pass.
- Additional test levels (integration, end-to-end) sit ON TOP of unit coverage.
  They never substitute for it.

### Cyclomatic Complexity

- Maximum 10 per function.
- Checked with `cargo clippy --workspace -- -W clippy::cognitive-complexity`.
- Over the limit means extract functions. Refactoring is mandatory, not optional.

### Testing Pyramid

| Level | Share | Location |
|-------|-------|----------|
| Unit | ~70% | `#[cfg(test)] mod tests` inside the crate module |
| Integration | ~20% | `crates/<crate>/tests/` |
| End-to-end | ~10% | `examples/reference-app/tests/` |

<!-- PENDING [PACT]
API contract tests (Pact) sit outside the pyramid. They are not a share of the
test count — every exposed API surface needs one, however few surfaces there are.
-->

### Level Separation (ABSOLUTE)

**A unit test is never an integration test in disguise.** The moment a
`#[cfg(test)]` test touches a database, broker, socket, or HTTP service — even a
loopback one it starts itself — it is misclassified. It does not get a feature
flag, an `#[ignore]`, or a "only in CI" guard. It moves up a level.

No test anywhere in this workspace may reach an **externally-provisioned**
resource today — there is no harness for it. A test that needs one is raised,
not written. Self-hosted loopback is fine at the integration level; see below.

<!-- PENDING [INFRA-CRATE] — activate once `crates/integration-tests/` exists.
     Replaces the paragraph above with the concrete destination.

Real external resources exist in exactly one place in this workspace:
`crates/integration-tests/` (`ego-integration-tests`), provisioned exclusively
through `testcontainers` / `testcontainers-modules`. Never a hardcoded
`localhost` URL, never a shared CI database, never a service the developer must
remember to start. A misclassified unit test moves there.

Starter config: `skills/testing/assets/integration-crate-template.toml`.
-->

## Testing Philosophy

- **Test behavior, not implementation.** Verify what a module does, never how.
- **Same-contract principle.** Test doubles must implement the real production
  trait. `crates/testkit` exists precisely so a test exercises real dispatch and
  real validation logic, not a look-alike that silently drifts from production.
  Prefer a TestKit type over a hand-rolled fake; prefer a hand-rolled fake over
  a parallel reimplementation of production behavior.
- **Test at the right level.** Domain rules in unit tests, wiring and adapters
  in crate integration tests, full request flows in the reference app.
- **Every crate is independently testable.** No crate may require another crate's
  runtime or real infrastructure to run its unit tests.

## Test Levels

### Unit Tests — no external resources

Unit tests exercise functions and types in complete isolation. Forbidden:
real databases, real brokers, real HTTP/gRPC calls, real filesystem writes
outside `tempfile`, any socket at all, real wall-clock dependence.

All boundary-crossing dependencies are mocked (`mockall`, in `[workspace.dependencies]`)
or supplied by `crates/testkit`.

What belongs here:
- Domain logic — entities, value objects, invariants, `Validate` impls
- Pure functions — evaluation, validation, transformation
- Use cases with mocked ports
- Compile-time assertions and trait-bound checks

### Integration Tests — crate boundary, no external infrastructure

Location: `crates/<crate>/tests/`. Nine crates already carry one
(`runtime`, `service-sdk`, `transport`, `security-jwt`, `security-sdk`,
`infrastructure`, `persistent-entity`, `ego-scheduler`, plus the reference app).

What belongs here:
- Adapter implementations against their real port, backed by doubles
- Cross-module wiring inside one crate
- Macro codegen output (`service_tag_codegen.rs`, `tenant_scoped_codegen.rs`)
- Security context propagation across layers
- A loopback server the test starts and stops itself

**Self-hosted loopback is not external infrastructure.** Binding
`TcpListener::bind("127.0.0.1:0")`, serving on the OS-assigned ephemeral port,
and dropping it at end of scope keeps the whole lifecycle inside the test
process. That is hermetic — it needs no container and no external harness.
`crates/transport/tests/server.rs`, `crates/infrastructure/tests/otlp_export_roundtrip.rs`
and `examples/reference-app/tests/e2e_register.rs` already do exactly this.

What is external is a service the test does not start: a fixed port, a shared
host, a database someone must remember to run.

<!-- PENDING [INFRA-CRATE] — activate once `crates/integration-tests/` exists.

### Integration Tests — real infrastructure

Location: `crates/integration-tests/` (`ego-integration-tests`).

The only place in the workspace where a real external resource may appear, and
only via testcontainers:

- Real PostgreSQL, Redis, Kafka, NATS — one container per test, torn down at
  end of scope
- Real socket-level transport behavior
- Migration and schema compatibility against a real engine

Rules: each test provisions and disposes its own container; no shared state; the
crate is excluded from the default workspace run and triggered by CI. Starter
config: `skills/testing/assets/integration-crate-template.toml`.
-->

<!-- PENDING [PACT] — activate once `pact_consumer` / `pact_provider` are
     workspace dependencies and a `contract_tests` target exists.

### Contract Tests — API surfaces, with Pact

Every HTTP API exposed to a consumer gets a consumer-driven contract test.

- **Consumer side**: declare what we expect from an API we call. Pact runs a
  mock server — no real network, no live third party.
- **Provider side**: load the contract, start a local server, verify our API
  satisfies it.
- Tooling: `pact_consumer`, `pact_provider`, `pact_mock_server`.
- A failing contract blocks the merge. Regenerating the contract to make it
  green deletes the exact signal the test exists to produce. If the break is
  intentional: document it, version the API, notify consumers — then update.

`make test` already runs `cargo test --test contract_tests`. That target does
not exist, so `make test` currently fails. The Pact suite is what closes it.
Recommended home: `examples/reference-app/tests/contract_tests.rs` — framework
crates expose no HTTP endpoints of their own.
-->

### End-to-End Tests — full pipeline

Location: `examples/reference-app/tests/`.

What belongs here:
- Host → `AppConfig` → service construction → `RuntimeBuilder` pipeline
- Full HTTP + JWT + authorization request flows
- Observability and partial-failure scenarios across the whole stack

Do NOT test domain logic at this level. That is what unit tests are for.

### Snapshot Tests

`insta` (used in `crates/service-sdk`) covers stable serialized formats —
generated code, error payloads, log records. Review with `cargo insta review`;
never blind-accept a changed snapshot.

## Decision Gates

| The test needs | Level | Location |
|----------------|-------|----------|
| Only in-memory state, mocks, TestKit | Unit | `#[cfg(test)]` in the crate |
| A domain rule or invariant | Unit | `#[cfg(test)]` in the crate |
| An adapter against its real port, with doubles | Integration | `crates/<crate>/tests/` |
| Macro-generated code | Integration | `crates/<crate>/tests/` |
| A loopback server the test starts and stops itself | Integration | `crates/<crate>/tests/` |
| The whole runtime pipeline, no external infra | End-to-end | `examples/reference-app/tests/` |
| A stable serialized format | Snapshot | `insta`, alongside its level |
| An externally-provisioned DB, broker, or service | — | No destination yet; raise it |

<!-- PENDING [INFRA-CRATE] — replaces the last row above.
| An externally-provisioned DB, broker, or service | Integration | `crates/integration-tests/` + testcontainers |
-->
<!-- PENDING [PACT] — adds this row.
| To pin an HTTP API contract with a consumer | Contract | Pact, `contract_tests` target |
-->

## Running Tests

```bash
cargo test --workspace          # everything
cargo test -p ego-service-sdk   # one crate
cargo test -p ego-domain validate_rejects   # one test
cargo test -- --nocapture       # with output
make test-cov                   # coverage gate (tarpaulin, 95%)
make clippy                     # -D warnings
```

`make test` currently FAILS: its `cargo test --test contract_tests` line points
at a target that does not exist. Use `cargo test --workspace` until that is
resolved.

<!-- PENDING [PACT]
cargo test --test contract_tests            # Pact contract suite
-->
<!-- PENDING [INFRA-CRATE]
cargo test -p ego-integration-tests         # testcontainers suite (needs Docker)
-->

## Test Naming

- Unit: `<function>_<scenario>_<expected>` — `validate_rejects_zero_capacity`
- Integration: descriptive behavior — `security_context_propagates_to_handler`
- End-to-end: the scenario — `valid_app_config_passes_validate_and_builds_runtime`

## Output Contract

Report, for every change:
1. Which level(s) the new tests live at, and why that level.
2. That `cargo test --workspace` passes with 0 failures.
3. Any function that crossed complexity 10 and how it was split.

## References

- `ego-rs-testing` — hard placement rules and forbidden patterns
- `ego-rs-testing-tdd` — the RED→GREEN→REFACTOR workflow and test patterns
- `crates/testkit/src/lib.rs` — the same-contract principle, in the source
