## Implementation Tasks

Implementation order: tasks MUST be completed in sequence (each task depends on previous).

---

### [ ] 1. Workspace setup

#### [x] 1.1 Add workspace members

MODIFY

file:
`Cargo.toml` (workspace root)

change:
Add `"crates/runtime"` and `"crates/runtime-tokio"` to workspace members array.

do not change:
Existing workspace members. Resolver. Shared dependencies.

---

#### [x] 1.2 Create `ego-runtime` crate scaffold

CREATE

crate:
`ego-runtime`

file:
`crates/runtime/Cargo.toml`

implement:
```toml
[package]
name = "ego-runtime"
version = "0.1.0"
edition = "2021"

[dependencies]
uuid = { version = "1", features = ["v4"] }
```

responsibility:
Runtime abstraction crate — uuid is a foundational utility dependency (id generation), NOT a runtime/backend coupling. Zero runtime/backend dependencies.

dependencies:
`uuid` (utility only)

forbidden dependencies:
tokio, goakt, protoactor, akka, persistence, transport

---

#### [ ] 1.3 Create `ego-runtime-tokio` crate scaffold

CREATE

crate:
`ego-runtime-tokio`

file:
`crates/runtime-tokio/Cargo.toml`

implement:
```toml
[package]
name = "ego-runtime-tokio"
version = "0.1.0"
edition = "2021"

[dependencies]
ego-runtime = { path = "../runtime" }
tokio = { version = "1", features = ["full"] }
```

responsibility:
Tokio backend crate — depends on `ego-runtime` for the trait and `tokio` for execution.

dependencies:
`ego-runtime`, `tokio`

forbidden dependencies:
goakt, protoactor, akka, persistence, transport

---

#### [ ] 1.4 Create runtime module directory structure

CREATE

files:
`crates/runtime/src/lib.rs`
`crates/runtime/src/runtime/mod.rs`
`crates/runtime/src/runtime/runtime.rs`
`crates/runtime/src/runtime/execution.rs`
`crates/runtime/src/runtime/lifecycle.rs`
`crates/runtime/src/runtime/failure.rs`
`crates/runtime/src/runtime/handle.rs`
`crates/runtime/src/runtime/scheduler.rs`
`crates/runtime/src/runtime/isolation.rs`

implement:
Create empty module files with `// TODO` stubs. Full implementation follows in subsequent tasks.

responsibility:
Module scaffold for the runtime abstraction crate.

---

### [ ] 2. Vocabulary types

#### [ ] 2.1 Create runtime/mod.rs

CREATE

file:
`crates/runtime/src/runtime/mod.rs`

implement:
```rust
pub mod runtime;
pub mod execution;
pub mod lifecycle;
pub mod failure;
pub mod handle;
pub mod scheduler;
pub mod isolation;
```

responsibility:
Module declarations for the `runtime` module.

---

#### [ ] 2.2 ExecutionId type

CREATE

crate:
`ego-runtime`

module:
`runtime::execution`

file:
`crates/runtime/src/runtime/execution.rs`

implement:
`ExecutionId` newtype wrapping `Uuid`. Constructor `new()` generates a random `Uuid` (v4). Implement `Clone`, `Copy`, `Debug`, `Eq`, `Hash`, `Send`, `Sync`.

responsibility:
Unique identifier for spawned execution units. Backend-neutral — no framework-specific fields.

---

#### [ ] 2.3 ExecutionState enum

CREATE

crate:
`ego-runtime`

module:
`runtime::lifecycle`

file:
`crates/runtime/src/runtime/lifecycle.rs`

implement:
`ExecutionState` enum with variants `Active`, `Draining`, `Terminated`, `Failed`. Add `#[non_exhaustive]`. Implement `Clone`, `Debug`, `PartialEq`, `Send`, `Sync`.

responsibility:
Lifecycle state of an execution unit. Backend-neutral lifecycle model.

---

