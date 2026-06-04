# Feature Specification: Effect API

**Feature Branch**: `006-effect-api`

**Created**: 2026-06-04

**Status**: Archived (2026-06-04)

**Input**: User specification: "Introduce a runtime-agnostic Effect API that represents the outcome of execution handlers."

## Problem

ExecutionContext intentionally owns only identity, correlation, and metadata. It does not own persistence, replies, scheduling, or observability. Execution handlers currently have no canonical mechanism to describe execution outcomes.

## Clarifications

### Session 2026-06-04

- Q: Should StateMutation(S) be a first-class Effect variant? → A: Keep StateMutation(S) as execution-model specific. Runtimes SHALL reject StateMutation during effect interpretation for execution models that do not support direct state mutation (e.g., event-sourced, workflows, sagas), returning `EffectInterpretationError::UnsupportedEffect`. Handlers in any model MAY construct `StateMutation` — rejection is deferred to runtime interpretation.
- Q: What are the runtime semantics of nested Composed effects? → A: Runtime SHALL recursively flatten nested Composed structures before interpretation (canonical recursive flattening). The Effect value type always preserves the handler's original structure (tests assert on exact value). Flattening does not change execution order — depth-first traversal of the unflattened tree produces identical leaf order to linear iteration of the flattened list. Nesting depth SHALL NOT alter execution semantics. The `and_then` combinator already flattens during construction; direct `compose()` preserves caller-provided structure.

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

- What happens when a handler returns an Effect the runtime cannot interpret? The runtime SHALL return `EffectInterpretationError::UnsupportedEffect`.
- What happens when composition produces conflicting effects? The runtime SHALL return `EffectInterpretationError::ConflictingEffects`.
- What happens when effects reference entities the handler does not own? The runtime MAY reject — this is beyond the canonical error model and runtime-specific.
- What happens when a handler returns StateMutation for an execution model that does not support direct state mutation? The runtime SHALL return `EffectInterpretationError::UnsupportedEffect` with an explanation that StateMutation is unsupported for the given model.
- What happens when a runtime receives nested Composed effects? The runtime SHALL recursively flatten the composition before interpretation. Nesting depth SHALL NOT alter execution semantics.

### Interpretation Error Model

Effect interpretation errors are owned by the runtime layer. Every runtime SHALL explicitly evaluate every `Effect` variant — unsupported variants SHALL NOT be silently ignored.

The canonical error type:

```rust
pub enum EffectInterpretationError {
    /// Runtime does not support a specific effect variant.
    UnsupportedEffect,
    /// Composition violates runtime rules (e.g., empty Composed).
    InvalidComposition,
    /// Mutually incompatible effects (e.g., multiple replies in a single-reply runtime).
    ConflictingEffects,
}
```

**Ownership**: This type is defined in the runtime layer, not in `ego-domain`. Runtimes MAY extend with additional error variants as needed.

**Semantics**:
- `UnsupportedEffect` — returned when the runtime does not implement a specific variant (e.g., `StateMutation` in an event-sourced runtime).
- `InvalidComposition` — returned when a `Composed` or composed structure violates runtime rules (e.g., empty `Composed` where prohibited).
- `ConflictingEffects` — returned when multiple effects in the same composition are mutually incompatible (e.g., two `Reply` values when the runtime allows only one).

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
- **StateMutation\<S\>**: An effect describing a state change. Execution-model specific — runtimes SHALL reject StateMutation during effect interpretation for models that do not support direct state mutation (e.g., event-sourced, workflows, sagas), returning `EffectInterpretationError::UnsupportedEffect`.
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
