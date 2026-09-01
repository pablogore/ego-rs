---
name: ego-rs-testing
description: "Trigger: test, unit test, integration test, mock, testcontainer, database test, broker test. Enforce testing conventions: unit tests use mocks only, integration tests live in a dedicated testcontainers crate."
license: Apache-2.0
metadata:
  author: "pablogore"
  version: "1.0"
---

## Activation Contract

Load this skill when:
- Writing any test (unit or integration)
- Reviewing a PR that adds or modifies tests
- Deciding where a test belongs (in-crate vs. integration crate)
- Adding a new crate that needs test coverage

## Hard Rules

### Rule 1 — Unit tests: no external resources (ABSOLUTE)

Every `#[test]` and `#[tokio::test]` inside a `crates/` module MUST be isolated.
**Forbidden in unit tests:**
- Real database connections (Postgres, Redis, SQLite)
- Real message brokers (Kafka, RabbitMQ, NATS)
- Real HTTP/gRPC calls to external APIs
- `std::fs` writes outside of `tempfile`-managed dirs
- Any `tokio::net` or `std::net` that binds or connects to a real socket

If a test needs any of the above → it is an integration test → move it to `crates/integration-tests/`.

### Rule 2 — Unit tests use mocks for all external dependencies

Dependencies that cross a crate or I/O boundary MUST be mocked:
- Use `mockall` (already in `[workspace.dependencies]`) for trait objects.
- Use `Arc<dyn Trait>` with a hand-rolled in-test stub when `mockall` is overkill.
- Never instantiate a real `PgPool`, `sqlx::Pool`, or broker client in a unit test.

### Rule 3 — Integration tests live in `crates/integration-tests/` (default) or a documented independent root-level workspace

The default integration crate:
- Path: `crates/integration-tests/`
- Must use `testcontainers` (or `testcontainers-modules`) for all external services.
- Each test module spins up and tears down its own container — no shared state between tests.
- Integration tests are excluded from the default `cargo test` run via `[[test]]` in `Cargo.toml` or a CI-only profile.

**Documented architectural exception**: a root-level `integration-tests/` crate, deliberately excluded from the workspace `members` list, is allowed when a written, approved architecture decision requires it (e.g. keeping Docker/Testcontainers out of the default root build — see `integration-tests/README.md` and GitHub issue #275). Under that exception:
- One shared external-service container (e.g. one PostgreSQL container) per test run, reused across test modules via a process-wide handle, is allowed in place of one container per test module — provided each test still runs against its own isolated database/schema within that shared container (e.g. `isolated_database()`), so tests remain independent even though the container is not.
- The exception must be traceable to the approving decision (issue/ADR/design doc) referenced from the crate's own README or module docs — an undocumented shared container is still a violation.

### Non-exceptions

Speed or convenience is never grounds for the exception above — only an approved architecture decision citing a concrete constraint (e.g. no-Docker root build) qualifies.

### Rule 4 — No `#[ignore]` as a workaround

Do not mark a test `#[ignore]` because it needs external resources. Move it to the integration crate instead. `#[ignore]` is reserved for known-flaky tests with a tracking issue.

## Decision Gates

| Test needs | Where it goes |
|------------|--------------|
| Only in-memory state, mocks, stubs | `#[cfg(test)]` inside the crate module |
| Real DB / broker / HTTP | `crates/integration-tests/` with testcontainers |
| Compile-time assertion (`static_assertions`, trait bounds) | `#[cfg(test)]` inside the crate module |
| Contract / property test (no I/O) | `#[cfg(test)]` inside the crate module |
| End-to-end scenario with multiple services | `crates/integration-tests/` |

## Execution Steps

1. Identify what the test exercises.
2. Check Decision Gates table → choose the location.
3. If unit test: replace every real dependency with a mock or in-test stub. Verify no network/disk I/O escapes.
4. If integration test: confirm `crates/integration-tests/` exists; create it if not (see `assets/integration-crate-template.toml`). Use `testcontainers` to provision the service.
5. After writing: run `cargo test -p <crate>` (unit) or the integration suite separately. Both must pass with 0 failures.

## Output Contract

- Unit tests: `#[cfg(test)] mod tests { ... }` inside the crate, zero real I/O.
- Integration tests: file under `crates/integration-tests/tests/`, using `testcontainers`.
- Report which rule applied and which mock strategy was chosen.

## References

- `assets/integration-crate-template.toml` — starter `Cargo.toml` for `crates/integration-tests/`