#### [ ] 2.4 SendError, SendErrorKind, SpawnError, SpawnErrorKind

CREATE

crate:
`ego-runtime`

module:
`runtime::failure`

file:
`crates/runtime/src/runtime/failure.rs`

implement:
`SendError` struct with `id: ExecutionId` and `cause: SendErrorKind`. `SendErrorKind` enum with `NotFound`, `Closed`. Add `#[non_exhaustive]` to `SendErrorKind`. Implement `Debug`, `Display`, `std::error::Error` for `SendError`.

Add `SpawnError` struct with `pub cause: SpawnErrorKind`. `SpawnErrorKind` enum with `Closed`, `Internal`. Add `#[non_exhaustive]` to both. Implement `Debug`, `Display`, `std::error::Error` for both.

Must NOT include:
- `MailboxFull` variant (actor-specific, not runtime-neutral)

responsibility:
Error types for message delivery failure and spawn failure. Runtime-neutral error kinds only.

---

#### [ ] 2.5 RuntimeHandle type

CREATE

crate:
`ego-runtime`

module:
`runtime::handle`

file:
`crates/runtime/src/runtime/handle.rs`

implement:
`RuntimeHandle` struct with closure-based internal structure. MUST NOT store `dyn Runtime` (Runtime is not object-safe — generic methods).

Internal model:
```rust
pub struct RuntimeHandle {
    id: ExecutionId,
    send_self_fn: Arc<dyn Fn(Box<dyn Any + Send>) -> Result<(), SendError> + Send + Sync>,
    shutdown_fn: Arc<dyn Fn() + Send + Sync>,
    state_fn: Arc<dyn Fn() -> Option<ExecutionState> + Send + Sync>,
}
```

Public methods: `id()`, `send_self(msg)`, `shutdown()`, `state()`. Implement `Clone`, `Send`, `Sync`.

`send_self` routes ONLY to the local execution unit (self-scoped). The `send_self_fn` closure is wired at spawn time to deliver messages to this unit — this is the token-based approach. `Any` boxing is an internal implementation detail, NOT exposed in the public generic API.

responsibility:
Scoped runtime access for spawned execution units (identity + lifecycle token). Closure-based to avoid impossible `dyn Runtime`.

dependencies:
`crate::runtime::execution::ExecutionId`
`crate::runtime::lifecycle::ExecutionState`
`crate::runtime::failure::SendError`
`std::sync::Arc`
`std::any::Any`

forbidden dependencies:
tokio, goakt, protoactor, akka

---

### [ ] 3. Runtime trait and semantics

#### [ ] 3.1 Runtime trait definition

CREATE

crate:
`ego-runtime`

module:
`runtime::runtime`

file:
`crates/runtime/src/runtime/runtime.rs`

implement:
```rust
use crate::runtime::execution::ExecutionId;
use crate::runtime::lifecycle::ExecutionState;
use crate::runtime::failure::{SendError, SpawnError};
use crate::runtime::handle::RuntimeHandle;
use std::future::Future;

pub trait Runtime: Send + Sync + 'static {
    fn spawn<F, Fut>(
        &self,
        f: F,
        name: Option<&str>,
    ) -> Result<ExecutionId, SpawnError>
    where
        F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;

    fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where M: Send + 'static;

    fn shutdown(&self, id: &ExecutionId);

    fn state(&self, id: &ExecutionId) -> Option<ExecutionState>;
}
```

responsibility:
Stable runtime abstraction contract. All runtime backends implement this trait.

forbidden:
- No Tokio types in signatures
- No actor vocabulary (no Actor, ActorId, ActorSystem, ActorLifecycleState, ActorHandle, mailbox types, actor handle types)
- No backend-specific trait bounds

---

#### [ ] 3.2 Isolation module

CREATE

crate:
`ego-runtime`

module:
`runtime::isolation`

file:
`crates/runtime/src/runtime/isolation.rs`

