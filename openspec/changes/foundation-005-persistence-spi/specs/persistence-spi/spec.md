## ADDED Requirements

### Requirement: Persistence abstraction model

The platform SHALL define persistence as a semantic durability contract — not a storage engine, ORM, repository, database abstraction, or serialization framework.

Persistence SHALL be responsible for:
- Durable recording and retrieval of state and events
- Providing deterministic guarantees for all persistence operations
- Supporting restoration of previously persisted state and events
- Enforcing fail-closed behavior on all persistence operations

Persistence SHALL NOT be responsible for:
- Business orchestration semantics
- Transport or network protocol management
- Workflow or saga execution
- Actor scheduling or lifecycle management
- Exposing vendor-specific storage primitives
- Exposing runtime-specific primitives
- Serialization format specification or enforcement

Persistence invariants:
- Persistence MUST have no observable side effects beyond the storage boundary
- Persistence MUST be replayable: identical inputs produce identical persistence outcomes
- Persistence MUST be storage-neutral: no constitutional requirement references a specific storage technology
- Persistence MUST be runtime-neutral: no constitutional requirement references a specific runtime
- Persistence MUST be representation-neutral: no constitutional requirement assumes byte, blob, payload, or serialization representation

#### Scenario: Persistence contract is semantic not mechanical

- **WHEN** a component depends on the Persistence SPI
- **THEN** it SHALL depend on semantic guarantees, not storage APIs, SQL, ORM methods, or repository interfaces

#### Scenario: Persistence non-responsibilities are enforceable

- **WHEN** a persistence operation completes
- **THEN** it SHALL NOT have triggered business orchestration, transport, workflow execution, or actor scheduling

### Requirement: Unified Persistence Contract

The platform SHALL expose a single coherent semantic persistence contract independent of storage realization. Storage realization MAY vary while the contractual surface remains identical.

The unified contract SHALL govern all persistence operations regardless of the underlying storage class. Storage classes (NON-NORMATIVE examples only) include relational, columnar, append-only, document, in-memory, and object persistence.

The unified contract SHALL NOT introduce vendor-specific, storage-class-specific, or runtime-specific semantics.

#### Scenario: Single contract across storage realizations

- **WHEN** a component depends on the Persistence SPI
- **THEN** it SHALL interact with a single coherent contract independent of which storage adapter is configured

### Requirement: Durability semantics

Persistence SHALL define explicit durability semantics with no ambiguity about persistence outcomes within the declared durability boundary.

Durability guarantees:
- A persist operation SHALL provide a deterministic acknowledgment indicating whether persistence artifacts have been persisted within the declared durability boundary
- The acknowledgment SHALL be unambiguous: either Persisted or Failed, with no intermediate state visible to the caller
- Durability visibility SHALL be bounded: the caller SHALL know the durability scope of the acknowledgment (e.g., single entity, batch, transaction)
- Silent durability ambiguity SHALL be forbidden: a persist operation MUST NOT return success if durability is not guaranteed
- Durability boundaries SHALL be explicit: the contract SHALL define what durability means within the declared boundary; durability realization is implementation-defined

Persistence SPI SHALL support multiple durability realizations including durable, in-memory, deterministic replay, transient workflow, and ephemeral persistence. Each realization SHALL explicitly declare its durability boundary semantics. Observable guarantees MUST remain deterministic within the declared boundary. Persistence SPI defines durability semantics, not durability implementation.

#### Scenario: Explicit durability acknowledgment

- **WHEN** a persist operation completes with success
- **THEN** the caller SHALL receive a deterministic acknowledgment that persistence artifacts are persisted within the declared durability boundary

#### Scenario: No silent durability ambiguity

- **WHEN** a persist operation encounters an ambiguous outcome (e.g., timeout, partial write, disconnected storage)
- **THEN** the operation SHALL fail closed — returning a failure acknowledgment — and MUST NOT indicate success

### Requirement: State persistence semantics

The platform SHALL define state persistence semantics supporting durable recording and deterministic restoration of actor and application state.

