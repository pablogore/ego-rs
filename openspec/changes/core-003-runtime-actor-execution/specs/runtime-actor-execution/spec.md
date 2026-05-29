## ADDED Requirements

---

### Requirement: ExecutionId type

owner crate:
`ego-runtime`

owner module:
`runtime::execution`

file:
`crates/runtime/src/runtime/execution.rs`

public types:
`ExecutionId`

evolution:
Identifier representation changes affect only `execution.rs`.

The system SHALL define an `ExecutionId` newtype that uniquely identifies a spawned execution unit within a runtime instance.

`ExecutionId` SHALL implement `Clone`, `Debug`, `Eq`, `Hash`, `Send`, `Sync`.

`ExecutionId` SHALL provide a `new()` constructor that generates a unique identifier.

`ExecutionId` SHALL be backend-neutral — no framework-specific fields or methods.

#### Scenario: Distinct ids per spawn
- **WHEN** two execution units are spawned on the same runtime
- **THEN** each receives a distinct `ExecutionId`
- **AND** `id1 != id2`

#### Scenario: Thread-safe identification
- **WHEN** `ExecutionId` is passed across thread boundaries
- **THEN** compilation succeeds (`Send + Sync` satisfied)

---

### Requirement: ExecutionState enum

owner crate:
`ego-runtime`

owner module:
`runtime::lifecycle`

file:
`crates/runtime/src/runtime/lifecycle.rs`

public types:
`ExecutionState`

evolution:
Adding new lifecycle variants affects only `lifecycle.rs`.

The system SHALL define an `ExecutionState` enum with these variants:

- `Active` — execution unit is processing
- `Draining` — shutdown requested, draining pending work
- `Terminated` — completed successfully
- `Failed` — terminated due to unrecoverable error

`ExecutionState` SHALL implement `Clone`, `Debug`, `PartialEq`, `Send`, `Sync`.

The enum SHALL be `#[non_exhaustive]` to allow future variants without breaking changes.

#### Scenario: Default state after spawn
- **WHEN** an execution unit is spawned on a runtime that tracks state
- **THEN** state is `ExecutionState::Active`

#### Scenario: State transition after shutdown
- **WHEN** `shutdown` is called and draining completes
- **THEN** state becomes `ExecutionState::Terminated`

#### Scenario: State on failure
- **WHEN** an execution unit panics or returns an unrecoverable error
- **THEN** state becomes `ExecutionState::Failed`

---

### Requirement: SendError type

owner crate:
`ego-runtime`

owner module:
`runtime::failure`

file:
`crates/runtime/src/runtime/failure.rs`

public types:
`SendError`, `SendErrorKind`

evolution:
Adding error kinds affects only `failure.rs`.

The system SHALL define a `SendError` struct returned when message delivery fails. It SHALL contain:

- `id: ExecutionId` — the target execution unit
- `cause: SendErrorKind` — reason for failure

`SendError` SHALL implement `Debug`, `Display`, `std::error::Error`.

The system SHALL define `SendErrorKind` enum with these runtime-neutral variants:

- `NotFound` — no execution unit with the given id
- `Closed` — target has shut down or runtime is closed

`SendErrorKind` SHALL be `#[non_exhaustive]`.

Backend-specific error kinds (e.g. `MailboxFull`) MUST NOT appear in the core error type. Backends MAY define their own error types for backend-specific extensions.

#### Scenario: Send to unknown id
- **WHEN** `send` is called with an `ExecutionId` that has no running unit
- **THEN** `SendError` is returned with `SendErrorKind::NotFound`
- **AND** no panic or unwinding occurs

#### Scenario: Send after shutdown
- **WHEN** `send` is called after the runtime has shut down
- **THEN** `SendError` is returned with `SendErrorKind::Closed`

---

### Requirement: RuntimeHandle type

owner crate:
`ego-runtime`

owner module:
`runtime::handle`

file:
`crates/runtime/src/runtime/handle.rs`

public types:
`RuntimeHandle`

evolution:
Handle capabilities change affects only `handle.rs`.

The system SHALL define a `RuntimeHandle` that provides spawned execution units access to runtime operations scoped to the local unit.

`RuntimeHandle` SHALL expose:

- `fn id(&self) -> &ExecutionId`
- `fn send<M: Send + 'static>(&self, msg: M) -> Result<(), SendError>`
- `fn shutdown(&self)`
- `fn state(&self) -> Option<ExecutionState>`