implement:
Module-level documentation defining the isolation contract:

```rust
/// Isolation guarantees for the Runtime abstraction.
///
/// ## Contract
///
/// - Each execution unit has an independent execution context.
/// - Failures in one unit MUST NOT cascade to other units.
/// - Unhandled panics in a unit MUST be caught by the runtime
///   and result in `ExecutionState::Failed` for that unit only.
/// - The runtime MUST remain operational after a single unit failure.
///
/// ## Implementation responsibility
///
/// All Runtime backends MUST enforce these guarantees.
```

No runtime code required — this module documents the isolation contract that all backends MUST implement.

responsibility:
Documents the isolation guarantee contract.

---

#### [ ] 3.3 Scheduler module

CREATE

crate:
`ego-runtime`

module:
`runtime::scheduler`

file:
`crates/runtime/src/runtime/scheduler.rs`

implement:
Module-level documentation defining the scheduling contract:

```rust
/// Scheduling contract for the Runtime abstraction.
///
/// ## Contract
///
/// - Runtime backends MUST provide reasonable forward progress.
/// - Backends SHOULD avoid starvation.
/// - Cross-unit fairness is implementation-defined.
/// - Within a single unit, messages are processed sequentially in arrival order.
```

No runtime code required — this module documents the scheduling contract that all backends MUST implement.

responsibility:
Documents the scheduling contract.

---

#### [ ] 3.4 lib.rs exports

CREATE

crate:
`ego-runtime`

module:
(lib root)

file:
`crates/runtime/src/lib.rs`

implement:
```rust
pub mod runtime;

pub use runtime::runtime::Runtime;
pub use runtime::execution::ExecutionId;
pub use runtime::lifecycle::ExecutionState;
pub use runtime::failure::{SendError, SendErrorKind, SpawnError, SpawnErrorKind};
pub use runtime::handle::RuntimeHandle;
```

responsibility:
Public API surface of the `ego-runtime` crate.

forbidden:
- Do NOT export `NullRuntime` (test-only)

---

### [ ] 4. Tokio runtime backend

#### [ ] 4.1 TokioRuntime struct — Runtime impl

CREATE

crate:
`ego-runtime-tokio`

file:
`crates/runtime-tokio/src/lib.rs`

implement:
`TokioRuntime` struct wrapping `tokio::runtime::Runtime`. Include an internal routing mechanism for message dispatch, mapping `ExecutionId` to deliverable units.

Implement `Runtime` trait:
- `spawn`: register execution unit, construct `RuntimeHandle` with `send_self_fn` wired to deliver messages to this unit, `shutdown_fn` to request termination, `state_fn` to query state, call factory closure `f(handle)` to create unit future, wrap unit future with sequential message processing, spawn wrapped future on tokio runtime, return `Ok(ExecutionId)`
- `send`: route message to target unit identified by `ExecutionId`
- `shutdown`: signal target unit to drain and terminate
- `state`: return current lifecycle state

Ensure:
- Sequential execution: messages delivered to a unit are processed one at a time in arrival order
- Isolation: execution unit panics MUST be caught and result in `ExecutionState::Failed` for that unit only, without cascading to other units
- Fail-closed: on runtime internal error, return `Err(SpawnError { cause: SpawnErrorKind::Closed })` from spawn, drain all units, reject new work

forbidden dependencies:
goakt, protoactor, akka, persistence, transport

responsibility:
Default Tokio-backed runtime implementation with sequential execution, isolation, and fail-closed guarantees.

---

#### [ ] 4.2 TokioRuntimeBuilder

CREATE

crate:
`ego-runtime-tokio`

file:
`crates/runtime-tokio/src/lib.rs`

implement:
`TokioRuntimeBuilder` with methods:
- `fn worker_threads(self, n: usize) -> Self`
- `fn current_thread(self) -> Self`
- `fn build(self) -> TokioRuntime`

