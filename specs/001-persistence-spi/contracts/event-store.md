# Contract: EventStore

## StoredEvent (correlation_id wrapper)

```rust
/// A domain event with optional correlation metadata.
/// The correlation_id ties this event to the command that produced it.
pub struct StoredEvent<E> {
    pub event: E,
    pub correlation_id: Option<String>,
}
```

## Trait Contract

```rust
pub trait EventStore<E: DomainEvent> {
    fn append(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError>;

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError>;

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
- Each event's `correlation_id` is preserved through append and load without modification or truncation
- Events appended without `correlation_id` (None) are returned with `correlation_id: None` (backward compatible)
- correlation_id flows from CommandContext through append to load without modification or regeneration
- correlation_id is bound to command identity, not execution attempt — retries of the same command carry identical correlation_id

### Correlation Ownership

correlation_id is exclusively owned by the EventStore. Repository and Snapshot contracts do not participate in the correlation lifecycle.

### Correlation ID Semantic Boundaries

The following negative semantics define what correlation_id is NOT:

- **NOT a security token**: correlation_id is opaque traceability metadata. It MUST NOT be used for authentication, authorization, session management, or any security-sensitive decision. No cryptographic or entropy guarantees.
- **NOT required for correctness**: Persistence operations succeed regardless of correlation_id presence. `correlation_id = None` is always valid and does not affect operation success.
- **NOT used for ordering**: Event ordering is determined by append sequence (stream version). correlation_id values MUST NOT influence event order.
- **NOT used for deduplication**: correlation_id is not an idempotency key. Multiple distinct events MAY share the same correlation_id. No event SHALL be suppressed based on correlation_id.
