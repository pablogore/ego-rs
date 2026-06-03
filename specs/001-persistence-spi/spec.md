# Persistence SPI — Requirements

**Feature Branch**: `003-persistence-spi`

**Created**: 2026-06-03

**Status**: Draft

**Input**: User description: "Define the persistence Service Provider Interface (SPI) as domain-owned contracts for event sourcing and aggregate persistence."

## User Scenarios & Testing

### User Story 1 - Define EventStore trait for event sourcing (Priority: P1)

A framework developer defines an `EventStore` trait in the domain layer that supports appending and loading events for tenant-scoped aggregate streams.

**Why this priority**: Event sourcing is the core persistence primitive the framework depends on.

**Independent Test**: Can be fully tested by instantiating an in-memory implementation and verifying append/load round-trips deliver the correct events.

**Acceptance Scenarios**:

1. **Given** an empty event stream for aggregate A in tenant T, **When** a batch of events is appended, **Then** the new stream version is returned
2. **Given** events have been appended to aggregate A in tenant T, **When** the stream is loaded, **Then** all events are returned in append order
3. **Given** events exist in tenant T1 for aggregate A, **When** loading from tenant T2 for aggregate A, **Then** no events are returned
4. **Given** `tenant_id` is `None` (single-tenant mode), **When** events are appended and loaded, **Then** the full stream is returned without tenant scoping

---

### User Story 2 - Define Repository trait for aggregate persistence (Priority: P1)

A framework developer defines a `Repository` trait in the domain layer that supports saving, loading, and deleting aggregates with tenant isolation.

**Why this priority**: This is the generic aggregate persistence contract required by application services.

**Independent Test**: Can be fully tested with an in-memory repository — save an aggregate, load it back, verify state matches, then delete and verify it is gone.

**Acceptance Scenarios**:

1. **Given** an aggregate with a specific ID in tenant T, **When** the aggregate is saved, **Then** it can be loaded back with matching state
2. **Given** a saved aggregate, **When** it is deleted, **Then** loading returns a not-found error
3. **Given** aggregates exist in tenant T1 and T2, **When** loading by ID, **Then** results are scoped to the requesting tenant
4. **Given** `tenant_id` is `None` (single-tenant mode), **When** an aggregate is saved and loaded, **Then** it is accessible without tenant scoping

---

### User Story 3 - Define PersistenceError for failure classification (Priority: P2)

A framework developer defines typed errors that distinguish between not-found, concurrency conflict, missing tenant, and internal failures.

**Why this priority**: Error classification is required before any production use, but basic IN PROGRESS/FAILURE reporting works without it.

**Independent Test**: Each error variant can be constructed and matched independently without a backing store.

**Acceptance Scenarios**:

1. **Given** a missing aggregate ID, **When** load is attempted, **Then** `PersistenceError::NotFound` is returned
2. **Given** a version mismatch, **When** append/save is attempted, **Then** `PersistenceError::Conflict` is returned
3. **Given** an empty or missing `tenant_id` in multi-tenant mode, **When** any persistence operation is attempted, **Then** `PersistenceError::MissingTenant` is returned
4. **Given** `tenant_id` is `None` (single-tenant mode), **When** any persistence operation is attempted, **Then** no tenant-related error is raised

---

### User Story 4 - Account for snapshot persistence (Priority: P2)

A framework developer defines a `Snapshot` trait in the domain layer for optional aggregate snapshotting.

**Why this priority**: Snapshots are an optimization — the system works without them, but performance degrades for long aggregate streams.

**Independent Test**: Save a snapshot at a known version, load it back, and verify version and payload match.

**Acceptance Scenarios**:

1. **Given** a snapshot saved at version N, **When** loading the latest snapshot, **Then** the snapshot with version N is returned
2. **Given** multiple snapshots for the same aggregate, **When** loading, **Then** only the latest (highest version) is returned
3. **Given** no snapshot exists, **When** loading, **Then** `None` is returned

---

### User Story 5 - Multi-tenant isolation (Priority: P1)

All persistence operations accept an optional `tenant_id`. When provided, same aggregate IDs in different tenants are isolated. When `None`, single-tenant mode applies.

**Why this priority**: Multi-tenancy is a core architectural requirement — without it, data leaks between tenants. The SPI must support both modes.

**Independent Test**: Create the same aggregate ID in two different tenants, write different data, verify reads return tenant-local data only. Then repeat with `tenant_id = None` and verify cross-scope reads succeed.

**Acceptance Scenarios**:

1. **Given** aggregate A exists in tenant T1 and tenant T2, **When** reading from T1, **Then** only T1's data is visible
2. **Given** an empty or missing `tenant_id` in multi-tenant mode, **When** any persistence operation is attempted, **Then** the operation fails with `PersistenceError::MissingTenant`
3. **Given** `tenant_id` is `None` (single-tenant mode), **When** any persistence operation is attempted, **Then** the operation succeeds without tenant scoping