`RuntimeHandle` SHALL implement `Clone`, `Send`, `Sync`.

#### Scenario: Unit sends to itself
- **WHEN** a spawned unit calls `handle.send(msg)`
- **THEN** the message is queued for the unit's own sequential processing

#### Scenario: Unit checks own state
- **WHEN** a spawned unit calls `handle.state()`
- **THEN** it receives the current `ExecutionState` (or `None` if untracked)

---

### Requirement: Runtime trait — abstraction contract

owner crate:
`ego-runtime`

owner module:
`runtime::runtime`

file:
`crates/runtime/src/runtime/runtime.rs`

public types:
`Runtime`

evolution:
Adding new methods (with default implementations) preserves backward compatibility. Changing existing method signatures is a breaking change.

The system SHALL define a `Runtime` trait that is the common execution interface for all runtime backends. The Runtime trait IS the platform identity.

The trait SHALL require `Send + Sync + 'static`.

The trait SHALL define these methods:

```
fn spawn<F>(&self, f: F, name: Option<&str>) -> ExecutionId
    where F: Future<Output = ()> + Send + 'static
```

Spawns an execution unit. Returns a unique `ExecutionId`. The `name` parameter is advisory — backends MAY use it for debugging, MAY ignore it.

```
fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where M: Send + 'static
```

Routes a message to the execution unit identified by `id`. Messages are processed sequentially per unit. Returns `SendError` if delivery fails.

```
fn shutdown(&self, id: &ExecutionId)
```

Requests graceful termination of the execution unit. This is a signal — the unit MAY complete current work before terminating. The method returns immediately without awaiting termination.

```
fn state(&self, id: &ExecutionId) -> Option<ExecutionState>
```

Returns the lifecycle state of the execution unit, or `None` if the runtime does not track per-unit state.

The trait SHALL provide a capability discovery mechanism:

```
fn capabilities(&self) -> Capabilities
```

Returns the set of optional capabilities the backend supports. Default implementation returns empty set.

#### Scenario: Trait compiles as generic bound
- **WHEN** a function accepts `R: Runtime` as a generic parameter
- **THEN** the code compiles and all trait methods are callable on `R`

#### Scenario: Spawning with name
- **WHEN** `runtime.spawn(future, Some("worker-1"))` is called
- **THEN** a new execution unit is created
- **AND** the runtime MAY associate the name with the id for debugging

#### Scenario: Shutdown is non-blocking
- **WHEN** `shutdown` is called
- **THEN** the method returns immediately without awaiting termination

#### Scenario: Send after spawn
- **WHEN** a unit is spawned and `send` is called with a message
- **THEN** the message is delivered and processed sequentially by the target unit

---

### Requirement: Sequential execution guarantee

owner crate:
`ego-runtime`

owner module:
`runtime::scheduler`

file:
`crates/runtime/src/runtime/scheduler.rs`

evolution:
Removing sequential guarantee is a breaking change to the Runtime contract.

Every backend implementing `Runtime` SHALL process messages for a given execution unit sequentially in arrival order. There is no ordering guarantee across different execution units.

#### Scenario: Messages processed in order
- **WHEN** messages `[A, B, C]` are sent to the same unit in that order
- **THEN** the unit processes `A`, then `B`, then `C`

#### Scenario: No cross-unit ordering
- **WHEN** two units receive messages from the same sender
- **THEN** no ordering guarantee exists between the two units' processing

---

### Requirement: Isolation guarantee

owner crate:
`ego-runtime`

owner module:
`runtime::isolation`

file:
`crates/runtime/src/runtime/isolation.rs`

evolution:
Weakening isolation is a breaking change to the Runtime contract.

The runtime SHALL isolate execution units such that a failure in one unit does not affect other units.

Each execution unit SHALL have an independent execution context.

Unhandled panics or errors in a unit SHALL be caught by the runtime and result in `ExecutionState::Failed` for that unit only.

The runtime SHALL remain operational after a single unit failure.

#### Scenario: Unit failure does not cascade
- **WHEN** an execution unit panics
- **THEN** the panic is caught
- **AND** the unit transitions to `Failed` state
- **AND** all other units continue running
- **AND** the runtime continues to accept `spawn` and `send` calls

---

### Requirement: Fail-closed runtime behavior

owner crate:
`ego-runtime`

