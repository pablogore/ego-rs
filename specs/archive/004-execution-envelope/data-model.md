# Data Model: Execution Envelope

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Research**: [research.md](research.md)

## Overview

The Execution Envelope data model defines the carrier that transports payload, identity, correlation, and metadata into the runtime. The envelope lives in `ego-domain` and is runtime-neutral. ExecutionContext is constructed from the envelope.

## Entities

### ExecutionEnvelope\<P\> (struct)

A transport-neutral carrier for execution input.

| Field | Type | Description |
|-------|------|-------------|
| `payload` | `P` | The input message. Always present — payload-less execution models use `ExecutionEnvelope<()>` where `()` is Rust's zero-sized type. |
| `aggregate_id` | `Option<AggregateId>` | Aggregate identity, if available |
| `entity_id` | `Option<EntityId>` | Entity identity, if available |
| `tenant_id` | `Option<TenantId>` | Tenant identity, if available |
| `correlation_id` | `Option<CorrelationId>` | Correlation identifier, if available |
| `causation_id` | `Option<CausationId>` | Causation identifier, if available |
| `request_id` | `Option<RequestId>` | Request identifier, if available |
| `metadata` | `Metadata` | Arbitrary key-value metadata (type alias for `HashMap<String, String>`) |

**Validation**: No validation beyond the identity/correlation type invariants from 002 (non-empty strings). The envelope itself is always constructable.

**State Transitions**: None. The envelope is immutable after construction.

**Derives**: Debug, Clone, PartialEq, Eq, Serialize, Deserialize.

### ExecutionContext Construction

`ExecutionContext` is a trait — it cannot directly implement `From`. Instead, conversion is owned by concrete implementations:

**Domain-owned (infallible, no runtime deps):**

```rust
impl<P> From<ExecutionEnvelope<P>> for DomainExecutionContext {
    fn from(envelope: ExecutionEnvelope<P>) -> Self {
        Self {
            aggregate_id: envelope.aggregate_id,
            entity_id: envelope.entity_id,
            tenant_id: envelope.tenant_id,
            correlation_id: envelope.correlation_id,
            causation_id: envelope.causation_id,
            request_id: envelope.request_id,
            metadata: envelope.metadata,
        }
    }
}
```

**Runtime-owned (named constructor):**

```rust
impl RuntimeExecutionContext {
    pub fn from_envelope<P>(envelope: ExecutionEnvelope<P>) -> Self { ... }
}
```

### Identity and Correlation Types

All types are reused from 002-execution-context — no new definitions:

| Type | Defined In | Wraps |
|------|-----------|-------|
| `AggregateId` | `ego-domain::context` | `String` |
| `EntityId` | `ego-domain::context` | `String` |
| `TenantId` | `ego-domain::context` | `String` |
| `CorrelationId` | `ego-domain::context` | `String` |
| `CausationId` | `ego-domain::context` | `String` |
| `RequestId` | `ego-domain::context` | `String` |
| `Metadata` | `ego-domain::context` | `HashMap<String, String>` |

### Relationship Diagram

```text
Transport Layer
    │
    ▼ constructs
ExecutionEnvelope<P>
    │
    ├── payload: P
    ├── aggregate_id: Option<AggregateId>
    ├── entity_id: Option<EntityId>
    ├── tenant_id: Option<TenantId>
    ├── correlation_id: Option<CorrelationId>
    ├── causation_id: Option<CausationId>
    ├── request_id: Option<RequestId>
    └── metadata: Metadata
    │
    ▼ constructs
DomainExecutionContext (002 concrete impl of ExecutionContext trait)
    │
    ▼ passed to handler as &dyn ExecutionContext
ExecutionHandler
    │
    ▼ returns
Effect<E, R, S> (003 type)
```

## Cross-References

- **Spec Requirements**: FR-001 through FR-009
- **002 Types**: AggregateId, EntityId, TenantId, CorrelationId, CausationId, RequestId, Metadata
- **Research Decisions**: AD-001 (struct), AD-002 (generic payload), AD-003 (From trait), AD-004 (domain ownership)
