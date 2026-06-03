# Contract: PersistenceError

## Enum Contract

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceError {
    NotFound { aggregate_id: String },
    Conflict { aggregate_id: String, expected: i64, actual: i64 },
    MissingTenant,
    Internal(String),
}
```

## Variant Semantics

| Variant | When Raised | Recoverable |
|---------|-------------|-------------|
| `NotFound` | Aggregate or stream does not exist | Yes (caller can create) |
| `Conflict` | Optimistic concurrency violation — `expected_version` != current | Yes (retry with fresh version) |
| `MissingTenant` | Empty/blank `tenant_id` in multi-tenant mode | Yes (provide valid tenant) |
| `Internal` | Infrastructure failures (connection, serialization, etc.) | Depends on root cause |
