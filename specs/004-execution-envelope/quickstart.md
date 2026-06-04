# Quickstart: Execution Envelope Validation

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Contracts**: [contracts/envelope.md](contracts/envelope.md)

## Prerequisites

- Rust toolchain (latest stable)
- Existing workspace compiles: `cargo build`
- Existing tests pass: `cargo test`
- 002-execution-context complete (types available)
- 003-effect-api complete (Effect type used by handlers that receive context)

## Validation Scenarios

### Scenario 1: Envelope Type Compilation

Verify `ExecutionEnvelope<P>` compiles in `ego-domain`:

```bash
cargo build -p ego-domain
```

**Expected**: Build succeeds. No runtime dependencies leaked into domain crate.

### Scenario 2: Envelope Construction

Verify an envelope with known fields constructs correctly:

```bash
cargo test -p ego-domain -- envelope_construction
```

**Expected**: Envelope constructed with identity, correlation, and metadata fields preserves all values.

### Scenario 3: Context Construction from Envelope

Verify ExecutionContext is constructable from envelope:

```bash
cargo test -p ego-domain -- envelope_to_context
```

**Expected**: Context accessors return the same values as the envelope fields.

### Scenario 4: Runtime Integration

Verify the runtime struct constructs from envelope end-to-end:

```bash
cargo test -p ego-runtime -- envelope_to_runtime_context
```

**Expected**: Runtime context returns all fields set on the envelope.

### Scenario 5: No Infrastructure

Verify no external infrastructure is required:

```bash
cargo test -p ego-domain -p ego-runtime
```

**Expected**: All tests pass with zero infrastructure, databases, or network access.

## Test Configuration

Tests only need plain struct construction — no external infrastructure:

| Component | Purpose |
|-----------|---------|
| ExecutionEnvelope | Construct with payload + identity + correlation + metadata |
| ExecutionContext | Convert from envelope, assert accessors |
| Runtime struct | Build from envelope, end-to-end test |

## Full Validation

```bash
cargo test -p ego-domain -p ego-runtime
```

**Expected**: All tests pass. No regressions in existing tests.
