## ADDED Requirements

### Requirement: Cluster model

The platform SHALL define a cluster as a set of nodes that participate in distributed actor execution. Nodes SHALL have unique identity. Node membership SHALL transition through states: Joining, Active, Leaving, Removed, Failed.

#### Scenario: Node joins cluster
- **WHEN** a node requests membership
- **THEN** it SHALL transition Joining → Active upon successful admission

#### Scenario: Node leaves cluster
- **WHEN** a node initiates graceful departure
- **THEN** it SHALL transition Active → Leaving → Removed

### Requirement: Actor placement

Actor placement SHALL map actors to nodes deterministically. Placement SHALL be a function of actor identity and cluster topology. Given identical topology, placement SHALL be identical.

#### Scenario: Deterministic placement
- **WHEN** an actor is spawned with a given identity and topology
- **THEN** its placement SHALL be deterministic

### Requirement: Partition semantics

Network partitions SHALL produce fail-closed behavior. Ambiguous membership (node unreachable but maybe alive) SHALL be treated as Failed, not Active.

#### Scenario: Partition fail-closed
- **WHEN** a node becomes unreachable
- **THEN** it SHALL be treated as Failed until membership is re-established; no work SHALL be routed to it

### Requirement: Distribution boundaries

Remote actor communication SHALL be transparent to the sender. Actor identity SHALL remain location-transparent. Message delivery across nodes SHALL preserve ordering guarantees between same sender and receiver.

### Requirement: Testing contract

Tests SHALL use mock cluster topologies. No test SHALL require real networking, real distribution, or real network partitions.

**Note:** CORE-007 is DEFERRED to post-MVP. Implement after actors, persistence, and transport are stable.