State persistence guarantees:
- State SHALL be persistable as a deterministic operation given identical inputs and identical state
- Restored state SHALL be identical to the state at the time of persistence, within the committed durability boundary
- State persistence SHALL NOT assume any specific storage representation (relational, document, key-value, append-only)
- State persistence SHALL support deterministic recovery: given identical persisted state, restoration SHALL produce identical outcomes
- State persistence SHALL be lifecycle-aware: the relationship between state persistence and entity lifecycle SHALL be explicit and deterministic

#### Scenario: Deterministic state persistence

- **WHEN** identical state is persisted with identical persistence context
- **THEN** the observable persistence outcome SHALL be identical

#### Scenario: Deterministic state restoration

- **WHEN** previously persisted state is restored
- **THEN** the restored state SHALL be identical to the state at the time of persistence

#### Scenario: Storage-neutral state persistence

- **WHEN** state persistence semantics are evaluated
- **THEN** no constitutional requirement SHALL reference a specific storage technology or storage representation

### Requirement: Event persistence semantics

The platform SHALL define event persistence semantics supporting durable recording of events with ordering guarantees and replayability. Event persistence MAY be realized through append-only storage but is not defined by it.

Event persistence guarantees:
- Events SHALL be persistable in append-only fashion — once persisted, events MUST NOT be mutated or deleted
- Event ordering SHALL be preserved: the order in which events were persisted SHALL be recoverable on replay
- Events SHALL be replayable: given identical events in identical order, replay SHALL produce identical observable outcomes
- Event persistence SHALL support idempotency expectations: persisting the same event twice SHALL produce deterministic behavior (either deduplicated or acknowledged as duplicate)
- Event persistence is a supported constitutional semantic. When event persistence is supported, append-only guarantees, ordering guarantees, replayability, and idempotency expectations SHALL apply. Persistence realizations that do not support event persistence remain compliant — state persistence is the universal minimum
- Event persistence SHALL NOT redefine Persistence SPI itself. Persistence SPI remains valid for actor persistence, workflow persistence, service persistence, process persistence, projection persistence, and state persistence independent of event persistence semantics
- Event persistence SHALL NOT define CQRS, Event Store, or event sourcing semantics — these are higher-order patterns that consume event persistence without being defined by it

#### Scenario: Append-only event persistence

- **WHEN** an event is persisted
- **THEN** it SHALL be immutable after persistence — no mutation or deletion

#### Scenario: Event ordering preservation

- **WHEN** events are persisted sequentially
- **THEN** their relative order SHALL be preserved and recoverable on replay

#### Scenario: Deterministic event replay

- **WHEN** identical events in identical order are replayed
- **THEN** the replay outcome SHALL be identical

### Requirement: Snapshot semantics

The platform SHALL define snapshot semantics establishing constitutional boundaries for state snapshots without prescribing storage mechanics.

Snapshot guarantees:
- A snapshot SHALL capture a consistent view of state at a deterministic point in time
- Snapshot restoration SHALL produce state identical to the state at the snapshot point
- Snapshot consistency SHALL be explicit: the contract SHALL define what "consistent" means within the declared persistence boundary
- Snapshot determinism SHALL be preserved: given identical pre-snapshot state and identical uncommitted inputs, snapshot outcomes SHALL be identical
- Snapshot lifecycle SHALL be constitutional — creation, validation, and restoration expectations — but storage lifecycle mechanics (e.g., compaction, pruning, archiving) SHALL be implementation concerns

#### Scenario: Snapshot state consistency

- **WHEN** a snapshot is created
- **THEN** it SHALL capture state that is internally consistent at a deterministic point

#### Scenario: Deterministic snapshot restoration

- **WHEN** a snapshot is restored
- **THEN** the restored state SHALL be identical to the state at the snapshot creation point

### Requirement: Replay persistence semantics

The platform SHALL define replay persistence semantics ensuring deterministic replay through persisted state and events.

Replay guarantees:
- Replay SHALL restore the system to the state that existed at a given deterministic point
- Replay SHALL preserve deterministic observable outcomes: replaying identical persisted inputs SHALL produce identical observable output
- Replay consistency SHALL be bounded: the contract SHALL define what consistency means within the declared replay boundary
- Replay visibility SHALL be explicit: the contract SHALL define what is observable during and after replay
- Replay SHALL fail closed on ambiguity: if replay cannot be completed deterministically, the system MUST NOT enter an ambiguous state