owner module:
`runtime::failure`

file:
`crates/runtime/src/runtime/failure.rs`

evolution:
Fail-closed semantics change affects `failure.rs` and may require updating all backends.

On an unrecoverable runtime-internal error, the runtime SHALL fail closed:
- SHALL transition all active execution units to `Failed` state
- SHALL refuse new `spawn` calls
- SHALL return `SendError::Closed` for all `send` calls
- SHALL return `None` for all `state` calls

#### Scenario: Runtime fails closed
- **WHEN** a runtime-internal error occurs (e.g., scheduler failure, resource exhaustion)
- **THEN** all units transition to `Failed`
- **AND** subsequent `spawn` calls return immediately without creating a unit
- **AND** subsequent `send` calls return `SendErrorKind::Closed`

---

### Requirement: Backend neutrality

owner crate:
`ego-runtime`

owner module:
`runtime::runtime`

file:
`crates/runtime/src/runtime/runtime.rs`

evolution:
Adding backend-specific constraints to the trait is forbidden.

The `Runtime` trait SHALL NOT expose backend-specific types, capabilities, or semantics. Any type, method, or trait bound that references a specific runtime backend (Tokio, Goakt, ProtoActor, Akka, etc.) is a violation.

Backend-specific extensions SHALL live in backend crates, not in the `ego-runtime` crate.

#### Scenario: Backend replacement without API change
- **WHEN** code written against `impl Runtime` is compiled with a different backend
- **THEN** the code compiles and runs without changes
- **AND** all Runtime trait methods behave according to their contract

---

### Requirement: Capability discovery

owner crate:
`ego-runtime`

owner module:
`runtime::runtime`

file:
`crates/runtime/src/runtime/runtime.rs`

public types:
`Capabilities`

evolution:
Capability discovery evolves independently; adding new capabilities does not break existing backends.

The system SHALL provide a `Capabilities` type and a capability discovery method on the `Runtime` trait with a default no-op implementation.

Backends MAY override to declare support for optional capabilities (supervision, mailbox sizing, priority, actor lifecycle, etc.).

Capabilities MUST NOT be required — all consumers MUST work with backends that declare no optional capabilities.

#### Scenario: Capability absent
- **WHEN** a backend does not declare a capability (e.g., supervision)
- **THEN** code that requires supervision detects its absence
- **AND** the code handles the absence gracefully (falls back, errors, or skips)

---

### Requirement: TokioRuntime implementation

owner crate:
`ego-runtime-tokio`

owner module:
(lib root)

file:
`crates/runtime-tokio/src/lib.rs`

public types:
`TokioRuntime`, `TokioRuntimeBuilder`

evolution:
Backend implementation changes affect only `runtime-tokio`.

The system SHALL provide a `TokioRuntime` struct in `ego-runtime-tokio` that implements the `Runtime` trait from `ego-runtime`.

`TokioRuntime` SHALL support two modes:
- **Multi-threaded** (default): backed by `tokio::runtime::Runtime` with `multi_thread` scheduler
- **Current-thread**: backed by `tokio::runtime::Runtime` with `current_thread` scheduler

`TokioRuntime` SHALL implement `Runtime`:
- `spawn`: delegates to tokio runtime
- `send`: uses internal id-to-channel routing table
- `shutdown`: sends stop signal via internal channel
- `state`: tracks state in shared map, returns `Some(ExecutionState)` for tracked units

`TokioRuntime` SHALL provide sequential execution per unit (messages dispatched to a unit are processed in order).

`TokioRuntime` SHALL provide isolation (panics in a unit are caught, other units unaffected).

`TokioRuntime` SHALL fail closed on unrecoverable runtime errors (tokio runtime shutdown propagates).

#### Scenario: Default multi-threaded runtime
- **WHEN** `TokioRuntime::default()` is called
- **THEN** a multi-threaded runtime is created
- **WHEN** a unit is spawned
- **THEN** it runs on the Tokio multi-threaded scheduler

#### Scenario: Current-thread runtime
- **WHEN** `TokioRuntime::builder().current_thread().build()` is called
- **THEN** a current-thread runtime is created
- **WHEN** a unit is spawned
- **THEN** it runs on the current thread

#### Scenario: Sequential delivery
- **WHEN** multiple messages are sent to the same unit
- **THEN** they are processed in arrival order

