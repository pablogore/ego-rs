## Context

ego-rs is a backend-platform / actor-platform framework. FOUNDATION-001 through FOUNDATION-006 ratified: Architecture Constitution, Canonical Contracts, Runtime Abstraction, Actor Model, Persistence SPI, and Cluster Model. These foundations expose observable semantics (execution visibility, lifecycle semantics, replay semantics, placement semantics) but define no canonical contract for consuming them.

FOUNDATION-007 establishes the Observability SPI — the constitutional contract that defines how observable semantics are consumed, correlated, and guaranteed across the platform. It is NOT an implementation, NOT a vendor abstraction, and NOT a telemetry pipeline. It is the semantic boundary contract between "what the platform does" and "what can be observed about what the platform does."

### Constraints

- FOUNDATION-004 (Actor Model), FOUNDATION-005 (Persistence SPI), FOUNDATION-006 (Cluster Model) are frozen
- FOUNDATION-007 MUST consume but MUST NOT modify prior constitutional surfaces
- MUST be runtime-neutral, transport-neutral, vendor-neutral, implementation-neutral
- MUST be deterministic-first, replay-safe, fail-closed
- MUST be hexagonal — depends on Canonical Contracts, Runtime, Actor, Persistence, Cluster; never vice versa

### Stakeholders

- Platform architect: needs constitutional guarantees around determinism, replay safety, and fail-closed semantics
- Actor developer: needs visibility into execution, lifecycle, and failure without coupling to telemetry backends
- Platform operator: needs cluster-aware observability without owning cluster behavior
- Adapter author: needs a stable SPI to implement vendor-specific adapters (OTel, Prometheus, etc.) without leaking vendor concerns into the platform

## Goals / Non-Goals

**Goals:**

1. Define the canonical Observability SPI as a hexagonal port — pure semantic contract, no infrastructure
2. Define observable semantics as first-class types: execution visibility, lifecycle visibility, message visibility, failure visibility, replay visibility, placement visibility, ownership visibility, persistence visibility, locality visibility, restoration visibility
3. Define a deterministic event model with semantic event categories expressed through canonical observability channels (trace, metric, log) — NOT vendor telemetry representations
4. Define a correlation model that is deterministic, replay-safe, topology-independent, transport-independent
5. Establish the Deterministic Observability Axiom as a constitutional invariant
6. Define replay-safe observability semantics that distinguish live execution from replay execution
7. Define cluster-aware observability semantics that observe placement/ownership/partition transitions without owning cluster behavior
8. Define a capability model with mandatory, optional, and forbidden capabilities
9. Define hexagonal dependency boundaries with constitutional enforcement
10. Define a testing contract requiring mock-only capture of observable semantics, deterministic tests, 95%+ semantic surface coverage

**Non-Goals:**

- NOT an OpenTelemetry abstraction, Prometheus abstraction, Datadog abstraction, or any vendor abstraction
- NOT a logging framework, tracing SDK, metrics implementation, exporter framework, or transport protocol
- NOT a telemetry backend, visualization system, dashboard system, or APM implementation
- NOT a runtime scheduler, cluster monitoring implementation, or orchestration system
- NOT defining how telemetry is exported, stored, or visualized — those are adapter concerns
- NOT defining payload serialization formats, wire protocols, or API endpoints
- NOT modifying FOUNDATION-001 through FOUNDATION-006 surfaces in any way

## Decisions

### Decision 1: Observability is a hexagonal port, not a service

**Choice**: Define Observability SPI as a pure port (trait/interface) with no runtime lifecycle of its own.

**Rationale**: Observability must observe, not own. Making it a service would couple it to runtime lifecycle and violate the hexagonal architecture. A port allows adapter-based implementations (test, OTel, Prometheus, none) without leaking vendor concerns.

**Alternatives considered**:
- Runtime plugin: Would couple observability to runtime lifecycle — rejected because runtime must be observable, not own observability
- Event bus: Would introduce transport concerns — rejected because transport is an adapter concern, not a constitutional one

### Decision 2: Three telemetry channels with semantic event categories

