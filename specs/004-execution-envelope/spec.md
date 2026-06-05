# Feature Specification: Execution Envelope

**Feature Branch**: `004-execution-envelope`

**Created**: 2026-06-04

**Status**: Draft

**Input**: User specification: "Introduce a canonical ExecutionEnvelope abstraction responsible for transporting execution metadata into the runtime."

## Clarifications

### Session 2026-06-04

- Q: Should CORE-005's offset be per-stream or global? → A: Per-stream — offset is a per-stream_key monotonic counter. event_id = deduplication identity; offset = read-side progress tracking. They are NOT interchangeable.
- Q: How should poison events (corrupted payloads) be handled? → A: DLQ + log — poison events MUST be routed to a dead-letter queue AND logged with full envelope metadata for forensic analysis. Poison events MUST NOT break stream processing.
- Q: Should stream_key be a derived property or a first-class field? → A: Derived — stream_key is computed as `tenant_id + ":" + aggregate_id`. It is NOT stored as a field on the envelope. Ordering is guaranteed ONLY within stream_key; no global ordering exists.
- Q: What is the source of logical timestamp — producer-supplied or clock service? → A: Producer-supplied — the event producer provides the logical timestamp at envelope construction time. Timestamp is for observability/debugging only and MUST NOT be used for ordering.
- Q: How should deduplication of event_id be scoped — per-consumer or global? → A: Per-consumer — each consumer tracks its own seen event_ids independently. Downstream systems MUST deduplicate using event_id to prevent duplicate side effects during replay.
- Q: How should the runtime handle version gaps (non-monotonic or missing versions)? → A: Reject — the runtime MUST reject the envelope and halt stream processing on any version gap. Gaps indicate missing or corrupted events and must not be silently skipped.
- Q: What is the event_type value format and taxonomy? → A: Option B — two-level taxonomy `domain::event_name` format with a registry (e.g., `"orders::OrderCreated"`). Provides namespace isolation and human-readable discriminators for CORE-005 projection routing.
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
- What happens when a version gap is detected? The envelope is rejected and stream processing halts.
- What happens when a payload is corrupted (poison event)? The event is routed to a dead-letter queue and logged with full envelope metadata. Stream processing continues.
- What happens when a duplicate event_id is received? The consumer's dedup check rejects the duplicate; no side effects are produced.

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
- **FR-010**: `event_type` MUST use a two-level taxonomy `domain::event_name` format (e.g., `"orders::OrderCreated"`) with a central registry for canonical event type registration.
- **FR-011**: `version` MUST be strictly monotonic per aggregate. Any version gap (non-monotonic or missing intermediate versions) MUST cause the envelope to be rejected and stream processing to halt.
- **FR-012**: `event_id` MUST be globally unique (UUID). Deduplication is per-consumer — each consumer tracks its own seen event_ids independently. Downstream systems MUST deduplicate using event_id to prevent duplicate side effects during replay.
- **FR-013**: `timestamp` is producer-supplied logical time (i64), NOT wall-clock. Timestamp MUST NOT be used for ordering. It MAY be used for observability and debugging only.
- **FR-014**: `stream_key` is a derived property computed as `tenant_id + ":" + aggregate_id`. It is NOT a stored field. stream_key defines the partitioning boundary; ordering is guaranteed ONLY within stream_key. No global ordering exists.
- **FR-015**: Invalid envelopes MUST be rejected. Corrupted payloads (poison events) MUST be routed to a dead-letter queue AND logged with full envelope metadata. Poison events MUST NOT break stream processing.
- **FR-016**: CORE-005 consumes ExecutionEnvelope as an immutable event stream. CORE-005 relies on stream_key ordering, uses event_id for deduplication, and uses version for ordering validation only. CORE-005's offset is a per-stream_key monotonic counter. event_id (dedup identity) and offset (read-side progress tracking) are NOT interchangeable.
- **FR-017**: Payload is opaque but schema-stable. Payload MUST support backward compatibility — payload evolution MUST NOT break consumers. Unknown fields in the payload MUST be ignored safely during deserialization. Deserialization MUST be forward-compatible; schema evolution MUST NOT break replay.

### Key Entities

- **ExecutionEnvelope\<P\>**: A transport-neutral carrier for payload, identity, correlation, and metadata. Generic over payload type P. Fields: `event_id` (UUID, globally unique dedup key), `aggregate_id` (String, partition key), `tenant_id` (String, mandatory multi-tenant boundary), `event_type` (String, two-level taxonomy `domain::event_name` with registry), `version` (i64, monotonic per aggregate), `timestamp` (i64, logical time only, NOT wall-clock), `payload` (P, opaque, schema-stable, backward-compatible).
- **ExecutionContext construction**: Conversion from `ExecutionEnvelope<P>` to `DomainExecutionContext` via `From` trait (domain-owned concrete type). Runtime implementations may also offer their own conversion (e.g. `RuntimeExecutionContext::from_envelope()`).
- **stream_key**: Derived property = `tenant_id + ":" + aggregate_id`. Defines partitioning boundary. Ordering guaranteed ONLY within stream_key. No global ordering exists.
- **offset (CORE-005)**: Per-stream_key monotonic counter for read-side progress tracking. NOT interchangeable with event_id.

### Invariant Rules

1. **Immutability**: ExecutionEnvelope is an immutable event structure. Once constructed, no field may be mutated.
2. **Global uniqueness**: `event_id` MUST be a globally unique UUID. Used as the deduplication key by all downstream consumers.
3. **Partition key**: `aggregate_id` defines the partition boundary within a tenant.
4. **Mandatory tenant**: `tenant_id` is mandatory — envelopes without a tenant_id are invalid.
5. **Event type taxonomy**: `event_type` uses `domain::event_name` format with a central registry.
6. **Monotonic version**: `version` is strictly monotonic per aggregate. Gaps cause rejection.
7. **Logical timestamp**: `timestamp` is producer-supplied logical time. NOT used for ordering. Observability only.
8. **Stream identity**: `stream_key = tenant_id + ":" + aggregate_id`. Derived, not stored. Ordering is per-stream only.
9. **Per-consumer dedup**: Each consumer tracks its own seen event_ids. Replay MUST NOT produce duplicate side effects.
10. **Payload stability**: Payload is opaque but schema-stable. Backward-compatible evolution only. Unknown fields ignored.
11. **Forward compatibility**: Deserialization ignores unknown fields. Schema evolution MUST NOT break replay.
12. **Failure isolation**: Invalid envelopes are rejected. Poison events go to DLQ. Stream processing continues.
13. **CORE-005 alignment**: CORE-005 consumes ExecutionEnvelope as immutable event stream. Uses stream_key for ordering, event_id for dedup, version for ordering validation only. Offset is per-stream progress tracking.

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
