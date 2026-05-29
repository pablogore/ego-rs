## Implementation Tasks

Implementation order: tasks MUST be completed in sequence (each task depends on previous).

---

### 1. Workspace setup

#### 1.1 Add workspace members

MODIFY

file:
`Cargo.toml` (workspace root)

change:
Add `"crates/runtime"` and `"crates/runtime-tokio"` to workspace members array.

do not change:
Existing workspace members. Resolver. Shared dependencies.

---

#### 1.2 Create `ego-runtime` crate scaffold

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
```

responsibility:
Runtime abstraction crate — zero required dependencies.

dependencies:
(none)

forbidden dependencies:
tokio, goakt, protoactor, akka, persistence, transport

---

#### 1.3 Create `ego-runtime-tokio` crate scaffold

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

#### 1.4 Create runtime module directory structure

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

### 2. Vocabulary types

#### 2.1 Create runtime/mod.rs

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

#### 2.2 ExecutionId type

CREATE

crate:
`ego-runtime`

module:
`runtime::execution`

file:
`crates/runtime/src/runtime/execution.rs`

implement:
`ExecutionId` newtype wrapping `Uuid`. Constructor `new()` generates a random `Uuid`. Implement `Clone`, `Debug`, `Eq`, `Hash`, `Send`, `Sync`.

responsibility:
Unique identifier for spawned execution units. Backend-neutral — no framework-specific fields.

---

#### 2.3 ExecutionState enum

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

#### 2.4 SendError and SendErrorKind

CREATE

crate:
`ego-runtime`

module:
`runtime::failure`

file:
`crates/runtime/src/runtime/failure.rs`

implement:
`SendError` struct with `id: ExecutionId` and `cause: SendErrorKind`. `SendErrorKind` enum with `NotFound`, `Closed`. Add `#[non_exhaustive]` to `SendErrorKind`. Implement `Debug`, `Display`, `std::error::Error` for `SendError`.

Must NOT include:
- `MailboxFull` variant (actor-specific, not runtime-neutral)

responsibility:
Error returned when message delivery fails. Runtime-neutral error kinds only.

---

#### 2.5 RuntimeHandle type

CREATE

crate:
`ego-runtime`

module:
`runtime::handle`

file:
`crates/runtime/src/runtime/handle.rs`

implement:
`RuntimeHandle` struct with methods `id()`, `send(msg)`, `shutdown()`, `state()`. Implement `Clone`, `Send`, `Sync`. Wraps `ExecutionId` and a sender to the runtime's routing layer.

responsibility:
Scoped runtime access for spawned execution units.

---

### 3. Runtime trait and semantics

#### 3.1 Runtime trait definition

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
use crate::runtime::failure::SendError;
use std::future::Future;

pub trait Runtime: Send + Sync + 'static {
    fn spawn<F>(&self, f: F, name: Option<&str>) -> ExecutionId
    where F: Future<Output = ()> + Send + 'static;

    fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where M: Send + 'static;

    fn shutdown(&self, id: &ExecutionId);

    fn state(&self, id: &ExecutionId) -> Option<ExecutionState>;
}
```

Also add:
```rust
#[derive(Clone, Debug, Default)]
pub struct Capabilities(u64);

impl Capabilities {
    pub const fn empty() -> Self { Self(0) }
    pub const fn new(bits: u64) -> Self { Self(bits) }
}
```

Add capability discovery method to Runtime trait with default impl:
```rust
fn capabilities(&self) -> Capabilities {
    Capabilities::empty()
}
```

responsibility:
Stable runtime abstraction contract. All runtime backends implement this trait.

forbidden:
- No Tokio types in signatures
- No actor vocabulary (no Actor, ActorId, ActorSystem, ActorLifecycleState, ActorHandle, mailbox types, actor handle types)
- No backend-specific trait bounds

---

#### 3.2 Isolation module

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

#### 3.3 Scheduler module

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
/// - Execution units are scheduled fairly.
/// - No single unit can starve others.
/// - Cross-unit ordering is not guaranteed.
/// - Within a single unit, messages are processed sequentially in arrival order.
```

