## ADDED Requirements

### Requirement: Mock-first testing strategy
All unit tests SHALL use mocked dependencies. No test SHALL access real network sockets, disk I/O, databases, message brokers (Kafka), or external HTTP endpoints. Every dependency crossing a layer boundary MUST be mockable via a trait.

#### Scenario: Command handler test uses mocked event store
- **WHEN** a test exercises a command handler that depends on `EventStore`
- **THEN** the test SHALL inject a mock `EventStore` and MUST NOT connect to any real database or Kafka

#### Scenario: Query handler test uses mocked query bus
- **WHEN** a test exercises a query handler
- **THEN** all its dependencies SHALL be mocks implementing the relevant traits

#### Scenario: Transport handler test uses mocked application traits
- **WHEN** a test exercises an HTTP handler (e.g., `/hello`)
- **THEN** the test SHALL inject mock `QueryBus`/`CommandBus` and MUST NOT bind a real TCP port

### Requirement: 95% minimum test coverage
The project-wide test coverage SHALL be at least 95% as measured by `cargo-tarpaulin`. Coverage SHALL be enforced in CI — a build MUST fail if coverage falls below 95%.

#### Scenario: CI rejects low coverage
- **WHEN** a pull request is submitted with coverage below 95%
- **THEN** the CI pipeline SHALL fail and block the merge

#### Scenario: CI passes at or above 95%
- **WHEN** a pull request is submitted with coverage at or above 95%
- **THEN** the CI pipeline SHALL pass the coverage check

### Requirement: No real resources in test suite
The test suite SHALL NOT require any external infrastructure to run. Running `cargo test` SHALL complete successfully without: a running Kafka cluster, a database connection, an HTTP server, filesystem writes outside temp directories, or any network access.

#### Scenario: cargo test runs offline
- **WHEN** `cargo test` is executed with no network connectivity and no external services running
- **THEN** all tests SHALL pass

#### Scenario: No database connection strings in test code
- **WHEN** the test suite is scanned for connection strings or URLs
- **THEN** no test SHALL contain database URLs, Kafka bootstrap servers, or external HTTP endpoints

### Requirement: Trait-based mocking infrastructure
Every trait that crosses an architectural boundary SHALL be annotated with `#[automock]` (from the `mockall` crate) or provide an equivalent manual mock implementation. This SHALL apply to: `CommandBus`, `QueryBus`, `EventStore`, and any future port traits.

#### Scenario: Trait is mockable
- **WHEN** a developer needs to mock a port trait in a test
- **THEN** the trait SHALL support `#[automock]` or have a documented mock implementation

#### Scenario: New port trait added without mock support
- **WHEN** a new port trait is introduced without mock support
- **THEN** the CI SHALL fail because the trait cannot be tested in isolation per governance rules

### Requirement: Test file co-location
Unit tests SHALL be co-located with the code they test using Rust's `#[cfg(test)] mod tests` pattern. Each source file SHALL contain its own test module covering all public and critical private functions.

#### Scenario: Source file has corresponding test module
- **WHEN** a source file `src/lib.rs` or `src/module.rs` is inspected
- **THEN** it SHALL contain a `#[cfg(test)] mod tests { ... }` block