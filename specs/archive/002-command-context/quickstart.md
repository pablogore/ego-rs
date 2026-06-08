# Quickstart: Execution Context Validation

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Contracts**: [contracts/execution_context.md](contracts/execution_context.md)

## Prerequisites

- Rust toolchain (latest stable)
- Existing workspace compiles: `cargo build`
- Existing tests pass: `cargo test`

## Validation Scenarios

### Scenario 1: Trait Compilation Check

Verify the `ExecutionContext` trait and associated types compile in `ego-domain`:

```bash
cargo build -p ego-domain
```

**Expected**: Build succeeds. No runtime dependencies leaked into domain crate.

### Scenario 2: Identity Access

Verify identity fields are accessible from an execution handler:

```bash
cargo test -p ego-runtime -- test_context_identity
```

**Expected**: Tests verify that aggregate_id, entity_id, and tenant_id set on the context are readable by the handler. Fields not set return `None` (no panic).

### Scenario 3: Correlation Access

Verify correlation fields are accessible:

```bash
cargo test -p ego-runtime -- test_context_correlation
```

**Expected**: Tests verify that correlation_id, causation_id, and request_id round-trip correctly. Absent fields return `None`.

### Scenario 4: Metadata Access

Verify metadata is readable:

```bash
cargo test -p ego-runtime -- test_context_metadata
```

**Expected**: Populated metadata is fully accessible. Empty metadata returns without error.

### Scenario 5: Runtime Portability

Verify the same handler works with different runtime implementations:

```bash
cargo test -p ego-runtime -- test_runtime_portability
```

**Expected**: The same handler code compiles and passes with the test/deterministic runtime without modification.

## Test Configuration

Tests only need plain struct construction — no external infrastructure:

| Component | Purpose |
|-----------|---------|
| Identity types | Construct and read AggregateId, EntityId, TenantId |
| Correlation types | Construct and read CorrelationId, CausationId, RequestId |
| Metadata | Construct and read `HashMap<String, String>` |

## Full Validation

```bash
cargo test -p ego-domain -p ego-runtime
```

**Expected**: All tests pass. No regressions in existing tests.
