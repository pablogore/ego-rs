# Feature Specification: Execution Envelope

**Feature Branch**: `004-execution-envelope`

**Created**: 2026-06-04

**Status**: Draft

**Input**: User specification: "Introduce a canonical ExecutionEnvelope abstraction responsible for transporting execution metadata into the runtime."

## Clarifications

### Session 2026-06-04

- Q: Who owns conversion from `ExecutionEnvelope<P>` to an `ExecutionContext` implementation? → A: `DomainExecutionContext` (Option C) — a domain-owned concrete type in `ego-domain` that implements `From<ExecutionEnvelope<P>>`. Runtime implementations provide their own named constructors (e.g. `RuntimeExecutionContext::from_envelope()`).
- Q: Should `payload` be mandatory (`payload: P`), optional (`payload: Option<P>`), or mandatory with `()` escape hatch (`payload: P`, payload-less models use `ExecutionEnvelope<()>`)? → A: Option C — `payload: P` is mandatory; payload-less execution models use `ExecutionEnvelope<()>`. `()` is Rust's idiomatic zero-sized type for "no data," preserving the strong contract while avoiding Option branching on every access.
- Q: Should ExecutionEnvelope derive `Serialize + Deserialize`? → A: Option B — derive serde's `Serialize + Deserialize`. `serde` is a format-agnostic serialization framework, not a wire format. The transport layer still owns the format decision (JSON, MessagePack, protobuf, etc.), but the envelope owns its serde trait impls for consistent round-trip behavior across all transports.
- Q: What is the canonical runtime context name — `RuntimeExecutionContext` or `CommandContext`? → A: `RuntimeExecutionContext` at `crates/runtime/src/context.rs:12`. `CommandContext` does not exist in the codebase and was a rejected alternative (spec §Architectural Decision). All documentation uses `RuntimeExecutionContext` consistently except the now-resolved checklist ambiguity CHK041.

### Architectural Decision: Conversion Ownership

**Decision**: `DomainExecutionContext` owns `From<ExecutionEnvelope<P>>` conversion.

**Rationale**: `ExecutionContext` is a trait — it cannot directly implement `From` or be constructed via `ExecutionContext::from()`. The domain crate already defines `DomainExecutionContext` (concrete struct implementing `ExecutionContext`) with `impl<P> From<ExecutionEnvelope<P>> for DomainExecutionContext` at `crates/domain/src/context.rs:149`. This satisfies all evaluation criteria:

| Criterion | Assessment |
|-----------|-----------|
| Consistency with 002 | ✅ `DomainExecutionContext` is the canonical domain-owned impl from 002 |
| Runtime ownership boundaries | ✅ Conversion happens in domain crate — no runtime deps leaked |
| Domain ownership boundaries | ✅ Envelope and context types coexist in `ego-domain` |
| Future runtime portability | ✅ `From` impl is runtime-agnostic; new runtimes reuse it |
| Actor runtime compatibility | ✅ No actor/mailbox/channel types involved |
| Tokio runtime compatibility | ✅ No Tokio types in domain crate |
| Testability | ✅ Direct envelope-to-context construction in tests (already tested) |

**Rejected alternatives**:
- Option A (`RuntimeExecutionContext` implements `From`): Would couple `From` trait to a runtime type; the named `from_envelope()` method already serves this purpose without trait semantics.
- Option B (new `CommandContext` type): Does not exist in codebase; would duplicate `DomainExecutionContext`, violating the "avoid duplicate modules" rule.

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
2. **Given** serializable identity and correlation fields, **When** the envelope is serialized and deserialized via serde, **Then** it round-trips correctly with all fields preserved.

---

### Edge Cases

- What happens when identity fields are absent? The envelope carries `None`, and the constructed context returns `None`.
- What happens when correlation fields are absent? Same — optional fields are always `None`-safe.
- What happens when an execution model has no payload? Use `ExecutionEnvelope<()>` — `()` is Rust's zero-sized type for "no data." The payload field is always `P`, never `Option<P>`. Payload-less models benefit from zero runtime overhead since `()` occupies no space.

## Requirements

### Functional Requirements

- **FR-001**: The framework MUST define a canonical ExecutionEnvelope abstraction.
- **FR-002**: ExecutionEnvelope MUST carry an arbitrary payload, identity fields, correlation fields, and metadata.
- **FR-003**: ExecutionEnvelope MUST NOT restrict payload type — commands, events, workflow messages, saga messages, and projection messages are all valid.
- **FR-004**: ExecutionEnvelope MUST NOT expose actor references, mailboxes, channels, Tokio types, or transport implementations.
- **FR-005**: ExecutionEnvelope MUST support in-process, actor, cluster, HTTP, gRPC, and messaging transports without modification.
- **FR-006**: `DomainExecutionContext` (domain-owned concrete type) MUST implement `From<ExecutionEnvelope<P>>` to provide infallible conversion.
- **FR-007**: ExecutionContext constructed from ExecutionEnvelope MUST remain read-only (per 002 contract).
- **FR-008**: Identity and correlation fields on ExecutionEnvelope MUST be optional — absent fields produce `None` in the context.
- **FR-009**: ExecutionEnvelope identity and correlation types MUST reuse the types defined in 002-execution-context (AggregateId, EntityId, TenantId, CorrelationId, CausationId, RequestId, Metadata).

### Key Entities

- **ExecutionEnvelope\<P\>**: A transport-neutral carrier for payload, identity, correlation, and metadata. Generic over payload type P.
- **ExecutionContext construction**: Conversion from `ExecutionEnvelope<P>` to `DomainExecutionContext` via `From` trait (domain-owned concrete type). Runtime implementations may also offer their own conversion (e.g. `RuntimeExecutionContext::from_envelope()`).

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
- Serde trait impls (`Serialize + Deserialize`) — format-agnostic, for round-trip integrity across all transports

DomainExecutionContext **owns**:
- `From<ExecutionEnvelope<P>>` conversion (infallible, domain-owned, no runtime deps)

RuntimeExecutionContext **owns**:
- `from_envelope()` named constructor (runtime-specific, delegates field mapping)

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
- Payload-less execution models use `ExecutionEnvelope<()>` — `()` is idiomatic Rust for "no data," zero-sized, no runtime overhead.
- Payload is never `Option<P>` — the envelope contract requires `payload: P` always.
- Identity and correlation types are the same newtypes defined in 002 — no new identity types.
- Envelope fields are optional — the framework does not require all fields to be present.
- Envelope construction may involve serialization/deserialization at transport boundaries, but the domain type remains the same.
- ExecutionEnvelope derives serde `Serialize + Deserialize` — this is a format-agnostic framework; the transport layer chooses the specific format (JSON, MessagePack, protobuf, etc.).
- Adding serde as a dependency to `ego-domain` is acceptable — serde is the Rust ecosystem standard for serialization trait definitions, not a wire format.
- Conversion ownership: `DomainExecutionContext` (domain crate) implements `From<ExecutionEnvelope<P>>`. Runtime implementations (e.g. `RuntimeExecutionContext`) provide their own envelope-to-context conversion via named constructors like `from_envelope()`.
- The existing `crates/runtime/src/context.rs` RuntimeExecutionContext provides `from_envelope()` to accept ExecutionEnvelope as input.
