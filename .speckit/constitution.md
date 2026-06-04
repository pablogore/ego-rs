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
