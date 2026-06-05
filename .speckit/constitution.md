# Engineering Quality Standards

## Principle: Test First Development

All production code MUST be developed using Test-Driven Development (TDD).

Required workflow:

1. Write a failing test.
2. Verify the test fails.
3. Implement the minimal code required.
4. Verify the test passes.
5. Refactor while keeping tests green.

Implementation without prior tests is a constitution violation.

---

## Principle: Minimum Coverage

All modified or newly created production code MUST maintain:

- Line Coverage >= 85%
- Branch Coverage >= 85%

Pull requests or changes below these thresholds MUST NOT be considered complete.

Coverage exceptions require explicit approval and written justification.

---

## Principle: No Real Infrastructure in Unit Tests

Unit tests MUST NOT access real external resources.

Forbidden:

- Real databases
- Real Kafka clusters
- Real NATS servers
- Real Redis instances
- Real HTTP APIs
- Real gRPC services
- Real filesystems
- Real cloud services

Unit tests MUST execute completely offline and deterministically.

---

## Principle: Mock-Based Isolation

All external dependencies MUST be isolated through interfaces, traits, ports, or adapters.

Unit tests MUST use:

- Mocks
- Stubs
- Fakes
- Test doubles

instead of real infrastructure.

Example:

Allowed:
- MockRepository
- MockEventStore
- MockMessageBus

Forbidden:
- PostgreSQL container
- Embedded Kafka
- Local Redis
- Test cloud accounts

---

## Principle: Deterministic Test Execution

Tests MUST:

- Produce identical results on every execution.
- Avoid time-based flakiness.
- Avoid network dependencies.
- Avoid environment-specific behavior.

Tests depending on timing, external services, or network availability are constitution violations.

---

## Principle: Testability by Design

New production code MUST be designed for testability.

Required:

- Constructor injection
- Dependency inversion
- Interface-driven design

Forbidden:

- Hidden singletons
- Global mutable state
- Hardcoded infrastructure dependencies

---

## Principle: Functional Programming

The codebase SHALL prefer functional programming techniques where practical.

Guidelines:

- Functions are first-class citizens
- Prefer pure functions
- Prefer immutable data
- Minimize shared mutable state
- Minimize side effects
- Side effects SHOULD be isolated at system boundaries
- Prefer composition over inheritance
- Prefer explicit inputs and outputs over hidden dependencies

Preferred:

```rust
fn calculate_total(items: &[Item]) -> Money
```

Avoid:

```rust
struct CartService {
    state: RefCell<State>,
}
```

unless mutable state is required by the runtime model.

---

## Principle: Deterministic Business Logic

Business logic MUST be deterministic.

Avoid:

- Hidden global state
- Current time lookups inside domain logic
- Random number generation inside domain logic
- Network access inside domain logic

Inject dependencies instead.

---

## Principle: Rustdoc Documentation

All public Rust APIs MUST include rustdoc documentation.

Requirements:

- Public structs
- Public enums
- Public traits
- Public functions
- Public modules

Documentation MUST explain purpose and usage.

---

## Compliance

A change is considered complete only when:

- All tests pass
- Coverage >= 85%
- No real infrastructure is accessed by unit tests
- All external dependencies are mocked or replaced by test doubles
- CI validation succeeds

Failure to satisfy any requirement constitutes a governance violation.

---

# Deterministic Execution Governance

## Rule 1: Single Source Of Truth

There SHALL be exactly one active feature.

The active feature SHALL be resolved only from:

```text
.speckit/state.yaml
```

Example:

```yaml
active_feature: 003-effect-api
status: implementation
```

The following files MUST NOT be used to resolve the active feature:

- AGENTS.md
- README.md
- plan.md
- tasks.md

These files are informational only.

---

## Rule 2: Executable Tasks

Every task MUST contain:

- exact file path
- modification type
- target symbol
- expected outcome
- validation criteria

Required format:

```markdown
- [ ] T012

  File:
  crates/runtime/src/context.rs

  Operation:
  Modify

  Symbol:
  RuntimeExecutionContext

  Expected Outcome:
  Rename CommandContext to RuntimeExecutionContext

  Validation:
  cargo test -p runtime
```

Tasks lacking these fields are invalid.

---

## Rule 3: Evidence Required For Completion

A task MUST NOT be marked [X] without evidence.

Required evidence:

```yaml
evidence:
  command: cargo test --workspace
  exit_code: 0
```

Examples:

```yaml
evidence:
  command: cargo test -p domain
  exit_code: 0
```

```yaml
evidence:
  command: cargo clippy --workspace
  exit_code: 0
```

Without evidence:

```text
[X]
```

is prohibited.

---

## Rule 4: Fail Closed Completion

Implementation completion claims MUST be backed by evidence.

Prohibited:

- "Implementation complete"
- "All tasks completed"
- "Feature implemented"
- "Ready to archive"

unless evidence exists.

Required:

```text
Task completion:
T012 -> evidence present

Task completion:
T013 -> evidence missing

Task status:
Remain [ ]
```

---

## Rule 5: Deterministic Workflow

Speckit SHALL operate using explicit workflow stages.

Allowed flow:

```text
/specify
    ↓
/clarify
    ↓
/plan
    ↓
/tasks
    ↓
/implement
    ↓
/review
    ↓
/archive
```

Commands MUST NOT regenerate previous artifacts unless explicitly requested.

Example:

```text
/implement
```

MUST NOT regenerate:

- spec.md
- plan.md
- tasks.md

---

## Rule 6: Implementation Ownership

Implementation tasks MUST specify ownership.

Required fields:

```text
Create
Modify
Refactor
Delete
```

Examples:

```text
Operation: Create
```

```text
Operation: Modify
```

```text
Operation: Refactor
```

```text
Operation: Delete
```

Generic descriptions are prohibited.

---

## Rule 7: Symbol-Level Precision

Every implementation task MUST identify the target symbol.

Examples:

```text
Struct:
RuntimeExecutionContext
```

```text
Trait:
ExecutionContext
```

```text
Enum:
Effect
```

```text
Function:
flatten_effects
```

File-level instructions alone are insufficient.

---

## Rule 8: Archive Gate

A feature SHALL NOT be archived unless:

- all tasks are [X]
- all tasks have evidence
- coverage >= 85%
- cargo test passes
- cargo clippy passes
- cargo fmt passes

Archive readiness requires:

```yaml
archive_check:
  tasks_complete: true
  evidence_complete: true
  coverage: >=85
  tests_passed: true
  clippy_passed: true
  fmt_passed: true
```

---

## Rule 9: Local Model Optimization

Speckit SHALL prefer deterministic instructions over reasoning-heavy prompts.

Prefer:

```text
Modify file X
Update symbol Y
Run validation Z
```

Avoid:

```text
Analyze
Review
Think deeply
Explore alternatives
Generate coverage map
```

unless explicitly executing /clarify.
>>>>>>> origin/develop
