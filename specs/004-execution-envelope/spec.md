# Feature Specification: Execution Envelope

**Feature Branch**: `007-execution-envelope`

**Created**: 2026-06-04

**Status**: Draft

**Input**: User specification: "Introduce a canonical ExecutionEnvelope abstraction responsible for transporting execution metadata into the runtime."

## Problem

ExecutionContext (002) exposes aggregate_id, tenant_id, correlation_id, and metadata, but the architecture does not yet define how this information enters the runtime. Different runtimes currently risk inventing different envelope models. The existing `crates/runtime/src/context.rs` only carries correlation_id and has no standard mechanism for payload, identity, or metadata transport.

## User Scenarios & Testing

### User Story 1 - Construct context from envelope (Priority: P1)

A runtime implementation needs to construct an ExecutionContext from an incoming envelope, ensuring identity, correlation, and metadata are available to execution handlers.

**Why this priority**: Every runtime needs a standard way to create ExecutionContext — without it, runtime implementations diverge.

**Independent Test**: Construct an ExecutionEnvelope with known fields, produce an ExecutionContext, assert all fields match.

**Acceptance Scenarios**:

1. **Given** an envelope with identity fields set, **When** ExecutionContext is constructed, **Then** the context exposes the same identity values.
2. **Given** an envelope with correlation fields set, **When** ExecutionContext is constructed, **Then** the context exposes the same correlation values.
3. **Given** an envelope with metadata, **When** ExecutionContext is constructed, **Then** the context exposes the same metadata.

---

### User Story 2 - Carry arbitrary payloads (Priority: P1)

An envelope must carry any payload type (command, event, workflow message, saga message, projection message) without assuming the payload type.

**Why this priority**: The envelope is execution-model agnostic by design.

**Independent Test**: Construct envelopes with different payload types, verify the payload is preserved.

**Acceptance Scenarios**:

1. **Given** an envelope with a command payload, **When** the envelope is constructed, **Then** the payload is accessible without type knowledge.
2. **Given** an envelope with an event payload, **When** the envelope is constructed, **Then** the payload type differs but the envelope structure is identical.

---

### User Story 3 - Transport independence (Priority: P2)

The same envelope type must work across in-process, actor, cluster, HTTP, gRPC, and messaging transports without modification.

**Why this priority**: Envelope is a domain concept — it must not carry transport-specific types.

**Independent Test**: Construct an envelope in a test without any transport infrastructure.

**Acceptance Scenarios**:

1. **Given** an envelope constructed in a unit test, **When** no transport is involved, **Then** the envelope is fully functional.
2. **Given** serializable identity and correlation fields, **When** the envelope is serialized, **Then** it round-trips correctly.

---

### Edge Cases

- What happens when identity fields are absent? The envelope carries `None`, and the constructed context returns `None`.
- What happens when correlation fields are absent? Same — optional fields are always `None`-safe.
- What happens when payload is absent? The envelope still carries identity/correlation/metadata — payload is optional per execution model.

## Requirements

### Functional Requirements

- **FR-001**: The framework MUST define a canonical ExecutionEnvelope abstraction.
- **FR-002**: ExecutionEnvelope MUST carry an arbitrary payload, identity fields, correlation fields, and metadata.
- **FR-003**: ExecutionEnvelope MUST NOT restrict payload type — commands, events, workflow messages, saga messages, and projection messages are all valid.
- **FR-004**: ExecutionEnvelope MUST NOT expose actor references, mailboxes, channels, Tokio types, or transport implementations.
- **FR-005**: ExecutionEnvelope MUST support in-process, actor, cluster, HTTP, gRPC, and messaging transports without modification.
- **FR-006**: ExecutionContext MUST be constructable from ExecutionEnvelope.
- **FR-007**: ExecutionContext constructed from ExecutionEnvelope MUST remain read-only (per 002 contract).
- **FR-008**: Identity and correlation fields on ExecutionEnvelope MUST be optional — absent fields produce `None` in the context.
- **FR-009**: ExecutionEnvelope identity and correlation types MUST reuse the types defined in 002-execution-context (AggregateId, EntityId, TenantId, CorrelationId, CausationId, RequestId, Metadata).

### Key Entities

- **ExecutionEnvelope\<P\>**: A transport-neutral carrier for payload, identity, correlation, and metadata. Generic over payload type P.
- **ExecutionContext construction**: `ExecutionContext::from(envelope)` or equivalent conversion from envelope to context.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Runtime implementations construct ExecutionContext from ExecutionEnvelope, not from ad-hoc transport types.
- **SC-002**: The same ExecutionEnvelope type works across all execution models without modification.
- **SC-003**: Identity, correlation, and metadata flow through the system unchanged from envelope to context to handler.
- **SC-004**: No runtime implementation defines its own envelope structure.

## Ownership Boundaries

ExecutionEnvelope **owns**:
- Payload carrier (generic, typed)
- Identity field slots (reusing 002 types)
- Correlation field slots (reusing 002 types)
- Metadata slot (reusing 002 types)
- Context construction from envelope

ExecutionEnvelope does **NOT own**:
- ExecutionContext — defined in 002
- Effect interpretation — defined in 003
- Transport adaptation — owned by runtime implementations
- Serialization format — owned by transport layer

## Future Architecture

```
Transport Layer (HTTP, gRPC, messaging, in-process)
    │
    ▼ deserializes into
ExecutionEnvelope<P>
    │
    ├── payload (the input message)
    ├── aggregate_id?, entity_id?, tenant_id?
    ├── correlation_id?, causation_id?, request_id?
    └── metadata
    │
    ▼ constructs
ExecutionContext (002) → provided to handler → returns Effect (003)
```

## Assumptions

- ExecutionEnvelope is a struct (or generic struct), not a trait — the canonical carrier is a concrete type.
- Payload type P is determined at construction time — envelopes are typed.
- Identity and correlation types are the same newtypes defined in 002 — no new identity types.
- Envelope fields are optional — the framework does not require all fields to be present.
- Envelope construction may involve serialization/deserialization at transport boundaries, but the domain type remains the same.
- The existing `crates/runtime/src/context.rs` CommandContext struct is refactored to accept ExecutionEnvelope as input.