---

### User Story 6 - Migration infrastructure (Priority: P3)

Concrete persistence backends own schema evolution. Shared migration infrastructure supports versioned, idempotent, deterministic migrations with startup validation.

**Why this priority**: Schema management is required before production deployment, but the SPI still functions for in-memory/testing use without it.

**Independent Test**: Register two versioned migrations, run them, verify they execute in order. Run again and verify idempotency.

**Acceptance Scenarios**:

1. **Given** migrations with versions 1 and 2, **When** migrations are executed, **Then** version 1 runs first, then version 2
2. **Given** already-applied migrations, **When** migrations are executed again, **Then** they are skipped (idempotent)
3. **Given** pending migrations, **When** the system starts, **Then** startup is rejected until migrations are applied

---

### Edge Cases

- What happens when an aggregate stream contains millions of events — how is loading bounded?
- How does the system behave when a `tenant_id` is provided but the tenant does not yet exist in the backing store?
- When `tenant_id` is `None` (single-tenant mode), how does the SPI differentiate between data from a previous multi-tenant session and single-tenant data?
- What happens when two concurrent writers attempt to append to the same aggregate stream at the same version?
- How does the system handle corrupted or unparseable event data in an existing stream?
- What happens when a snapshot exists but the event stream since the snapshot is empty?
- How does the `list_aggregate_ids` operation behave in a tenant with zero aggregates?

## Requirements

### Functional Requirements

- **FR-001 (EventStore)**: The domain SHALL define an `EventStore` trait for event sourcing persistence with `append`, `load`, and `list_aggregate_ids` operations scoped by aggregate ID and tenant.
- **FR-002 (EventStore — append)**: `append(aggregate_id, tenant_id, expected_version, events)` SHALL persist events to a tenant-scoped stream and return the new stream version. It SHALL fail with a conflict error if `expected_version` does not match the current stream version.
- **FR-003 (EventStore — load)**: `load(aggregate_id, tenant_id)` SHALL return all events for an aggregate in a tenant, ordered by append sequence.
- **FR-004 (EventStore — list)**: `list_aggregate_ids(tenant_id)` SHALL return all aggregate IDs that have events in the given tenant.
- **FR-005 (Repository)**: The domain SHALL define a `Repository` trait for generic aggregate persistence with `save`, `load`, and `delete` operations.
- **FR-006 (Repository — save)**: `save(aggregate, tenant_id, expected_version)` SHALL persist aggregate state and return the new version. It SHALL fail with a conflict error on version mismatch.
- **FR-007 (Repository — load)**: `load(aggregate_id, tenant_id)` SHALL return the aggregate state or fail with not-found.
- **FR-008 (Repository — delete)**: `delete(aggregate_id, tenant_id)` SHALL remove the aggregate from the store.
- **FR-009 (Snapshot)**: The domain SHALL define a `Snapshot` trait for optional aggregate snapshot persistence with `save_snapshot` and `load_snapshot` operations.
- **FR-010 (Snapshot — save)**: `save_snapshot(aggregate_id, tenant_id, version, payload)` SHALL persist a snapshot at a specific aggregate version.
- **FR-011 (Snapshot — load)**: `load_snapshot(aggregate_id, tenant_id)` SHALL return the latest snapshot (highest version) or `None` if none exists.
- **FR-012 (PersistenceError)**: The domain SHALL define a `PersistenceError` enum with variants: `NotFound`, `Conflict` (including expected/actual version), `MissingTenant`, and `Internal`.
- **FR-013 (Multi-tenant)**: All persistence operations SHALL accept an optional `tenant_id` (`Option<&str>`). When `Some(tenant_id)`, event streams, repository data, and snapshots SHALL be tenant-scoped and isolated. When `None`, the system operates in single-tenant mode with no isolation boundaries.
- **FR-014 (No cross-tenant reads)**: Implementations MUST NOT return data from tenants other than the one specified in the request.
- **FR-015 (Fail closed on missing tenant)**: When multi-tenant mode is active (`Some(tenant_id)` with empty/blank value), operations SHALL fail with `PersistenceError::MissingTenant`. When `tenant_id` is `None` (single-tenant mode), operations SHALL proceed without tenant scoping.
- **FR-016 (Migrations)**: Concrete persistence implementations SHALL own their schema evolution with versioned, deterministic, idempotent migrations and startup validation.
- **FR-017 (Production backend)**: The system SHALL provide at least one production-grade persistence backend that implements all SPI operations.

### Key Entities

