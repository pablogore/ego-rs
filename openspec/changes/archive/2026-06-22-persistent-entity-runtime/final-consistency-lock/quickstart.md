# Quickstart: CORE-006 Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime/final-consistency-lock`
**Created**: 2026-06-07

## Prerequisites

- Rust >= 1.85 (2024 edition)
- `cargo test` for unit/integration tests
- Optional: PostgreSQL 16+ for production persistence (in-memory works for tests)

## Validation Scenarios

### Scenario 1: Define and Run a Counter Entity

**Setup**:
```bash
cd crates/persistent-entity
```

**Test**: Define a `Counter` entity with `Increment` and `Decrement` commands.

Expected flow:
1. Define Counter entity implementing `PersistentEntity`
2. Build EntityRuntime with in-memory backend
3. Send `Increment(1)` → receives event `Incremented(1)`
4. Send `Increment(2)` → receives event `Incremented(2)`
5. State = 3 (1 + 2)
6. Recovery: restart runtime → state = 3 (replay from events)

**Run**:
```bash
cargo test -- test_counter_entity_lifecycle
```

**Expected outcome**: All assertions pass; state is deterministic on recovery.

---

### Scenario 2: Concurrent Commands — FIFO Ordering Preserved

**Test**: Send 100 concurrent commands to the same entity.

**Run**:
```bash
cargo test -- test_concurrent_fifo_ordering
```

**Expected outcome**: All 100 commands execute sequentially in send order. Mailbox FIFO is proved.

---

### Scenario 3: Single-Flight Activation Under Concurrency

**Test**: Send 100 concurrent commands to a PASSIVATED entity.

**Run**:
```bash
cargo test -- test_single_flight_activation
```

**Expected outcome**: Exactly 1 actor task spawned. All 100 commands processed sequentially by that task. No duplicate spawns.

---

### Scenario 4: Replay == Live Execution

**Test**: Execute 100 commands live, record state. Restart. Verify recovery state == live state.

**Run**:
```bash
cargo test -- test_replay_equals_live
```

**Expected outcome**: Post-recovery state identical to pre-restart state.

---

### Scenario 5: Backend Independence — Same Output Across Backends

**Test**: Execute same commands through TokioBackend and SyncTestBackend.

**Run**:
```bash
cargo test -- test_backend_determinism
```

**Expected outcome**: Identical events and state from both backends.

---

### Scenario 6: Concurrency Budget Enforcement

**Test**: Set budget=2. Activate 5 entities with commands. Verify at most 2 process simultaneously.

**Run**:
```bash
cargo test -- test_concurrency_budget_enforcement
```

**Expected outcome**: No more than 2 actors processing concurrently; all 5 entities eventually complete.

---

### Scenario 7: Fairness Window — No Starvation

**Test**: Entity A gets 1000 commands/sec, Entity B gets 1 command. Verify B processes before A's 1001st command.

**Run**:
```bash
cargo test -- test_fairness_no_starvation
```

**Expected outcome**: Entity B activated within fairness_window scheduling decisions.

---

### Scenario 8: Zero-Event Query — No Version Advance

**Test**: Send a read-only query to entity at version V. Verify version remains V.

**Run**:
```bash
cargo test -- test_zero_event_query
```

**Expected outcome**: Version unchanged, no events persisted, no publications triggered.

---

### Scenario 9: Passivation and Reactivation

**Test**: Let entity passivate (inactivity timeout). Send new command. Verify transparent reactivation.

**Run**:
```bash
cargo test -- test_passivation_reactivation
```

**Expected outcome**: Command sent to PASSIVATED entity triggers single-flight recovery → ACTIVE → processes command → returns result. Transparent to caller.

---

### Scenario 10: Failure Recovery — Determistic Replay of Bug

**Test**: Entity with buggy `apply_event` that panics at event #50. Verify recovery replays events 1-50 and reproduces panic at #50.

**Run**:
```bash
cargo test -- test_deterministic_applier_bug_recovery
```

**Expected outcome**: Entity transitions to FAILED. On recovery, replays events 1-50 and reproduces identical panic. Code fix required before recovery succeeds.
