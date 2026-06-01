Implementation order: tasks MUST be completed in sequence. Each task assumes all previous tasks are done.

---

## Phase 1: Workspace setup

---

### core-003-1-1-fix-runtime-cargo-toml

**Goal:** Replace `crates/runtime/Cargo.toml` dependencies to match spec: remove `ego-domain`, `chrono`, `serde`, `serde_json`, `mockall`; add `uuid = { version = "1", features = ["v4"] }`.

**Inputs:**
- `design.md:220-228` (dependency boundaries)
- `spec.md:376-394` (cargo toml requirement)
- `crates/runtime/Cargo.toml` (to modify)

**Expected files changed:**
- `crates/runtime/Cargo.toml` — replace dependency block

**Completion criteria:**
- `cargo check -p ego-runtime` succeeds with uuid as only runtime dependency
- No ego-domain, chrono, serde, serde_json, mockall in `[dependencies]`

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
grep -q 'uuid' crates/runtime/Cargo.toml
grep -q 'features.*v4' crates/runtime/Cargo.toml
! grep -q 'ego-domain' crates/runtime/Cargo.toml
```

**Suggested aider scope:**
```
crates/runtime/Cargo.toml proposal.md design.md spec.md
```

**Context risk:** `low` — single file edit, well-specified content.

---

### core-003-1-2-verify-runtime-tokio-cargo-toml

**Goal:** Verify `crates/runtime-tokio/Cargo.toml` matches spec (ego-runtime path dep + tokio full features); fix if diverged.

**Inputs:**
- `spec.md:397-413` (runtime-tokio cargo toml requirement)
- `crates/runtime-tokio/Cargo.toml` (to verify/modify)

**Expected files changed:**
- `crates/runtime-tokio/Cargo.toml` (if diverged)

**Completion criteria:**
- `cargo check -p ego-runtime-tokio` compiles
- Deps match spec exactly

**Validation:**
```bash
cargo check -p ego-runtime-tokio 2>&1
```

**Suggested aider scope:**
```
crates/runtime-tokio/Cargo.toml spec.md
```

**Context risk:** `low` — verification only; fix if needed.

---

### core-003-1-3-create-runtime-mod-scaffold

**Goal:** Create `crates/runtime/src/runtime/mod.rs` with module declarations for all runtime submodules: `runtime`, `execution`, `lifecycle`, `failure`, `handle`, `scheduler`, `isolation`.

**Inputs:**
- `design.md:42-54` (physical structure — module layout)

**Expected files changed:**
- `crates/runtime/src/runtime/mod.rs` (CREATE)

**Completion criteria:**
- File exists with all 7 module declarations
- `cargo check -p ego-runtime` compiles (may get dead_code warnings — acceptable)

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/mod.rs design.md
```

**Context risk:** `low` — trivial file creation.

---

## Phase 2: Vocabulary types

---

### core-003-2-1-rewrite-execution-id

**Goal:** Replace `crates/runtime/src/execution.rs` with `crates/runtime/src/runtime/execution.rs` containing a `Uuid`-backed `ExecutionId` newtype implementing `Clone`, `Copy`, `Debug`, `Eq`, `Hash`, `Send`, `Sync` with a `new()` constructor.

**Inputs:**
- `spec.md:6-37` (ExecutionId requirement + scenarios)
- `crates/runtime/src/execution.rs` (old file — remove after migration)
- `crates/runtime/src/runtime/mod.rs` (already has `pub mod execution`)

**Expected files changed:**
- `crates/runtime/src/runtime/execution.rs` (CREATE — was `src/execution.rs`)
- `crates/runtime/src/execution.rs` (DELETE — old flat file)

**Completion criteria:**
- `ExecutionId` wraps `Uuid`
- `ExecutionId::new()` generates random v4 uuid
- Implements `Clone + Copy + Debug + Eq + Hash + Send + Sync`
- No imports from `ego-domain`, no actor types
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/execution.rs crates/runtime/src/runtime/mod.rs spec.md
```

**Context risk:** `low` — isolated newtype, no cross-module coupling beyond mod.rs.

---

### core-003-2-2-create-execution-state

**Goal:** Create `crates/runtime/src/runtime/lifecycle.rs` with `#[non_exhaustive]` enum `ExecutionState` with variants `Active`, `Draining`, `Terminated`, `Failed`. Implement `Clone`, `Debug`, `PartialEq`, `Send`, `Sync`.

