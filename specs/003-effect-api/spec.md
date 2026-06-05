# Feature Specification: Effect API

**Feature Branch**: `006-effect-api`

**Created**: 2026-06-04

**Status**: Implemented (2026-06-04)

**Input**: User specification: "Introduce a runtime-agnostic Effect API that represents the outcome of execution handlers."

## Problem

ExecutionContext intentionally owns only identity, correlation, and metadata. It does not own persistence, replies, scheduling, or observability. Execution handlers currently have no canonical mechanism to describe execution outcomes.

## User Scenarios & Testing

### User Story 1 - Return reply (Priority: P1)

A developer writing an execution handler needs to describe a reply as the outcome of handling, without coupling to the transport layer.

**Why this priority**: Reply is the most common execution outcome — handlers must be able to describe what to send back.

**Independent Test**: A handler returns an Effect describing a reply. The test asserts the Effect value without executing it.

**Acceptance Scenarios**:

1. **Given** a handler that produces a reply, **When** the handler executes, **Then** it returns an Effect containing the reply value.
2. **Given** a handler that produces no reply, **When** the handler executes, **Then** it returns a NoEffect.

---

### User Story 2 - Emit event (Priority: P1)

A developer writing an execution handler needs to describe event emission as an outcome, without coupling to the event store.

**Why this priority**: Event emission is the primary persistence mechanism for event-sourced systems.

**Independent Test**: A handler returns an Effect describing event emission. The test asserts the Effect value without a database.

**Acceptance Scenarios**:

1. **Given** a handler that emits one or more events, **When** the handler executes, **Then** it returns an Effect containing the events.
2. **Given** a handler that does not emit events, **When** the handler executes, **Then** it returns an Effect without events.

---

### User Story 3 - Multiple outcomes (Priority: P1)

A developer needs to describe multiple outcomes (e.g., emit events AND reply) in a single handler.

**Why this priority**: Real handlers commonly need multiple side effects from one execution.

**Independent Test**: A handler returns a composed Effect containing both events and a reply. The test asserts the composed structure.

**Acceptance Scenarios**:

1. **Given** a handler that emits events and sends a reply, **When** the handler executes, **Then** it returns a composed Effect containing both.
2. **Given** a handler that performs a state mutation and emits events, **When** the handler executes, **Then** it returns a composed Effect containing both.

---

### Edge Cases

- What happens when a handler returns an Effect the runtime cannot interpret? The runtime SHOULD reject with a clear error.
- What happens when composition produces conflicting effects? The runtime SHOULD fail with a description of the conflict.
- What happens when effects reference entities the handler does not own? The runtime SHOULD fail with an authorization error.

## Requirements

### Functional Requirements

- **FR-001**: The framework MUST define a canonical Effect abstraction describing execution outcomes.
- **FR-002**: Effects MUST describe desired outcomes without executing them directly.
- **FR-003**: The runtime MUST interpret Effects and execute the described outcomes.
- **FR-004**: Effects MUST NOT expose Tokio types, actors, channels, mailboxes, network primitives, or database APIs.
- **FR-005**: Effects MUST support the following core outcomes: NoEffect, StateMutation, EventEmission, Reply.
- **FR-006**: Effects MUST be composable — a handler MAY return a single Effect, multiple Effects, or chained Effects.
- **FR-007**: The Effect API MUST support event-sourced entities, stateful entities, CRUD entities, workflows, sagas, and projections.
- **FR-008**: The Effect API MUST NOT assume DomainEvent exists — event types are generic.
- **FR-009**: Effects MUST be testable without databases, message brokers, runtimes, or network access.
- **FR-010**: Handlers MUST be testable by asserting on returned Effect values.
- **FR-011**: The Effect API MUST NOT depend on ExecutionContext — handlers MAY use both independently.

### Key Entities

- **Effect**: A value type describing a desired execution outcome. Composable, runtime-neutral.
- **NoEffect**: An effect describing no side effects.
- **StateMutation\<S\>**: An effect describing a state change.
- **EventEmission\<E\>**: An effect describing event emission.
- **Reply\<R\>**: An effect describing a reply.
- **Composed\<E, R, S\>**: An effect combining multiple outcomes.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Developers can describe execution outcomes without importing runtime types.
- **SC-002**: The same Effect types compile and execute on different runtime implementations without handler modification.
- **SC-003**: Handlers returning Effects are testable with plain assertions, no infrastructure.
- **SC-004**: Effect composition produces predictable, assertable structures.

## Ownership Boundaries

Effect API **owns**:
- Effect value types (enum hierarchy)
- Composition logic
- Handler return type contract

Effect API does **NOT own**:
- Effect interpretation/execution — owned by the runtime
- ExecutionContext — already defined in 002-execution-context
- Event store integration — owned by runtime implementations
- Reply transport — owned by runtime implementations

## Future Architecture

```
ExecutionHandler
    -> receives ExecutionContext (002)
    -> returns Effect (003)
    -> runtime interprets Effect

ExecutionContext (002) → pure context, no side effects
Effect API      (003) → describes side effects, no execution
Runtime         (future) → interprets Effects, executes side effects
Scheduling API  (future) → delayed/recurring execution
Observability   (existing FOUNDATION-003) → tracing, metrics, logging
```

## Assumptions

- Effects are value types — Clone, Debug, PartialEq.
- Effects are returned from handlers, not injected.
- Runtime implementations provide interpreters for each effect variant.
- Effect composition is recursive — Composed may contain Effects which may themselves be Composed.
- Event types are generic — the API does not constrain what constitutes an event.
- Reply types are generic — the API does not constrain what constitutes a reply.
- Handlers remain synchronous — async execution belongs to the runtime layer.
