## Context

FOUNDATION-006 defines the Cluster Model of ego-rs as a constitutional, runtime-neutral, backend-platform clustering abstraction. This is not a distributed systems implementation, a service mesh, a Kubernetes abstraction, a gossip protocol, a Raft implementation, or a network framework. It is the semantic cluster contract that all future cluster realizations must satisfy.

The architecture builds on:
- **FOUNDATION-001 Architecture Constitution**: hexagonal architecture, dependency inversion, governance
- **FOUNDATION-002 Canonical Contracts**: determinism, serialization neutrality, capability contracts
- **FOUNDATION-003 Runtime Abstraction & Execution Model**: runtime neutrality, capability-based execution
- **FOUNDATION-004 Actor Model**: actor lifecycle, identity, communication, supervision, determinism
- **FOUNDATION-005 Persistence SPI**: durability semantics, deterministic persistence, restoration

Constraints:
- FOUNDATION-004 and FOUNDATION-005 are frozen. FOUNDATION-006 must consume rather than modify the actor model and persistence SPI constitutional surfaces.
- Cluster Model is a platform capability — not a user-facing API, not a distributed systems framework, not an infrastructure abstraction.

## Goals / Non-Goals

**Goals:**
- Define the constitutional Cluster Model as a semantic, runtime-neutral, transport-neutral clustering abstraction
- Define what a cluster is: semantic distributed coordination boundaries — not transport mechanics or orchestration
- Define cluster non-responsibilities: what a cluster MUST NOT own (networking, orchestration, runtime scheduling, cloud-provider assumptions, Kubernetes concerns, consensus mechanics)
- Define cluster invariants: isolation, determinism, fail-closed partition semantics, constitutional governance
- Define node identity semantics: deterministic, observable, location-independent, agnostic to hostname/IP/cloud identity
- Define membership semantics with states and transitions: Joining, Active, Leaving, Removed, Failed
- Define placement semantics: actor/service/workflow placement determinism, tenant-aware placement boundaries, locality visibility — no scheduler algorithms
- Define ownership semantics: execution ownership, restoration ownership, failover ownership, ownership visibility, ownership transfer — split-brain ambiguity forbidden
- Define partition semantics: partition handling, fail-closed behavior, ownership ambiguity resolution, deterministic recovery expectations
- Define locality semantics: local vs remote, locality transparency, locality visibility, tenant locality boundaries
- Define replayability semantics: replay across cluster boundaries, restoration determinism, placement restoration, ownership restoration
- Define cluster lifecycle with states and transitions: Initializing, Joining, Active, Partitioned, Recovering, Leaving, Terminated, Failed
- Define cluster failure model: fail-closed, split-brain ambiguity handling, ownership ambiguity, placement ambiguity, restoration ambiguity
- Define cluster capability model: mandatory, optional, and forbidden capabilities
- Define the Deterministic Cluster Axiom as a constitutional invariant
- Define hexagonal boundaries: Cluster Contract depends only on Canonical Contracts, Runtime Contract, Actor Model, and Persistence SPI
- Define governance: constitutional invariants, forbidden patterns, capability inflation protection, vendor neutrality
- Define testing contract: deterministic tests, mock-only, simulated partitions, replay reproducibility, 95%+ coverage

**Non-Goals:**
- NOT a distributed systems implementation
- NOT a service mesh abstraction
- NOT a Kubernetes abstraction or operator
- NOT a gossip protocol or consensus implementation (Raft, Paxos, CRDT)
- NOT a leader election algorithm
- NOT a networking protocol or transport specification
- NOT a service discovery framework (Consul, etcd, ZooKeeper)
- NOT a cloud-provider deployment abstraction
- NOT a container orchestration abstraction
- NOT a scheduler or load balancer implementation
- NOT an Akka Cluster or Orleans abstraction
- NOT an observability infrastructure implementation
- NOT a language-level trait or interface definition
- NOT a concrete cluster adapter implementation

## Decisions

**Decision 1: Cluster Model is a constitutional platform capability, not a systems abstraction**

The Cluster Model SHALL define semantic distributed coordination boundaries — what the cluster guarantees about membership, placement, ownership, and partition behavior. It SHALL NOT define how those guarantees are realized through transport mechanics, consensus protocols, service discovery, or infrastructure orchestration.

