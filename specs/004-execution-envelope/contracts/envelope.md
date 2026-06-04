# ExecutionEnvelope Type Contract

**Status**: Draft | **Spec**: [spec.md](../spec.md) | **Data Model**: [data-model.md](../data-model.md)

## Purpose

Domain-owned carrier transporting payload, identity, correlation, and metadata into the runtime. ExecutionContext is constructed from ExecutionEnvelope.

## Contract

```rust
use std::collections::HashMap;
use crate::context::{AggregateId, EntityId, TenantId, CorrelationId, CausationId, RequestId};

/// Transport-neutral carrier for execution input.
///
/// Carries the payload (command, event, workflow message, etc.) alongside
/// identity, correlation, and metadata from the incoming message.
///
/// # Type parameters
///
/// - `P`: Payload type — determined by the execution model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionEnvelope<P> {
    /// The input message payload.
    pub payload: P,
    /// Aggregate identity, if available.
    pub aggregate_id: Option<AggregateId>,
    /// Entity identity, if available.
    pub entity_id: Option<EntityId>,
    /// Tenant identity, if available.
    pub tenant_id: Option<TenantId>,
    /// Correlation identifier, if available.
    pub correlation_id: Option<CorrelationId>,
    /// Causation identifier, if available.
    pub causation_id: Option<CausationId>,
    /// Request identifier, if available.
    pub request_id: Option<RequestId>,
    /// Arbitrary key-value metadata.
    pub metadata: HashMap<String, String>,
}
```

## Implementer Requirements

1. **Transport implementations** MUST:
   - Construct `ExecutionEnvelope<P>` from incoming messages
   - Populate identity, correlation, and metadata fields when available
   - Leave absent fields as `None`

2. **Runtime implementations** MUST:
   - Accept `ExecutionEnvelope<P>` for ExecutionContext construction
   - Map envelope fields to ExecutionContext accessors

3. **Entities (execution models)**:
   - Choose payload type `P` appropriate to the model
   - Construct envelopes in tests without transport infrastructure

## Derives

ExecutionEnvelope MUST derive: `Debug, Clone, PartialEq, Eq`

## Testability

Envelope construction and context conversion are testable by:
- Constructing envelopes directly in tests (no transport)
- Asserting field values after construction
- Constructing ExecutionContext from envelope and asserting context accessors
