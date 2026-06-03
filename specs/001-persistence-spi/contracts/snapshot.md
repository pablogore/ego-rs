# Contract: Snapshot

## Trait Contract

```rust
pub trait Snapshot {
    fn save_snapshot(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: i64,
        payload: serde_json::Value,
    ) -> Result<(), PersistenceError>;

    fn load_snapshot(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError>;
}
```

## Behavioral Contract

See [Contract Invariants](../spec.md#contract-invariants) in spec.md for full behavioral guarantees.

### Critical Invariants

- `load_snapshot` returns the highest version snapshot or `None`
- `save_snapshot` version tracks the aggregate version
- No snapshot exists → `Ok(None)`, never an error
- Snapshot does not carry correlation_id — trace continuity is maintained by EventStore delta replay
- Snapshot SHALL NOT define, store, or require correlation_id. Correlation_id is an Event-only concept.
- correlation_id is NOT a Snapshot concern. Snapshot operations are correlation_id-agnostic. Correlation_id is exclusively owned by the EventStore.
