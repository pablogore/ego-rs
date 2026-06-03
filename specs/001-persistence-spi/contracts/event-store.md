# Contract: EventStore

## Trait Contract

```rust
pub trait EventStore<E: DomainEvent> {
    fn append(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<E>,
    ) -> Result<i64, PersistenceError>;

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<E>, PersistenceError>;

    fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<String>, PersistenceError>;
}
```

## Behavioral Contract

See [Contract Invariants](../spec.md#contract-invariants) in spec.md for full behavioral guarantees.

### Critical Invariants

- Events are returned in append order
- `append` is atomic (all-or-nothing)
- `expected_version` enforces optimistic concurrency
- `tenant_id = None` = single-tenant mode (no isolation)
- `tenant_id = Some("")` in multi-tenant mode = `PersistenceError::MissingTenant`