**Choice**: The SPI exposes three telemetry channels — `trace`, `metric`, `log` — as the interface. Semantic event categories (`execution_started`, `actor_created`, `replay_completed`) are embedded within these channels through the `SemanticEvent` type.

**Rationale**: Three telemetry channels match the standard observability pillars (tracing, metrics, logging) that every adapter maps to. Semantic event categories define WHAT is emitted through these channels. The `SemanticEvent` struct carries the semantic payload; adapters map to vendor-specific representations. This avoids forcing every adapter author to re-invent the channel mapping.

**Alternatives considered**:
- Pure semantic categories without telemetry channels: Would force each adapter to independently decide which telemetry type maps to which semantic event — rejected because it shifts non-trivial mapping decisions to every adapter author.
- Define spans/logs/metrics as the only interface without semantic categories: Would lose semantic specificity — rejected because type safety for deterministic guarantees requires structured event types.

### Decision 3: Deterministic correlation over vendor trace IDs

**Choice**: Correlation is based on deterministic platform identifiers (actor ID, execution ID, replay sequence, logical time) rather than vendor trace/span IDs.

**Rationale**: Vendor trace IDs (e.g., W3C traceparent) are transport-dependent and topology-dependent. Platform identifiers are deterministic, replay-safe, and topology-independent. Correlation semantics must survive replay without external trace context. Adapters MAY generate vendor trace IDs from platform identifiers.

**Alternatives considered**:
- Use W3C trace context: Would couple to HTTP/gRPC transport semantics — rejected because observability is transport-neutral
- Use UUIDs: Non-deterministic — rejected because replay requires deterministic correlation

### Decision 4: Fail-closed on ambiguity

**Choice**: When observable state is ambiguous (e.g., replay vs live ambiguity, correlation gap), the SPI MUST NOT expose observable semantics. It SHALL fail closed rather than exposing potentially misleading semantics.

**Rationale**: Misleading observability is worse than no observability. A failed replay that produces no events is debuggable; a failed replay that produces plausible-but-wrong events is not. This is a constitutional invariant.

**Alternatives considered**:
- Fail-open (emit with warning): Would violate determinism guarantees — rejected
- Best-effort: Would make replay safety unprovable — rejected

### Decision 5: Cluster-aware but cluster-independent

**Choice**: Observability SPI consumes cluster observable semantics (placement, ownership, partition) as input but defines its own correlation model independent of cluster topology.

**Rationale**: Cluster topology changes (membership changes, partition reassignment) MUST NOT change observable semantics for the same deterministic inputs. Observability observes placement outcomes, not cluster internals.

**Alternatives considered**:
- Embed cluster topology in correlation: Would make observability topology-dependent — rejected because replay must produce identical semantics regardless of which node executes
- Ignore cluster context entirely: Would lose placement/ownership visibility — rejected because operators need cluster awareness

## Risks / Trade-offs

- **[Risk] Over-abstraction**: The SPI might define abstractions that are too generic to be useful. **Mitigation**: Each semantic category has defined observable semantics; adapter authors have clear extension points for vendor-specific concerns.
- **[Risk] Performance overhead in deterministic paths**: Correlation and fail-closed checks add cost to every observable semantic path. **Mitigation**: Observability SPI carries no semantic overhead when no consumers are attached; enabled paths are designed for deterministic overhead.
- **[Risk] Adapter complexity**: Semantic-to-telemetry mapping requires adapter authors to understand both platform semantics and vendor telemetry models. **Mitigation**: Provide reference adapters and a detailed mapping guide; the SPI semantic surface is bounded and explicit.
- **[Risk] Replay safety vs. real-time observability**: Distinguishing live vs replay execution adds complexity to observable semantics. **Mitigation**: Replay context is a deterministic input to observable semantics, set by the execution layer, not derived by observability.
- **[Risk] Constitutional freeze prevents future evolution**: FOUNDATION-007 is a freeze-grade foundation, making changes costly. **Mitigation**: The capability model explicitly includes optional capabilities that can be added without constitutional amendment; forbidden capabilities prevent scope creep.
