# Quickstart: Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime`

Validation scenarios that prove the feature works end-to-end. Run after implementation to verify correctness.

---

## Prerequisites

- Rust 1.75+ toolchain installed
- Repository cloned and workspace configured: `cargo build` succeeds
- In-memory test backend available (from `ego-infrastructure`)

---

## Setup

```bash
# From workspace root
cargo build --package ego-persistent-entity
```

---

## Validation Scenarios

### Scenario 1: Define a Counter Entity

Define a minimal counter entity to verify the `PersistentEntity` trait compiles and works.

```rust
// tests/entity_lifecycle.rs

use ego_domain::command::Command;
use ego_domain::event::DomainEvent;
use ego_persistent_entity::*;

// --- Domain types ---

#[derive(Clone, Debug)]
struct CounterCommand {
    delta: i32,
}

impl Command for CounterCommand {}

#[derive(Clone, Debug)]
struct CounterEvent {
    delta: i32,
}

impl DomainEvent for CounterEvent {}

#[derive(Clone, Debug, PartialEq)]
struct CounterState {
    value: i32,
}

// --- Entity implementation ---

struct Counter;

#[async_trait]
impl PersistentEntity<CounterCommand, CounterEvent, CounterState> for Counter {
    type Error = CounterError;

    fn initial_state() -> CounterState {
        CounterState { value: 0 }
    }

    async fn handle_command(
        state: &CounterState,
        command: CounterCommand,
        _ctx: CommandContext,
    ) -> Result<Vec<CounterEvent>, Self::Error> {
        let new_value = state.value + command.delta;
        if new_value < 0 {
            return Err(CounterError::NegativeNotAllowed);
        }
        Ok(vec![CounterEvent { delta: command.delta }])
    }

    async fn apply_event(
        state: &CounterState,
        event: CounterEvent,
    ) -> CounterState {
        CounterState {
            value: state.value + event.delta,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum CounterError {
    #[error("counter cannot go negative")]
    NegativeNotAllowed,
}
```

**Expected**: Compiles. `Counter` implements `PersistentEntity<CounterCommand, CounterEvent, CounterState>`.

**Run**: `cargo test --package ego-persistent-entity --test entity_lifecycle counter_entity_definition`

---

### Scenario 2: Send Command, Verify Result

```rust
#[tokio::test]
async fn test_increment_counter() {
    // Arrange
    let event_store = InMemoryEventStore::new();
    let snapshot_store = InMemorySnapshotStore::new();
    let runtime = EntityRuntimeBuilder::new()
        .with_event_store(event_store)
        .with_snapshot_store(snapshot_store)
        .build();

    // Act
    let entity_ref = runtime.entity_ref::<CounterCommand, CounterEvent, CounterState>(
        TenantId("".into()),
        "counter",
        EntityId("counter-1".into()),
    );
    let result = entity_ref
        .send(
            CounterCommand { delta: 5 },
            CommandContext::for_test(),
            None,
        )
        .await
        .unwrap();

    // Assert
    match result {
        CommandResult::Events { events, new_state, new_version } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].delta, 5);
            assert_eq!(new_state.value, 5);
            assert_eq!(new_version, 1);
        }
        _ => panic!("expected Events variant"),
    }
}
```

**Expected**: Command succeeds, version advances from 0 to 1, state reflects the applied event.

**Run**: `cargo test --package ego-persistent-entity --test entity_lifecycle test_increment_counter`

---

### Scenario 3: Recovery After Restart

```rust
#[tokio::test]
async fn test_recovery_after_restart() {
    // Arrange: persist events with one runtime instance
    let event_store = Arc::new(InMemoryEventStore::new());
    let snapshot_store = Arc::new(InMemorySnapshotStore::new());

    {
        let runtime = EntityRuntimeBuilder::new()
            .with_event_store(event_store.clone())
            .with_snapshot_store(snapshot_store.clone())
            .build();

        let entity_ref = runtime.entity_ref::<CounterCommand, CounterEvent, CounterState>(
            TenantId("".into()),
            "counter",
            EntityId("counter-1".into()),
        );
        entity_ref
            .send(CounterCommand { delta: 10 }, CommandContext::for_test(), None)
            .await
            .unwrap();
        entity_ref
            .send(CounterCommand { delta: 5 }, CommandContext::for_test(), Some(1))
            .await
            .unwrap();
    }
    // Runtime drops — simulates restart

    // Act: new runtime loads from same stores
    let runtime = EntityRuntimeBuilder::new()
        .with_event_store(event_store.clone())
        .with_snapshot_store(snapshot_store.clone())
        .build();

    let entity_ref = runtime.entity_ref::<CounterCommand, CounterEvent, CounterState>(
        TenantId("".into()),
        "counter",
        EntityId("counter-1".into()),
    );
    let result = entity_ref
        .send(CounterCommand { delta: 1 }, CommandContext::for_test(), Some(2))
        .await
        .unwrap();

    // Assert: state reflects all prior events (10 + 5 = 15) plus new command (1)
    match result {
        CommandResult::Events { new_state, new_version, .. } => {
            assert_eq!(new_state.value, 16);
            assert_eq!(new_version, 3);
        }
        _ => panic!("expected Events variant"),
    }
}
```

