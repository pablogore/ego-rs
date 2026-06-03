# ego-rs Constitution

**Version**: 2.0.0 | **Ratified**: 2026-06-03 | **Last Amended**: 2026-06-03

Non-negotiable engineering laws. Applies to all `/speckit.*` commands, implementation, and review. Supersedes all other guidance when conflict exists.

---

## A. Anti Over-Engineering

### Core Rule
All design and implementation SHALL be minimal necessary to satisfy the specification. The simplest valid implementation SHALL win.

### Prohibited Practices
- Speculative abstractions — no "we might need this later"
- Future-proofing without explicit requirements
- Solving hypothetical problems
- "Just in case" design patterns
- Premature optimization of any kind
- Scalability assumptions without measured evidence

### Operational Rules
- **Burden of proof** belongs to complexity — any abstraction beyond direct implementation MUST be justified in the design
- **No** registries, factories, adapters, plugin systems, builders, middleware chains, or orchestration layers UNLESS required by specification
- **No** pagination, retries, caching, batching, concurrency optimizations, connection pooling, or scaling abstractions UNLESS explicitly required

### Rule of Two
A generic abstraction SHOULD NOT exist before two concrete use cases require it.

### Exception
Framework SPI traits, domain contracts, and extension points explicitly required by a specification ARE exempt from the Rule of Two.

---

## B. Architecture Escalation Rule

Architectural complexity MAY only increase when justified by one or more of:

1. **Explicit specification requirement** — the spec demands it
2. **Multiple required implementations** — two or more concrete backends or adapters exist and must share a contract
3. **Contract invariant requirements** — the abstraction enforces an invariant that direct implementation cannot
4. **Measured operational constraints** — profiling or measurement proves the simpler approach is insufficient
5. **Explicit human instruction** — a maintainer directs it

When none apply, the simpler implementation MUST be chosen.

---

## C. Spec Scope Discipline

### Executable Spec Rule
Every spec MUST represent an independently implementable capability. A spec MUST:
- Be implementable in a single cycle
- Be independently completable
- Be archivable after implementation
- Produce executable work

A spec MUST NOT become an endless architectural umbrella.

### Atomic Capability Rule
One spec = one capability. If a feature contains multiple independently implementable concerns, split it.

Split triggers include:
- Different runtime responsibilities
- Different deployment responsibilities
- Different ownership boundaries
- Different implementation timelines
- Different archival points

The burden of proof belongs to keeping specs large. Default to smaller specs.

### Refactor Safety Rule
Specs define outcomes, not prison walls. Refactoring is allowed during implementation when:
- Behavior is preserved
- Invariants remain valid
- Design improves
- Complexity decreases

---

## D. Minimal Artifact Policy

Default required artifacts per spec:

| Artifact | Required | Purpose |
|----------|----------|---------|
| `spec.md` | ALWAYS | Behavior, requirements, invariants, outcomes |
| `plan.md` | ALWAYS | Implementation structure, module placement, decisions |
| `tasks.md` | ALWAYS | Executable work units |

Optional artifacts (generate only when justified):
- `research.md` — when design choices need documented tradeoff analysis
- `quickstart.md` — when validation steps are non-trivial

Forbidden unless explicitly required:
- `contracts/*` — generated only when contract complexity justifies separate files
- `data-model.md` — inline in spec.md or plan.md instead
- `checklists/*` — ceremonial documentation
- Any artifact that duplicates information already present in required files

Burden of proof belongs to extra documents. Generate extra artifacts only if:
1. Contract complexity justifies them
2. Human explicitly requests them
3. System risk materially increases without them

---

## E. Architecture Freeze Prevention

Do NOT freeze implementation prematurely.

Forbidden in spec.md:
- Framework names (tokio, sqlx, axum, etc.)
- Library names
- Crate names
- File paths
- Runtime choices (async, sync)
- Concrete type/struct names
- SQL or migration filenames
- API endpoint paths
- Serialization frameworks (serde_json, etc.)

Specs describe behavior. Design MAY choose mechanism later. Tasks MAY refine design. Implementation MAY refactor design.

Research decisions and technology choices belong in `plan.md`, never in `spec.md`.

---

## F. Task Precision & Granularity

### Precision Rule
Every task MUST be implementation-ready with:

1. **Exact file path** — absolute or relative to repo root
2. **Modification type** — Create | Modify | Refactor | Delete
3. **Section identifier** — method/module/trait name if known
4. **Expected outcome** — what exists after completion
5. **Validation criteria** — command or assertion that proves completion

### Granularity Rules
Tasks MUST:
- Be atomic (one logical change)
- Be independently executable
- Specify explicit files
- Define expected outcome
- Include validation criteria
- Remain minimal in scope

Tasks MUST NOT:
- Redesign architecture
- Silently expand scope beyond what plan.md defines
- Create umbrella tasks
- Introduce speculative abstractions

### Format
```
- [ ] T001 [US1] Create crates/domain/src/persistence/event_store.rs
      Action: Create
      File: crates/domain/src/persistence/event_store.rs
      Section: pub trait EventStore
      Outcome: Runtime-neutral EventStore<E: DomainEvent> trait exists
      Validation: cargo check -p ego-domain passes
```

---

## G. Separation of WHAT vs HOW

Strict layer separation:

| Layer | Content | Forbidden |
|-------|---------|-----------|
| `spec.md` | Behavior, requirements, acceptance criteria, invariants, outcomes | Framework names, library names, crate names, file paths, runtime choices, concrete types, SQL, migration filenames, serialization frameworks |
| `plan.md` | Architecture decisions, module placement, crate boundaries, technology choices | Implementation code, task assignments |
| `tasks.md` | Executable work with file paths, validation criteria | Design decisions, rationale |
| Implementation | Code conforming to spec + design | — |

