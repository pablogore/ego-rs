# Contract: SPI Traits

**Files**: `crates/persistent-entity/src/` (EventPublisher, SnapshotStrategy)

## Purpose

Service Provider Interface traits that plug into the entity runtime. Users implement these to customize behavior without modifying the runtime.

---

## `EventPublisher`

```rust
/// Publishes committed events to downstream consumers.
/// Called AFTER event commit. Failure is logged, NOT propagated.
#[async_trait]
pub trait EventPublisher<E>: Send + Sync {
    async fn publish(&self, events: &[StoredEvent<E>]);
}
```

### Contract Rules

- Called with the list of events that were just committed.
- MUST NOT block entity command execution (publisher failure is logged only).
- MAY retry internally (implementation-defined).
- MUST be idempotent: given the same events, multiple calls produce the same outcome downstream.
- Implementation options: in-memory channel, outbox table, Kafka producer, NATS, etc.

### `StoredEvent<E>` (from `ego-domain`)

```rust
pub struct StoredEvent<E> {
    pub seq: u64,
    pub event: E,
    pub metadata: EventMetadata,
    pub timestamp: DateTime<Utc>,
    pub entity_type: String,
    pub entity_id: String,
    pub tenant_id: String,
}
```

---

## `SnapshotStrategy`

```rust
/// Determines when snapshots should be taken.
pub trait SnapshotStrategy: Send + Sync {
    /// Return true if a snapshot should be taken after persisting
    /// an event at the given stream version.
    fn should_snapshot(&self, new_version: u64) -> bool;
}
```

### Built-in Strategies

| Strategy | Behavior |
|----------|----------|
| `Never` | Always returns false. No snapshots taken. |
| `EveryN(u64)` | Returns true if `new_version % n == 0`. |
| `Custom` | User-defined closure or struct implementing the trait. |

### Default

`EveryN(100)` — snapshot every 100 events.

---

## SPIs vs `ego-domain` Traits

The entity runtime reuses `EventStore`, `Snapshot`, and `Repository` traits from `ego-domain`. The SPIs defined here (`EventPublisher`, `SnapshotStrategy`) are NEW contracts specific to the entity runtime. They bridge the entity runtime to external concerns (event consumption, snapshot policy) that are outside the core persistence layer.
