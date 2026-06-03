# Quickstart: Persistence SPI

## Prerequisites

- Rust toolchain (latest stable)
- Workspace built: `cargo build` from repo root

## Validation Scenarios

### 1. Verify SPI trait compilation

```bash
cargo check -p ego-domain
```

**Expected**: Compiles without errors. The `PersistenceError` enum and `EventStore`, `Repository`, `Snapshot` traits are present in the domain crate.

### 2. InMemory backend contract tests

```bash
cargo test -p ego-infrastructure
```

**Expected**: All contract tests pass for `InMemoryEventStore`, `InMemoryRepository`, `InMemorySnapshotStore`. Tests cover:
- Append/load round-trip (single-tenant)
- Append/load round-trip (multi-tenant isolation)
- Optimistic concurrency conflict detection
- `PersistenceError::NotFound` on missing aggregate
- `PersistenceError::MissingTenant` on empty tenant
- Snapshot save/load latest
- Delete and verify inaccessible

### 3. PostgreSQL backend contract tests

```bash
# Requires running PostgreSQL instance with DATABASE_URL configured
cargo test -p ego-infrastructure -- --ignored
```

**Expected**: Same contract tests as InMemory, passing against a PostgreSQL database.

### 4. Correlation ID propagation

```rust
// Verify correlation_id is preserved through append and load
let mut store = InMemoryEventStore::new();

// Event with correlation_id
let event_with_cid = StoredEvent {
    event: TestEvent::Created("order-1".into()),
    correlation_id: Some("cmd-abc-123".into()),
};
store.append("agg-1", None, 0, vec![event_with_cid]).unwrap();
let events = store.load("agg-1", None).unwrap();
assert_eq!(events[0].correlation_id.as_deref(), Some("cmd-abc-123"));

// Event without correlation_id (backward compatible)
let event_without_cid = StoredEvent {
    event: TestEvent::Updated("new-status".into()),
    correlation_id: None,
};
store.append("agg-1", None, 1, vec![event_without_cid]).unwrap();
let events = store.load("agg-1", None).unwrap();
assert_eq!(events[1].correlation_id, None);
```

### 5. Multi-tenancy toggle verification

```rust
// In-memory test demonstrating both modes
let mut store = InMemoryEventStore::new();

// Single-tenant mode (tenant_id = None)
store.append("agg-1", None, 0, vec![event_a]).unwrap();
let events = store.load("agg-1", None).unwrap();
assert_eq!(events.len(), 1);

// Multi-tenant mode (tenant_id = Some)
store.append("agg-1", Some("tenant-1"), 0, vec![event_b]).unwrap();
let events_t1 = store.load("agg-1", Some("tenant-1")).unwrap();
let events_t2 = store.load("agg-1", Some("tenant-2")).unwrap();
assert_eq!(events_t1.len(), 1); // tenant-1 has event_b
assert_eq!(events_t2.len(), 0); // tenant-2 has nothing
```

## Contract References

| Contract | Defined In |
|----------|------------|
| EventStore | [spec.md §Contract Invariants](spec.md#contract-invariants) |
| Repository | [spec.md §Contract Invariants](spec.md#contract-invariants) |
| Snapshot | [spec.md §Contract Invariants](spec.md#contract-invariants) |
| PersistenceError | [spec.md §Functional Requirements FR-012](spec.md#functional-requirements) |
| Migration Infrastructure | Deferred to future spec |

See [plan.md](plan.md) for entity definitions, relationships, and validation rules.
