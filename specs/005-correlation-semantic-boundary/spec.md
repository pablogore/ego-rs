# Correlation Semantic Boundary — Requirements

**Spec**: Semantic clarification for correlation_id contract (spec 001, 002)
**Created**: 2026-06-03
**Status**: Draft
**Input**: User description: "Missing semantic contract boundary for correlation_id — currently defines FR-018, data model, invariants, but lacks 'what correlation_id is NOT': NOT a security token, NOT required for persistence correctness, NOT used for ordering, NOT used for deduplication."

## User Scenarios & Testing

### User Story 1 — SPI consumer understands correlation_id semantic boundaries (Priority: P1)

A framework developer implementing a persistence backend reads the correlation_id contract and knows exactly what guarantees correlation_id does and does not provide, preventing misuse (e.g., using correlation_id for security decisions, ordering, or deduplication).

**Why this priority**: Without explicit negative semantics, implementors may assign false responsibilities to correlation_id, leading to security vulnerabilities, ordering bugs, or idempotency failures.

**Independent Test**: Read the correlation_id contract and verify it explicitly states the four "NOT" boundaries: not a security token, not required for correctness, not for ordering, not for deduplication.

**Acceptance Scenarios**:

1. **Given** the correlation_id contract, **When** a developer reads it, **Then** it explicitly states that correlation_id is NOT a security token and MUST NOT be used for authentication or authorization decisions
2. **Given** the correlation_id contract, **When** a developer reads it, **Then** it explicitly states that correlation_id is NOT required for persistence correctness — events with `correlation_id = None` are valid
3. **Given** the correlation_id contract, **When** a developer reads it, **Then** it explicitly states that correlation_id is NOT used for event ordering — ordering is determined by append sequence
4. **Given** the correlation_id contract, **When** a developer reads it, **Then** it explicitly states that correlation_id is NOT used for deduplication — duplicate correlation_ids do not imply duplicate events

---

### User Story 2 — Security boundary is respected (Priority: P1)

A security reviewer examines the codebase and confirms that correlation_id is never used for authentication, authorization, or any security-sensitive decision.

**Why this priority**: Misuse of traceability identifiers as security tokens is a common anti-pattern that leads to authentication bypass vulnerabilities.

**Independent Test**: Search the codebase for any code path where correlation_id influences access control decisions — if found, it violates the contract.

**Acceptance Scenarios**:

1. **Given** a security review of the correlation_id usage, **When** all references are examined, **Then** correlation_id never appears in authentication or authorization logic
2. **Given** a persistence backend, **When** a security audit is performed, **Then** correlation_id is not used as a session identifier, API key, or access token

---

### User Story 3 — Ordering is not derived from correlation_id (Priority: P2)

An event consumer processes events from a stream and correctly relies on append order (stream version) rather than correlation_id for sequencing.

**Why this priority**: If consumers mistakenly ordered events by correlation_id, events from different commands could be interleaved incorrectly.

**Independent Test**: Append two events with the same correlation_id but in reversed append order. Load the stream and verify they are returned in append order, not correlation_id order.

**Acceptance Scenarios**:

1. **Given** two events with the same `correlation_id` appended at versions 1 and 2, **When** the stream is loaded, **Then** events are returned in append order (version 1 first, then version 2), regardless of correlation_id value
2. **Given** events with different correlation_ids, **When** loaded, **Then** ordering is by append sequence, not by correlation_id

---

### User Story 4 — Deduplication is not based on correlation_id (Priority: P2)

An event consumer sees two events with the same correlation_id and correctly treats them as distinct events (not duplicates).

**Why this priority**: If consumers treat correlation_id as a deduplication key, they may silently drop events that share a correlation_id (e.g., multiple events from the same command).

**Independent Test**: Append two different events with the same correlation_id. Load the stream and verify both events are present and distinct.

**Acceptance Scenarios**:

1. **Given** two distinct events with the same `correlation_id = "abc-123"`, **When** the stream is loaded, **Then** both events are returned — neither is suppressed as a duplicate
2. **Given** an event with `correlation_id = "abc-123"` appended twice (duplicate append), **When** loaded, **Then** both copies are returned (deduplication, if needed, is a separate concern)

---

### Edge Cases

- What if a downstream consumer uses correlation_id as a cache key? (Acceptable for trace grouping, not for deduplication)
- What if an external system sends a correlation_id that looks like a JWT or security token? (Must still be treated as opaque trace metadata)
- What if two events have the same correlation_id but different causation paths? (Distinct events — correlation_id is not a causality proof)
- What if correlation_id is empty string vs null/None? (Both are valid "no correlation" signals — neither affects ordering, correctness, or security)

