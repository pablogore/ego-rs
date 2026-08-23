---
name: ego-rs-testing-tdd
description: "Trigger: TDD, red green refactor, write a failing test first, test double, mockall, testkit, test builder, fixture, arrange act assert. Apply the TDD workflow and the concrete test patterns used in ego-rs."
license: Apache-2.0
metadata:
  author: "pablogore"
  version: "1.0"
  ported-from: "syntegrity-platform docs/en/development/testing_skill.md"
---

<!--
  STAGED RULES — sections requiring infrastructure that does not exist yet are
  commented out and marked `PENDING [<id>]` with their activation condition.

  Pending: [INFRA-CRATE], [PACT]

  Do not follow a commented rule. Do not uncomment on your own initiative.
-->


## Activation Contract

Load this skill when:
- Implementing any task under strict TDD
- Choosing a test double (TestKit type vs. `mockall` vs. hand-rolled stub)
- Writing a test builder or fixture
- Reviewing a test for shape, independence, or hidden implementation coupling

Pair with `ego-rs-testing-strategy` for the level and coverage gates.

## Scope

This workflow governs **new tests and tests you modify**. The existing suite is
not rewritten retroactively — an untouched test stays as it is, and cleanup is
separate, deliberate work with its own review.

## Quick Reference

| Standard | Value |
|----------|-------|
| TDD | Test before implementation, always |
| Coverage | `make test-cov` → tarpaulin `--fail-under 95` |
| Complexity | ≤ 10 per function |
| Pyramid | ~70% unit / ~20% integration / ~10% end-to-end |
| Mock framework | `mockall` (workspace dependency) |
| Shared doubles | `crates/testkit` |
| Real infrastructure | Not available yet — no test may reach one |

<!-- PENDING [INFRA-CRATE] — replaces the last row above.
| Real infrastructure | `crates/integration-tests/` + testcontainers, nowhere else |
-->
<!-- PENDING [PACT] — adds this row.
| API contracts | Pact (`pact_consumer` / `pact_provider`) |
-->

```bash
cargo test --workspace
cargo test -p ego-service-sdk security_context
cargo test -- --nocapture
make test-cov
cargo clippy --workspace -- -W clippy::cognitive-complexity
```

## TDD Workflow

### Step 1 — RED: write the failing test

The test must fail for the RIGHT reason. A test that fails to compile because
the type does not exist yet is a valid RED; a test that fails because of a typo
in the assertion is not.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::Validate;

    #[test]
    fn validate_rejects_zero_capacity() {
        let mut config = AppConfig::default();
        config.scheduler.capacity = 0;

        assert!(
            config.validate().is_err(),
            "AppConfig::validate must reject an invalid subtree"
        );
    }
}
```

Never write a unit test against a real resource:

```rust
// WRONG — real database in a unit test
let pool = PgPool::connect("postgres://localhost/db").await.unwrap();

// WRONG — real network call
let body = reqwest::get("https://api.example.com/x").await.unwrap();

// WRONG — real wall clock
let expired = token.issued_at + Duration::hours(1) < Utc::now();

// WRONG in a unit test — even self-hosted loopback. Hermetic, but it belongs
// at the integration level, in `crates/<crate>/tests/`.
let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
```

Use a double instead:

```rust
// TestKit first — real production contract, isolated instance
let ctx = ego_testkit::test_context();
let logger = ego_testkit::CapturingLogger::new();
let executor = ego_testkit::RecordingExecutor::always_succeeds();

// mockall when the port has no TestKit type
let mut repo = MockUserRepository::new();
repo.expect_find_by_id().returning(|_| Ok(None));
```

### Step 2 — GREEN: minimal implementation

Write the least code that turns the test green. No speculative branches, no
unused parameters, no "we will need this later" hooks. Untested branches added
now are the ones that break in production later.

### Step 3 — REFACTOR: split anything over complexity 10

```bash
cargo clippy --workspace -- -W clippy::cognitive-complexity
```

```rust
// Before — one function carrying the whole decision tree
pub fn dispatch(&self, cmd: Command, ctx: &ServiceContext) -> Result<Response> {
    // guard chain, tenant check, authorization, handler lookup, effect queue...
}

