# Engineering Architecture

Describes ego-rs engineering structure, design conventions, and spec integration. Complements `ARCHITECTURE.md` which covers runtime architecture.

---

## A. Architectural Principles

### Hexagonal Architecture

- **Domain** owns contracts (traits, types, invariants)
- **Infrastructure** owns adapters (concrete implementations, persistence, migrations)
- **Application** orchestrates use cases (commands, queries)
- **Transport** owns protocol handlers (HTTP, gRPC)
- **Runtime** owns actor execution (mailbox, supervision, scheduling)

### Dependency Direction

```
domain ← application ← infrastructure
domain ← transport
domain ← runtime
```

No layer may depend on a layer to its right in this chain. Violations are enforced by `layers.toml` and `scripts/verify-layers.sh`.

### Runtime Neutrality

Domain contracts MUST be runtime-neutral — no `async`, no Tokio, no runtime-specific types in trait signatures. Infrastructure owns all async integration.

---

## B. Crate Boundaries

| Crate | Layer | Responsibility | Depends On |
|-------|-------|---------------|------------|
| `ego-domain` | domain | Core contracts, traits, types, invariants | Nothing internal |
| `ego-application` | application | Use case orchestration | `ego-domain` |
| `ego-infrastructure` | infrastructure | Concrete adapters, persistence backends, migrations | `ego-application`, `ego-domain` |
| `ego-transport` | transport | HTTP/gRPC protocol handlers | `ego-application`, `ego-domain` |
| `ego-runtime` | foundation | Actor system, mailbox, supervision | `ego-domain` |
| `ego-runtime-tokio` | infrastructure | Tokio-based runtime execution | `ego-runtime`, `ego-domain` |
| `runtime-slice` | domain | Deterministic execution types | Nothing internal |

### Module Ownership

- **ego-domain/persistence/** — persistence SPI traits and types (EventStore, Repository, Snapshot, PersistenceError)
- **ego-domain/actor/** — Actor trait, ActorId, message types
- **ego-domain/event/** — DomainEvent contract
- **ego-infrastructure/persistence/** — concrete backends (in_memory/, postgres/)
- **ego-infrastructure/persistence/in_memory/** — reference/testing implementations
- **ego-infrastructure/persistence/postgres/** — production PostgreSQL adapter

---

## C. Design Preferences

- **Concrete first** — prefer a concrete implementation over an abstraction. Extract abstractions only when a second use case emerges.
- **Abstractions require evidence** — every abstraction MUST cite which specific requirement or constraint from the spec justifies it.
- **Patch over rewrite** — extend existing modules. Create new modules only when existing structure cannot accommodate the change without violating layering.
- **Avoid duplication** — when similar code appears in two places, extract once verified patterns exist (see Rule of Two in constitution).
- **Explicit file ownership** — each crate module has a documented responsibility. New code goes in the crate that owns that concern.
- **No infrastructure in domain** — database types, network types, runtime types, and serialization frameworks MUST NOT appear in domain contracts.

---

## D. Runtime & Dependency Rules

- `ego-domain` MUST NOT depend on `tokio`, `async`, or any async runtime
- `ego-infrastructure` owns all async integration and runtime-specific behavior
- Traits in `ego-domain` use synchronous signatures; implementations MAY wrap in async behind the SPI boundary
- `ego-application` SHOULD remain runtime-neutral but MAY depend on runtime traits for use case orchestration
- Dependency direction is enforced by `layers.toml` and verified by `scripts/verify-layers.sh`

---

## E. Spec Integration Rules

### Workflow

The project follows the Spec Kit workflow with review gates:

```
spec → clarify → design → review → tasks → implement → review → archive
```

### Mandatory Artifacts

| Artifact | Purpose | Produced By | Contains |
|----------|---------|-------------|----------|
| `spec.md` | What — behavior, requirements, invariants, outcomes | `/speckit.specify` | MUST NOT contain framework names, file paths, crate names, runtime choices, concrete types, serialization frameworks, SQL, migration filenames |
| `plan.md` | How — architecture decisions, module placement, technology choices, rationale | `/speckit.plan` | Design decisions linked to spec sections |
| `tasks.md` | Executable work — file paths, modification type, expected outcome, validation | `/speckit.tasks` | Task items only — no design rationale |
| Source code | Implementation conforming to spec + design | `/speckit.implement` | — |

### Optional Artifacts

| Artifact | When Justified |
|----------|---------------|
| `research.md` | Design choices need documented tradeoff analysis |
| `quickstart.md` | Validation steps are non-trivial |

### Rules

- A specification MUST NOT prescribe implementation file paths, crate names, framework names, runtime choices, or technology choices
- A design document MUST cite the relevant spec section for each design decision
- Tasks MUST reference the design decisions they implement
- Architecture changes MUST be reflected in the design before implementation begins
- Each phase has a review gate; rejected artifacts must be revised before proceeding
- Extra artifacts beyond the mandatory three require justification per `.speckit/constitution.md` §D
