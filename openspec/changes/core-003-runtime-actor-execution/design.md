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
│           ├── runtime.rs       — Runtime trait definition
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
    /// Spawn an execution unit with handle injection.
    fn spawn<F, Fut>(
        &self,
        f: F,
        name: Option<&str>,
    ) -> Result<ExecutionId, SpawnError>
    where
        F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;

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

- ## Backend Adapter Model
- `spawn` takes a `FnOnce(RuntimeHandle)` that constructs the execution future — the runtime constructs the `RuntimeHandle`, injects it, then executes the returned future
- `spawn` returns `Result<ExecutionId, SpawnError>` — fails with `SpawnErrorKind::Closed` if runtime has shut down
- The runtime internally maintains message delivery state per spawned unit, enabling `handle.send_self(msg)` to route messages to the local unit. Remote routing is performed via `Runtime::send(id, msg)`.
- The spawned future runs until completion or until `shutdown` is called
- Each unit processes messages sequentially (in-order delivery per unit)
- No ordering guarantee across different execution units

### Lifecycle contract

States: `Active` → `Draining` → `Terminated` | `Failed`

- `Active`: unit is running and can receive messages
- `Draining`: shutdown requested, completing in-flight work, stops accepting new messages. Draining does not guarantee eventual `Terminated` — if the unit's handler hangs, the unit MAY remain in `Draining` permanently. Backends SHOULD make reasonable efforts to complete draining.
- `Terminated`: completed successfully
- `Failed`: terminated due to unrecoverable error

### Scheduling contract

- Runtime backends MUST provide reasonable forward progress
- Backends SHOULD avoid starvation
- Cross-unit fairness is implementation-defined
- Within a single unit, messages are processed sequentially in arrival order
- Backends MAY provide priority scheduling as an extension (not in core trait)

### Isolation contract

- Failures in one execution unit MUST NOT cascade to other units
- Each unit has an independent execution context
- Unhandled panics in a unit MUST be caught and result in `Failed` state only for that unit
- The runtime MUST remain operational when a single unit fails

### Failure contract — two distinct modes

**UNIT FAILURE** (single execution unit fails):
- The runtime SHALL survive: other units continue running
- Unit state transitions to `Failed`
- `spawn` and `send` continue to work for unaffected units
- No cascading failure

**RUNTIME INTERNAL FAILURE** (unrecoverable runtime-internal error):
- The runtime SHALL fail closed: shutdown all units and refuse new work
- All units transition to `Failed`
- `spawn` returns `Err(SpawnError { cause: SpawnErrorKind::Closed })` — no panic, no fake id, no noop
- `send` to any id returns `SendError`
- `state` returns `None`
- All runtime errors are non-panicking (return values, not unwinding)

## Backend Adapter Model

Each backend crate implements `Runtime` for its type:

- **TokioRuntime**: wraps `tokio::runtime::Runtime`, routes messages to execution units
- **GoaktRuntime**: wraps Goakt actor system, maps native actor refs to `ExecutionId`
- **ProtoActorRuntime**: wraps ProtoActor PID, maps to `ExecutionId`
- **AkkaRuntime**: wraps Akka native ref, maps to `ExecutionId`

Actor framework backends integrate by:
1. Implementing `Runtime` trait methods
2. Mapping their native handle/PID/ref types to `ExecutionId`
3. Adapting their message routing to the `send` contract
4. Translating their lifecycle model to `ExecutionState`


## Design Decisions

### Decision 1: Runtime trait uses `ExecutionId` as handle

No generic associated types. `ExecutionId` is the universal handle returned from `spawn` and accepted by all operations. Simple, backend-neutral, no coupling to per-message handle types.

Alternatives considered:
- GAT `type Handle<M>`: over-engineering for v1, couples trait to message types
- Caller-provided id: shifts id generation burden, inconsistent across backends

### Decision 2: `spawn` takes FnOnce(RuntimeHandle) -> Future

