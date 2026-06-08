# Research: CORE-006 Persistent Entity Runtime Implementation

**Feature**: `006-persistent-entity-runtime/final-consistency-lock`
**Created**: 2026-06-07

## Research Items

### 1. Actor Per Entity — Async Task Pattern

**Decision**: `tokio::spawn(EntityActor::run())` with `loop { rx.recv().await; process; }`

**Rationale**:
- Directly maps to canonical spec Section 1: "1 entity = 1 dedicated Tokio task"
- Existing `EntityActor` already uses this pattern — aligns with existing crate code
- `tokio::sync::mpsc::Receiver` integrates naturally with async `.recv().await`
- Task ownership of state/lifecycle/mailbox maps to struct fields on `EntityActor`
- Task completes on passivation (channel close → `recv()` returns `None` → task returns)

**Alternatives considered**:
- Event-loop worker pool (fixed workers claim entities): rejected — violates actor-per-entity model, creates shared state between workers
- Hybrid state machine per entity: rejected — over-engineered for Rust's async model; `async fn` + loop is idiomatic

### 2. Scheduler — Event-Driven vs Polling

**Decision**: Event-driven trigger system using `tokio::sync::Notify`

**Rationale**:
- The Scheduler is a policy engine, not an executor. It should be reactive, not polling.
- `tokio::sync::Notify` is the standard Rust primitive for event-driven wakeup — zero-cost when no events
- Events: Actor signals slot freed (via Notify), command arrival notification, fairness circuit-breaker expiry
- Each event triggers one scheduling decision cycle (pick next entity from activation queue)
- Avoids busy-polling overhead; minimal resource usage when no entities need activation
- Aligns with spec: "The Scheduler MUST NOT execute, own state, or handle failures"

**Alternatives considered**:
- Background polling loop (`tokio::time::interval`): rejected — wastes CPU polling when system is idle, adds latency floor from poll interval
- Hybrid tick + event: unnecessary complexity at this stage; fairness can be driven by operation count, not wall-clock

### 3. ExecutionBackend — Sync vs Async Trait

**Decision**: Sync trait `fn execute(...) -> Result`, invoked directly by Actor

**Rationale**:
- ExecutionUnit is pure computation (Handler Safety Contract: no I/O, no await, no async)
- Sync trait maximizes portability: WASM, no_std, embedded — no async runtime dependency in the trait itself
- Actor (which owns the async context) calls `backend.execute()` synchronously in its async loop
- If backend isolation is needed (prevent panics from crashing Actor), wrap in `tokio::task::spawn_blocking`
- Default `TokioExecutionBackend` is a zero-overhead wrapper — calls `execute` inline
- Aligns with spec: "The ExecutionBackend MUST NOT introduce semantic differences in execution meaning"

**Alternatives considered**:
- `#[async_trait]` with async fn: rejected — forces all backends to be async (WASM constraint), over-complicates the contract for a pure computation
- Message-passing adapter (separate task + channels): rejected — adds latency, unnecessary for pure computation

### 4. Mailbox — Bounded tokio::sync::mpsc

**Decision**: `tokio::sync::mpsc::channel(capacity)` with `try_send`

**Rationale**:
- Canonical spec already specifies "bounded Tokio mpsc channel"
- `try_send` maps to synchronous `MailboxFull` rejection (spec FR-020)
- `Sender::clone()` for `EntityRef` sharing — cheap, reference-counted
- Channel close on Actor termination provides stale sender detection (passivation/reactivation)
- Bounded capacity prevents memory leaks under sustained load
- Existing `BoundedMailbox` already uses this — no change needed

**Alternatives considered**:
- `crossbeam::channel`: rejected — introduces unnecessary dependency, no async integration benefit
- Backend-provided channel: rejected — violates spec (backend is execution-only, not channel provider)

### 5. Recovery — Synchronous Replay Inside Actor

**Decision**: Inline synchronous `apply_event` loop during RECOVERING, before command processing starts

**Rationale**:
- Spec FR-012: "During recovery replay, the framework MUST NOT execute side effects, emit new events, invoke external services, or trigger publications"
- Recovery is an internal Actor concern — no scheduler, no backend, no budget slot
- Synchronous replay is simple, deterministic, and directly verifiable
- Event applier is a pure function `(state, event) -> state` — no reason for async
- Aligns with existing `EntityActor::recover_state()` pattern
- Single-threaded — no concurrency concerns during recovery

**Alternatives considered**:
- Async replay via Scheduler coordination: rejected — RECOVERING is exempt from budget per scheduling-policy spec; Scheduler is recovery-agnostic
- Backend-assisted replay: rejected — backend is for ExecutionUnit execution (commands), not event replay

### 6. Concurrency Budget — Activation-Guard Level

**Decision**: Budget enforced at `SharedActivation::try_activate()` before mailbox creation

**Rationale**:
- Audit Issue #1 resolution: budget check before mailbox creation prevents orphan mailboxes
- `SharedActivation` already uses a `Mutex` guard — natural serialization point
- Budget is per-process: `Arc<Semaphore>` with capacity = budget slots
- `try_acquire()` at guard — if saturated, blocks (async wait on semaphore permit)
- Budget applies to ACTIVE state only (RECOVERING exempt per Issue #4 resolution)
- Budget slot released when entity passivates (Actor task completes, semaphore permit dropped)

**Alternatives considered**:
- Scheduler-level budget: rejected — Scheduler is policy-only, doesn't enforce; Actor owns enforcement
- Backend-level budget: rejected — backend is execution-only per Issue #3 resolution

### 7. ExecutionKey — Content-Based Hash

**Decision**: `ExecutionKey = hash(blake3(entity_id || command_payload || state_version))`

**Rationale**:
- Spec defines `ExecutionKey = hash(entity_id, command, state_version)`
- `blake3` is fast, non-cryptographic, deterministic — suitable for execution deduplication
- Serialize inputs deterministically (e.g., postcard or bincode for command_payload)
- Actor computes before execution; tracks in a per-lifecycle-window `HashSet`
- Zero-event commands exempt from deduplication per Issue #5 resolution

**Alternatives considered**:
- UUID-based key: rejected — non-deterministic (random); violates spec requirement for deterministic recomputation
- SHA-256: functional but overkill; blake3 is faster and equally deterministic

### 8. Existing Code Integration Strategy

**Decision**: Refactor within existing `persistent-entity` crate, not rewrite

**Rationale**:
- Significant existing implementation: `EntityActor`, `EntityRegistry`, `BoundedMailbox`, `LifecycleStateMachine`, `EntityRuntime`, `EntityRuntimeBuilder`, `EntityRef`
- All existing tests must pass after refactor
- New modules added alongside existing ones
- The `PersistenceFacade` stub must be replaced with a real implementation delegating to domain `EventStore` trait
- The `ego-domain` dependency must be declared in `persistent-entity` Cargo.toml (currently used in builder.rs without declaration)
