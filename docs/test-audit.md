# Test Audit — External Resource Violations

> Generated: 2026-06-27
> Convention reference: [`skills/testing/SKILL.md`](../skills/testing/SKILL.md)

## Summary

| Metric | Value |
|--------|-------|
| Test files scanned | 83 |
| Violating files | **1** |
| Total violations | **5** (4 test functions + 1 helper) |
| `#[ignore]` misuses | 0 |
| Missing `crates/integration-tests/` | ⚠️ Yes |

---

## Violations

### `crates/infrastructure/src/persistence/postgres/mod.rs`

This file contains a `#[cfg(test)]` block that opens a real PostgreSQL connection — no mocks, no `testcontainers`, not in `crates/integration-tests/`. Doubly non-compliant.

| Test | Violation | Detail |
|------|-----------|--------|
| `test_read_side_store_fetch` | Real DB connection | Calls `setup_test_db()` → `PgPool::connect(url).await` |
| `test_dedup_store_seen_and_mark_seen` | Real DB connection | Same `setup_test_db()` |
| `test_projection_state_store_save_and_load` | Real DB connection | Same `setup_test_db()` |
| `test_offset_store_save_and_load` | Real DB connection | Same `setup_test_db()` |
| `setup_test_db` (helper) | Env-based live URL + hardcoded fallback | `std::env::var("TEST_DATABASE_URL")` with fallback `postgres://postgres:postgres@localhost:5432/test`, then `PgPool::connect(...)` |

**Required action:** move all four tests (and their helper) to `crates/integration-tests/` and rewrite using `testcontainers-modules::postgres::Postgres`.

---

## Items Worth Human Review (not violations, policy calls)

### 1. `crates/persistent-entity/tests/` placement

Five integration test binaries (outside `src/`, compiled separately by Cargo):
- `tests/activation_ordering_tests.rs`
- `tests/entity_definition_tests.rs`
- `tests/persistence_failure_tests.rs`
- `tests/real_actor_path_tests.rs`
- `tests/runtime_verification_suite.rs`

All use only in-memory stubs (`InMemoryEventStore`, `InMemorySnapshotStore`, hand-rolled `FailingEventStore`). **Isolation requirement: met.** Whether they should live in a future `crates/integration-tests/` crate is a policy decision.

### 2. `mockall` adoption is partial

Only `ego-security-sdk` uses `#[cfg_attr(test, mockall::automock)]` on traits. Other crates write hand-rolled stubs (e.g., `FailingEventStore`, `CapturingResolver`). Hand-rolled stubs satisfy isolation but are inconsistent with the `mockall`-first convention in `ego-rs-testing Rule 2`. Worth aligning in a follow-up.

### 3. `crates/integration-tests/` does not exist yet

The convention designates this as the exclusive home for integration tests using `testcontainers`. It has not been created. A starter `Cargo.toml` is available at [`skills/testing/assets/integration-crate-template.toml`](../skills/testing/assets/integration-crate-template.toml).

---

## Clean Files

All 79 remaining test files passed — zero external I/O, zero real sockets, zero `#[ignore]` misuses.

<details>
<summary>Full list</summary>

**crates/application/** — `src/tests.rs`

**crates/domain/** — `src/actor.rs`, `src/auth/claims.rs`, `src/auth/clock.rs`, `src/auth/credential.rs`, `src/auth/error.rs`, `src/command.rs`, `src/context.rs`, `src/effect.rs`, `src/envelope.rs`, `src/observability.rs`, `src/read_side/config.rs`, `src/read_side/error.rs`, `src/read_side/event_stream.rs`, `src/read_side/event_tag.rs`, `src/read_side/offset.rs`, `src/read_side/progress.rs`, `src/read_side/state.rs`

**crates/ego-scheduler/** — `src/policy.rs`, `tests/backpressure.rs`, `tests/determinism.rs`, `tests/gap_detection.rs`, `tests/per_entity_ordering.rs`, `tests/replay_buffer.rs`, `tests/round_robin.rs`

**crates/infrastructure/** — `src/observability.rs`

**crates/persistent-entity/** — `src/command_envelope.rs`, `src/passivation_signal.rs`, `tests/activation_ordering_tests.rs`, `tests/entity_definition_tests.rs`, `tests/persistence_failure_tests.rs`, `tests/real_actor_path_tests.rs`, `tests/runtime_verification_suite.rs`

**crates/runtime/** — `src/context.rs`, `src/interpreter.rs`

**crates/security-jwt/** — `src/authenticator.rs`, `src/config.rs`, `src/key_resolver.rs`, `src/validation.rs`

**crates/security-sdk/** — `src/authentication/mod.rs`, `src/authorization/access_request.rs`, `src/authorization/decision.rs`, `src/authorization/mod.rs`, `src/context/mod.rs`, `src/error/mod.rs`, `src/policy/mod.rs`, `src/principal/principal.rs`, `src/principal/subject_id.rs`, `src/providers/allow_all/mod.rs`, `src/providers/basic/mod.rs`, `src/providers/deny_all/mod.rs`, `src/providers/rbac/mod.rs`, `tests/basic_auth.rs`, `tests/declarative_authz.rs`, `tests/error_mapping.rs`, `tests/rbac.rs`

**crates/service-sdk-macros/** — `src/tests.rs`

**crates/service-sdk/** — `src/context/mod.rs`, `src/contract/descriptor.rs`, `src/contract/version.rs`, `src/di/mod.rs`, `src/implementation.rs`, `src/registry/registry.rs`, `src/runtime/builder.rs`, `src/runtime/permit.rs`, `src/runtime/runtime_builder.rs`, `tests/cancellation.rs`, `tests/context_cross_service.rs`, `tests/context_explicit_propagation.rs`, `tests/context_propagation.rs`, `tests/cross_tenant_access_contract.rs`, `tests/deadline_expiry.rs`, `tests/golden_codegen.rs`, `tests/interceptor_error.rs`, `tests/interceptor_invocation.rs`, `tests/proxy_codegen.rs`, `tests/security_context_propagation.rs`, `tests/security_integration.rs`, `tests/simple_tests.rs`, `tests/smoke.rs`

</details>
