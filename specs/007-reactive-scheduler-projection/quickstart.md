# CORE-007 Quickstart: Validation Guide

## Prerequisites

- Rust toolchain (stable)
- `cargo make` or `make` for running workspace tests
- Existing CORE-006 runtime compiled (`crates/runtime`, `crates/persistent-entity`)

## Setup

```bash
# Checkout feature branch
git checkout 007-reactive-scheduler-projection

# Build the entire workspace (includes new ego-scheduler crate)
cargo build --workspace

# Run scheduler unit tests
cargo test -p ego-scheduler
```

## Validation Scenarios

### Scenario 1: Event Consumption

```bash
cargo test -p ego-scheduler -- test_event_consumption
```

**Expected**: `SchedulerState.total_events_consumed` increments for each event consumed.
**Given**: An event with `SchedulerEvent::ExecutionCompleted { entity: "A", version: 1 }`
**When**: Consumed by Scheduler
**Then**: `state.total_events_consumed == 1`, `state.last_sequence_id == Some(1)`

### Scenario 2: Deterministic Projection

```bash
cargo test -p ego-scheduler -- test_deterministic_projection
```

**Expected**: Two Scheduler instances fed identical event sequences produce identical state.
**Given**: Event sequence `[E1, E2, E3]` fed to Scheduler A and Scheduler B
**When**: Both consume all events
**Then**: `SchedulerState(A) == SchedulerState(B)` (field-by-field equality)

This test MUST pass regardless of:
- Loss configuration (Block vs DropNewest)
- event_id strategy
- Replay buffer contents

### Scenario 3: RoundRobin Policy

```bash
cargo test -p ego-scheduler -- test_round_robin_policy
```

**Expected**: `suggest_activation` cycles through pending entities deterministically.
**Given**: `pending_entities = {A, B, C}`, `total_events_consumed = 5`
**When**: `RoundRobin.suggest_activation(state, pending)` called
**Then**: Returns entity determined by `sorted(pending)[state.total_events_consumed % 3]`

### Scenario 4: Policy Determinism

```bash
cargo test -p ego-scheduler -- test_policy_determinism
```

**Expected**: Same `(SchedulerState, pending_entities)` → same suggestion (1000 random inputs).
**Given**: Property-based test across 1000 random `(state, pending)` pairs
**When**: `suggest_activation` called twice with same inputs
**Then**: Both calls return identical `Option<EntityTriple>`

### Scenario 5: Gap Detection

```bash
cargo test -p ego-scheduler -- test_gap_detection
```

**Expected**: Missing `sequence_id` values are detected and recorded.
**Given**: Events with `sequence_id = [1, 2, 4, 5]` (gap at 3)
**When**: Consumed by Scheduler
**Then**: `state.detected_gaps >= 1`, gap range `(2, 4)` recorded

### Scenario 6: Backpressure — Block Policy

```bash
cargo test -p ego-scheduler -- test_backpressure_block
```

**Expected**: Default `Block` policy prevents event loss.
**Given**: Event bus with capacity 10, `DropPolicy::Block`
**When**: 15 events emitted faster than consumption
**Then**: Sender blocks at capacity, all 15 events eventually consumed (no drops)

### Scenario 7: Backpressure — DropNewest Policy

```bash
cargo test -p ego-scheduler -- test_backpressure_drop_newest
```

**Expected**: `DropNewest` policy drops events without blocking Actor.
**Given**: Event bus with capacity 10, `DropPolicy::DropNewest`
**When**: 100 events emitted faster than consumption
**Then**: `state.detected_gaps > 0`, Actor execution never blocked

### Scenario 8: CORE-006 Unchanged

```bash
# Validate no modifications to CORE-006
git diff --name-only main...HEAD -- crates/runtime/ crates/persistent-entity/ crates/domain/
```

**Expected**: Empty diff. No CORE-006 files were modified.

### Scenario 9: Replay Buffer Diagnostic Only

```bash
cargo test -p ego-scheduler -- test_replay_buffer_bounded
```

**Expected**: Replay buffer is bounded (1024) and does NOT serve as full-state source.
**Given**: 2000 events consumed
**When**: Checking replay buffer
**Then**: `replay_buffer.len() <= 1024`, only the most recent events retained

## Running All Validation

```bash
# Unit + integration tests for scheduler
cargo test -p ego-scheduler

# Full workspace tests (confirms CORE-006 unchanged)
cargo test --workspace

# Determinism verification (property-based)
cargo test -p ego-scheduler -- test_policy_determinism
```

## Key Contracts

| Contract | File | Description |
|----------|------|-------------|
| `SchedulingPolicy` trait | `contracts/scheduling-policy.md` | Pure function contract for activation suggestion |
| `SchedulerEventEnvelope` | `data-model.md` | Event envelope format between CORE-006 and CORE-007 |
| `SchedulerState` | `data-model.md` | Deterministic projection state |

## Architecture Invariant Verification

After setup, verify the three hard invariants:

```bash
# Invariant 1: Observed stream only source of determinism
cargo test -p ego-scheduler -- test_deterministic_projection

# Invariant 2: Per-actor ordering only (cross-actor ordering is unspecified)
# Verified by design — SchedulerState tracks per-actor sequence independently

# Invariant 3: Non-self-healing (recovery is external)
# Verified by design — Scheduler has no recovery code path
```