#### Scenario: Deterministic replay outcome

- **WHEN** identical persisted inputs are replayed
- **THEN** the observable outcomes SHALL be identical

#### Scenario: Fail-closed replay ambiguity

- **WHEN** replay cannot complete deterministically (e.g., missing events, corrupted state)
- **THEN** the system SHALL fail closed and MUST NOT produce non-deterministic output

### Requirement: Tenant Isolation Semantics

Persistence SHALL support single-tenant and multi-tenant configurations through constitutional tenant isolation semantics.

Tenant boundaries SHALL be:
- Explicit: the tenant context SHALL be identifiable for each persistence operation
- Observable: tenant isolation behavior SHALL be verifiable through the contract
- Deterministic: given identical tenant context, persistence outcomes SHALL be identical

Persistence realization MAY implement logical tenant isolation, physical tenant isolation, or hybrid isolation. Constitutional contracts MUST NOT assume schema-per-tenant, database-per-tenant, shared table models, or vendor-specific tenant strategies.

Persistence SPI SHALL support:
- Tenant-scoped persistence boundaries
- Deterministic tenant restoration
- Tenant context visibility through the persistence contract

When tenant portability exists, tenant boundary determinism MUST be preserved, tenant context integrity MUST be preserved, and observable persistence guarantees MUST remain deterministic. Tenant portability MUST NOT violate isolation guarantees, restoration guarantees, replay determinism, or tenant-scoped persistence boundaries.

#### Scenario: Deterministic tenant boundaries

- **WHEN** a persistence operation is performed within a tenant context
- **THEN** the operation SHALL respect the tenant's isolation boundary and SHALL NOT leak data across tenant contexts

#### Scenario: Tenant implementation neutrality

- **WHEN** tenant isolation semantics are evaluated
- **THEN** no constitutional requirement SHALL assume a specific tenant isolation implementation (schema-per-tenant, database-per-tenant, or shared table)

### Requirement: Persistence Evolution Semantics

Persistence realizations MUST support deterministic persistence evolution. Evolution MUST be self-contained, reproducible, and deterministic.

Persistence consumers MUST NOT depend on manual infrastructure coordination, out-of-band migration steps, or vendor-specific migration tooling.

This requirement defines persistence evolution semantics — NOT database migrations, schema tooling, DDL, or SQL migration engines. Evolution semantics SHALL ensure that persisted artifacts can be evolved to a newer contract without breaking determinism or requiring non-deterministic manual steps.

#### Scenario: Self-contained evolution

- **WHEN** persistence evolution occurs
- **THEN** it SHALL complete without requiring manual coordination or out-of-band infrastructure changes

#### Scenario: Deterministic evolution

- **WHEN** identical evolution inputs are applied to identical persistence state
- **THEN** the evolution outcome SHALL be identical

### Requirement: Persistence versioning semantics

The platform SHALL define constitutional versioning semantics for persistence artifacts. Versioning SHALL address evolution of state schemas, event schemas, and snapshot formats without coupling to vendor migration tooling, DDL, or SQL.

Versioning guarantees:
- Persistence artifacts SHALL carry an explicit version identifier that is deterministic and reproducible given identical artifact content and identical versioning context
- Version identifiers SHALL be comparable: given two version identifiers, it SHALL be possible to determine equality and relative ordering
- Version-aware persistence SHALL record the artifact version alongside the artifact content; version context SHALL be preserved across persistence and restoration
- Version compatibility SHALL be explicit: the contract SHALL define what compatibility means for each versioned artifact class (state schema, event schema, snapshot format)
- Version mismatch SHALL fail closed: if a persisted artifact cannot be restored because its version is incompatible with the current contract, the restoration SHALL fail deterministically rather than producing silently corrupted or degraded state
- Version-aware restoration SHALL validate version compatibility before restoring; incompatible versions MUST NOT produce usable restored state
- Versioning SHALL be deterministic: given identical artifact content and identical versioning context, version identifiers, compatibility outcomes, and mismatch handling SHALL be identical

Versioning SHALL NOT define:
- Schema definition languages, DDL, or data definition formats
- Migration scripts, transformation pipelines, or upgrade sequences
- Vendor-specific versioning schemes or format registries
- Serialization format or encoding for version identifiers
- Automatic migration on version mismatch