Implementation details MUST NOT leak upward. Technology choices MUST NOT appear in specifications.

---

## H. Modify Before Duplicate

Before creating any new:
- Module
- Trait
- File
- Abstraction
- Adapter

Verify whether an appropriate implementation already exists in the codebase. If equivalent structure exists, modify or extend it.

Duplication is forbidden unless justified by:
1. Different architectural layer (domain vs infrastructure)
2. Different responsibility boundary
3. Explicit specification requirement

---

## I. Ambiguity & Human Escalation

When ambiguity materially changes architecture (type, structure, boundary, dependency direction):

1. **Stop** — do not proceed
2. **Ask** — request human guidance
3. **Do not guess** — undocumented assumptions are not architecture decisions
4. **Never invent architecture silently**

Minor ambiguities (naming, formatting, error message text) SHOULD be resolved by following existing conventions in the codebase.

---

## J. Testing Principles

- Contract tests SHALL be first-class citizens — every SPI trait SHALL have a shared contract test suite that backends must pass
- Invariants MUST be testable — untestable invariants are not invariants
- Implementation-specific behavior MUST remain behind interfaces — tests validate observable behavior, not internals
- Tests SHALL validate observable behavior through public contracts, not internal implementation details

---

## K. Context Efficiency

Minimize context size. Prefer:
- Small specs
- Small tasks
- Minimal artifacts
- Precise scope
- Maximum signal per artifact

Avoid:
- Document sprawl
- Prompt entropy
- Ceremonial documentation
- Information duplication across artifacts

Target: minimal stable context, maximum execution quality.

---

## L. Mock-Only Testing Rule (Strict Isolation)

All tests MUST use ONLY mocks.

### Forbidden
- In-memory implementations of domain/persistence/infrastructure traits
- Fake databases with mutable state
- Embedded message brokers (Kafka, RabbitMQ, etc.)
- Real HTTP, gRPC, or API calls
- Filesystem-backed test harnesses
- Any test double that contains business logic, persists state, or simulates workflows

### Allowed
- Pure mocks generated by `mockall` (or equivalent framework)
- Stubs that return fixed responses without state or logic
- Contract doubles that operate at the interface level only

### Mock Requirements
Mocks MUST:
- NOT contain business logic
- NOT persist state across invocations
- NOT simulate workflows or multi-step processes
- ONLY simulate inputs, outputs, and failure conditions

### Relationship to Section J
Section J.1 ("contract tests SHALL be first-class citizens — every SPI trait SHALL have a shared contract test suite that backends must pass") is SUPERSEDED by this rule. Contract tests that exercise real or in-memory backends are forbidden. The only valid test doubles are mocks as defined above.

---

## M. Minimum Coverage Rule (Quality Gate)

All repositories MUST maintain minimum global test coverage >= 85%.

### Rules
1. Coverage is evaluated at the repository level (aggregate across all crates)
2. Any drop below 85% is a BLOCKING violation
3. Partial compliance per module or per crate is NOT sufficient — the aggregate must meet or exceed threshold
4. Coverage includes line, branch, and function metrics

### Enforcement
- No release, merge, or validation is permitted below the 85% threshold
- If coverage drops below threshold, the violating change MUST be corrected before acceptance
- Coverage MUST be measured and validated at every change

---

## N. Rustdoc Completeness Rule (API Transparency)

All Rust code MUST be fully documented using `///` (or `//!`) rustdoc comments.

### Mandatory Coverage
- All public modules (`//!` or `///` on `pub mod`)
- All public structs, enums, and unions
- All public traits and their methods
- All public functions and methods
- All public type aliases and constants
- All `pub impl` blocks (inherent implementations on public types)
- Important internal business logic components and non-trivial private functions

### Documentation Requirements
Each documentation comment MUST explain:
- **Purpose** — what the item is for and why it exists
- **Behavior** — contract-level semantics (not implementation internals)
- **Inputs/Outputs** — parameters, return values, associated types
- **Failure modes** — errors, panics, invariants, preconditions (when applicable)

### Prohibited
- No undocumented public API items
- No empty or placeholder documentation (e.g., `/// Foo` with no further explanation)
- No documentation that merely restates the type signature without adding semantics

### Refactoring Obligation
If a component cannot be documented clearly and concisely, it MUST be refactored until it can.

---

## Governance

These rules are **global, permanent, non-overridable, system-wide invariants** belonging to the lowest-level governance layer:

| Rule | Type | Scope | Bypassable |
|------|------|-------|------------|
| L — Mock-Only Testing | Invariant | All tests | Never |
| M — Minimum Coverage | Invariant | All repositories | Never |
| N — Rustdoc Completeness | Invariant | All Rust code | Never |

- Sections L, M, and N take precedence over any conflicting earlier sections of this constitution
- This constitution supersedes all other development guidance when conflict exists
- Amendments require a dedicated specification and maintainer approval
- All `/speckit.plan` and `/speckit.implement` runs MUST include a constitution check
- Violations discovered during review MUST block merge until resolved
- Burden of proof always belongs to the party introducing complexity
- **Generation time**: any generated code MUST comply immediately with L, M, N
- **Validation time**: any change MUST be validated against L, M, N before acceptance
- **Fail-closed**: if any rule is violated, the operation MUST be rejected and a corrected version MUST be proposed; no partial acceptance is allowed
