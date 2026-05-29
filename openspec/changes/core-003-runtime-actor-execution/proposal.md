## Why

Previous CORE-003 spec modeled `ActorSystem` as the runtime entry point. This was architecturally incorrect.

ego.rs is NOT an actor framework. It is NOT an actor-first system. It provides a runtime abstraction contract as its platform identity. Actor frameworks are optional backend implementations that integrate through a shared runtime interface — they are integrations, not the platform.

`ActorSystem` MUST NOT be the platform entry point. The `Runtime` trait replaces it as the public contract. Actor semantics (handle types, mailbox patterns, supervision strategies) are optional backend features implemented behind the Runtime interface, not core requirements.

Tokio is the Default runtime engine, not the platform identity. Tokio MUST remain hidden behind the abstraction. The public API MUST be backend-neutral.

This change replaces the previous runtime architecture with a runtime-neutral execution abstraction. Runtime neutrality means the contract is identical regardless of backend. Tokio is the default runtime engine but invisible to consumers of the Runtime trait. The default runtime engine can be swapped without changing consumer code.

## Rename

TO: `CORE-003: Runtime Execution Abstraction`

## Architectural Intent

```
┌──────────────────────────────────────────────────┐
│                   Domain Code                     │
│         (ego-domain: Actor, ActorId, etc.)       │
│         consumes impl Runtime (backend-agnostic) │
└────────────────────┬─────────────────────────────┘
                     │  depends on
                     ▼
┌──────────────────────────────────────────────────┐
│         Runtime Abstraction Contract             │
│           Runtime trait (platform API)           │
│                                                   │
│  ExecutionId  │  ExecutionState  │  SendError     │
│  RuntimeHandle│  isolation       │  fail-closed   │
│  sequential   │  lifecycle       │  scheduling    │
└────────────────────┬─────────────────────────────┘
                     │  implemented by
                     ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────┐
│  TokioRuntime    │ │  GoaktRuntime    │ │  ...more │
│  (default)       │ │  (actor backend) │ │          │
│  hidden behind   │ │  optional        │ │          │
│  abstraction     │ │                  │ │          │
└──────────────────┘ └──────────────────┘ └──────────┘
```

The Runtime trait IS the platform. Actor concepts are implementation details of specific backends.

## What Changes

### New artifacts

crate:
`crates/runtime` (package: `ego-runtime`)

module:
`runtime`

files:
`crates/runtime/src/runtime/runtime.rs`      — Runtime trait
`crates/runtime/src/runtime/execution.rs`     — ExecutionId, execution semantics
`crates/runtime/src/runtime/lifecycle.rs`     — ExecutionState, lifecycle semantics
`crates/runtime/src/runtime/scheduler.rs`     — Scheduling contract
`crates/runtime/src/runtime/isolation.rs`     — Isolation guarantees
`crates/runtime/src/runtime/failure.rs`       — SendError, fail-closed behavior
`crates/runtime/src/runtime/handle.rs`        — RuntimeHandle
`crates/runtime/src/runtime/mod.rs`           — Module root
`crates/runtime/src/lib.rs`                   — Re-exports

responsibility:
Runtime abstraction contract — backend-neutral execution interface.

dependencies:
`uuid = { version = "1", features = ["v4"] }` — foundational utility for `ExecutionId`

forbidden dependencies:
tokio, goakt, protoactor, akka, persistence, transport

dependency rationale:
`uuid` is a foundational utility dependency (unique id generation), NOT a runtime coupling.
The `runtime` crate has zero RUNTIME/BACKEND dependencies.

---

crate:
`crates/runtime-tokio` (package: `ego-runtime-tokio`)

module:
(lib root)

files:
`crates/runtime-tokio/src/lib.rs`

responsibility:
Default Tokio backend implementing the Runtime trait.

dependencies:
`ego-runtime`, `tokio`

forbidden dependencies:
goakt, protoactor, akka, persistence, transport

### Modified artifacts

file:
`layers.toml`

change:
Add entries:
```toml
"ego-runtime"      = "foundation"
"ego-runtime-tokio" = "infrastructure"
```

do not change:
Existing layer definitions. Existing dependency rules.

file:
`Cargo.toml` (workspace root)

change:
Add `ego-runtime` and `ego-runtime-tokio` to workspace members.

do not change:
Existing workspace members. Resolver. Shared dependencies.

### What Does NOT Change

- The change id (`core-003`) remains
- The existing `ego-domain` crate (Actor, ActorId, ActorLifecycleState, SupervisionStrategy remain)
- The existing `ego-runtime-slice` crate (CORE-001 remains unchanged)
- Tokio remains the default runtime engine

### What Must NOT Be Modified

- `Runtime` trait MUST NOT expose backend-specific types
- `Runtime` trait MUST NOT expose actor-specific semantics
- No Tokio types in `ego-runtime` crate public API
- No actor vocabulary in `ego-runtime` crate public API
- No `ActorSystem` anywhere in the platform API
- No mailbox types, actor handle types, or actor handle abstractions in core contract
- ActorSystem, mailbox, and supervision types from old spec are REMOVED — not ported

## New Features

- `runtime-execution-abstraction`: Runtime trait with spawn/send/shutdown/state semantics, backend-neutral vocabulary, sequential execution guarantee, isolation guarantee, fail-closed behavior
- `tokio-runtime-engine`: TokioRuntime implementing Runtime trait, default engine

## Dependency Boundaries

```
ego-runtime-tokio ──depends on──▶ ego-runtime ──depends on──▶ uuid (utility only)
                                    │
                              forbidden deps:
                              tokio, goakt, protoactor,
                              akka, persistence, transport
```

`uuid` is a foundational utility dependency (id generation), NOT a runtime/backend coupling.
The `runtime` crate has zero RUNTIME/BACKEND dependencies.

## Implementation Order

1. `Cargo.toml` workspace: add `crates/runtime` and `crates/runtime-tokio` members
2. `crates/runtime/Cargo.toml`: create with zero required dependencies
3. `crates/runtime/src/runtime/`: vocabulary types (ExecutionId, ExecutionState, SendError, RuntimeHandle)
4. `crates/runtime/src/runtime/`: Runtime trait with execution, lifecycle, scheduling, isolation, failure semantics
5. `crates/runtime/src/lib.rs`: re-exports
6. `crates/runtime-tokio/Cargo.toml`: create with ego-runtime + tokio deps
7. `crates/runtime-tokio/src/lib.rs`: TokioRuntime implementing Runtime
8. `crates/runtime-tokio/src/lib.rs`: TokioRuntimeBuilder, DefaultRuntime alias
9. `layers.toml`: add ego-runtime and ego-runtime-tokio entries
10. Verification: NullRuntime, contract tests, integration tests