**Inputs:**
- `spec.md:41-79` (ExecutionState requirement + scenarios)
- `design.md:119-126` (lifecycle contract — Active → Draining → Terminated | Failed)
- `crates/runtime/src/execution.rs` (currently contains old `ExecutionState` with variants Created/Starting/Running/Stopping/Stopped/Failed — this will be removed)

**Expected files changed:**
- `crates/runtime/src/runtime/lifecycle.rs` (CREATE)
- `crates/runtime/src/runtime/mod.rs` (already has `pub mod lifecycle`)

**Completion criteria:**
- `ExecutionState` has exactly 4 variants: Active, Draining, Terminated, Failed
- Enum is `#[non_exhaustive]`
- Implements `Clone + Debug + PartialEq + Send + Sync`
- No actor vocabulary, no `ActorLifecycleState` import
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/lifecycle.rs spec.md design.md
```

**Context risk:** `low` — single enum type.

---

### core-003-2-3-create-failure-types

**Goal:** Create `crates/runtime/src/runtime/failure.rs` with `SendError` (fields: `id: ExecutionId`, `cause: SendErrorKind`), `SendErrorKind` (`NotFound`, `Closed`), `SpawnError` (field: `pub cause: SpawnErrorKind`), `SpawnErrorKind` (`Closed`, `Internal`). All types implement `Debug`, `Display`, `std::error::Error` as applicable. All enums `#[non_exhaustive]`.

**Inputs:**
- `spec.md:83-152` (SendError + SpawnError requirements + scenarios)
- `spec.md:154-183` (fail-closed behavior)
- `crates/runtime/src/error.rs` (old file — DELETE after creation)
- `design.md:143-157` (failure contract — unit failure vs runtime internal failure)

**Expected files changed:**
- `crates/runtime/src/runtime/failure.rs` (CREATE)
- `crates/runtime/src/error.rs` (DELETE — old flat file with `RuntimeError` enum)
- `crates/runtime/src/runtime/mod.rs` (already has `pub mod failure`)

**Completion criteria:**
- 4 types exist with correct field/variant structure
- `SendError` impl `Debug + Display + std::error::Error`
- `SpawnError` impl `Debug + Display + std::error::Error`
- `SendErrorKind` is `#[non_exhaustive]`
- `SpawnErrorKind` is `#[non_exhaustive]`
- No `MailboxFull` variant
- No `RuntimeError` enum anywhere in crate
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/failure.rs crates/runtime/src/runtime/execution.rs spec.md design.md
```

**Context risk:** `low` — four related types, one file, depends on `ExecutionId` (from execution.rs).

---

### core-003-2-4-create-runtime-handle

**Goal:** Create `crates/runtime/src/runtime/handle.rs` with `RuntimeHandle` struct using closure-based internals (`send_self_fn`, `shutdown_fn`, `state_fn` as `Arc<dyn Fn>`). Public methods: `id() -> ExecutionId`, `send_self<M: Send + 'static>(msg) -> Result<(), SendError>`, `shutdown()`, `state() -> Option<ExecutionState>`. Implements `Clone`, `Send`, `Sync`.

**Inputs:**
- `design.md:197-203` (RuntimeHandle design — closure-based, scoped access)
- `design.md:219-227` (RuntimeHandle internal model)
- `crates/runtime/src/runtime/mod.rs` (already has `pub mod handle`)

**Expected files changed:**
- `crates/runtime/src/runtime/handle.rs` (CREATE)
- `crates/runtime/src/runtime/mod.rs` (already done — no change)

