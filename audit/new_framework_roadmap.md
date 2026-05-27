# New Framework Roadmap

## Principle

Every spec is atomic, implementable, testable, and archivable. No intention documents. No philosophical specs. No umbrella specs. Every spec produces runnable code. Every spec can be archived after implementation.

**Domain owns contracts. Runtime owns execution.** No leakage between layers.

---

## PHASE 1 — CORE (Framework Kernel)

### CORE-001 — Runtime Kernel Slice

| Field | Value |
|-------|-------|
| **Objective** | Finish the `runtime-slice` crate. Make it a workspace member. |
| **Why** | Only change with real code. Foundation for all runtime operations. |
| **Dependencies** | FOUNDATION-001 (workspace structure) — DONE |
| **Deliverables** | 1. Add `runtime-slice` to workspace members. 2. Implement `executor.rs`. 3. Implement `projection.rs`. 4. Implement `validation.rs`. 5. Integration tests. |
| **Tests** | At least 5 tests covering executor, projection, validation lifecycle. |
| **Archive criteria** | `cargo test --workspace` passes. `cargo clippy` clean. Workspace member. |

### CORE-002 — Actor Primitive (Domain Only)

| Field | Value |
|-------|-------|
| **Objective** | Minimal domain contract: `Actor` trait, `ActorId`, `actor_id!` macro, semantic lifecycle states, supervision contract. |
| **Why** | Actors are THE core abstraction. Domain defines the contract. Runtime (CORE-003) owns execution. |
| **Dependencies** | CORE-001 (runtime kernel) |
| **Deliverables** | 1. `Actor` trait — only `type Message;`. 2. `ActorId` — location-transparent identity. 3. `actor_id!` — compile-time deterministic identity macro. 4. `ActorLifecycleState` enum. 5. `SupervisionStrategy` enum. 6. Rust docs on everything. |
| **Domain owns:** | `Actor` trait, `ActorId`, `actor_id!`, semantic lifecycle states, supervision contract |
| **Domain does NOT own:** | `ActorSystem`, mailbox, dispatch, supervision execution, scheduling |
| **Tests** | `ActorId` construction/validation. `actor_id!` produces `&'static`. Lifecycle states. Domain only — no runtime. |
| **Archive criteria** | `cargo test -p ego-domain` passes. Zero runtime dependencies in domain crate. |

### CORE-003 — Runtime Actor Execution

| Field | Value |
|-------|-------|
| **Objective** | Runtime mechanics for actors: `ActorSystem`, `Mailbox`, `ActorRef`, sequential processing, supervision execution. One spec — previously split into mailbox, dispatch, and supervision. |
| **Why** | These are runtime mechanics of a single concern: actor execution. Separating them encouraged premature API decisions. |
| **Dependencies** | CORE-002 (Actor trait, ActorId, lifecycle states, supervision contract) |
| **Deliverables** | 1. `crates/runtime/` crate. 2. `ActorSystem` with `spawn`/`stop`/`state`. 3. `ActorRef<M>` sendable handle. 4. `Mailbox<M>` — bounded, FIFO, non-blocking. 5. Sequential processing. 6. `RuntimeSupervisor` — restart/stop/escalate execution. |
| **Tests** | Spawn→send→process. Mailbox FIFO + bounded. Sequential processing. Supervisor restart/escalate. |
| **Archive criteria** | `cargo test -p ego-runtime` passes. Unidirectional dep: runtime→domain only. |

### CORE-004 — Persistence SPI

| Field | Value |
|-------|-------|
| **Objective** | Hexagonal persistence: `EventStore` + `SnapshotStore` traits in domain, in-memory adapter in infrastructure. |
| **Why** | Stateful actors need durable state. Hexagonal keeps backends pluggable. |
| **Dependencies** | CORE-002 (actors) |
| **Deliverables** | 1. `EventStore` trait (domain). 2. `SnapshotStore` trait (domain). 3. `InMemoryEventStore` (infrastructure). 4. Event replay semantics. |
| **Tests** | Append→read→replay. Snapshots + event catch-up. Version conflicts. No real database. |
| **Archive criteria** | In-memory adapter passes all SPI contract tests. Pluggable without domain changes. |

