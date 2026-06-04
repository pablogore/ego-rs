# Research: Effect API Design Decisions

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Decision 1: Effect as Enum

### Decision
Define `Effect` as an enum (not a trait) with explicit variants for each outcome type.

### Rationale
- Effects describe outcomes — they are data, not behavior
- An enum provides exhaustive matching in the runtime interpreter
- Adding new variants is a compile-error at every interpreter, ensuring no variant is silently unhandled
- Traits would allow open-ended extension but lose exhaustiveness guarantees
- Follows the Lagom `Effects` pattern and Akka Typed `Effect` pattern

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Trait-based (open for extension) | Loses exhaustiveness — runtime could silently ignore variants |
| Boxed `dyn Effect` | Heap allocation; unnecessary indirection for pure data |
| Function/closure-based | Effects must be assertable by value equality |

## Decision 2: Generic Event and Reply Types

### Decision
Effect variants use generic type parameters for events and replies. No `DomainEvent` bound.

### Rationale
- The API supports event-sourced entities, stateful entities, CRUD entities, workflows, sagas, and projections
- Each execution model defines its own event/reply types
- A `DomainEvent` bound would exclude CRUD and projection use cases
- Generic parameters keep the API model-agnostic

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Concrete event type (DomainEvent trait) | Excludes non-event-sourced models |
| Associated types on Effect | Adds complexity without benefit — Effect is data, not a trait |

## Decision 3: Composition via Composed Variant

### Decision
Define a `Composed` variant that holds multiple Effects. Composition is recursive — `Composed` may contain `Composed`.

### Rationale
- Handlers commonly need multiple outcomes (e.g., emit events AND reply)
- Recursive composition is simple to implement and reason about
- The runtime interpreter walks the tree depth-first, executing each leaf Effect
- Follows the `Effects` tuple pattern from Lagom (generalized to a list)

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Tuples (Effect<A, B>) | Limited to fixed cardinality |
| Builder pattern | More ceremony than a simple enum variant |
| Flattened list | Loses structural information about effect grouping |

## Decision 4: NoEffect as Explicit Variant

### Decision
Include `NoEffect` as a first-class variant, not as `Option<Effect>`.

### Rationale
- Handlers that do nothing still return a value — `NoEffect` is self-documenting
- The `Option` pattern suggests "no result" rather than "no effect needed"
- Consistent with Lagom's `Effects.none()` and Akka's `Effect.none`

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| `Option<Effect>` | Ambiguous — None could mean "handler not called" vs "handler chose no effect" |
| Empty `Composed` | Confusing — an empty composition is a logic error in most cases |

## Decision 5: Crate Ownership

### Decision
Define Effect types in `ego-domain` (the domain contracts crate). Interpretation lives in runtime crates.

### Rationale
- Follows the established pattern (ExecutionContext, Observability, EventStore all live in `ego-domain`)
- Effect types are pure value types — no runtime dependencies
- Runtime crates import Effect types and provide interpreters
- Handlers at the domain/application boundary import Effect types without runtime coupling

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| New crate (`ego-effect`) | Unnecessary module — contradicts "avoid duplicate modules" rule |
| `ego-runtime` | Handler code would need to depend on runtime to describe outcomes |

## Decision 6: No ExecutionContext Dependency

### Decision
The Effect API does not depend on ExecutionContext. Handlers MAY use both independently.

### Rationale
- ExecutionContext (002) provides read-only context; Effect API describes outcomes
- These are orthogonal concerns — a handler reads context and returns an effect
- No structural coupling between the two abstractions
- Execution models that don't need context can use Effects alone

## Decision 7: Sync Handler Return, Async Execution

### Decision
Handlers return Effect values synchronously. Runtime interprets Effects asynchronously.

### Rationale
- Handler logic stays pure and testable — no async in handler signatures
- The Effect is a description, not an execution
- Runtime is responsible for executing side effects (IO, persistence, messaging)
- Clean separation: sync description (handler) vs async execution (runtime)

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Async handler returning Effect | Forces all handlers to be async; complicates testing |
| Effect executes inline | No separation of description vs execution; untestable |

## Decision 8: StateMutation Semantics

### Decision
Keep `StateMutation(S)` as a first-class Effect variant, but define it as execution-model specific. Runtimes MAY reject `StateMutation` for execution models that do not support direct state mutation.

### Rationale
- Event-sourced entities derive state from events — `StateMutation` is not applicable
- Stateful entities mutate state directly — `StateMutation` is the natural description
- CRUD entities replace state directly — `StateMutation` maps to the replace operation
- Workflows and sagas may not expose mutable state — runtime rejects `StateMutation`
- Keeping the variant in the enum preserves exhaustiveness across all runtimes
- Each runtime decides which variants are valid for its execution model

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Keep StateMutation as unconditional (Option A) | Forces event-sourced runtimes to support an irrelevant variant |
| Remove StateMutation entirely (Option B) | Excludes stateful and CRUD entities from describing state changes |

## Decision 9: Nested Composed Flattening Semantics

### Decision
The runtime MAY flatten nested Composed structures before interpretation. The Effect value type always preserves the handler's original structure; flattening is an internal runtime optimization.

### Rationale
- Depth-first traversal of the unflattened tree produces identical leaf order to linear iteration of the flattened list — flattening does not change observable execution behavior
- Each runtime chooses the strategy best suited to its execution model:
  - Tokio-based runtimes may flatten for simple linear dispatch
  - Actor runtimes may preserve hierarchy for actor-specific dispatch
  - Workflow runtimes may preserve hierarchy for step/state correlation
- Effect remains a pure value type — tests assert on the exact value, not the runtime's internal representation
- The `and_then` combinator already flattens during composition (via `collect_children`); direct `compose()` preserves caller-provided structure

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Preserve hierarchy exactly (Option A) | Restricts runtime optimization without behavioral benefit — runtime MUST walk nesting; cannot flatten even when linear dispatch would be simpler |
| MUST flatten before interpretation (Option C) | Imposes flattening mandate on all runtimes, contradicting runtime-agnostic goals — Tokio, actor, and workflow runtimes each lose the freedom to choose their optimal strategy |