- **Aggregate**: A domain entity with identity, state, and a version tracked by the persistence layer.
- **Event**: A record of something that happened to an aggregate, implementing the `DomainEvent` contract.
- **Tenant**: An optional isolation boundary — when provided, all persistence operations are scoped to a single tenant. When `None`, the system operates in single-tenant mode.
- **Snapshot**: A cached representation of aggregate state at a specific version for read optimization.
- **Stream**: An ordered sequence of events for a single aggregate within a tenant.

## Contract Invariants

The following behavioral guarantees apply to all SPI trait implementations.

### Ordering

- Events appended to a stream MUST be loaded in the same order they were appended.
- Snapshots MUST return the version with the highest version number on load.
- Migrations MUST execute in monotonically increasing version order.

### Atomicity

- `append` MUST either persist all provided events or none (no partial writes).
- `save` MUST persist the aggregate state atomically or fail without side effects.
- Concurrent conflicting writes MUST NOT produce partial or interleaved state.

### Concurrency

- Optimistic concurrency MUST be enforced via `expected_version`: if the current version differs, the operation MUST fail with `PersistenceError::Conflict`.
- The version returned by `append`/`save` MUST reflect the stream/aggregate version after a successful write.

### Consistency

- After a successful `append`/`save`, a subsequent `load` with the same `aggregate_id` and `tenant_id` MUST reflect the written data (read-your-writes consistency).
- `delete` MUST make the aggregate inaccessible to subsequent `load` calls.

### Error Translation

- All infrastructure-level errors (connection failures, timeouts, serialization errors) MUST be translated to `PersistenceError::Internal` at the SPI boundary.
- `PersistenceError` variants MUST NOT expose underlying implementation types.

### Empty-State Behavior

- `load` on a non-existent aggregate or stream MUST return `PersistenceError::NotFound`.
- `load_snapshot` when no snapshot exists MUST return `Ok(None)` — never an error.
- `list_aggregate_ids` in a tenant with no aggregates MUST return an empty collection.
- In single-tenant mode (`tenant_id` is `None`), `list_aggregate_ids` MUST return all aggregates in the default scope.

### Tenant Isolation

- When `tenant_id` is `Some(id)`, implementations MUST guarantee that data written under one `tenant_id` is not observable under any other `tenant_id`.
- When `tenant_id` is `None` (single-tenant mode), no tenant isolation is applied — all data shares a single default scope.
- In multi-tenant mode, operations with an empty or absent `tenant_id` value MUST fail with `PersistenceError::MissingTenant`.

## Constraints

- Domain contracts MUST be runtime-neutral — no async, no runtime-specific types in trait signatures.
- Implementations MAY add runtime-specific behavior behind the SPI boundary.
- Migrations MUST be infrastructure-owned — never in domain.
- No database-specific types MAY appear in SPI trait signatures.
- The SPI MUST support at least a reference implementation and a production-grade implementation.

## Out of Scope

The following concerns are NOT addressed by this specification and belong in future specs:

- Concrete persistence implementations (PostgreSQL adapter, SQLite adapter, etc.) — these will be specified in separate infrastructure specs.
- Migration scripts and database schema designs — the SPI defines the migration infrastructure contract, not specific migration content.
- Connection pooling, transaction management, or retry logic — these are infrastructure concerns outside the SPI contract.
- Performance optimization — caching strategies, batch loading, read-model projections, and query optimization are separate concerns.
- Event serialization format beyond standard JSON — schema evolution of event data is out of scope.
- Cross-tenant queries or administrative operations.
- Event replay, stream subscriptions, or projections (eventual consistency patterns).
- Data retention policies, archival, or deletion of historical data.
- Monitoring, metrics, or observability hooks within the persistence layer.
- Integration with specific application frameworks (Actix, Axum, etc.) — the SPI is framework-agnostic.

## Clarifications

### Session 2026-06-03

- Q: How should the SPI handle multi-tenancy as an optional feature? → A: `tenant_id` becomes an optional parameter; when absent, single-tenant mode applies (default scope, no isolation). When present, full isolation is enabled.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A developer can write an in-memory implementation of all SPI traits and verify correct behavior within a single test session.
- **SC-002**: Tenant isolation is verifiable in under 5 minutes by writing the same aggregate ID in two tenants and confirming isolated reads. Single-tenant mode (`tenant_id` = `None`) is verifiable by performing the same operations without tenant scoping.
- **SC-003**: All error variants (`NotFound`, `Conflict`, `MissingTenant`, `Internal`) are exercised by test scenarios with no implementation-specific dependencies.
- **SC-004**: Contract invariants are validated by a shared contract test suite that any new backend implementation can run to prove compliance.

## Assumptions

- A `DomainEvent` contract exists and defines the event contract the SPI depends on.
- Tenant identity is provided by the application layer — the SPI does not authenticate or validate tenants, only scopes by the provided identifier.
- Actor identity types are available for use as aggregate identifiers.
- The initial focus is on the SPI definition only — concrete backends are not part of this spec.
- Serialization support exists within the domain layer.