// After — each step independently testable, each under the limit
pub fn dispatch(&self, cmd: Command, ctx: &ServiceContext) -> Result<Response> {
    self.check_guards(&cmd, ctx)?;
    let handler = self.resolve_handler(&cmd)?;
    handler.handle(cmd, ctx)
}
```

The refactor step is not optional and the tests must stay green through it.

## Choosing a Test Double

Apply in order. Stop at the first that fits.

| Order | Double | When |
|-------|--------|------|
| 1 | `crates/testkit` type | A TestKit type already covers the contract |
| 2 | `mockall` mock | The port is a trait with no TestKit type |
| 3 | Hand-rolled stub | `mockall` is overkill for a two-method trait |
| 4 | No double — move the test | It needs a real resource → it is not a unit test |

Step 4 is not a suggestion. A unit test that reaches a database, broker, socket,
or HTTP service is **misclassified**, and the fix is never a feature flag, an
`#[ignore]`, or a "skip unless CI" guard.

Where it goes depends on what it actually needs:

- **A loopback server it starts and stops itself** → hermetic. Move it to
  `crates/<crate>/tests/`. No container, no harness.
- **An externally-provisioned service** → no harness exists in this workspace
  today. Raise it; do not write it.

<!-- PENDING [INFRA-CRATE] — replaces the last sentence above.
It moves to `crates/integration-tests/` and gets its resource from
testcontainers. That crate is the only place in the workspace where real
infrastructure is allowed.
-->

**Same-contract principle** (`crates/testkit/src/lib.rs`): a double implements
the real production trait. It never becomes a parallel implementation with its
own behavior, because that version silently drifts from production and the test
keeps passing while the system breaks.

TestKit already provides:

- Context: `test_context()`, `TestContextBuilder`
- Identity: `principal()`, `PrincipalBuilder`, `authenticated()`, `authenticated_with_claims()`
- Authorization: `DenyAllAuthorizationProvider`, `ScriptedAuthorizationProvider`
- Effects: `RecordingExecutor`, `RecordedAttempt`
- Logging: `CapturingLogger`, `CapturedRecord`
- Data: `StaticDataProvider`, `RecordingDataProvider`
- Health: `StaticHealthContributor`
- Config: `TestConfig`
- JWT: `TestJwtBuilder`
- Assertions: `assert_authorized`, `assert_denied`
- Fixtures: `FixtureBuilder`, `ServiceTestFixture`

Check this list before writing a new double. A duplicate double is a maintenance
liability and a drift risk.

## Test Patterns

### Arrange–Act–Assert

```rust
#[tokio::test]
async fn register_user_records_one_effect_attempt() {
    // Arrange
    let executor = RecordingExecutor::always_succeeds();
    let ctx = test_context();
    let handler = RegisterUser::new(executor.clone());

    // Act
    let result = handler.handle(RegisterUserCommand::sample(), &ctx).await;

    // Assert
    assert!(result.is_ok());
    assert_eq!(executor.attempts().len(), 1);
}
```

### Cover every outcome

Enumerate the variants the function can return. One test per outcome, or one
table-driven test — never a single happy-path assertion.

```rust
#[test]
fn validate_covers_every_rejection_reason() {
    for (mutate, reason) in [
        (zero_capacity as fn(&mut AppConfig), "capacity"),
        (empty_tenant as fn(&mut AppConfig), "tenant"),
    ] {
        let mut config = AppConfig::default();
        mutate(&mut config);
        assert!(config.validate().is_err(), "must reject: {reason}");
    }
}
```

### Test the error paths

Error paths are where coverage gates are usually gamed. Assert the specific
error, not just `is_err()`:

```rust
let err = config.validate().unwrap_err();
assert!(matches!(err, ValidationError::OutOfRange { field, .. } if field == "capacity"));
```

### Assert on observable effects

