## Context

The system has no runtime abstraction. Previous design treated `ActorSystem` as the runtime entry point, coupling the platform API to actor concepts. This design resets to a runtime abstraction contract: the `Runtime` trait is the platform entry point, actor frameworks are optional backend implementations.

The `ego-domain` crate already defines actor semantics (`Actor`, `ActorId`, `ActorLifecycleState`, `SupervisionStrategy`). Domain code consumes `impl Runtime` for execution. The Runtime trait is backend-agnostic — it does not reference actor types.

## Runtime Abstraction Architecture

```
┌──────────────────────────────────────────────────┐
│                   Domain Code                     │
│         (ego-domain: Actor, ActorId, etc.)       │
│         consumes impl Runtime for execution      │
└────────────────────┬─────────────────────────────┘
                     │  depends on (for execution)
                     ▼
┌──────────────────────────────────────────────────┐
│         Runtime Abstraction Contract             │
│           Runtime trait (platform API)           │
│                                                   │
│  ExecutionId    RuntimeHandle    SendError        │
│  ExecutionState  isolation      fail-closed      │
│  sequential      lifecycle      scheduling       │
└────────────────────┬─────────────────────────────┘
                     │  implemented by
                     ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────┐
│  TokioRuntime    │ │  GoaktRuntime    │ │  ...more │
│  (default)       │ │  (actor adapter) │ │          │
│  crate: runtime- │ │  maps handles    │ │          │
│  tokio           │ │  to ExecutionId  │ │          │
└──────────────────┘ └──────────────────┘ └──────────┘
```

The `Runtime` trait is the sole platform abstraction. Every backend implements this trait. Domain code consumes `impl Runtime` and is fully backend-agnostic. Actor frameworks (Goakt, ProtoActor, Akka) integrate by implementing `Runtime` and mapping their native handles to `ExecutionId`.

## Physical Structure

```
Cargo.toml  (workspace root — add members: crates/runtime, crates/runtime-tokio)
crates/
├── runtime/                     [ego-runtime — runtime abstraction contract]
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs               — re-exports public API
│       └── runtime/
│           ├── mod.rs           — module declarations
│           ├── runtime.rs       — Runtime trait definition, capability discovery
│           ├── execution.rs     — ExecutionId, spawn semantics
│           ├── lifecycle.rs     — ExecutionState, lifecycle semantics
│           ├── scheduler.rs     — Scheduling contract
│           ├── isolation.rs     — Isolation guarantees
│           ├── failure.rs       — SendError, fail-closed behavior
│           └── handle.rs        — RuntimeHandle scoped access
│
├── runtime-tokio/               [ego-runtime-tokio — Tokio default engine]
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs               — TokioRuntime, TokioRuntimeBuilder,
│                                   DefaultRuntime
│
├── domain/                      (existing — unchanged)
├── application/                 (existing — unchanged)
├── infrastructure/              (existing — unchanged)
├── transport/                   (existing — unchanged)
└── ... existing crates
```

### Module Justification

| Module | File | Why it exists |
|---|---|---|
| `runtime` | `runtime.rs` | `Runtime` trait is the core abstraction; isolated for explicit signature review |
| `execution` | `execution.rs` | `ExecutionId` and spawn semantics independent from lifecycle; evolves when execution model changes |
| `lifecycle` | `lifecycle.rs` | `ExecutionState` enum evolves independently (new states added without touching other modules) |
| `scheduler` | `scheduler.rs` | Scheduling contract describes ordering guarantees; may grow with priority/affinity support |
| `isolation` | `isolation.rs` | Isolation guarantees are a distinct concern; defines failure containment boundaries |
| `failure` | `failure.rs` | `SendError` and fail-closed behavior; error kinds evolve independently from the trait |
| `handle` | `handle.rs` | `RuntimeHandle` is a different abstraction than `Runtime`; scoped access for spawned units |

## Runtime Interface Design

### Runtime trait

```rust
pub trait Runtime: Send + Sync + 'static {
    /// Spawn an execution unit.
    fn spawn<F>(&self, f: F, name: Option<&str>) -> ExecutionId
    where F: Future<Output = ()> + Send + 'static;

    /// Route a message to an execution unit.
    fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where M: Send + 'static;

    /// Request graceful shutdown of an execution unit.
    fn shutdown(&self, id: &ExecutionId);

    /// Query lifecycle state. Returns None if state tracking unsupported.
    fn state(&self, id: &ExecutionId) -> Option<ExecutionState>;
}
```

### Execution contract

- `spawn` creates an addressable execution unit identified by `ExecutionId`
- The spawned future runs until completion or until `shutdown` is called
- Each unit processes messages sequentially (in-order delivery per unit)
- No ordering guarantee across different execution units

### Lifecycle contract

States: `Active` → `Draining` → `Terminated` | `Failed`

- `Active`: unit is running and can receive messages
- `Draining`: shutdown requested, completing in-flight work, stops accepting new messages
- `Terminated`: completed successfully
- `Failed`: terminated due to unrecoverable error

### Scheduling contract

- Units are scheduled fairly; no single unit can starve others
- Backends MAY provide priority scheduling as an extension (not in core trait)
- Scheduling is implementation-defined; core trait does not mandate specific scheduler

### Isolation contract

- Failures in one execution unit MUST NOT cascade to other units
- Each unit has an independent execution context
- Unhandled panics in a unit MUST be caught and result in `Failed` state only for that unit
- The runtime MUST remain operational when a single unit fails

### Failure contract

- The runtime SHALL fail closed: on unrecoverable internal error, shutdown all units and refuse new work
- `send` to a non-existent or terminated id returns `SendError`
- `send` to a closed runtime returns `SendError`
- All runtime errors are non-panicking (return values, not unwinding)