#### Scenario: Version-aware persistence
- **WHEN** a persistence artifact is persisted
- **THEN** its version identifier SHALL be recorded and verifiable on restoration

#### Scenario: Fail-closed version mismatch
- **WHEN** a persisted artifact has a version incompatible with the current contract
- **THEN** restoration SHALL fail deterministically and MUST NOT produce usable state

#### Scenario: Deterministic versioning
- **WHEN** identical artifact content is persisted twice with identical versioning context
- **THEN** the version identifier SHALL be identical

### Requirement: Persistence capability model

The platform SHALL define a capability model distinguishing mandatory, optional, and forbidden persistence capabilities.

Mandatory capabilities:
- Deterministic durability visibility: caller SHALL know whether persistence artifacts are persisted within the declared durability boundary
- Tenant boundary awareness: persistence SHALL preserve deterministic tenant isolation boundaries
- Deterministic persistence evolution compatibility: persistence SHALL support self-contained deterministic evolution
- Version-aware persistence: persistence SHALL record and validate version identifiers for persistence artifacts
- Persist state with durability acknowledgment
- Restore state from persistence
- Fail closed on ambiguity

Optional capabilities:
- Snapshot creation and restoration
- Replay optimization
- Compaction or pruning of stored data
- Batch persistence for throughput optimization
- Persistence artifact inspection capabilities (inspection, not query abstraction, repository, or DAO semantics)
- Multi-tenant isolation optimization
- Tenant portability
- Automatic version migration on compatible version mismatch

Forbidden capabilities:
- Transport ownership: persistence MUST NOT own network transport or protocol management
- Orchestration ownership: persistence MUST NOT own business or workflow orchestration
- Runtime scheduling ownership: persistence MUST NOT own actor or process scheduling
- Vendor-specific semantics: persistence MUST NOT expose storage-vendor-specific behavior as constitutional guarantees
- Vendor-specific migration semantics: persistence MUST NOT require vendor-specific migration tooling or formats
- Manual out-of-band persistence assumptions: persistence MUST NOT assume human or external coordination for durability
- Tenant implementation leakage: persistence MUST NOT expose tenant isolation implementation details as constitutional guarantees
- ORM semantics: persistence MUST NOT define ORM, repository, or DAO abstractions
- Schema definition coupling: persistence MUST NOT define schema definition languages, DDL, or data definition formats as constitutional requirements

#### Scenario: Mandatory capability — persist state

- **WHEN** state persistence is requested
- **THEN** the platform SHALL attempt to persist the state within the declared durability boundary and return a deterministic acknowledgment

#### Scenario: Forbidden capability — transport ownership

- **WHEN** the persistence contract is evaluated
- **THEN** it SHALL NOT include transport, network, or protocol management responsibilities

### Requirement: Persistence Ownership Boundary

Persistence SHALL NOT own domain lifecycle semantics. Persistence stores and restores persistence artifacts; it does not decide domain meaning.

Persistence MUST NOT determine:
- Entity termination
- Workflow completion
- Actor lifecycle transitions
- Aggregate archival policies
- Business orchestration completion

Persistence ownership is limited to:
- Recording committed persistence artifacts within the declared durability boundary
- Deterministic restoration of previously recorded persistence artifacts
- Enforcement of durability boundaries
- Enforcement of persistence invariants

#### Scenario: Persistence does not own domain lifecycle

- **WHEN** a persistence operation completes
- **THEN** the operation SHALL NOT have triggered entity termination, workflow completion, actor lifecycle transitions, or business orchestration decisions

### Requirement: Persistence failure model

The platform SHALL define a fail-closed persistence failure model.

Failure guarantees:
- All persistence operations SHALL fail closed on ambiguity — no silent data loss, silent durability failure, or silent state corruption
- Durability ambiguity SHALL be resolved deterministically: either the persistence artifacts are persisted within the declared durability boundary (Persisted) or they are not (Failed). No third state visible to the caller
- Partial write ambiguity SHALL be handled deterministically: if persistence artifacts cannot be committed atomically within the defined durability boundary, the operation SHALL fail as a whole
- Restoration ambiguity SHALL be handled deterministically: if state cannot be fully and correctly restored, the operation SHALL fail rather than returning partial or corrupted state
- Deterministic failure behavior SHALL be guaranteed: given identical failure conditions, the failure outcome and error semantics SHALL be identical

