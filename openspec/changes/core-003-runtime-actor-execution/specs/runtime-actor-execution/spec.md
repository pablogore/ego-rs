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

`ExecutionId` SHALL implement `Clone`, `Copy`, `Debug`, `Eq`, `Hash`, `Send`, `Sync`.

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

---

### Requirement: SpawnError type

owner crate:
`ego-runtime`

owner module:
`runtime::failure`

file:
`crates/runtime/src/runtime/failure.rs`

public types:
`SpawnError`, `SpawnErrorKind`

evolution:
Adding spawn error variants affects only `failure.rs`.

The system SHALL define a `SpawnError` struct returned when spawning an execution unit fails. It SHALL contain:

- `pub cause: SpawnErrorKind` — reason for failure

`SpawnError` SHALL implement `Debug`, `Display`, `std::error::Error`.

The system SHALL define `SpawnErrorKind` enum with these variants:

- `Closed` — runtime has shut down, cannot spawn new units
- `Internal` — unrecoverable runtime internal error

`SpawnErrorKind` SHALL be `#[non_exhaustive]`.

#### Scenario: Spawn after failure
- **WHEN** `spawn` is called after the runtime has failed
- **THEN** `SpawnError` is returned with `SpawnError { cause: SpawnErrorKind::Closed }`
- **AND** no panic, no fake id, no noop occurs

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

**This is distinct from unit failure.** When a single execution unit fails (panics, error), the runtime survives — other units continue, spawn and send continue. Fail-closed applies ONLY to unrecoverable runtime-internal errors.

On an unrecoverable runtime-internal error, the runtime SHALL fail closed:
- SHALL transition all active execution units to `Failed` state
- SHALL return `Err(SpawnError { cause: SpawnErrorKind::Closed })` for all subsequent `spawn` calls
- SHALL return `SendError::Closed` for all subsequent `send` calls
- SHALL return `None` for all subsequent `state` calls
- No panic, no fake id, no noop

#### Scenario: Runtime fails closed
- **WHEN** a runtime-internal error occurs (e.g., scheduler failure, resource exhaustion)
- **THEN** all units transition to `Failed`
- **AND** subsequent `spawn` calls return `Err(SpawnError { cause: SpawnErrorKind::Closed })`
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

The `Runtime` trait SHALL NOT expose backend-specific types, features, or semantics. Any type, method, or trait bound that references a specific runtime backend (Tokio, Goakt, ProtoActor, Akka, etc.) is a violation.

Backend-specific extensions SHALL live in backend crates, not in the `ego-runtime` crate.

#### Scenario: Backend replacement without API change
- **WHEN** code written against `impl Runtime` is compiled with a different backend
- **THEN** the code compiles and runs without changes
- **AND** all Runtime trait methods behave according to their contract

---

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
- `send`: routes messages to target execution unit
- `shutdown`: requests execution unit termination
- `state`: returns execution unit state through implementation-defined tracking, returns `Some(ExecutionState)` for tracked units

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
- Track spawned units in an internal state registry
- Implement `send` by looking up the unit and storing the message for test assertion (test double — does not process messages)
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
uuid = { version = "1", features = ["v4"] }
```

`uuid` is a foundational utility dependency (id generation), NOT a runtime/backend coupling.
The crate has ZERO RUNTIME/BACKEND DEPENDENCIES.

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
- Mailbox type — removed; mailbox semantics are optional backend concern
- Supervision type — removed; supervision semantics are optional backend concern
- MailboxFull error variant — removed; `SendErrorKind` uses runtime-neutral `NotFound` / `Closed`
- Old communication guarantees (FIFO, at-most-once, message immutability) — replaced by Runtime trait contract
- `spawn() -> ExecutionId` — replaced by `spawn() -> Result<ExecutionId, SpawnError>` for fail-closed consistency
- `spawn(Future)` without handle injection — replaced by `spawn(FnOnce(RuntimeHandle) -> Future)` for RuntimeHandle injection
- `dyn Runtime` in RuntimeHandle — RuntimeHandle uses closure-based internal structure; `dyn Runtime` is impossible (Runtime is not object-safe)
- Strong scheduling guarantees — replaced by "reasonable forward progress / SHOULD avoid starvation"