**Expected**: After "restart", entity state is correctly reconstructed from persisted events. Version 3 means 3 events total.

**Run**: `cargo test --package ego-persistent-entity --test recovery test_recovery_after_restart`

---

### Scenario 4: Version Conflict

```rust
#[tokio::test]
async fn test_version_conflict() {
    // Arrange
    let runtime = EntityRuntimeBuilder::new()
        .with_event_store(InMemoryEventStore::new())
        .with_snapshot_store(InMemorySnapshotStore::new())
        .build();

    let entity_ref = runtime.entity_ref::<CounterCommand, CounterEvent, CounterState>(
        TenantId("".into()),
        "counter",
        EntityId("counter-1".into()),
    );

    // Act: first command succeeds at version 0 → 1
    entity_ref
        .send(CounterCommand { delta: 1 }, CommandContext::for_test(), Some(0))
        .await
        .unwrap();

    // Send with stale expected version
    let result = entity_ref
        .send(CounterCommand { delta: 1 }, CommandContext::for_test(), Some(0))
        .await;

    // Assert
    match result {
        Err(EntityError::VersionConflict { expected: 0, current: 1 }) => {}
        _ => panic!("expected VersionConflict"),
    }
}
```

**Expected**: Second command with stale version `0` receives `VersionConflict { expected: 0, current: 1 }`.

**Run**: `cargo test --package ego-persistent-entity --test version_conflict test_version_conflict`

---

### Scenario 5: Passivation and Reactivation

```rust
#[tokio::test]
async fn test_passivation_reactivation() {
    // Arrange
    let event_store = InMemoryEventStore::new();
    let snapshot_store = InMemorySnapshotStore::new();
    let runtime = EntityRuntimeBuilder::new()
        .with_event_store(event_store)
        .with_snapshot_store(snapshot_store)
        .passivation_timeout(Duration::from_millis(100))
        .build();

    let entity_ref = runtime.entity_ref::<CounterCommand, CounterEvent, CounterState>(
        TenantId("".into()),
        "counter",
        EntityId("counter-1".into()),
    );

    // Act: send command to active entity
    entity_ref
        .send(CounterCommand { delta: 5 }, CommandContext::for_test(), None)
        .await
        .unwrap();

    // Wait for passivation timeout
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Send command — triggers auto-reactivation
    let result = entity_ref
        .send(CounterCommand { delta: 3 }, CommandContext::for_test(), Some(1))
        .await
        .unwrap();

    // Assert
    match result {
        CommandResult::Events { new_state, new_version, .. } => {
            assert_eq!(new_state.value, 8);
            assert_eq!(new_version, 2);
        }
        _ => panic!("expected Events variant"),
    }
}
```

**Expected**: After passivation timeout, entity reactivates transparently. State is correct (5 + 3 = 8).

**Run**: `cargo test --package ego-persistent-entity --test passivation test_passivation_reactivation`

---

### Scenario 6: Zero-Event Query

```rust
#[tokio::test]
async fn test_zero_event_query() {
    // Arrange
    let event_store = InMemoryEventStore::new();
    let snapshot_store = InMemorySnapshotStore::new();
    let runtime = EntityRuntimeBuilder::new()
        .with_event_store(event_store)
        .with_snapshot_store(snapshot_store)
        .build();

    let entity_ref = runtime.entity_ref::<CounterCommand, CounterEvent, CounterState>(
        TenantId("".into()),
        "counter",
        EntityId("counter-1".into()),
    );

    // First persist some state
    entity_ref
        .send(CounterCommand { delta: 10 }, CommandContext::for_test(), None)
        .await
        .unwrap();

    // Act: zero-event query
    let result = entity_ref
        .send(CounterCommand { delta: 0 }, CommandContext::for_test(), Some(1))
        .await
        .unwrap();

    // Assert: version unchanged, no events
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 10);
        }
        _ => panic!("expected NoEvents variant"),
    }
}
```

**Expected**: Command with delta=0 produces no events. Version stays at 1. State returned directly.

**Run**: `cargo test --package ego-persistent-entity --test entity_lifecycle test_zero_event_query`

---

## Running All Validation Tests

```bash
cargo test --package ego-persistent-entity
```

All tests MUST pass without external infrastructure. Tests use `InMemoryEventStore` and `InMemorySnapshotStore` exclusively.

## What These Scenarios Validate

| Scenario | Validates |
|----------|-----------|
| 1. Entity definition | `PersistentEntity` trait compiles and works |
| 2. Send command | Command produces events, version advances |
| 3. Recovery restart | State reconstructed from events after runtime drop |
| 4. Version conflict | Optimistic concurrency works |
| 5. Passivation/React | Passivation timeout → auto-reactivation → correct state |
| 6. Zero-event query | No version change, no events, state returned |