`spawn` takes `FnOnce(RuntimeHandle) -> Future<Output = ()>`. The runtime constructs the `RuntimeHandle`, injects it into the closure, then executes the returned future. This allows units to receive their scoped handle at creation time. Returns `Result<ExecutionId, SpawnError>` to support fail-closed behavior. No `Actor` trait in the platform API.

### Decision 3: Message sending is a Runtime operation

`send` is on `Runtime`, not on the spawned unit. The runtime knows how to route messages. Backends implement routing internally (implementation-defined transport). This keeps the trait minimal and backend-neutral.

### Decision 4: State query is optional

`state()` returns `Option<ExecutionState>`. Backends that track lifecycle return `Some`. Simple backends return `None`. Prevents forcing state tracking on all implementations.

### Decision 5: RuntimeHandle scoped access (closure-based)

Spawned units receive a `RuntimeHandle` scoped to the local unit. This token provides: identity (`id()`), self-send (`send_self(msg)`), and lifecycle control (`shutdown()`, `state()`).

`send_self` is generic (`send_self<M: Send + 'static>`). The `send_self_fn` closure is wired at spawn time to deliver messages to this unit; boxing is an internal detail hidden from the caller.

RuntimeHandle uses closure-based internals for all operations, avoiding `dyn Runtime` (impossible because `Runtime` is generic and not object-safe). Remote routing uses `Runtime::send(id, msg)`.

### Decision 6: DefaultRuntime lives in runtime-tokio, not runtime

`DefaultRuntime` aliases `TokioRuntime` in the `runtime-tokio` crate. Avoids circular dependency: `runtime-tokio` depends on `runtime` for the trait; `DefaultRuntime` is defined where the implementation lives. The `runtime` crate has `uuid` (utility only) and zero runtime/backend dependencies.

### Decision 7: Sequential execution per unit

Each execution unit processes messages in arrival order. This is a core guarantee that all backends MUST provide. Cross-unit ordering is not guaranteed. Enables deterministic reasoning about per-unit behavior.

### Decision 8: Failure isolation

Unhandled errors in one unit MUST NOT affect other units. The runtime catches panics and transitions the failed unit to `Failed` state. The runtime itself continues operating. Unrecoverable runtime errors cause fail-closed shutdown.

## Dependency Boundaries

```
ego-runtime (crates/runtime):
  dependencies:    uuid = { version = "1", features = ["v4"] } (utility only)
  dev-dependencies: (none)
  forbidden:       tokio, goakt, protoactor, akka, persistence, transport

  uuid is a foundational utility dependency (id generation),
  NOT a runtime/backend coupling.
  The runtime crate has ZERO RUNTIME/BACKEND DEPENDENCIES.

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
- Mailbox types SHALL NOT be in the core contract
- Supervision types SHALL NOT be in the core contract
- No Tokio types in the `ego-runtime` crate
- No framework-specific semantics in the `Runtime` trait
- No assumption that all backends provide mailbox/supervision/actor behavior
- No coupling between runtime abstraction and actor execution model
- No mailbox-full error variant in core error types
- RuntimeHandle MUST NOT store `dyn Runtime` (Runtime is not object-safe)
- spawn contract MUST NOT return raw `ExecutionId` (must return `Result` for fail-closed)

## Risks / Trade-offs

- [Minimal surface] The trait may prove too narrow. Mitigation: start with spawn/send/shutdown/state, expand only when a second backend proves the need.
- [Sequential guarantee] Enforcing per-unit sequential execution may constrain high-throughput backends. Mitigation: sequential applies to message handling within a unit; backends can parallelize across units.
- [Tokio coupling] The ecosystem may still converge on Tokio. Mitigation: CI runs with `NullRuntime` to verify abstraction works without Tokio.

## Anti-Hallucination Assumptions

ASSUMPTION: `ego-runtime` crate will be created at `crates/runtime/`. Not yet on disk.

ASSUMPTION: `ego-runtime-tokio` crate will be created at `crates/runtime-tokio/`. Not yet on disk.

ASSUMPTION: Existing workspace crates (`ego-domain`, `ego-application`, `ego-infrastructure`, `ego-transport`, `ego-runtime-slice`) remain unchanged.
