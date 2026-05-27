## ADDED Requirements

### Requirement: Observability abstraction model

Observability SHALL define semantic visibility over platform behavior. The Observability SPI SHALL expose observable semantics, correlate execution visibility, expose actor lifecycle visibility, expose persistence visibility, expose cluster visibility, expose failure visibility, and expose replay visibility.

Responsibilities SHALL include:
- Exposing observable semantics to authorized consumers
- Correlating execution visibility across actor, persistence, cluster, and replay boundaries
- Exposing actor lifecycle visibility (creation, stop, restart, ownership changes)
- Exposing persistence visibility (replay, restoration, snapshot)
- Exposing cluster visibility (placement, ownership, partition, membership, locality)
- Exposing failure visibility (execution failure, actor failure, persistence failure, partition detection)
- Exposing replay visibility (replay start, replay complete, replay failure)

Observability MUST NOT own:
- Execution or runtime scheduling
- Persistence lifecycle
- Cluster coordination
- Transport
- Telemetry backend
- Exporter lifecycle
- Vendor integration

#### Scenario: Observability SPI exposes execution visibility
- **WHEN** an actor message is dispatched
- **THEN** execution_started observable semantics SHALL become visible, exposing actor identity, message category, and causal ordering

#### Scenario: Observability SPI does not own runtime scheduling
- **WHEN** a runtime scheduling decision is made
- **THEN** observability SHALL observe the scheduling outcome but MUST NOT influence scheduling decisions

#### Scenario: Observability SPI does not own persistence lifecycle
- **WHEN** a persistence operation completes
- **THEN** observability SHALL observe the persistence outcome but MUST NOT control persistence lifecycle

### Requirement: Observable semantics

Observable semantics SHALL include:
- Execution visibility: message dispatch start, completion, and failure
- Actor lifecycle visibility: creation, stop, restart, ownership change
- Message visibility: message category, source, target, correlation
- Failure visibility: execution failure, actor crash, persistence failure, partition detection
- Replay visibility: replay start, replay complete, replay failure, replay progress
- Placement visibility: actor placement decisions and placement changes
- Ownership visibility: actor ownership transfers and ownership conflicts
- Persistence visibility: persist start, persist complete, replay progress, snapshot events
- Locality visibility: local vs. remote execution context
- Restoration visibility: restoration start, restoration complete, restoration failure

Observability SHALL expose semantic visibility — NOT infrastructure telemetry.

#### Scenario: Actor lifecycle visibility
- **WHEN** an actor is created
- **THEN** actor_created observable semantics SHALL become visible, exposing actor identity, actor categorization, and causal parent

#### Scenario: Failure visibility
- **WHEN** an actor fails during message processing
- **THEN** failure observable semantics SHALL become visible, exposing actor identity, message category, failure reason, and replay context

#### Scenario: Replay visibility on recovery
- **WHEN** an actor's persisted state is replayed after restart
- **THEN** replay_started observable semantics SHALL become visible, indicating replay context

### Requirement: Event model

Observability SHALL define canonical semantic event categories. Event categories SHALL be semantic categories — NOT telemetry payload formats, NOT tracing spans, NOT logs, NOT metrics schema.

Mandatory semantic event categories SHALL include:
- execution_started: a message dispatch began execution
- execution_completed: a message dispatch completed successfully
- execution_failed: a message dispatch failed
- actor_created: an actor instance was created
- actor_stopped: an actor instance was stopped
- actor_restarted: an actor instance was restarted
- ownership_changed: actor ownership transferred across cluster locations
- placement_changed: actor placement transitioned across cluster topology
- replay_started: actor state replay from persisted events began
- replay_completed: actor state replay completed successfully
- restoration_started: actor restoration from snapshot began
- restoration_completed: actor restoration completed
- partition_detected: a cluster partition was detected
- failure_detected: an unrecoverable failure was detected

Observable semantics for each semantic category SHALL expose:
- actor identity for actor-scoped semantics
- execution correlation for causally linked visibility
- causal ordering for temporally deterministic visibility
- replay context indicating whether visibility arises from live or replay execution
- correlation chain for parent-child execution relationships

#### Scenario: Semantic categories are not transport categories
- **WHEN** execution_started observable semantics become visible
- **THEN** the semantics SHALL expose actor identity, execution correlation, and causal ordering and MUST NOT expose transport identifiers