#### Scenario: Fail-closed on durability ambiguity

- **WHEN** a persist operation encounters ambiguous durability (e.g., storage backend unavailable, write acknowledgment not received)
- **THEN** the operation SHALL fail closed — returning a failure acknowledgment — and MUST NOT indicate successful persistence

#### Scenario: Deterministic failure outcome

- **WHEN** identical failure conditions occur during persistence
- **THEN** the failure outcome and error semantics SHALL be identical

### Requirement: Persistence lifecycle

The platform SHALL define a persistence lifecycle with explicit states, transitions, invariants, and fail-closed behavior.

Persistence lifecycle states:
- **Requested** — a persistence operation has been requested but not yet initiated
- **Persisting** — persistence artifacts are being persisted
- **Persisted** — persistence artifacts have been persisted and acknowledged
- **Restoring** — previously persisted artifacts are being retrieved and reconstructed
- **Restored** — artifacts have been successfully retrieved and reconstructed
- **Failed** — the persistence operation has failed; no persistence outcome is guaranteed

Lifecycle invariants:
- Transitions SHALL be deterministic: from a given state, only defined transitions are valid
- Failed SHALL be reachable from any processing state (Persisting, Restoring)
- From Persisted, the only valid transition is to a new Requested or to Restoring
- From Restored, the only valid transition is to a new Requested
- From Failed, the only valid transition is to a new Requested (retry)

Fail-closed lifecycle behavior:
- If state cannot be determined, the lifecycle SHALL transition to Failed
- Ambiguous transitions (e.g., partially persisted, partially restored) MUST NOT exist

#### Scenario: Lifecycle transition — Persisting to Persisted

- **WHEN** a persist operation completes successfully
- **THEN** the lifecycle SHALL transition from Persisting to Persisted

#### Scenario: Lifecycle transition — any processing state to Failed

- **WHEN** a persistence operation encounters a failure
- **THEN** the lifecycle SHALL transition to Failed from either Persisting or Restoring

### Requirement: Deterministic Persistence Axiom

Given identical state, identical events, identical logical time, identical runtime capabilities, and identical persistence context, the observable persistence outcome SHALL be identical.

Observable persistence outcomes include:
- Persisted state
- Persisted events
- Restoration outcome
- Snapshot outcome
- Replay outcome
- Failure outcome

The Deterministic Persistence Axiom SHALL govern all persistence operations uniformly. There SHALL be no carve-out for non-deterministic persistence paths.

This axiom SHALL NOT be interpreted as requiring transactional consistency, linearizability, or serializability from the storage layer. It requires that given identical inputs, the observable outcome be reproducible — which is consistent with both transactional and eventually-consistent storage backends, as long as the eventual outcome is deterministic.

#### Scenario: Axiom governs state persistence

- **WHEN** identical state is persisted with identical logical time, capabilities, and context
- **THEN** the persisted state and acknowledgment SHALL be identical

#### Scenario: Axiom governs replay

- **WHEN** identical events are replayed with identical runtime state and capabilities
- **THEN** the replay outcomes SHALL be identical

### Requirement: Hexagonal boundaries

The platform SHALL define hexagonal architectural boundaries for the Persistence SPI.

Dependency direction:
Core
├── Actor Contract
├── Persistence Contract
└── Runtime Contract

Persistence Contract depends on Runtime Contract and Canonical Contracts.
Actor Contract MAY consume Persistence Contract but does not define it.
Persistence remains valid for actors, workflows, services, process orchestration, and non-actor execution.

Persistence Contract SHALL depend on:
- FOUNDATION-002 Canonical Contracts (determinism, serialization neutrality, capability contracts)
- FOUNDATION-003 Runtime Contract (runtime abstraction, execution guarantees)

Persistence Contract SHALL NOT depend on:
- FOUNDATION-004 Actor Contract (persistence semantics are valid independent of the actor model)
- Any storage adapter, platform implementation, or vendor-specific library

Actor Contract and higher-level contracts SHALL consume the Persistence Contract — they do not define it.

