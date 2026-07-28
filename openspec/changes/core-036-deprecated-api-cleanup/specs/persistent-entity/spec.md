# Delta for persistent-entity

## ADDED Requirements

### Requirement: No Command-Execution Backend Abstraction On The Public Surface

The `persistent-entity` crate MUST NOT expose any `ExecutionBackend`-style command-execution
abstraction on its public surface. Command execution happens by awaiting handler methods directly
inside the spawned actor task; there MUST be no trait or type whose purpose is to bridge async
handler invocation to a synchronous `execute` call, and no `block_on`-based execution path.

The symbols `ExecutionBackend` (trait), `TokioExecutionBackend`, and `SyncTestBackend`, and the
modules `execution_backend` and `execution_backend_tokio`, MUST NOT exist in the crate. After
removal, a workspace-wide search for these identifiers MUST return zero matches outside historical
OpenSpec artifacts.

#### Scenario: Deprecated execution backends are absent

- GIVEN the `persistent-entity` crate after CORE-036
- WHEN the workspace is scanned with `rg 'TokioExecutionBackend|SyncTestBackend|ExecutionBackend|execution_backend' crates/`
- THEN the search returns zero matches

#### Scenario: The crate compiles without the execution-backend modules

- GIVEN `crates/persistent-entity/src/lib.rs` no longer declares `pub mod execution_backend;` or `pub mod execution_backend_tokio;`
- WHEN `cargo build -p ego-persistent-entity` runs
- THEN it succeeds with no unresolved-module or missing-symbol error

#### Scenario: Command execution proceeds without any backend type

- GIVEN a persistent entity processing a command through its actor
- WHEN the command is executed
- THEN the handler is awaited directly inside the actor task, using no `ExecutionBackend` implementation and no `block_on`

## REMOVED Requirements

### Requirement: Execution Backend Abstraction

(Reason: `TokioExecutionBackend`, `SyncTestBackend`, and the `ExecutionBackend` trait were
`#[deprecated]` stubs — `TokioExecutionBackend::execute` only ever returned
`EntityError::Internal("… deprecated …")` — with zero external callers. The `block_on` execution
path was already removed; the actor awaits handlers directly. Retaining them violated `PRD.md:140`
("No shims — no deprecated aliases in pre-stable crates").)

(Migration: no successor type is introduced — the concept was deleted, not renamed. Callers that
previously constructed `TokioExecutionBackend`/`SyncTestBackend` drive an `EntityActor` directly
(with in-memory stores in tests). No in-repo production or test caller existed, so migration touches
only the removed files themselves and the two `pub mod` lines in `lib.rs`.)
