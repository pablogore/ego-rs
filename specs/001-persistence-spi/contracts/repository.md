# Contract: Repository

## Trait Contract

```rust
pub trait Repository<A> {
    fn save(
        &mut self,
        aggregate: A,
        tenant_id: Option<&str>,
        expected_version: i64,
    ) -> Result<i64, PersistenceError>;

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<A, PersistenceError>;

    fn delete(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), PersistenceError>;
}
```

## Behavioral Contract

See [Contract Invariants](../spec.md#contract-invariants) in spec.md for full behavioral guarantees.

### Critical Invariants

- `save` is atomic (all-or-nothing)
- `expected_version` enforces optimistic concurrency
- `delete` makes aggregate inaccessible to subsequent `load` calls
- `tenant_id = None` = single-tenant mode
- correlation_id is NOT a Repository concern. Repository operations are correlation_id-agnostic. Correlation_id is exclusively owned by the EventStore.