Builder configures `tokio::runtime::Builder` internally.

responsibility:
Configuration API for `TokioRuntime`.

---

#### [ ] 4.3 DefaultRuntime alias

CREATE

crate:
`ego-runtime-tokio`

file:
`crates/runtime-tokio/src/lib.rs`

implement:
`pub type DefaultRuntime = TokioRuntime;`

Implement `impl Default for TokioRuntime` that creates a multi-threaded runtime with default settings (worker threads = available parallelism).

responsibility:
Convenience alias and default constructor. Lives in `ego-runtime-tokio` to avoid circular dependency on `ego-runtime`.

---

### [ ] 5. Integration

#### [ ] 5.1 Update layers.toml

MODIFY

file:
`layers.toml`

change:
Add entries:
```toml
"ego-runtime"      = "foundation"
"ego-runtime-tokio" = "infrastructure"
```

do not change:
Existing layer definitions. Dependency direction rules.

---

### [ ] 6. Verification

#### [ ] 6.1 Runtime trait contract tests

CREATE

crate:
`ego-runtime`

file:
`crates/runtime/src/runtime/runtime.rs`

location:
`#[cfg(test)]` module

implement `NullRuntime`:
- Returns distinct `ExecutionId` for each `spawn` call
- Tracks spawned units in an internal state registry
- `send` stores received messages for test assertion
- `shutdown` sets state to `Terminated`
- `state` returns tracked state

tests:
- `test_spawn_returns_unique_id`: spawn twice via `spawn(factory, None).unwrap()`, verify ids differ
- `test_spawn_after_shutdown_returns_error`: spawn after runtime shutdown, expect `Err(SpawnError { cause: SpawnErrorKind::Closed })`
- `test_spawn_after_failure_returns_internal_error`: trigger runtime failure, spawn, expect `Err(SpawnError { cause: SpawnErrorKind::Internal })`
- `test_send_to_unknown_id_returns_error`: send to non-existent id, expect `SendError`
- `test_send_to_closed_returns_error`: shutdown then send, expect `SendError`
- `test_shutdown_terminates_unit`: `spawn(factory, None).unwrap()`, shutdown, verify state transitions
- `test_failure_isolation`: `spawn(factory, None).unwrap()` a unit that panics, verify other units `spawn(...).unwrap()` unaffected

responsibility:
Verify Runtime trait contract semantics regardless of backend.

---

#### [ ] 6.2 TokioRuntime integration tests

CREATE

crate:
`ego-runtime-tokio`

file:
`crates/runtime-tokio/tests/tokio_runtime_test.rs`

tests:
- `test_multi_threaded_default`: create default TokioRuntime, `spawn(factory, None).unwrap()`, verify state
- `test_current_thread`: create current-thread TokioRuntime, `spawn(factory, None).unwrap()`, verify
- `test_spawn_after_failure_returns_error`: trigger internal error, verify `spawn` returns `Err(SpawnError { cause: SpawnErrorKind::Closed })`
- `test_send_message`: `spawn(factory, None).unwrap()`, send message, verify delivery and processing
- `test_sequential_delivery`: `spawn(factory, None).unwrap()`, send multiple messages to same unit, verify order
- `test_failure_isolation`: `spawn(factory, None).unwrap()` unit that panics, verify other units `spawn(...).unwrap()` unaffected
- `test_shutdown`: `spawn(factory, None).unwrap()`, shutdown, verify termination
- `test_configured_worker_threads`: build with 4 workers, `spawn(factory, None).unwrap()`, verify
- `test_fail_closed`: trigger internal error, verify spawn returns `Err(...)` and send returns `Err(...)`

responsibility:
Integration tests for TokioRuntime against the full Runtime contract.

---

#### [ ] 6.3 Workspace verification

VERIFY

commands:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

acceptance criteria:

- workspace compiles without errors
- all tests pass
- clippy produces no warnings
- no regressions in existing workspace members