#### Scenario: Replay context in observable semantics
- **WHEN** observable semantics arise during state replay
- **THEN** replay context SHALL indicate replay execution, and replay sequence SHALL be exposed for ordering

### Requirement: Correlation model

Correlation SHALL be deterministic, replay-safe, topology-independent, and transport-independent.

Correlation SHALL support:
- Execution correlation: causally link message dispatch through actor boundaries
- Actor correlation: group all observable semantics for a given actor identity across restarts and placements
- Replay correlation: link replay observable semantics with the original execution semantics
- Restoration correlation: link restoration semantics with the snapshot source
- Cluster correlation: link semantics across cluster boundaries without topology dependence
- Ownership correlation: link ownership transfer semantics across cluster locations

Correlation identifiers MUST be deterministic given the same inputs. Correlation MUST NOT depend on:
- Trace vendor identifiers
- Runtime handles
- Transport identifiers
- Network topology

#### Scenario: Correlation survives replay
- **WHEN** the same message is processed twice (once live, once during replay)
- **THEN** both executions SHALL produce the same correlation identifiers for the same logical inputs

#### Scenario: Correlation is topology-independent
- **WHEN** an actor experiences a placement transition across cluster locations and is replayed
- **THEN** the correlation identifiers for the original execution and the replayed execution SHALL be identical, regardless of cluster topology

### Requirement: Replay-safe observability

Replay MUST NOT create semantic ambiguity. Replay observability SHALL distinguish:
- Live execution: message processing in normal operational mode
- Replay execution: message processing during state recovery

Replay context SHALL be deterministic input, never inferred state. The replay/live distinction is semantic context, not a structural modification of observable semantics. Replaying identical inputs SHALL produce identical observable semantics, where replay context is part of the semantic outcome.

Replay SHALL preserve:
- Semantic determinism: the same semantic categories become visible in the same order
- Causal ordering: causal order is preserved
- Correlation determinism: correlation identifiers are identical

#### Scenario: Replay context preserves determinism
- **WHEN** an actor replays 10 messages during recovery
- **THEN** observable semantics SHALL expose replay context for each of the 10 replay execution semantics, and replaying the same 10 messages a second time SHALL produce identical replay-annotated observable semantics

#### Scenario: Identical replay produces identical observability
- **WHEN** the same set of persisted messages is replayed twice
- **THEN** both replays SHALL produce identical observable semantics (same semantic categories, same correlation, same order, same causal ordering progression)

### Requirement: Cluster-aware observability

Observability MAY consume cluster observable semantics. Observability MUST NOT own cluster behavior.

Observability SHALL observe:
- Placement transitions: cluster-visible actor placement changes
- Ownership transitions: cluster-visible actor ownership transfers
- Partition transitions: cluster partition state changes
- Membership visibility: cluster membership transitions
- Locality visibility: local or remote execution distinction

Cluster SHALL remain authoritative for cluster state. Observability SHALL observe cluster state — not derive, infer, or validate it.

#### Scenario: Placement change visibility
- **WHEN** an actor's placement transitions across cluster locations
- **THEN** ownership_changed observable semantics SHALL become visible, exposing ownership transition semantics

#### Scenario: Observability does not validate cluster state
- **WHEN** cluster reports a placement change
- **THEN** observability SHALL expose the observable semantics as reported and MUST NOT validate or reject the cluster state

### Requirement: Deterministic Observability Axiom

Given identical inputs, logical time, ownership state, placement state, replay state, and execution semantics, observable semantics MUST be identical.

Observability MUST fail closed on ambiguity. When observable state is ambiguous, observable semantics SHALL NOT be exposed.

Observability SHALL fail closed when unable to determine:
- replay context
- correlation context
- ownership context
- placement context
- causal ordering

No best effort. No inferred semantics. No silent fallback. No partial observability.

#### Scenario: Deterministic replay axiom
- **WHEN** the same message is processed at the same logical time with the same ownership and placement state
- **THEN** observable semantics SHALL be identical in semantic category, order, correlation, and causal context

#### Scenario: Fail-closed on ambiguity
- **WHEN** replay context cannot be determined
- **THEN** observable semantics SHALL NOT be exposed