```rust
#[tokio::test]
async fn failed_read_side_logs_at_error_level() {
    let logger = CapturingLogger::new();
    let ctx = TestContextBuilder::new().logger(logger.handle()).build();

    let _ = failing_projection().run(&ctx).await;

    assert!(logger.records().iter().any(|r| r.level == Level::Error));
}
```

<!-- PENDING [PACT] — activate once `pact_consumer` / `pact_provider` are
     workspace dependencies and a `contract_tests` target exists.

### Contract tests with Pact

A contract test pins the shape of an API across a boundary. TDD still applies:
write the expectation first, then make the API satisfy it.

**Consumer side** — declare what we expect from an API we call. Pact stands up
a mock server, so this is still hermetic: no real network, no live third party.

```rust
// examples/reference-app/tests/contract_tests.rs
#[tokio::test]
async fn register_user_contract() {
    let mock = PactBuilder::new("ReferenceAppClient", "ReferenceAppAPI")
        .interaction("register a user", |mut i| {
            i.request().method("POST").path("/register")
                .json_body(json!({ "email": "user@example.com" }));
            i.response().status(201);
            i
        })
        .start_async()
        .await;

    let client = ApiClient::new(mock.url());
    let response = client.register("user@example.com").await.unwrap();

    assert_eq!(response.status, 201);
    mock.verify_async().await;
}
```

**Provider side** — load the contract, start a local server, verify our API
satisfies it.

Rules:
- A failing contract blocks the merge.
- **Never regenerate a contract to make it green.** That deletes the exact
  signal the test exists to produce. If the break is intentional: document it,
  version the API, notify consumers, then update the contract deliberately.
-->

### Snapshot tests with `insta`

`insta` pins a stable serialized format (generated code, error payloads, log
records). It is already a dependency in `crates/service-sdk`.

Never blind-accept a changed snapshot. `cargo insta review` and read the diff —
`cargo insta accept` without reading turns a regression into a committed
expectation.

### Builders over literal construction

```rust
// Brittle — every new field breaks every test
let p = Principal { id: Id::new(), tenant: "acme".into(), roles: vec![], /* ... */ };

// Stable — the builder absorbs new fields
let p = PrincipalBuilder::new().tenant("acme").build();
```

Add a builder when a type is constructed in three or more tests.

## Common Mistakes

**Testing implementation, not behavior**

```rust
// BAD — couples the test to internal storage
assert_eq!(use_case.repository.items.len(), 1);

// GOOD — asserts through the public contract
assert_eq!(repository.find_all().await.unwrap().len(), 1);
```

**Interdependent tests.** Shared mutable state or order dependence. Every test
constructs its own fixture. `cargo test` runs in parallel — a test that only
passes in sequence is already broken.

**Swallowed results**

```rust
let _ = use_case.execute(cmd).await;   // BAD
let result = use_case.execute(cmd).await;
assert!(result.is_ok());               // GOOD
```

**`#[ignore]` as an escape hatch.** Reserved for a known-flaky test with a
tracking issue. Needing a real resource means it is the wrong level — move it,
do not silence it.

**Blind-accepting snapshots.** `cargo insta accept` without reading the diff
turns a regression into a committed expectation. Use `cargo insta review`.

## Coverage Checklist

Before opening a PR:

- [ ] Every new public function has a test
- [ ] Every domain rule and invariant is tested
- [ ] Every error path asserts the specific error
- [ ] Edge cases: empty, zero, missing, unauthorized, concurrent
- [ ] Effects and log output are asserted where they are part of the contract
- [ ] `cargo test --workspace` passes with 0 failures
- [ ] `make clippy` is clean
- [ ] No function exceeds complexity 10

## Output Contract

- Tests written before implementation, RED verified before GREEN.
- Report which double was chosen at each boundary and why (TestKit / mockall / stub).
- Report any TestKit type added, and why no existing one fit.

## References

- `ego-rs-testing-strategy` — levels, pyramid, coverage gates
- `ego-rs-testing` — hard placement rules
- `crates/testkit/src/lib.rs` — same-contract principle and full export list
- `examples/reference-app/tests/pipeline.rs` — a reference end-to-end test