#### Scenario: Persistence Contract does not depend on Actor Contract

- **WHEN** the dependency graph is evaluated
- **THEN** Persistence Contract SHALL NOT have a dependency on the Actor Contract

#### Scenario: Adapters depend on contracts, not the inverse

- **WHEN** adapter dependencies are evaluated
- **THEN** storage adapters SHALL depend on the Persistence Contract, not the reverse

### Requirement: Governance

The platform SHALL define constitutional governance for the Persistence SPI.

Constitutional invariants:
- Storage neutrality: Persistence SPI MUST NOT contain references to specific storage products, vendors, or technologies
- Runtime neutrality: Persistence SPI MUST NOT contain references to specific runtimes
- Determinism: All persistence operations MUST preserve deterministic outcomes
- Fail-closed: All persistence operations MUST fail closed on ambiguity
- No ORM leakage: Persistence SPI MUST NOT define ORM semantics, repository patterns, or data access objects
- No SQL assumptions: Persistence SPI MUST NOT reference SQL, query languages, or relational concepts
- No serialization prescription: Persistence SPI MUST NOT mandate a serialization format
- Representation neutrality: Persistence SPI MUST NOT assume byte, blob, payload, or serialization representations
- Versioning determinism: Version identifiers SHALL be deterministic given identical artifact content and versioning context

Forbidden patterns:
- Vendor-specific persistence primitives in constitutional requirements
- Repository or DAO terminology in constitutional requirements
- Query abstraction or data-access semantics in constitutional requirements
- Storage engine performance characteristics as constitutional guarantees
- Vendor-specific migration tooling or format coupling in constitutional requirements

Capability inflation protection:
- New mandatory capabilities SHALL require constitutional amendment
- Optional capabilities SHALL NOT become de facto mandatory through test requirements or convention
- Forbidden capabilities SHALL be enforced at the constitutional level — no extension may introduce them

Vendor neutrality enforcement:
- All constitutional requirement examples SHALL use generic terminology
- References to specific products SHALL be non-normative and clearly marked as examples
- No constitutional test SHALL depend on a specific storage backend

#### Scenario: Storage neutrality invariant

- **WHEN** constitutional requirements are evaluated
- **THEN** no requirement SHALL reference a specific storage product, vendor, technology, or database system

#### Scenario: No ORM leakage

- **WHEN** constitutional requirements are evaluated
- **THEN** no requirement SHALL define ORM semantics, repository patterns, DAO abstractions, or data access interfaces

### Requirement: Testing contract

The platform SHALL define a testing contract for Persistence SPI compliance validation.

Deterministic testing:
- Compliance tests SHALL be deterministic — repeatable with identical outcomes on every run
- Compliance tests SHALL use mock persistence backends and MUST NOT require infrastructure dependencies (databases, storage services, network)
- Replay reproducibility SHALL be tested: given identical persisted inputs, the replay SHALL produce identical outcomes
- Versioning SHALL be tested: version identifier determinism, version-aware persistence and restoration, fail-closed version mismatch, version compatibility validation
- All persistence failure model behaviors SHALL be tested: durability ambiguity, partial write ambiguity, restoration ambiguity
- Lifecycle transitions SHALL be tested: every defined transition, including all failure transitions
- Capability model SHALL be tested: mandatory capabilities MUST pass, optional capabilities SHALL be testable independently, forbidden capabilities SHALL be verifiably absent
- Compliance test coverage SHALL target 95%+ of constitutional requirements and scenarios

#### Scenario: Deterministic compliance test

- **WHEN** a compliance test is executed twice with identical inputs
- **THEN** both runs SHALL produce identical outcomes

#### Scenario: No infrastructure dependency

- **WHEN** compliance tests are executed
- **THEN** they SHALL NOT require a running database, storage service, or network dependency

#### Scenario: Versioning determinism test

- **WHEN** identical artifact content is persisted twice with identical versioning context in a compliance test
- **THEN** both persistence operations SHALL produce identical version identifiers

#### Scenario: Version mismatch fail-closed test

- **WHEN** a persisted artifact with an incompatible version identifier is restored in a compliance test
- **THEN** the restoration SHALL fail deterministically and MUST NOT produce usable state