Rationale: Defining the cluster as a constitutional capability preserves runtime and infrastructure neutrality. Any realization that satisfies the cluster contract is a valid cluster adapter, regardless of whether it uses in-memory, simulation, embedded, thread-based, or async runtime realizations.

Alternatives considered:
- *Cluster as an infrastructure abstraction (Kubernetes CRD, Consul service)* — rejected because it couples the platform to specific infrastructure technologies, violating vendor neutrality.
- *Cluster as a distributed systems library* — rejected because the platform must define coordination semantics, not implement distributed algorithms.

**Decision 2: Node identity is deterministic and location-independent**

Node identity SHALL be a deterministic identifier with no inherent location, transport, or deployment semantics. The identity MUST NOT encode hostname, IP address, cloud provider instance ID, container ID, or any infrastructure-specific information.

Rationale: Location-independent identity preserves the ability to transition between deployment topologies, run simulation clusters, and test cluster behavior without infrastructure dependencies.

Alternatives considered:
- *Hostname-based identity* — rejected because hostnames are not portable across deployment environments.
- *IP-based identity* — rejected because IPs are ephemeral in cloud environments and violate location transparency.
- *Cloud-provider identity* — rejected because it couples the platform to specific cloud vendors.

**Decision 3: Membership is constitutional, discovery is not**

Membership semantics (states, transitions, visibility, determinism) are constitutional. Discovery mechanics (how nodes find each other, how heartbeats work, how failure detection operates) are implementation concerns.

Rationale: Separating membership semantics from discovery mechanics preserves implementation independence. In-memory, simulated, and async-runtime-based cluster realizations can all satisfy the same membership contract using radically different discovery mechanisms.

Alternatives considered:
- *Including discovery in the constitutional contract* — rejected because discovery mechanics vary widely across deployment environments and would constrain future implementations.

**Decision 4: Placement is deterministic, not optimized**

Placement SHALL produce deterministic outcomes given identical cluster topology and state. Placement optimization (load balancing, latency minimization, resource-aware scheduling) is an optional capability, never a constitutional requirement.

Rationale: Deterministic placement ensures replayability and testability. Optimization is a runtime-specific concern that must not affect constitutional determinism guarantees.

Alternatives considered:
- *Placement optimization as mandatory* — rejected because it would require all cluster realizations to implement scheduling algorithms, violating runtime neutrality.
- *Random placement* — rejected because non-deterministic placement breaks replayability and makes cluster behavior unpredictable.

**Decision 5: Ownership is explicit and split-brain ambiguous behavior is forbidden**

Ownership SHALL be explicit and unambiguous at all times. The cluster MUST fail closed when ownership cannot be determined. Split-brain — two nodes believing they own the same entity — is forbidden by constitutional invariant.

Rationale: A backend platform framework must never silently execute duplicative work or corrupt state due to ambiguous ownership. Fail-closed ownership is the only safe default.

Alternatives considered:
- *Lease-based ownership with eventual consistency* — rejected because it introduces split-brain windows that violate fail-closed determinism.
- *Optimistic ownership with conflict resolution* — rejected because conflict resolution in backend execution can produce side effects that are not safely reversible.

**Decision 6: Partition semantics are fail-closed**

When a cluster experiences a partition, the cluster SHALL fail closed on all ownership and placement decisions that involve ambiguous membership. The cluster MUST NOT assume success, completion, availability, or consistency across the partition boundary.

Rationale: Fail-closed partition semantics prevent silent data corruption, duplicate execution, and inconsistent state that can arise from partial failure scenarios.

Alternatives considered:
- *Partition tolerance with quorum-based decisions* — rejected because quorum assumptions are implementation-specific and cannot be constitutionally mandated across all cluster realizations.
- *Best-effort partition handling* — rejected because it introduces non-deterministic behavior that violates the Deterministic Cluster Axiom.

**Decision 7: Locality is observable but semantically transparent**

The cluster SHALL expose locality information for visibility and optimization purposes, but locality MUST NOT alter the semantic outcome of actor execution, message delivery, placement decisions, or ownership resolution.

Rationale: Locality transparency ensures that actor code does not need to change when actors are moved between nodes. Locality may be observable for debugging without coupling semantics to topology.

Alternatives considered:
- *Locality-agnostic cluster* — rejected because operators need visibility into where work executes for debugging, capacity planning, and performance analysis.
- *Locality-aware semantics* — rejected because it breaks location transparency and would require actor code to handle locality distinctions.

**Decision 8: Replay determinism spans cluster boundaries**