## Backend Adapter Model

Each backend crate implements `Runtime` for its type:

- **TokioRuntime**: wraps `tokio::runtime::Runtime`, uses internal routing table for message dispatch
- **GoaktRuntime**: wraps Goakt actor system, maps native actor refs to `ExecutionId`
- **ProtoActorRuntime**: wraps ProtoActor PID, maps to `ExecutionId`
- **AkkaRuntime**: wraps Akka native ref, maps to `ExecutionId`

Actor framework backends integrate by:
1. Implementing `Runtime` trait methods
2. Mapping their native handle/PID/ref types to `ExecutionId`
3. Adapting their message routing to the `send` contract
4. Translating their lifecycle model to `ExecutionState`

## Optional Capabilities

These capabilities MAY exist in backend implementations but MUST NOT be required by the core `Runtime` trait:

- **Mailbox semantics**: backends MAY implement bounded/unbounded mailboxes; core `send` does not specify buffering
- **Supervision semantics**: backends MAY implement restart policies; core `shutdown` is a signal, not a directive
- **Actor lifecycle**: backends MAY expose actor-specific lifecycle hooks as backend-specific extensions
- **Priority scheduling**: backends MAY prioritize messages; core trait provides no priority mechanism
- **Timers/delays**: backends MAY expose timer capabilities as backend-specific APIs

All optional capabilities are discoverable through the capability discovery mechanism on the Runtime trait.

## Design Decisions

### Decision 1: Runtime trait uses `ExecutionId` as handle

No generic associated types. `ExecutionId` is the universal handle returned from `spawn` and accepted by all operations. Simple, backend-neutral, no coupling to per-message handle types.

Alternatives considered:
- GAT `type Handle<M>`: over-engineering for v1, couples trait to message types
- Caller-provided id: shifts id generation burden, inconsistent across backends

### Decision 2: `spawn` accepts a future, not an actor trait

`spawn` takes `Future<Output = ()> + Send + 'static`. Actor-backed runtimes wrap their actor creation in a future. Task-based runtimes spawn directly. No `Actor` trait in the platform API.

### Decision 3: Message sending is a Runtime operation

`send` is on `Runtime`, not on the spawned unit. The runtime knows how to route messages. Backends implement routing internally (channel, mailbox, direct call). This keeps the trait minimal and backend-neutral.

### Decision 4: State query is optional

`state()` returns `Option<ExecutionState>`. Backends that track lifecycle return `Some`. Simple backends return `None`. Prevents forcing state tracking on all implementations.

### Decision 5: RuntimeHandle is injected into spawned futures

Spawned units receive a `RuntimeHandle` scoped to the local unit. This allows units to communicate and self-manage without access to the full `Runtime` trait. `RuntimeHandle` exposes `id()`, `send()`, `shutdown()`, `state()`.

### Decision 6: DefaultRuntime lives in runtime-tokio, not runtime

`DefaultRuntime` aliases `TokioRuntime` in the `runtime-tokio` crate. Avoids circular dependency: `runtime-tokio` depends on `runtime` for the trait; `DefaultRuntime` is defined where the implementation lives. The `runtime` crate has zero dependencies and zero features.

### Decision 7: Sequential execution per unit

Each execution unit processes messages in arrival order. This is a core guarantee that all backends MUST provide. Cross-unit ordering is not guaranteed. Enables deterministic reasoning about per-unit behavior.

### Decision 8: Failure isolation

Unhandled errors in one unit MUST NOT affect other units. The runtime catches panics and transitions the failed unit to `Failed` state. The runtime itself continues operating. Unrecoverable runtime errors cause fail-closed shutdown.

## Dependency Boundaries

```
ego-runtime (crates/runtime):
  dependencies:    (none — foundational crate)
  dev-dependencies: (none)
  forbidden:       tokio, goakt, protoactor, akka, persistence, transport

ego-runtime-tokio (crates/runtime-tokio):
  dependencies:    ego-runtime, tokio
  dev-dependencies: (none)
  forbidden:       goakt, protoactor, akka, persistence, transport

Existing crates (domain, application, etc.):
  - NO dependency changes
  - Domain code MAY add dependency on ego-runtime (to consume impl Runtime)
  - Domain code MUST NOT depend on ego-runtime-tokio or tokio
```

## Forbidden Architecture

- `ActorSystem` SHALL NOT be the platform entry point
- Actor handle types SHALL NOT be in the core contract

- Supervision types SHALL NOT be in the core contract
- No Tokio types in the `ego-runtime` crate
- No framework-specific semantics in the `Runtime` trait
- No assumption that all backends provide mailbox/supervision/actor behavior
- No coupling between runtime abstraction and actor execution model

## Risks / Trade-offs

- [Minimal surface] The trait may prove too narrow. Mitigation: start with spawn/send/shutdown/state, expand only when a second backend proves the need.
- [Sequential guarantee] Enforcing per-unit sequential execution may constrain high-throughput backends. Mitigation: sequential applies to message handling within a unit; backends can parallelize across units.
- [Tokio coupling] The ecosystem may still converge on Tokio. Mitigation: CI runs with `NullRuntime` to verify abstraction works without Tokio.

## Anti-Hallucination Assumptions

ASSUMPTION: `ego-runtime` crate will be created at `crates/runtime/`. Not yet on disk.

ASSUMPTION: `ego-runtime-tokio` crate will be created at `crates/runtime-tokio/`. Not yet on disk.

ASSUMPTION: Existing workspace crates (`ego-domain`, `ego-application`, `ego-infrastructure`, `ego-transport`, `ego-runtime-slice`) remain unchanged.