#### Scenario: Fail-closed on correlation ambiguity
- **WHEN** correlation context is ambiguous for an observable semantic
- **THEN** observable semantics SHALL NOT become visible

### Requirement: Capability model

The Observability SPI SHALL define three capability tiers:

Mandatory capabilities:
- Execution visibility
- Failure visibility
- Replay visibility
- Correlation semantics
- Restoration visibility
- Actor lifecycle visibility

Optional capabilities — MUST NOT suppress or weaken mandatory semantic visibility, replay determinism, fail-closed behavior, or deterministic correlation:
- Metrics aggregation
- Sampling optimization
- Export optimization
- Locality optimization
- Visualization optimization

Forbidden capabilities — ownership violations:
- Runtime ownership
- Cluster ownership
- Persistence ownership
- Exporter ownership
- Telemetry backend ownership
- Vendor coupling

#### Scenario: Mandatory capability establishment
- **WHEN** an Observability SPI capability is established
- **THEN** all mandatory capabilities SHALL be available before any observable semantics are exposed

#### Scenario: Forbidden capability prevention
- **WHEN** a runtime component attempts to delegate scheduling to observability
- **THEN** the Observability SPI SHALL prevent this through constitutional enforcement

### Requirement: Governance

Governance SHALL validate:
- Determinism: all correlation paths are deterministic
- Replay safety: replay produces identical observable semantics
- Dependency neutrality: only constitutionally allowed dependencies
- Semantic purity: no implementation or telemetry concepts in the SPI
- Capability inflation protection: no forbidden capabilities introduced
- Mock-only testability: tests capture observable semantics without telemetry infrastructure

#### Scenario: Dependency boundary enforcement
- **WHEN** a change is proposed to the Observability SPI constitutional boundary
- **THEN** governance SHALL verify no forbidden dependencies (telemetry, runtime, cluster, or vendor implementation concerns) are introduced

#### Scenario: Determinism audit
- **WHEN** a new correlation path is added
- **THEN** governance SHALL verify all identifiers in the path are derived from deterministic inputs

### Requirement: Hexagonal boundaries

Observability SPI MUST depend ONLY on:
- Canonical Contracts (FOUNDATION-002)
- Runtime Contract (FOUNDATION-003)
- Actor Contract (FOUNDATION-004)
- Persistence SPI (FOUNDATION-005)
- Cluster Contract (FOUNDATION-006)

Observability SPI MUST NOT depend on:
- Telemetry implementation concerns
- Runtime realization concerns
- Cluster realization concerns
- Vendor-specific dependencies

#### Scenario: No telemetry implementation dependency
- **WHEN** the Observability SPI is established as a dependency boundary
- **THEN** it SHALL NOT depend on telemetry implementation concerns

#### Scenario: No vendor-specific dependency
- **WHEN** the Observability SPI is established as a dependency boundary
- **THEN** it SHALL NOT depend on vendor-specific concerns

### Requirement: Testing contract

Testing SHALL require:
- Mock-only capture of observable semantics in all tests
- Deterministic observability tests producing identical results across runs
- Replay reproducibility verification with identical semantic assertion across replays
- Fail-closed ambiguity testing for each ambiguity category
- Replay versus live distinction semantic verification
- Deterministic correlation verification across replay and topology change
- No telemetry infrastructure during tests
- No external services during tests
- Simulated failure scenarios covering all failure semantic categories
- 95%+ coverage of the Observability SPI semantic surface

#### Scenario: Mock-only capture of observable semantics
- **WHEN** an actor test executes
- **THEN** observability SHALL use a mock implementation that captures observable semantics without transport

#### Scenario: Replay reproducibility verification
- **WHEN** a test replays the same semantic sequence twice
- **THEN** both replays SHALL produce identical captured observable semantics, and assertions on deterministic replay semantics SHALL validate semantic identity

#### Scenario: Fail-closed ambiguity test
- **WHEN** replay, correlation, ownership, placement, or causal ordering context is ambiguous in a test
- **THEN** the test SHALL verify that observable semantics do NOT become visible

#### Scenario: Replay versus live distinction verification
- **WHEN** a test exercises both live and replay execution of the same semantic sequence
- **THEN** the test SHALL verify that replay context distinguishes replay semantics from live semantics while preserving determinism
