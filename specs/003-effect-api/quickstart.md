# Quickstart: Effect API Validation

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Contracts**: [contracts/effect.md](contracts/effect.md)

## Prerequisites

- Rust toolchain (latest stable)
- Existing workspace compiles: `cargo build`
- Existing tests pass: `cargo test`
- 002-execution-context implementation complete (recommended, not required)

## Validation Scenarios

### Scenario 1: Effect Type Compilation

Verify the `Effect` enum and variants compile in `ego-domain`:

```bash
cargo build -p ego-domain
```

**Expected**: Build succeeds. No runtime dependencies leaked into domain crate.

### Scenario 2: Reply Effect

Verify a reply effect is constructable and assertable:

```bash
cargo test -p ego-domain -- test_reply_effect
```

**Expected**: Tests verify that `Effect::reply(value)` constructs correctly and matches by value equality.

### Scenario 3: Event Emission Effect

Verify event emission effect is constructable:

```bash
cargo test -p ego-domain -- test_event_emission
```

**Expected**: Tests verify that `Effect::emit(events)` constructs correctly with single and multiple events.

### Scenario 4: Composition

Verify composed effects are constructable and decomposable:

```bash
cargo test -p ego-domain -- test_effect_composition
```

**Expected**: Tests verify that composed effects contain the expected child effects and the structure is assertable.

### Scenario 5: No Infrastructure

Verify no external infrastructure is required:

```bash
cargo test -p ego-domain
```

**Expected**: All tests pass with zero infrastructure, databases, or network access. Effects are pure value types.

## Handler Validation Pattern

Execution handlers return `Effect<E, R, S>` synchronously. Tests validate handlers by calling them and asserting the returned Effect:

```rust
let result = my_handler(input);
assert_eq!(result, Effect::reply(expected_value));
```

No runtime, no infrastructure required.

## Test Configuration

Tests only need plain struct/enum construction — no external infrastructure:

| Component | Purpose |
|-----------|---------|
| Effect variants | Construct and assert NoEffect, Reply, EventEmission, StateMutation, Composed |
| Composition | Combine effects, assert nested structure |
| Value equality | Verify effects equal by value, not by reference |

## Full Validation

```bash
cargo test -p ego-domain
```

**Expected**: All tests pass. No regressions in existing tests.
