# CORE-007 Quickstart: Validation Guide

## Prerequisites

- Rust toolchain (stable)
- Existing CORE-006 runtime compiled (`crates/runtime`, `crates/persistent-entity`)

## Setup

```bash
git checkout 007-reactive-scheduler-projection
cargo build --workspace
cargo test -p ego-scheduler
```

## Validation Scenarios

### Scenario 1: Event Consumption
```bash
cargo test -p ego-scheduler -- test_event_consumption
```
**Expected**: `SchedulerState.total_events_consumed` increments for each event consumed.

### Scenario 2: Deterministic Projection
```bash
cargo test -p ego-scheduler -- test_deterministic_projection
```
**Expected**: Two instances fed identical streams → identical SchedulerState. Passes regardless of DropPolicy, replay buffer contents, concurrency scheduling, or execution order.

### Scenario 3: RoundRobin Policy
```bash
cargo test -p ego-scheduler -- test_round_robin_policy
```
**Expected**: Cycles through pending entities deterministically by identity, not cross-entity sequence_id.

### Scenario 4: Policy Determinism
```bash
cargo test -p ego-scheduler -- test_policy_determinism
```
**Expected**: Property-test: 1000 random (state, pending) pairs → identical output.

### Scenario 5: Gap Detection
```bash
cargo test -p ego-scheduler -- test_gap_detection
```
**Expected**: Missing sequence_ids detected per-actor. System continues under gaps.

### Scenario 6: Backpressure (Block)
```bash
cargo test -p ego-scheduler -- test_backpressure_block
```
**Expected**: Block policy prevents loss. Sender blocks, all events consumed.

### Scenario 7: Backpressure (DropNewest)
```bash
cargo test -p ego-scheduler -- test_backpressure_drop_newest
```
**Expected**: DropNewest drops without blocking Actor. Drop pattern deterministic.

### Scenario 8: CORE-006 Unchanged
```bash
git diff --name-only main...HEAD -- crates/runtime/ crates/persistent-entity/ crates/domain/
```
**Expected**: Empty diff.

### Scenario 9: Replay Buffer Diagnostic Only
```bash
cargo test -p ego-scheduler -- test_replay_buffer_bounded
```
**Expected**: Bounded at 1024. Never used for reconstruction.

## Running All Validation

```bash
cargo test -p ego-scheduler
cargo test --workspace
```

## Key Contracts

| Contract | File | Description |
|----------|------|-------------|
| `SchedulingPolicy` trait | `contracts/scheduling-policy.md` | Pure function contract |
| `SchedulerEventEnvelope` | `data-model.md` | Event envelope format |
| `SchedulerState` | `data-model.md` | Deterministic projection state |

## Invariant Verification

```bash
# I1: Determinism — identical streams → identical state
cargo test -p ego-scheduler -- test_deterministic_projection

# I2: Per-entity ordering — no cross-entity sequence_id comparison
cargo test -p ego-scheduler -- test_no_cross_entity_sequence_compare

# I3: No execution authority — CORE-006 has zero dependency on CORE-007
# Verified by design + compile-time check (no import of ego-scheduler in CORE-006 crates)

# I4: ReplayBuffer non-semantic — never reconstruction, validation, or recovery
cargo test -p ego-scheduler -- test_replay_buffer_non_semantic

# I5: Deterministic DropPolicy — same arrival order → same drops under any load
cargo test -p ego-scheduler -- test_drop_policy_determinism
```