#### Scenario: Failure isolation
- **WHEN** a spawned unit panics
- **THEN** the panic is caught
- **AND** only the failed unit is affected
- **AND** the TokioRuntime continues operating

---

### Requirement: TokioRuntimeBuilder

owner crate:
`ego-runtime-tokio`

owner module:
(lib root)

file:
`crates/runtime-tokio/src/lib.rs`

public types:
`TokioRuntimeBuilder`

evolution:
Builder options affect only `lib.rs` in `ego-runtime-tokio`.

The system SHALL provide a `TokioRuntimeBuilder` that exposes Tokio-specific configuration:

- `fn worker_threads(self, n: usize) -> Self` — number of worker threads (multi-thread only)
- `fn current_thread(self) -> Self` — switch to current-thread scheduler
- `fn build(self) -> TokioRuntime` — finalizes and returns the runtime

#### Scenario: Configured build
- **WHEN** `TokioRuntime::builder().worker_threads(4).build()` is called
- **THEN** a runtime with exactly 4 worker threads is created

---

### Requirement: DefaultRuntime alias

owner crate:
`ego-runtime-tokio`

owner module:
(lib root)

file:
`crates/runtime-tokio/src/lib.rs`

public types:
`DefaultRuntime` (type alias)

evolution:
Default backend changes are a new crate, not a change to this alias.

The system SHALL define `DefaultRuntime` as a public type alias for `TokioRuntime` in the `ego-runtime-tokio` crate.

`DefaultRuntime` SHALL implement `Default` (delegates to `TokioRuntime::default()`).

#### Scenario: DefaultRuntime resolves to TokioRuntime
- **WHEN** code imports `DefaultRuntime` from `ego-runtime-tokio`
- **THEN** it resolves to `TokioRuntime`
- **WHEN** `DefaultRuntime::default()` is called
- **THEN** a default Tokio multi-threaded runtime is created
- **AND** no feature flags or conditional compilation is required

---

### Requirement: NullRuntime test double

owner crate:
`ego-runtime`

owner module:
`runtime::runtime` (test)

file:
`crates/runtime/src/runtime/runtime.rs` (in `#[cfg(test)]` module)

public types:
`NullRuntime` (test-only, not exported)

evolution:
Test double changes do not affect public API.

The system SHALL provide a `NullRuntime` struct (in `#[cfg(test)]`) that implements `Runtime` for unit testing.

`NullRuntime` SHALL:
- Return distinct `ExecutionId` for each `spawn` call
- Track spawned units in a `HashMap<ExecutionId, ExecutionState>`
- Implement `send` by looking up the unit and discarding the message
- Implement `shutdown` by marking the unit as `Terminated`
- Implement `state` by returning the tracked state

#### Scenario: NullRuntime validates trait contract
- **WHEN** a unit test uses `NullRuntime` as `impl Runtime`
- **THEN** all trait methods are callable without Tokio or any backend dependency

---

### Requirement: Workspace Cargo.toml

owner:
workspace root

file:
`Cargo.toml`

The workspace root Cargo.toml SHALL declare `"crates/runtime"` and `"crates/runtime-tokio"` in its workspace members array.

---

### Requirement: ego-runtime Cargo.toml

file:
`crates/runtime/Cargo.toml`

The crate SHALL declare:

```toml
[package]
name = "ego-runtime"
version = "0.1.0"
edition = "2021"

[dependencies]
# No required dependencies. Runtime trait is dependency-free.
```

---

### Requirement: ego-runtime-tokio Cargo.toml

file:
`crates/runtime-tokio/Cargo.toml`

The crate SHALL declare:

```toml
[package]
name = "ego-runtime-tokio"
version = "0.1.0"
edition = "2021"

[dependencies]
ego-runtime = { path = "../runtime" }
tokio = { version = "1", features = ["full"] }
```

---

### REMOVED Requirements

The following requirements from the previous CORE-003 spec are REMOVED and MUST NOT be implemented:

- ActorSystem concept — replaced by `Runtime` trait
- Actor handle types — removed; not part of core contract
- Mailbox type — removed; mailbox semantics are optional backend capability
- Supervision type — removed; supervision semantics are optional backend capability
- MailboxFull error variant — removed; `SendErrorKind` uses runtime-neutral `NotFound` / `Closed`
- Old communication guarantees (FIFO, at-most-once, message immutability) — replaced by Runtime trait contract