No runtime code required — this module documents the scheduling contract that all backends MUST implement.

responsibility:
Documents the scheduling contract.

---

#### 3.4 lib.rs exports

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
pub use runtime::failure::{SendError, SendErrorKind};
pub use runtime::handle::RuntimeHandle;
```

responsibility:
Public API surface of the `ego-runtime` crate.

forbidden:
- Do NOT export `NullRuntime` (test-only)

---

### 4. Tokio runtime backend

#### 4.1 TokioRuntime struct — Runtime impl

CREATE

crate:
`ego-runtime-tokio`

file:
`crates/runtime-tokio/src/lib.rs`

implement:
`TokioRuntime` struct wrapping `tokio::runtime::Runtime`. Include internal routing table: `Arc<Mutex<HashMap<ExecutionId, (ExecutionState, mpsc::Sender<Box<dyn Any + Send>>)>>>`.

Implement `Runtime` trait:
- `spawn`: spawn future on tokio runtime, register id in routing table, create per-unit `mpsc` channel for sequential message processing
- `send`: look up id in routing table, send message via channel
- `shutdown`: send stop signal, mark state as `Draining`
- `state`: return current state from routing table

Ensure:
- Sequential execution: per-unit channel processes messages in FIFO order
- Isolation: wrap each spawned unit in a panic boundary (`catch_unwind` or task-local error handling)
- Fail-closed: on runtime internal error, drain all units and reject new work

forbidden dependencies:
goakt, protoactor, akka, persistence, transport

responsibility:
Default Tokio-backed runtime implementation with sequential execution, isolation, and fail-closed guarantees.

---

#### 4.2 TokioRuntimeBuilder

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

#### 4.3 DefaultRuntime alias

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

### 5. Integration

#### 5.1 Update layers.toml

MODIFY

file:
`layers.toml`

change:
Add entries:
```toml
"ego-runtime"      = "domain"
"ego-runtime-tokio" = "domain"
```
(or appropriate layer level consistent with existing rules)

do not change:
Existing layer definitions. Dependency direction rules.

---

### 6. Verification

#### 6.1 Runtime trait contract tests

CREATE

crate:
`ego-runtime`

file:
`crates/runtime/src/runtime/runtime.rs`

location:
`#[cfg(test)]` module

implement `NullRuntime`:
- Returns distinct `ExecutionId` for each `spawn` call
- Tracks spawned units in `HashMap<ExecutionId, ExecutionState>`
- `send` looks up the unit and discards the message
- `shutdown` sets state to `Terminated`
- `state` returns tracked state

tests:
- `test_spawn_returns_unique_id`: spawn twice, verify ids differ
- `test_send_to_unknown_id_returns_error`: send to non-existent id, expect `SendError`
- `test_send_to_closed_returns_error`: shutdown then send, expect `SendError`
- `test_shutdown_terminates_unit`: spawn, shutdown, verify state transitions
- `test_failure_isolation`: spawn unit that panics, verify other units unaffected
- `test_capability_discovery_default`: verify default capabilities are empty

responsibility:
Verify Runtime trait contract semantics regardless of backend.

---

#### 6.2 TokioRuntime integration tests

CREATE

crate:
`ego-runtime-tokio`

file:
`crates/runtime-tokio/tests/tokio_runtime_test.rs`

tests:
- `test_multi_threaded_default`: create default TokioRuntime, spawn unit, verify state
- `test_current_thread`: create current-thread TokioRuntime, spawn, verify
- `test_send_message`: spawn unit, send message, verify delivery and processing
- `test_sequential_delivery`: send multiple messages to same unit, verify order
- `test_failure_isolation`: spawn unit that panics, verify other units unaffected
- `test_shutdown`: spawn unit, shutdown, verify termination
- `test_configured_worker_threads`: build with 4 workers, verify
- `test_fail_closed`: trigger internal error, verify runtime refuses new work

responsibility:
Integration tests for TokioRuntime against the full Runtime contract.

---

#### 6.3 Workspace verification

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

responsibility:
Full workspace compilation and test pass.
