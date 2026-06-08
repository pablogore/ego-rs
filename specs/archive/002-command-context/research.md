# Research: Execution Context Design Decisions

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Decision 1: Crate Ownership of ExecutionContext Trait

### Decision
Define the `ExecutionContext` trait in `ego-domain` as a domain-owned contract.

### Rationale
- The architecture doc states **Domain owns contracts** (traits, types, invariants)
- The spec requires the trait to be runtime-independent: "MUST NOT expose Tokio types, actor references, mailboxes, channels, networking primitives, or cluster internals"
- `ego-domain` has zero runtime dependencies — this guarantees no accidental runtime coupling
- Follows the established pattern: `Observability` trait and `EventStore` trait both live in `ego-domain` with runtime-neutral signatures
- Execution handlers are defined at the domain/application boundary — the context they receive must not pull in runtime types

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| `ego-runtime` (keep existing struct) | Already depends on Tokio — exposes runtime coupling; existing `context.rs` has a struct, not a trait, no abstraction for different runtimes |
| New crate (`ego-command-context`) | Unnecessary module — contradicts "avoid duplicate modules" rule |

## Decision 2: Read-Only Trait (No Side Effects)

### Decision
Define `ExecutionContext` as a trait with only `&self` methods. No `&mut self` methods. No persistence, reply, scheduling, or observability methods.

### Rationale
- ExecutionContext represents pure execution context — it is a **view** into the current execution scope, not a runtime facade
- Side-effect capabilities (persist, reply, schedule) are modeled as independent abstractions (Effect API, Scheduling API) following the Lagom programming model
- Read-only access makes the context safe to share, clone, and pass by reference without lifetime complexity
- `&self` signals to developers that reading context has no side effects

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| `&mut self` trait (previous design) | Implied side effects; context became a service locator |
| Include persistence/reply/schedule in trait | Violates Interface Segregation Principle; creates coupling to subsystems |
| `dyn ExecutionContext` with opaque methods | Unnecessary indirection for a pure data view |

## Decision 3: Identity and Correlation Types

### Decision
Define lightweight domain types in a new `ego-domain::context` module:
- `AggregateId` (newtype over String, following `ActorId` pattern)
- `EntityId` (newtype over String, following `ActorId` pattern)
- `TenantId` (newtype over String, following `ActorId` pattern)
- `CorrelationId` (moved from `crates/runtime/src/context.rs`)
- `CausationId` (newtype over String)
- `RequestId` (newtype over String)
- `Metadata` (type alias for `HashMap<String, String>`)

### Rationale
- Newtype wrappers provide type safety (prevent mixing up IDs) following the existing `ActorId` pattern
- `ActorId` uses fail-closed construction (empty name rejected) — identity types follow same pattern
- All types derive common traits: `Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize`
- The existing `CorrelationId` in `crates/runtime/src/context.rs` is the canonical implementation — move it to domain rather than duplicate
- Optional presence via `Option<...>` in the trait accessors handles the "when available" spec requirement

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Raw `String`/`Option<String>` for all IDs | No type safety; easy to mix up aggregate_id with tenant_id |
| Single `Identity` enum with all ID variants | Less ergonomic for handlers that need specific fields |

## Decision 4: Refactoring Existing Runtime Struct

### Decision
The existing `crates/runtime/src/context.rs` is refactored:
1. `CorrelationId` type moves to `ego-domain` as part of the context types
2. The existing runtime struct in `crates/runtime/src/context.rs` becomes a concrete implementation of the new domain `ExecutionContext` trait
3. The struct retains its current correlation_id logic and gains identity + metadata fields

### Rationale
- Preserves existing functionality and tests — "patch over rewrite"
- The existing `CorrelationId` type is a domain concept (not runtime-specific), so it belongs in `ego-domain`
- The existing struct becomes one of potentially many runtime implementations (test runtime, Tokio runtime, etc.)

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Delete existing context.rs and start fresh | Throws away working code with tests; violates "patch over rewrite" |
| Keep context.rs unchanged, create parallel module | Creates duplicate concepts; violates "avoid duplicate modules" |

## Decision 5: Effect API is a Future Spec

### Decision
Persistence, replies, and no-op effects are modelled as a separate `Effect` abstraction in a future spec. The ExecutionContext does not expose these capabilities.

### Rationale
- Follows the Lagom programming model: handlers return `Effects` rather than calling methods on a context
- `Effect` is a value type (enum) that the runtime interprets, not a service interface
- Separates "what the handler wants to do" (Effect) from "how the runtime executes it" (interpreter)
- Enables deterministic testing: effects can be asserted on without a runtime

### Conceptual Design (not implemented in this spec)

```rust
enum Effect<E: DomainEvent, R> {
    Persist(Vec<E>),
    Reply(R),
    PersistAndReply(Vec<E>, R),
    None,
}
```

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Methods on ExecutionContext (previous design) | Violates ISP; creates service locator |
| Direct EventStore injection | Exposes persistence internals to handlers |

## Decision 6: Scheduling is a Future Spec

### Decision
One-shot and recurring execution are modelled as a separate `Scheduling` abstraction in a future spec. The ExecutionContext does not expose scheduling.

### Rationale
- Scheduling is an orthogonal concern — not all execution handlers need it
- A future `Scheduling` API can return scheduled operation handles independently
- Avoids coupling the context to timer/clock abstractions

## Decision 7: Observability Reuses Existing Abstraction

### Decision
Observability is already owned by FOUNDATION-003 (`ego-domain::observability::Observability`). ExecutionContext does not expose an observability accessor — handlers that need tracing use the existing `Observability` trait directly, passing context data manually.

### Rationale
- Reuses existing abstraction — no duplication, no new interface
- `Observability` trait already accepts `SemanticEvent` with correlation_id, actor_id
- Keeping observability out of ExecutionContext maintains separation of concerns
- Auto-enrichment can be implemented as a helper function that reads from context and delegates to observability

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Context accessor for Observability (previous design) | Duplicates existing FOUNDATION-003 trait |
| Context auto-enrichment | Creates hidden coupling between context and observability backend |

## Decision 8: Ownership Boundaries

### Decision
ExecutionContext has explicit ownership boundaries. It owns identity, correlation, and metadata only. Persistence, replies, scheduling, observability, transport, and runtime execution are explicitly excluded and belong to separate abstractions.

### Rationale
- Clear ownership prevents scope creep — the context does not become a service locator
- Each excluded concern has (or will have) its own dedicated abstraction
- Developers can reason about what the context provides without guessing

## Decision 9: Future Specialization

### Decision
ExecutionContext is the root execution abstraction. Future specialized contexts (CommandExecutionContext, EventExecutionContext, WorkflowExecutionContext) MAY extend it, but are outside this specification's scope.

### Rationale
- Command-specific fields (e.g., expected_version, command_type) may emerge in a future CommandExecutionContext
- Event-specific fields (e.g., event_type, sequence_number) may emerge in a future EventExecutionContext
- Keeping this spec to the common denominator avoids premature commitment to any execution model
- Forward-compatible: specialized contexts simply add fields on top of the base ExecutionContext
