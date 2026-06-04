# Feature Specification: Execution Context

**Feature Branch**: `005-command-context`

**Created**: 2026-06-04

**Status**: Revised (2026-06-04) — renamed to ExecutionContext following architectural review. Execution scope, not command scope.

**Input**: User description: "Introduce a canonical Execution Context Runtime abstraction that provides the execution context across all eGo runtime implementations."

## User Scenarios & Testing

### User Story 1 - Access identity context during execution (Priority: P1)

A developer writing an execution handler (command handler, event handler, workflow, etc.) needs to know which aggregate, entity, and tenant the current execution relates to, so they can enforce authorization rules and scope business logic correctly.

**Why this priority**: Identity access is a prerequisite for every execution handler — without it, no meaningful business logic can execute.

**Independent Test**: A test can verify that an execution handler receives the correct aggregate_id and tenant_id when handling a message, and returns the expected result.

**Acceptance Scenarios**:

1. **Given** an execution targeting a specific aggregate, **When** the handler executes, **Then** the context exposes the aggregate_id matching the execution target.
2. **Given** an execution scoped to a specific tenant, **When** the handler executes, **Then** the context exposes the tenant_id matching the execution scope.
3. **Given** an execution without tenant context, **When** the handler executes, **Then** the context exposes tenant_id as absent rather than failing.

---

### User Story 2 - Access correlation information (Priority: P1)

A developer needs to trace execution through the system using correlation IDs, causation IDs, and request IDs to support distributed tracing and debugging.

**Why this priority**: Correlation is an integral part of execution context — every execution carries tracing identifiers that handlers need to propagate.

**Independent Test**: A test can verify that correlation IDs set on the incoming message are available through the context inside the handler.

**Acceptance Scenarios**:

1. **Given** an execution with a correlation_id, **When** the handler accesses the context, **Then** the correlation_id matches the incoming value.
2. **Given** an execution with causation and request identifiers, **When** the handler accesses those fields, **Then** the values are available through the context.

---

### User Story 3 - Access request metadata (Priority: P1)

A developer needs to read arbitrary key/value metadata attached to the incoming request, such as headers, authentication tokens, or custom attributes.

**Why this priority**: Metadata is an integral part of execution context — handlers commonly need access to transport-level attributes.

**Independent Test**: A test can attach metadata to a message and verify the handler reads it through the context.

**Acceptance Scenarios**:

1. **Given** an execution with request metadata, **When** the handler reads metadata from the context, **Then** all key/value pairs from the request are available.
2. **Given** an execution without metadata, **When** the handler reads metadata from the context, **Then** an empty metadata set is returned without error.

---

### Edge Cases

- What happens when the context is accessed outside of an execution scope? The context SHOULD clearly indicate it is not active.
- What happens when identity fields are absent? The context returns `None` rather than failing.
- What happens when metadata is missing? An empty metadata set is returned.

## Requirements

### Functional Requirements

- **FR-001**: The framework MUST provide a canonical ExecutionContext that is available to all execution participants (command handlers, event handlers, workflows, etc.) during execution.
- **FR-002**: Execution participants MUST receive a read-only context during execution (`&ExecutionContext`).
- **FR-003**: ExecutionContext MUST expose aggregate_id when available in the execution scope.
- **FR-004**: ExecutionContext MUST expose entity_id when available in the execution scope.
- **FR-005**: ExecutionContext MUST expose tenant_id when available in the execution scope.
- **FR-006**: ExecutionContext MUST expose correlation_id when available in the execution scope.
- **FR-007**: ExecutionContext MUST expose causation_id when available in the execution scope.
- **FR-008**: ExecutionContext MUST expose request_id when available in the execution scope.
- **FR-009**: ExecutionContext MUST provide read-only access to request metadata supporting arbitrary key/value pairs.
- **FR-010**: ExecutionContext MUST NOT expose Tokio types, actor references, mailboxes, channels, networking primitives, or cluster internals.
- **FR-011**: The abstraction MUST remain portable across all runtime implementations without handler code modification.

### Ownership Boundaries

ExecutionContext **owns**:
- Identity (aggregate_id, entity_id, tenant_id)
- Correlation (correlation_id, causation_id, request_id)
- Metadata (arbitrary key/value pairs)

ExecutionContext does **NOT** own:
- Persistence — belongs to the Effect API (future spec)
- Replies — belong to the Effect API (future spec)
- Scheduling — belongs to the Scheduling API (future spec)
- Observability — belongs to FOUNDATION-003 Observability SPI (existing)
- Transport — belongs to the runtime layer
- Runtime execution — belongs to the runtime layer

### Key Entities

- **ExecutionContext**: The execution context received by execution participants. Contains identity fields, correlation fields, and metadata — nothing else. This is the root execution abstraction; future specialized contexts (e.g., CommandExecutionContext, EventExecutionContext, WorkflowExecutionContext) MAY extend it but are outside this specification's scope.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Developers can access aggregate_id, entity_id, and tenant_id from any execution handler without runtime-specific imports.
- **SC-002**: Developers can access correlation_id, causation_id, and request_id from any execution handler without runtime-specific imports.
- **SC-003**: Developers can read request metadata from any execution handler without runtime-specific imports.
- **SC-004**: An execution handler written against one runtime implementation executes without modification on a different runtime implementation.

## Future Architecture

This section documents the intended architectural evolution. It is informational only — no new requirements are introduced.

### Execution Model Patterns

ExecutionContext is the root execution abstraction. Future specialized contexts MAY extend it but are outside this specification's scope:

```
ExecutionContext (this spec)
    ├── aggregate_id, entity_id, tenant_id     ← identity
    ├── correlation_id, causation_id, request_id ← correlation
    └── metadata                                ← request metadata
    
    Future specializations (not in scope):
    ├── CommandExecutionContext  (extends ExecutionContext)
    ├── EventExecutionContext    (extends ExecutionContext)
    └── WorkflowExecutionContext (extends ExecutionContext)
```

### Execution / Effect Pattern

Execution participants receive ExecutionContext and return an Effect:

```
ExecutionHandler
    -> receives ExecutionContext
    -> returns Effect

CommandHandler   → receives ExecutionContext → returns Effect
EventHandler     → receives ExecutionContext → returns Effect
Workflow         → receives ExecutionContext → returns Effect
```

### Future Specifications

The following capabilities are owned by separate abstractions:

| Capability | Owner | Description |
|-----------|-------|-------------|
| Event Persistence | Effect API | Persist domain events as a side effect of execution |
| Typed Replies | Effect API | Send typed replies back to the caller |
| No-op Effect | Effect API | Explicitly signal no side effects |
| Delayed Execution | Scheduling API | Execute after a delay |
| Recurring Execution | Scheduling API | Execute repeatedly at an interval |
| Observability | FOUNDATION-003 SPI | Tracing, metrics, logging — already exists

## Assumptions

- Identity fields (aggregate_id, entity_id, tenant_id) may be absent in some execution scopes; the context handles absences gracefully rather than failing.
- Correlation fields (correlation_id, causation_id, request_id) may be absent; the context handles absences gracefully.
- Metadata values are treated as opaque strings — no type enforcement is performed at the context level.
- The framework (runtime layer) is responsible for constructing and providing the ExecutionContext to execution participants — participants do not create contexts themselves.
- Correlation IDs follow industry conventions (e.g., W3C Trace Context) unless otherwise specified by the runtime.