**Completion criteria:**
- `RuntimeHandle` uses `Arc<dyn Fn(...)>` for all operations (no `dyn Runtime`)
- `send_self` is generic but uses internal `Any` boxing
- All 4 public methods exist
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/handle.rs crates/runtime/src/runtime/execution.rs crates/runtime/src/runtime/lifecycle.rs crates/runtime/src/runtime/failure.rs design.md
```

**Context risk:** `low` — one struct, closure pattern is well-specified in design.md.

---

## Phase 3: Runtime trait + contract modules

---

### core-003-3-1-rewrite-runtime-trait

**Goal:** Rewrite `crates/runtime/src/runtime/runtime.rs` with the spec-compliant `Runtime` trait: `spawn<F, Fut>(&self, f: F, name: Option<&str>) -> Result<ExecutionId, SpawnError>` where `F: FnOnce(RuntimeHandle) -> Fut + Send + 'static`, `Fut: Future<Output = ()> + Send + 'static`; `send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>`; `shutdown(&self, id: &ExecutionId)`; `state(&self, id: &ExecutionId) -> Option<ExecutionState>`. Trait bound: `Send + Sync + 'static`. Remove all GATs and actor types.

**Inputs:**
- `design.md:83-107` (Runtime trait interface design)
- `spec.md:186-208` (backend neutrality requirement)
- `spec.md:209-370` (all type requirements — for imports)
- `crates/runtime/src/runtime.rs` (old file — REPLACE)

**Expected files changed:**
- `crates/runtime/src/runtime/runtime.rs` (REWRITE — was flat `src/runtime.rs`)
- `crates/runtime/src/runtime.rs` (DELETE — old flat file)

**Completion criteria:**
- Trait has exactly 4 methods with correct signatures (spawn, send, shutdown, state)
- No GATs (`type ExecutionId`, `type ExecutionHandle`, etc. removed)
- No imports from `ego-domain`
- No `ActorId`, `ActorLifecycleState`, `SupervisionStrategy` references
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/runtime.rs crates/runtime/src/runtime/execution.rs crates/runtime/src/runtime/lifecycle.rs crates/runtime/src/runtime/failure.rs crates/runtime/src/runtime/handle.rs design.md spec.md
```

**Context risk:** `medium` — trait depends on all 4 vocabulary types; one wrong import or bound breaks compilation.

---

### core-003-3-2-rewrite-isolation-module

**Goal:** Rewrite `crates/runtime/src/runtime/isolation.rs` as a doc-only module defining the isolation contract. Remove the `Isolation` enum entirely. Replace with module-level doc comment specifying: independent execution context per unit, no cascading failures, panic must be caught -> unit `Failed`, runtime survives single unit failure.

**Inputs:**
- `design.md:136-141` (isolation contract)
- `spec.md:136-141` (isolation guarantee)
- `crates/runtime/src/isolation.rs` (old flat file — DELETE)

**Expected files changed:**
- `crates/runtime/src/runtime/isolation.rs` (REWRITE — was flat `src/isolation.rs` with enum)
- `crates/runtime/src/isolation.rs` (DELETE — old flat file)
- `crates/runtime/src/runtime/mod.rs` (already has `pub mod isolation`)

**Completion criteria:**
- File contains only doc comments
- No `Isolation` enum
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/isolation.rs design.md spec.md
```

**Context risk:** `low` — pure doc module, no code.

---

### core-003-3-3-rewrite-scheduler-module

**Goal:** Create `crates/runtime/src/runtime/scheduler.rs` as a doc-only module defining the scheduling contract. Remove the `SchedulingPolicy` enum from the old `scheduling.rs`. Replace with module-level doc comment specifying: reasonable forward progress, avoid starvation, cross-unit fairness implementation-defined, sequential in-order processing within a unit.

**Inputs:**
- `design.md:129-135` (scheduling contract)
- `crates/runtime/src/scheduling.rs` (old flat file — DELETE)

**Expected files changed:**
- `crates/runtime/src/runtime/scheduler.rs` (CREATE)
- `crates/runtime/src/scheduling.rs` (DELETE — old flat file)
- `crates/runtime/src/runtime/mod.rs` — module name is `scheduler` per spec