Replay MUST produce identical observable outcomes regardless of which cluster node performs the replay, which cluster topology is active, or which partition boundaries exist at the time of replay. Cluster topology and partition state are not replay inputs — they are environmental conditions that MUST NOT affect replay determinism.

Rationale: Deterministic replay is a platform requirement that must hold across cluster reconfiguration, node failure, and partitions. If replay behavior depends on cluster topology, replay becomes non-deterministic from the actor or workflow perspective.

Alternatives considered:
- *Replay is node-local* — rejected because node failure would invalidate replay guarantees, making durable actors unreliable across cluster reconfiguration.
- *Replay respects current cluster topology* — rejected because topology changes would produce different replay outcomes from identical inputs, violating determinism.

**Decision 9: Cluster Model consumes Actor Model and Persistence SPI without modifying them**

The Cluster Contract depends on the Actor Contract (FOUNDATION-004) and the Persistence SPI (FOUNDATION-005) as consumers. Cluster semantics extend actor placement, ownership, and restoration without changing the actor abstraction or the persistence contract.

Dependency direction:
- Cluster Contract → Canonical Contracts, Runtime Contract, Actor Contract, Persistence SPI
- Cluster Contract MUST NOT → networking adapters, infrastructure systems, vendor libraries

Rationale: Following FOUNDATION-001 hexagonal architecture, the Cluster Contract is an inward-dependency layer that consumes existing constitutional contracts. This preserves the frozen status of FOUNDATION-004 and FOUNDATION-005.

Alternatives considered:
- *Cluster as a peer of Actor Model* — rejected because cluster semantics require actor placement, ownership, and restoration visibility, making the Actor Contract a natural dependency.
- *Cluster embedded in Actor Model* — rejected because it would modify the frozen actor contract surface and conflate actor semantics with cluster coordination.

**Decision 10: Capability model distinguishes mandatory, optional, and forbidden at the constitutional level**

Mandatory capabilities include deterministic membership visibility, ownership visibility, fail-closed partition handling, deterministic placement visibility, and restoration determinism. Optional capabilities include tenant-aware placement optimization, locality optimization, placement optimization, replication optimization, and geo-distribution optimization. Forbidden capabilities include transport ownership, orchestration ownership, runtime scheduling ownership, cloud-provider assumptions, networking protocol assumptions, and service mesh assumptions.

Rationale: The capability model governs what every cluster realization must, may, and must not provide. This prevents capability inflation (optional becoming mandatory) and vendor lock-in (vendor-specific capabilities becoming de facto requirements).

Alternatives considered:
- *Flat capability list without tiering* — rejected because it provides no guidance about which capabilities are essential vs. complementary.
- *All capabilities optional* — rejected because it would allow cluster realizations that provide no constitutional guarantees.

## Risks / Trade-offs

- [Risk] Over-specification of cluster semantics constrains future distributed implementations → [Mitigation] Capability model distinguishes mandatory vs. optional; forbidden list is narrow and targets specific infrastructure coupling patterns.
- [Risk] Under-specification leads to ambiguous cluster implementations → [Mitigation] Each requirement includes deterministic scenarios with WHEN/THEN format; testing contract enforces 95%+ coverage.
- [Risk] Fail-closed partition semantics may be interpreted as requiring always-available cluster → [Mitigation] Fail-closed means safe failure, not prevention of failure. Partitioned nodes explicitly report their degraded state.
- [Risk] Deterministic placement may be interpreted as requiring globally consistent placement state → [Mitigation] Deterministic placement means the same inputs produce the same placement outcome, not that placement state is globally consistent at all times.
- [Risk] Location transparency may conflict with visibility needs → [Mitigation] Locality is observable but semantically transparent; locality may be visible without affecting actor semantics.
- [Risk] Split-brain prohibition may be viewed as unrealistic in real networks → [Mitigation] Split-brain is forbidden as a constitutional invariant; cluster realizations must detect and prevent ambiguous ownership through their implementation mechanisms (leases, fencing, timeouts, etc.).
- [Risk] Replay determinism across cluster boundaries may be technically challenging → [Mitigation] Replay determinism is a constitutional requirement that constrains implementation choices; the spec defines the what, not the how.
- [Risk] Actor model dependency may create circular concerns → [Mitigation] Explicit prohibition: Cluster Contract depends on Actor Contract, not the reverse. Actors remain valid without clustering.