## Requirements

### Functional Requirements

- **FR-001 (NOT a security token)**: The correlation_id SHALL be treated as opaque traceability metadata. It SHALL NOT be used for authentication, authorization, session management, or any security-sensitive decision. Correlation_id values SHALL NOT be assumed to have any cryptographic properties, uniqueness guarantees, or entropy requirements.
- **FR-002 (NOT required for correctness)**: Persistence operations SHALL succeed regardless of whether correlation_id is present (`Some`) or absent (`None`). A `correlation_id = None` event SHALL be appended, stored, and loaded identically to an event with a correlation_id value — the only difference is the traceability link.
- **FR-003 (NOT used for ordering)**: Event ordering SHALL be determined exclusively by append sequence (stream version). Correlation_id values SHALL NOT influence event order in any way. Two events with the same correlation_id but different versions SHALL be ordered by version, not by correlation_id.
- **FR-004 (NOT used for deduplication)**: Correlation_id SHALL NOT be treated as a deduplication key. Multiple distinct events MAY share the same correlation_id. Deduplication, if required, SHALL use an explicit mechanism (e.g., idempotency key) separate from correlation_id.
- **FR-005 (Explicit documentation)**: The correlation_id contract SHALL explicitly document both positive semantics (what correlation_id IS) and negative semantics (what correlation_id is NOT) in a single, consolidated section.

### Key Entities

- **CorrelationId**: An opaque traceability identifier. Purpose: link events to their originating command. Semantics: exactly and only that — no security, ordering, or deduplication responsibilities.
- **Stream Version**: Monotonically increasing append sequence number. The sole source of event ordering.
- **Idempotency Key**: A separate mechanism (if needed) for deduplication. Not provided by correlation_id.

## Contract Invariants

The following semantic boundaries apply to correlation_id across all SPI contracts.

### Security

- Correlation_id MUST NOT be used as an authentication or authorization mechanism.
- Correlation_id MUST NOT be treated as a secret, credential, or token.
- Correlation_id values MUST NOT be validated, signed, or encrypted by the persistence layer.
- Correlation_id MUST NOT influence access control decisions at any layer.

### Correctness

- Correlation_id presence or absence MUST NOT affect the success or failure of any persistence operation.
- All persistence operations SHALL produce identical results for `correlation_id = Some(x)` and `correlation_id = None`, except for the traceability field itself.

### Ordering

- Event ordering is a function of stream version, not correlation_id.
- The ordering invariant is: if event A was appended before event B, A precedes B on load — regardless of correlation_id values.

### Deduplication

- Correlation_id is not an idempotency key. Two events with identical correlation_ids are two distinct events.
- No SPI implementation SHALL suppress, skip, or overwrite an event based on its correlation_id.

## Constraints

- Correlation_id SHALL remain `Option`-like — it is never required at the SPI level.
- No SPI implementation SHALL add security semantics to correlation_id (e.g., validation, signing, encryption).
- No SPI consumer SHALL depend on correlation_id for ordering — ordering guarantees come from stream version only.
- Deduplication, if needed, SHALL be implemented as a separate concern above or below the SPI boundary, not within the SPI contract.

## Out of Scope

- Deduplication mechanism or idempotency key design — these are separate concerns.
- Security token design, authentication, or authorization — correlation_id is explicitly not these.
- Cryptographic properties of correlation_id (signing, encryption, validation).
- Format or schema of correlation_id values — they remain opaque strings.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A developer can read the correlation_id section of the SPI and identify four explicit negative boundaries (security, correctness, ordering, deduplication) in under 2 minutes.
- **SC-002**: A security audit of the codebase finds zero cases where correlation_id influences authentication or authorization decisions.
- **SC-003**: A developer can append events with the same correlation_id in reverse order and verify the stream returns them in append order, not correlation_id order, in a single test.
- **SC-004**: A developer can append two distinct events with identical correlation_ids and verify both events are present on load (no deduplication).

## Assumptions

- The existing Persistence SPI (spec 001) defines `StoredEvent<E>` with `correlation_id: Option<String>` as opaque metadata.
- The Correlation Lifecycle Contract (spec 002) defines how correlation_id is created, propagated, and preserved.
- The Correlation Scope Boundary spec (spec 004) defines which contracts own correlation_id (EventStore only).
- SPI consumers are responsible for their own security, ordering, and deduplication mechanisms — the SPI provides event sourcing primitives, not infrastructure policies.
