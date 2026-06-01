# Runtime Execution Abstraction

## Intent

Replace `ActorSystem` as platform entry point with backend-neutral `Runtime` trait. The `Runtime` trait IS the platform — actor frameworks (Tokio, Goakt, ProtoActor) are optional backend implementations behind the Runtime interface.

## Why

Previous architecture had `ActorSystem` as entry point — architecturally incorrect. ego.rs is NOT an actor framework. Actor backends are integrations, not the platform.

## Contract

- `Runtime` trait: `spawn`, `send`, `shutdown`, `state` — backend-neutral, no GATs, no actor types
- `ExecutionId`: Uuid-backed unique handle for spawned units
- `ExecutionState`: `Active -> Draining -> Terminated | Failed`, `#[non_exhaustive]`
- `SendError` / `SpawnError`: runtime-neutral error kinds (`NotFound`, `Closed`, `Internal`)
- `RuntimeHandle`: closure-based scoped access for spawned units (no `dyn Runtime`)
- Sequential per-unit message processing, failure isolation, fail-closed on internal error

## Forbidden

- No `ActorSystem` anywhere in platform API
- No actor handle/mailbox/supervision types in core contract
- No Tokio types in `ego-runtime` crate public API
- No GATs in `Runtime` trait
- `RuntimeHandle` MUST NOT store `dyn Runtime`

## Physical Structure

```
crates/runtime/src/
├── lib.rs                      — re-exports
└── runtime/
    ├── mod.rs                  — module declarations
    ├── runtime.rs              — Runtime trait, NullRuntime (test)
    ├── execution.rs            — ExecutionId
    ├── lifecycle.rs            — ExecutionState
    ├── failure.rs              — SendError, SpawnError
    ├── handle.rs               — RuntimeHandle
    ├── scheduler.rs            — scheduling contract (doc)
    └── isolation.rs            — isolation contract (doc)

crates/runtime-tokio/src/
    └── lib.rs                  — TokioRuntime, TokioRuntimeBuilder, DefaultRuntime
```

## Dependencies

- `ego-runtime` depends on `uuid` (utility only) — zero runtime/backend deps
- `ego-runtime-tokio` depends on `ego-runtime` + `tokio`

## References

- `openspec/changes/core-003-runtime-actor-execution/proposal.md` — architectural rationale
- `openspec/changes/core-003-runtime-actor-execution/design.md` — interface contracts, design decisions
- `openspec/changes/core-003-runtime-actor-execution/spec.md` — verifiable requirements, scenarios
- `openspec/changes/core-003-runtime-actor-execution/tasks.md` — rewritten FIRE execution tasks