### CORE-005 — Observability SPI

| Field | Value |
|-------|-------|
| **Objective** | Hexagonal observability: `Observability` trait in domain, in-memory + noop adapters in infrastructure. |
| **Why** | Built-in observability from the start. Non-mutating, replay-safe, vendor-neutral. |
| **Dependencies** | CORE-001 (runtime kernel) |
| **Deliverables** | 1. `Observability` trait (domain). 2. `InMemoryObservability` (infrastructure). 3. `NoopObservability` (infrastructure). |
| **Tests** | Trace/metric/log capture. Non-mutating verification. Replay determinism. |
| **Archive criteria** | In-memory adapter inspectable. Noop has zero allocation. No vendor deps in domain. |

---

## PHASE 2 — RUNTIME INFRASTRUCTURE

### CORE-006 — Transport

| Field | Value |
|-------|-------|
| **Objective** | Actors communicate across processes. gRPC transport in `crates/transport/`. |
| **Dependencies** | CORE-003 (ActorSystem + ActorRef) |
| **Deliverables** | 1. gRPC service definition. 2. Actor message serialization. 3. Remote ActorRef resolution. |
| **Archive criteria** | Two actors on different processes exchange messages. |

### CORE-007 — Cluster Model (DEFERRED)

| Field | Value |
|-------|-------|
| **Objective** | Distributed coordination: node membership, actor placement, partition semantics. |
| **Dependencies** | CORE-003, CORE-004, CORE-006 |
| **Status** | DEFERRED to post-MVP. Not needed for single-node MVP. |

---

## PHASE 3 — DEVELOPER EXPERIENCE

### CORE-008 — SDK + Developer API

| Field | Value |
|-------|-------|
| **Objective** | Ergonomic derive macros, `#[actor]` proc macro, config builder. |
| **Dependencies** | CORE-002 through CORE-005 |
| **Deliverables** | 1. `#[derive(Actor)]` macro. 2. `ActorSystemBuilder`. 3. Config loading. |
| **Archive criteria** | `#[derive(Actor)]` compiles and produces working actor. |

### CORE-009 — Examples

| Field | Value |
|-------|-------|
| **Objective** | Working examples: ping-pong, stateful counter, supervision demo, distributed KV. |
| **Dependencies** | CORE-002 through CORE-008 |
| **Archive criteria** | All examples compile and pass CI. |

---

## DEFERRED (Post-MVP)

| ID | Objective | When |
|----|-----------|------|
| CORE-010 | Runtime Governance (fail-closed validation) | After runtime is stable |
| CORE-011 | Replay + Time Travel Debugging | After determinism is proven |
| CORE-012 | CLI / TUI | After SDK is stable |

---

## Domain vs Runtime Ownership

| Domain (CORE-002) | Runtime (CORE-003) |
|---|---|
| `Actor` trait (`type Message`) | `ActorSystem` |
| `ActorId` + `actor_id!` | `ActorRef<M>` |
| `ActorLifecycleState` (semantic) | Lifecycle transition execution |
| `SupervisionStrategy` (semantic) | `RuntimeSupervisor` (execution) |
| Communication semantics (what) | `Mailbox<M>` + dispatch (how) |
| Determinism Axiom (contract) | Determinism enforcement (runtime) |

---

## Immediate Next Step

**CORE-001 — Runtime Kernel Slice.** Only change with real working code. Tasks:
1. Add `core/runtime-slice/` to workspace Cargo.toml members
2. Implement `executor.rs`
3. Implement `projection.rs`
4. Implement `validation.rs`
5. Integrate with workspace test suite