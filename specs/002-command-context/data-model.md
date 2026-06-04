# Data Model: Execution Context

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Research**: [research.md](research.md)

## Overview

The Execution Context data model defines the types that represent execution context for all execution models (commands, events, workflows, sagas, projections, process managers). These types live in `ego-domain` and are runtime-neutral. The context is a **read-only view** — it carries information, not capabilities.

### Ownership Boundaries

ExecutionContext **owns**: identity, correlation, metadata.
ExecutionContext does **NOT own**: persistence, replies, scheduling, observability, transport, runtime execution.

## Entities

### ExecutionContext (trait)

The read-only execution context received by execution participants (command handlers, event handlers, workflows, etc.). This is the root execution abstraction; future specialized contexts (CommandExecutionContext, EventExecutionContext, WorkflowExecutionContext) MAY extend it but are outside this specification's scope.

| Method | Signature | Description |
|--------|-----------|-------------|
| `aggregate_id` | `(&self) -> Option<&AggregateId>` | Aggregate identity, if available |
| `entity_id` | `(&self) -> Option<&EntityId>` | Entity identity, if available |
| `tenant_id` | `(&self) -> Option<&TenantId>` | Tenant identity, if available |
| `correlation_id` | `(&self) -> Option<&CorrelationId>` | Correlation identifier, if available |
| `causation_id` | `(&self) -> Option<&CausationId>` | Causation identifier, if available |
| `request_id` | `(&self) -> Option<&RequestId>` | Request identifier, if available |
| `metadata` | `(&self) -> &Metadata` | Read-only key-value metadata |

**Validation**: Implementations MUST return `None` for identity/correlation fields when the value is absent, rather than failing. The context is always constructable.

**State Transitions**: None. The context is immutable for the duration of handler execution.

### Identity Types

All identity types follow the `ActorId` pattern: newtype over `String` with fail-closed construction rejecting empty/whitespace-only values.

| Type | Wraps | Derives | Validation |
|------|-------|---------|------------|
| `AggregateId` | `String` | Debug, Clone, PartialEq, Eq, Hash | Non-empty, non-whitespace |
| `EntityId` | `String` | Debug, Clone, PartialEq, Eq, Hash | Non-empty, non-whitespace |
| `TenantId` | `String` | Debug, Clone, PartialEq, Eq, Hash | Non-empty, non-whitespace |
| `CorrelationId` | `String` | Debug, Clone, PartialEq, Eq, Hash | Non-empty, non-whitespace |
| `CausationId` | `String` | Debug, Clone, PartialEq, Eq, Hash | Non-empty, non-whitespace |
| `RequestId` | `String` | Debug, Clone, PartialEq, Eq, Hash | Non-empty, non-whitespace |

### Value Types

| Type | Definition | Description |
|------|-----------|-------------|
| `Metadata` | `HashMap<String, String>` | Arbitrary key-value pairs from the incoming request |

### Relationship Diagram

```text
ExecutionMessage (command, event, workflow input, etc.)
    │
    ▼ handled by
ExecutionHandler (CommandHandler, EventHandler, Workflow, etc.)
    │
    │ receives
    ├── Message (the input payload)
    │
    ▼
ExecutionContext (read-only trait)
    ├── carries AggregateId? ──► AggregateId (newtype)
    ├── carries EntityId? ──► EntityId (newtype)
    ├── carries TenantId? ──► TenantId (newtype)
    ├── carries CorrelationId? ──► CorrelationId (newtype)
    ├── carries CausationId? ──► CausationId (newtype)
    ├── carries RequestId? ──► RequestId (newtype)
    └── carries Metadata ──► HashMap<String, String>
```

Side-effect capabilities (persist, reply, schedule) are NOT part of this model. They belong to future Effect and Scheduling specs.

ExecutionContext is the root execution abstraction. Future specialized contexts (CommandExecutionContext, EventExecutionContext, WorkflowExecutionContext) MAY extend it but are outside this specification's scope.

## Cross-References

- **Spec Requirements**: FR-001 through FR-011
- **Research Decisions**: AD-001 (read-only), AD-003 (identity types), AD-005 (future specs)
