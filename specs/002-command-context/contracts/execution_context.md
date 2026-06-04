# ExecutionContext Trait Contract

**Status**: Revised | **Spec**: [spec.md](../spec.md) | **Data Model**: [data-model.md](../data-model.md)

## Purpose

Domain-owned contract providing execution participants with read-only access to execution context: identity, correlation, and metadata. No side effects, no service locator.

## Contract

```rust
/// Read-only execution context for all execution models
/// (commands, events, workflows, sagas, projections, process managers).
///
/// Carries identity, correlation, and metadata from the incoming
/// message — nothing else. Side-effect capabilities (persist, reply,
/// schedule) belong to separate abstractions (Effect API, Scheduling API).
///
/// Implementations are provided by runtime crates. The trait is
/// domain-owned and runtime-neutral.
pub trait ExecutionContext {
    /// The aggregate identity, if available in the execution scope.
    fn aggregate_id(&self) -> Option<&AggregateId>;

    /// The entity identity, if available in the execution scope.
    fn entity_id(&self) -> Option<&EntityId>;

    /// The tenant identity, if available in the execution scope.
    fn tenant_id(&self) -> Option<&TenantId>;

    /// The correlation identifier, if available.
    fn correlation_id(&self) -> Option<&CorrelationId>;

    /// The causation identifier, if available.
    fn causation_id(&self) -> Option<&CausationId>;

    /// The request identifier, if available.
    fn request_id(&self) -> Option<&RequestId>;

    /// Read-only access to request metadata (arbitrary key/value pairs).
    fn metadata(&self) -> &Metadata;
}
```

## Implementer Requirements

1. **Runtime implementations** MUST:
   - Inject identity, correlation, and metadata from the incoming message/envelope
   - Implement all accessor methods — no default implementations

2. **Implementations MUST NOT**:
   - Expose Tokio types, actor references, mailboxes, or channels in their public API
   - Require handlers to import runtime-specific types to use the context
   - Add side-effect methods (persist, reply, schedule) to the trait

## Associated Types

| Type | Location | Description |
|------|----------|-------------|
| `AggregateId` | `ego-domain::context` | Non-empty string newtype for aggregate identity |
| `EntityId` | `ego-domain::context` | Non-empty string newtype for entity identity |
| `TenantId` | `ego-domain::context` | Non-empty string newtype for tenant identity |
| `CorrelationId` | `ego-domain::context` | Non-empty string newtype for correlation tracing |
| `CausationId` | `ego-domain::context` | Non-empty string newtype for causation tracing |
| `RequestId` | `ego-domain::context` | Non-empty string newtype for request tracing |
| `Metadata` | `ego-domain::context` | `HashMap<String, String>` |

## Ownership Boundaries

ExecutionContext **owns**: identity, correlation, metadata.
ExecutionContext does **NOT own**: persistence, replies, scheduling, observability, transport, runtime execution.

## Testability

Contract tests verify that implementations:
- Return identity fields correctly (match input → output)
- Return `None` for absent fields (no crash)
- Return metadata correctly (populated and empty cases)

No persistence, reply, scheduling, or observability infrastructure is needed.