**Completion criteria:**
- File contains only doc comments
- No `SchedulingPolicy` enum
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/scheduler.rs crates/runtime/src/runtime/mod.rs design.md spec.md
```

**Context risk:** `low` — pure doc module, no code.

---

## Phase 4: Public API exports

---

### core-003-4-1-rewrite-lib-reexports

**Goal:** Rewrite `crates/runtime/src/lib.rs` to declare `pub mod runtime` and re-export `Runtime`, `ExecutionId`, `ExecutionState`, `SendError`, `SendErrorKind`, `SpawnError`, `SpawnErrorKind`, `RuntimeHandle`. Remove all old exports (`Isolation`, `SchedulingPolicy`, `RuntimeError`).

**Inputs:**
- `spec.md:375-393` (lib.rs re-export content)
- `design.md:56-66` (physical crate structure)
- `crates/runtime/src/lib.rs` (old file — REWRITE)

**Expected files changed:**
- `crates/runtime/src/lib.rs` (REWRITE)

**Completion criteria:**
- Exports: `Runtime`, `ExecutionId`, `ExecutionState`, `SendError`, `SendErrorKind`, `SpawnError`, `SpawnErrorKind`, `RuntimeHandle`
- Does NOT export: `NullRuntime`, `Isolation`, `SchedulingPolicy`, `RuntimeError`
- `cargo check -p ego-runtime` passes

**Validation:**
```bash
cargo check -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/lib.rs crates/runtime/src/runtime/*.rs design.md spec.md
```

**Context risk:** `low` — single file, follows established pattern.

---

## Phase 5: Tokio backend

---

### core-003-5-1-impl-tokio-runtime-struct

**Goal:** Implement `TokioRuntime` struct in `crates/runtime-tokio/src/lib.rs` wrapping `tokio::runtime::Runtime` with an internal execution unit registry. Implement `spawn`: create `RuntimeHandle` with closures wired to this unit's channel, register unit, spawn the wrapped future on tokio, return `Ok(ExecutionId)`.

**Inputs:**
- `design.md:159-167` (backend adapter model)
- `spec.md:213-268` (TokioRuntime requirements)
- `design.md:186-188` (spawn contract — FnOnce(RuntimeHandle) -> Future)
- `crates/runtime-tokio/src/lib.rs` (current stub — REWRITE)

**Expected files changed:**
- `crates/runtime-tokio/src/lib.rs` (MAJOR REWRITE)

**Completion criteria:**
- `TokioRuntime` struct exists wrapping tokio runtime + unit registry
- `spawn` creates `RuntimeHandle`, registers unit, returns `ExecutionId`
- Unit registry stores: id, channel sender, state
- `RuntimeHandle::send_self_fn` wired to channel sender (tokio::mpsc)
- `cargo check -p ego-runtime-tokio` passes (may have unused methods — acceptable at this stage)

**Validation:**
```bash
cargo check -p ego-runtime-tokio 2>&1
```

**Suggested aider scope:**
```
crates/runtime-tokio/src/lib.rs crates/runtime/src/runtime/runtime.rs crates/runtime/src/runtime/handle.rs design.md spec.md
```

**Context risk:** `medium` — most complex task. Requires RuntimeHandle closure wiring, tokio::mpsc channels, unit registry map.

---

### core-003-5-2-impl-tokio-send-routing

**Goal:** Implement `send` on `TokioRuntime`: look up unit by `ExecutionId`, send message via unit's channel, return `Result<(), SendError>`. Wire a sequential message processing loop per unit that receives messages from the channel and passes them to the unit's message handler in arrival order.

**Inputs:**
- `spec.md:236-238` (send requirement)
- `design.md:112-117` (send routing — runtime-internal delivery)
- `spec.md:131-134` (sequential within-unit delivery)
- `crates/runtime-tokio/src/lib.rs` (modified in previous task)

**Expected files changed:**
- `crates/runtime-tokio/src/lib.rs` (add send + message loop)

**Completion criteria:**
- `send` routes messages to correct unit by `ExecutionId`
- Messages processed sequentially in arrival order within each unit
- `SendError::NotFound` returned for unknown id
- `SendError::Closed` returned for shut-down units
- `cargo check -p ego-runtime-tokio` passes

**Validation:**
```bash
cargo check -p ego-runtime-tokio 2>&1
```

**Suggested aider scope:**
```
crates/runtime-tokio/src/lib.rs design.md spec.md
```

**Context risk:** `medium` — requires tokio::mpsc::Receiver polling loop and sequential dispatch within a single spawned task.

---

### core-003-5-3-impl-tokio-shutdown-isolation

**Goal:** Implement `shutdown` (signal unit -> unit transitions to Draining -> completes in-flight -> Terminated) and `state` (return tracked `ExecutionState`). Add panic catching per unit: wrap unit future in `std::panic::catch_unwind`, on panic set unit state to `Failed`, other units unaffected. Add fail-closed: on unrecoverable runtime error, set all units to `Failed`, reject all new spawn/send.

**Inputs:**
- `design.md:119-126` (lifecycle: Active -> Draining -> Terminated | Failed)
- `design.md:136-157` (isolation + failure contract)
- `spec.md:166-183` (fail-closed behavior)
- `crates/runtime-tokio/src/lib.rs` (modified in previous tasks)

**Expected files changed:**
- `crates/runtime-tokio/src/lib.rs` (add shutdown, state, panic catch, fail-closed)

**Completion criteria:**
- `shutdown` transitions Active/Draining -> Terminated
- `state` returns correct execution state for tracked units
- Panic in a unit catches -> unit state = Failed, other units continue
- Runtime internal error -> fail-closed (all units Failed, spawn returns Closed, send returns Closed)
- `cargo check -p ego-runtime-tokio` passes

**Validation:**
```bash
cargo check -p ego-runtime-tokio 2>&1
```

**Suggested aider scope:**
```
crates/runtime-tokio/src/lib.rs design.md spec.md
```

**Context risk:** `medium` — state machine transitions, Arc<Atomic> or RwLock on unit registry state, catch_unwind on Send + 'static closures.

---

### core-003-5-4-add-tokio-runtime-builder

**Goal:** Add `TokioRuntimeBuilder` struct with methods: `worker_threads(self, n: usize) -> Self` (multi-thread only), `current_thread(self) -> Self`, `build(self) -> TokioRuntime`. Builder configures a `tokio::runtime::Builder` internally.

**Inputs:**
- `spec.md:272-298` (TokioRuntimeBuilder requirement + scenarios)
- `crates/runtime-tokio/src/lib.rs` (to modify)

**Expected files changed:**
- `crates/runtime-tokio/src/lib.rs` (add builder struct + methods)

**Completion criteria:**
- `TokioRuntimeBuilder` exists with 3 methods
- `build()` returns `TokioRuntime`
- `current_thread()` switches tokio builder to `new_current_thread()`
- `worker_threads(n)` sets worker threads on tokio builder
- `cargo check -p ego-runtime-tokio` passes

**Validation:**
```bash
cargo check -p ego-runtime-tokio 2>&1
```

**Suggested aider scope:**
```
crates/runtime-tokio/src/lib.rs spec.md
```

**Context risk:** `low` — builder pattern, thin wrapper around tokio::runtime::Builder.

---

### core-003-5-5-add-default-runtime-alias

**Goal:** Add `pub type DefaultRuntime = TokioRuntime;` and `impl Default for TokioRuntime` that creates a multi-threaded Tokio runtime with default settings (worker threads = available parallelism via `std::thread::available_parallelism()`).

**Inputs:**
- `spec.md:302-328` (DefaultRuntime requirement + scenarios)
- `design.md:206-207` (DefaultRuntime lives in runtime-tokio, not runtime)
- `crates/runtime-tokio/src/lib.rs` (to modify)

**Expected files changed:**
- `crates/runtime-tokio/src/lib.rs` (add alias + Default impl)

**Completion criteria:**
- `DefaultRuntime` is a public type alias for `TokioRuntime`
- `TokioRuntime` impl `Default` creating multi-threaded runtime
- `cargo check -p ego-runtime-tokio` passes

**Validation:**
```bash
cargo check -p ego-runtime-tokio 2>&1
```

**Suggested aider scope:**
```
crates/runtime-tokio/src/lib.rs spec.md
```

**Context risk:** `low` — 4 lines of code.

---

## Phase 6: Integration

---

### core-003-6-1-update-layers-toml

**Goal:** Add `"ego-runtime" = "foundation"` and `"ego-runtime-tokio" = "infrastructure"` to `layers.toml`.

**Inputs:**
- `proposal.md:104-114` (layers.toml change)
- `layers.toml` (current state — modify)

**Expected files changed:**
- `layers.toml` (add 2 lines)

**Completion criteria:**
- `layers.toml` includes both new entries
- No existing entries modified

**Validation:**
```bash
grep -q 'ego-runtime' layers.toml
grep -q 'ego-runtime-tokio' layers.toml
```

**Suggested aider scope:**
```
layers.toml proposal.md
```

**Context risk:** `low` — 2-line addition.

---

## Phase 7: Verification

---

### core-003-7-1-add-null-runtime-tests

**Goal:** Add `NullRuntime` test double in `#[cfg(test)]` module of `crates/runtime/src/runtime/runtime.rs` implementing `Runtime`. `spawn` returns distinct `ExecutionId`, tracks units, `send` stores messages for assertion, `shutdown` sets `Terminated`, `state` returns tracked state. Include contract tests: spawn unique id, spawn after shutdown returns error, send to unknown id returns error, shutdown terminates, failure isolation.

**Inputs:**
- `spec.md:332-360` (NullRuntime requirement + tests)
- `design.md:258` (CI runs with NullRuntime)
- `crates/runtime/src/runtime/runtime.rs` (to add test module)

**Expected files changed:**
- `crates/runtime/src/runtime/runtime.rs` (add `#[cfg(test)]` module)

**Completion criteria:**
- `NullRuntime` exists in test module
- All contract tests compile and pass
- `cargo test -p ego-runtime` passes all tests

**Validation:**
```bash
cargo test -p ego-runtime 2>&1
```

**Suggested aider scope:**
```
crates/runtime/src/runtime/runtime.rs crates/runtime/src/runtime/execution.rs crates/runtime/src/runtime/lifecycle.rs crates/runtime/src/runtime/failure.rs crates/runtime/src/runtime/handle.rs spec.md
```

**Context risk:** `medium` — NullRuntime has its own internal state management (registry, message store). Tests exercise all contract semantics.

---

### core-003-7-2-add-tokio-integration-tests

**Goal:** Create `crates/runtime-tokio/tests/tokio_runtime_tests.rs` with integration tests covering: multi-threaded default, current-thread, send message, sequential delivery, failure isolation, shutdown, configured worker threads, fail-closed, spawn after failure returns error.

**Inputs:**
- `spec.md:248-268` (TokioRuntime scenarios)
- `crates/runtime-tokio/tests/` (directory — may not exist)

**Expected files changed:**
- `crates/runtime-tokio/tests/tokio_runtime_tests.rs` (CREATE)

**Completion criteria:**
- All integration tests compile and pass
- Tests cover: default mode, current-thread mode, send, sequential ordering, panic isolation, shutdown, worker threads config, fail-closed
- `cargo test -p ego-runtime-tokio` passes

**Validation:**
```bash
cargo test -p ego-runtime-tokio 2>&1
```

**Suggested aider scope:**
```
crates/runtime-tokio/src/lib.rs crates/runtime-tokio/tests/tokio_runtime_tests.rs spec.md
```

**Context risk:** `medium` — async test setup required (tokio::test). Sequential delivery test requires multi-message ordering assertion. Fail-closed test requires simulating internal error.

---

### core-003-7-3-workspace-verification

**Goal:** Full workspace compilation, test execution, and clippy lint pass with no errors, no warnings, no regressions in existing workspace members.

**Inputs:**
- All previous tasks complete

**Expected files changed:**
- None (verification only)

**Completion criteria:**
- `cargo check --workspace` succeeds
- `cargo test --workspace` passes all tests
- `cargo clippy --workspace -- -D warnings` succeeds
- No regressions in existing crates (domain, application, infrastructure, transport, runtime-slice)

**Validation:**
```bash
cargo check --workspace 2>&1
cargo test --workspace 2>&1
cargo clippy --workspace -- -D warnings 2>&1
```

**Suggested aider scope:**
```
(entire workspace — for context only)
```

**Context risk:** `low` — verification only.
