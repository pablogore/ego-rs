## ADDED Requirements

### Requirement: Hexagonal architecture layers
Every crate in the project SHALL belong to exactly one architectural layer: domain, application, infrastructure, or transport. Dependencies between layers MUST flow inward: transport → application → domain, and infrastructure → domain. No layer SHALL depend on a layer further outward.

#### Scenario: Transport depends on application and domain
- **WHEN** the transport crate declares its dependencies in `Cargo.toml`
- **THEN** it SHALL depend on `application` and `domain`, and MUST NOT depend on `infrastructure`

#### Scenario: Domain has no internal project dependencies
- **WHEN** the domain crate declares its dependencies in `Cargo.toml`
- **THEN** it SHALL NOT depend on `application`, `infrastructure`, or `transport`

#### Scenario: Infrastructure depends on domain
- **WHEN** the infrastructure crate declares its dependencies in `Cargo.toml`
- **THEN** it SHALL depend on `domain` and MAY depend on `application`, and MUST NOT depend on `transport`

### Requirement: Ports and adapters pattern
Every external concern (HTTP, message brokers, databases, file system) SHALL be accessed through a trait defined in the domain or application layer. Concrete implementations SHALL live in the infrastructure layer. Application code MUST depend only on traits, never on concrete infrastructure types.

#### Scenario: Event store accessed through trait
- **WHEN** a command handler needs to persist events
- **THEN** it SHALL receive an `EventStore` trait object, not a concrete `PostgresEventStore` or `KafkaEventStore`

#### Scenario: HTTP handler receives trait, not concrete type
- **WHEN** a transport handler is constructed
- **THEN** it SHALL receive `QueryBus` and `CommandBus` trait objects, not concrete bus implementations

### Requirement: SOLID principles compliance
Every new type introduced in the project SHALL comply with SOLID principles. Specifically: single responsibility (each struct/enum has one reason to change), open/closed (behavior extended via traits, not modification), Liskov substitution (trait implementations are substitutable), interface segregation (traits are minimal), and dependency inversion (depend on abstractions, not concretions).

#### Scenario: Trait has single concern
- **WHEN** a trait is defined in domain or application layer
- **THEN** it SHALL have at most 3 methods directly related to a single concern

#### Scenario: New behavior added via new implementation
- **WHEN** a new storage backend is added (e.g., PostgreSQL event store)
- **THEN** it SHALL implement the existing `EventStore` trait without modifying the trait itself

### Requirement: No cross-layer imports in application code
Application layer code (command handlers, query handlers, use cases) SHALL NOT import types from infrastructure or transport layers. Application code SHALL only reference domain types and application-level trait abstractions.

#### Scenario: Handler imports only domain and application traits
- **WHEN** a query handler file is inspected for `use` statements
- **THEN** all imports SHALL be from `domain::` or `application::` modules, never from `infrastructure::` or `transport::`