# Design: CORE-019 — Reliable External Effects

## 1. Technical Approach

Add a runtime-owned **effect delivery subsystem** beside the dormant
`EffectInterpreter`, not inside it. Handlers describe effects through a new
default-valued trait method on `PersistentEntity` (compile-unchanged, opt-in).
The `EntityActor` calls it in its existing post-persist sequence and hands the
effects to an `EffectAcceptor` port (defined in `persistent-entity`, mirroring
`EventPublisher`), whose runtime implementation mints the effect id, attaches
the established tenant, and awaits bounded-queue capacity before the command
reply. A single `DeliveryRunner` drains a `tokio::sync::mpsc` queue, consults
the `EffectStateStore`/`EffectDedupStore` public ports, and drives one attempt
per effect through the `effect_type`-keyed `ExternalEffectExecutor` registry.
Slice 1 ships one in-memory composite store; retry/backoff reuses the read-side
precedent. The runtime never branches on `effect_type`/`destination`.

## 2. Module / Crate Placement

`crates/runtime` already owns `EffectInterpreter` and depends on `ego-domain`;
`crates/service-sdk` depends on both `runtime` and `persistent-entity`. That
existing dependency shape decides placement — no new crate earns its keep.

| File | Action | Contents |
|------|--------|----------|
| `crates/runtime/src/effects/mod.rs` | Create | Subsystem root, re-exports |
| `crates/runtime/src/effects/store.rs` | Create | `EffectStateStore`, `EffectDedupStore` traits, `EffectState`, `EffectStoreError` |
| `crates/runtime/src/effects/executor.rs` | Create | `ExternalEffectExecutor`, `AttemptOutcome`, `EffectContext` |
| `crates/runtime/src/effects/registry.rs` | Create | `ExecutorRegistry` (HashMap keyed by `effect_type`), duplicate fail-closed |
| `crates/runtime/src/effects/queue.rs` | Create | Internal `EffectQueue` (bounded `mpsc` wrapper) — **not public** |
| `crates/runtime/src/effects/runner.rs` | Create | `DeliveryRunner`: drain loop, backoff re-enqueue, semaphore, `watch` shutdown |
| `crates/runtime/src/effects/policy.rs` | Create | `RetryPolicy`, `DeliveryConfig`, `DeliveryConfig::immediate()` |
| `crates/runtime/src/effects/acceptor.rs` | Create | `RuntimeEffectAcceptor` (mints id, attaches tenant, `send().await`); `EffectEnvelope` metadata wrapper |
| `crates/persistent-entity/src/effect_acceptor.rs` | Create | `EffectAcceptor` port trait (mirrors `publisher.rs`) |
| `crates/persistent-entity/src/persistent_entity.rs` | Modify | New default `external_effects(...)` method |
| `crates/persistent-entity/src/actor.rs` | Modify | Optional `effect_acceptor` field; call in post-persist sequence |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `register_effect_executor`, delivery config, runner start + `register_async_teardown` drain |
| `crates/testkit`, `examples/reference-app` | Modify | Recording executor + one trivial executor (deferred to spec/tasks) |

Both public ports live in `crates/runtime` per proposal §16 (Runtime owns
delivery state). The acceptor **trait** lives in `persistent-entity` so the
actor depends only on its own crate (exactly as it does for `EventPublisher`),
keeping the `runtime → persistent-entity` direction and adding no cross-layer edge.

### Extension-surface classification

One place to check "is this thing I'm touching public API or not?" before
changing it. Public SPI changes are semver-breaking; the other two rows are free
to evolve.

| Category | Meaning | Types / modules this design introduces |
|----------|---------|------------------------------------------|
| **Public SPI** | Traits external code implements or depends on | `EffectStateStore`, `EffectDedupStore`, `ExternalEffectExecutor`; plus the `EffectAcceptor` port trait in `persistent-entity` |
| **Internal runtime** | Exists and is used across the subsystem, but is not a public extension point | `EffectQueue` (bounded `mpsc` + `watch` wake-up), `DeliveryRunner`, `ExecutorRegistry`, `RuntimeEffectAcceptor` (the port impl) |
| **Private helper** | Implementation detail, not exposed even within the crate's public module tree; free to change | `EffectEnvelope` (metadata wrapper — permanently private, see §4), internal state-transition helpers in `store.rs` |

## 3. Architecture Decisions

| AD | Decision | Rejected | Rationale |
|----|----------|----------|-----------|
| AD-1 Acceptance seam | Hook the **actor post-persist sequence** (`actor.rs:294`, beside the fire-and-forget publish), not the `EffectInterpreter::ExternalEffects` arm | Activate dormant interpreter arm | The actor already owns the established tenant (`entity_id.tenant_id`), entity identity, serial ordering, and the exact post-commit point. Routing through the generic `EffectInterpreter<E,R,S>` adds one indirection and still needs a runtime handle inside it, for one variant. Interpreter keeps its exhaustive-match contract untouched (§15). |
| AD-2 Handler API | New `async fn external_effects(&self, cmd, new_state, events, ctx) -> Vec<ExternalEffectDescription>` with `{ Vec::new() }` default | Change `handle_command` return; context sink mutation | Default body = existing impls compile unchanged + zero cost (§20). Receives committed `events` + `new_state` so effects are derived from what actually persisted. |
| AD-3 Acceptor port location | `EffectAcceptor` trait in `persistent-entity`; impl in `runtime` | Trait in `runtime` | Actor references its own crate only, identical to `EventPublisher`. No new layer edge. |
| AD-4 Ports in runtime | `EffectStateStore` + `EffectDedupStore` in `crates/runtime/src/effects/` | Domain (like read-side `DedupStore`) | Delivery state is a runtime responsibility (§6); domain stays I/O- and state-free. `-Store` suffix still matches the read side. |
| AD-5 Backoff defaults | Ship the read-side precedent as **runtime default constants**, explicitly **not** spec-normative numbers: `DEFAULT_MAX_ATTEMPTS: u32 = 3`, `DEFAULT_BASE_BACKOFF = Duration::from_millis(100)`, `DEFAULT_MAX_BACKOFF = Duration::from_secs(10)`, exponential + full jitter | Retune for external calls; pin the numbers in the spec | The spec's *behavioral* contract (retry with backoff, capped, jittered) is what is normative; these specific values are just this implementation's chosen defaults, overridable per-`effect_type`/per-adapter. The only in-repo retry precedent (`read_side/error.rs:9`) gives ops familiarity. Cap = 3 retries (4 attempts total). |
| AD-6 Wake-up | Bounded `tokio::sync::mpsc` for work + `tokio::sync::watch<bool>` for shutdown + `Semaphore` for concurrency | `Notify`, polling tick | Matches `runtime.rs:147` (mpsc handoff) and `scheduler.rs:140` (watch stop); zero idle cost. Retryable effects re-enter via `tokio::time::sleep` then re-`send`. |
| AD-7 Bookkeeping-failure semantics | On post-success bookkeeping failure (`mark_succeeded`/`commit_success`), bounded-retry the idempotent write; if it still fails, keep the effect `in-flight`/`retryable` and re-dispatch rather than losing it | Swallow the error and mark succeeded anyway; treat the bookkeeping failure as terminal | Preserves the at-least-once guarantee (§7): re-dispatch is safe because idempotency-key propagation is mandatory, so a cooperating destination collapses the duplicate. Consequence: a rare double-attempt on the destination, never a silent loss or a false-terminal. Failure-window detail in §6.5. |

**Why the spec leaves the numbers open while the design pins them (AD-5):** the
spec states a *behavioral* contract; this design is **one valid implementation**
of that contract, not a restatement of it. The constants above live in
`crates/runtime/src/effects/policy.rs` as defaults — explicitly **not** a
spec-normative requirement. A durable adapter or a per-`effect_type` override
may choose different values without violating the spec, which is precisely why
the spec does not pin exact numbers.

## 4. Public Contracts (Rust)

```rust
// crates/persistent-entity/src/effect_acceptor.rs
#[async_trait]
pub trait EffectAcceptor: Send + Sync {
    /// Post-commit acceptance. Awaits queue capacity (backpressure, §9);
    /// never refuses. Mints the effect id and attaches `tenant` internally.
    async fn accept(&self, tenant: &TenantId, effects: Vec<ExternalEffectDescription>);
}

// crates/runtime/src/effects/executor.rs
pub enum AttemptOutcome { Success, RetryableFailure(String), TerminalFailure(String) }

#[async_trait]
pub trait ExternalEffectExecutor: Send + Sync {
    /// Exactly one attempt of one effect. No retry/backoff/dedup/persistence here.
    async fn execute(&self, effect: &ExternalEffectDescription, ctx: &EffectContext) -> AttemptOutcome;
    /// Doc-only signal for end-to-end semantics; does not change runtime behavior.
    fn honors_idempotency_key(&self) -> bool { false }
}
// EffectContext { effect_id, tenant: TenantId (read-only fact), attempt: u32, idempotency_key }

// crates/runtime/src/effects/store.rs
pub enum EffectState { Pending, InFlight, Succeeded, RetryableFailed, TerminalFailed }

/// Persistable, portable point in time — wraps `chrono::DateTime<Utc>` (the
/// same convention `ego_domain::Clock` already returns), never
/// `std::time::Instant`, which is monotonic/process-local and cannot survive
/// a restart or be compared across processes.
pub struct Timestamp(chrono::DateTime<chrono::Utc>);
impl Timestamp {
    pub fn now() -> Self { /* ... */ }
    pub fn from_utc(dt: chrono::DateTime<chrono::Utc>) -> Self { /* ... */ }
    pub fn into_utc(self) -> chrono::DateTime<chrono::Utc> { /* ... */ }
}

/// Public DTO the trait actually takes — every field is public API, unlike
/// the crate-private `EffectEnvelope`.
pub struct AcceptedEffect {
    pub id: EffectId,
    pub tenant: TenantId,
    pub attempt: u32,
    pub description: ExternalEffectDescription,
}

/// Everything needed to re-execute one accepted effect after a restart.
pub struct StoredEffect {
    pub id: EffectId,
    pub tenant: TenantId,
    pub description: ExternalEffectDescription,
    pub attempt: u32,
    pub state: EffectState,
    pub next_at: Timestamp,
}

#[async_trait]
pub trait EffectStateStore: Send + Sync {
    async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError>;
    async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError>;
    async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError>;
    async fn mark_retryable(&self, id: EffectId, attempt: u32, next_at: Timestamp) -> Result<(), EffectStoreError>;
    async fn mark_terminal(&self, id: EffectId, reason: TerminalReason) -> Result<(), EffectStoreError>;
    /// Effects due for (re-)dispatch at `now`, up to `limit` — with enough
    /// data (`tenant` + `description`) to actually re-execute them.
    async fn claim_due(&self, now: Timestamp, limit: usize) -> Result<Vec<StoredEffect>, EffectStoreError>;
    /// Crash recovery: returns any `InFlight` effect to `Pending`; returns
    /// the count recovered.
    async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError>;
}

/// Errors returned by `EffectStateStore`/`EffectDedupStore`. Beyond
/// bookkeeping errors, a minimal transient/permanent split lets AD-7's
/// future delivery runner classify a bookkeeping failure as retryable vs
/// terminal.
pub enum EffectStoreError {
    NotFound(EffectId),
    InvalidTransition { id: EffectId, from: EffectState, to: EffectState },
    Conflict(String),
    TemporarilyUnavailable(String),
    Backend(String),
}

#[async_trait]
pub trait EffectDedupStore: Send + Sync {
    /// Single-flight reservation for the scoped key (tenant, effect_type, key).
    async fn reserve(&self, scope: &DedupScope, fingerprint: u64) -> Result<DedupOutcome, EffectStoreError>;
    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError>;
    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError>; // retryable → back to pending
}
// DedupOutcome { Fresh, Duplicate /* already in-flight/succeeded */, Conflict /* same scope, different fingerprint → InvalidEffect */ }
```

`EffectEnvelope` is the runtime-owned metadata wrapper (§15) that now wraps
`AcceptedEffect` plus room for internal-only metadata (trace id, correlation
id, `created_at`, `accepted_at`) added later without a semver break. The
frozen `ExternalEffectDescription` gains no fields. Slice 1 ships one
`InMemoryEffectStore` implementing **both** ports (convenience only, §3
caveat), and now also retains `tenant`/`description` per accepted effect (not
just state bookkeeping) so `claim_due`/`recover_in_flight` return real,
re-dispatchable data.

**`EffectEnvelope` is permanently runtime-private and MUST NOT be exposed as
public API.** It occupies the **Private helper** row of the extension-surface
table (§2) and is used internally by the future acceptor/queue/runner
(PR2/PR3) — it is no longer part of any `EffectStateStore` signature.

**Consequence (revised):** because `EffectStateStore::accept` takes the
public `AcceptedEffect` (not `EffectEnvelope`), this port — like
`EffectDedupStore` — is genuinely implementable from any crate that depends
on `ego-runtime`, not only from within it. A future durable-store crate needs
only the public `AcceptedEffect`, `StoredEffect`, `Timestamp`, and
`EffectStoreError` types; nothing crate-private. There is no remaining
constraint that ties a durable implementation to living inside
`crates/runtime::effects`.

## 5. Data Flow

    handle_command ─► persist_events (COMMIT, actor.rs:221) ─► apply_events
         │                                                         │
         │              external_effects(cmd,state,events,ctx) ◄───┘
         ▼
    RuntimeEffectAcceptor.accept(tenant, effects)   ── mint id + attach tenant
         │   send().await  ◄── BACKPRESSURE (delays REPLY, never commit)
         ▼
    [ bounded mpsc EffectQueue ]  ──recv().await──►  DeliveryRunner
         ▲                                               │ reserve(scope) single-flight
         │ re-enqueue after backoff (tokio::time)        │ mark_in_flight
         │                                               ▼
         └──── RetryableFailure ◄── ExternalEffectExecutor.execute(effect, ctx)
                                          │ Success ─► commit_success + mark_succeeded
                                          │ Terminal/Missing/Invalid ─► mark_terminal + signal

## 6. Open-Question Resolutions

1. **Seam** → actor post-persist (AD-1). Plugs in between `apply_events` success
   (`actor.rs:254`) and `reply.send` (`301`), replacing the discarded
   `publish` position with an awaited `accept`.
2. **Backoff** → 3/100ms/10s + jitter, read-side precedent (AD-5).
3. **`EventPublisher` migration** → **explicitly deferred**, out of slice-1
   scope. The two post-commit channels (reliable effects vs. fire-and-forget
   publish) stay inconsistent; flagged on the roadmap, not fixed here.
4. **One executor / N types** → builder sugar
   `register_effect_executor(["s3.put","s3.delete"], Arc::new(exec))` iterates
   the keys and inserts the same `Arc` clone per key; duplicate key →
   `RegistryError::DuplicateEffectType` surfaced at `.build()` (fail-closed, §11).
5. **Executor succeeds, state-update fails** → **accepted at-least-once stance**,
   promoted to a first-class decision in **AD-7**.
   `mark_succeeded`/`commit_success` are retried a bounded number of times
   (idempotent inserts). If they still fail, the effect stays `in-flight`/
   `retryable` and is re-dispatched — safe because idempotency-key propagation
   is mandatory and the guarantee is already at-least-once (§7). With the
   in-memory store this window is a same-process mutex insert (effectively
   unfailable); the contract is stated for durable stores where it is real.

## 7. `ImmediateDeliveryPolicy`

Not a bypass — a `DeliveryConfig` profile of the one pipeline:
`DeliveryConfig::immediate()` sets `queue_capacity: 1`, `retry:
RetryPolicy::none()` (0 retries), `runner_mode: Inline`. Default profile:
`queue_capacity` configurable, `retry` = AD-5 defaults, `runner_mode: Deferred`
(spawned drain task).

**What `Inline` means — model (b), not model (a).** The effect is still
enqueued through the *same* admission path and still traverses every pipeline
stage: `accept → EffectQueue → DeliveryRunner → ExternalEffectExecutor`. The
only difference from `Deferred` is *where the runner runs* — instead of a
separately spawned drain task, `accept` drives one drain step on the caller's
own task/call stack before returning. There is **no** direct
`accept()`→`execute()` short-circuit and **no** queue-skipping; that would be
model (a), a bypass, which the locked decision forbids
("ImmediateDeliveryPolicy is a configuration of the same pipeline, not a
bypass"). The pipeline stages are the exact same code path, only configured for
immediacy. Sequence:

    accept(tenant, effects)                          // Inline == model (b)
      ├─ mint id, attach tenant
      ├─ queue.send(env).await                       // same EffectQueue stage
      └─ runner.drain_one().await                    // same DeliveryRunner code,
         │                                            //   run on the caller's task
         └─ reserve → mark_in_flight → execute → mark_succeeded | mark_terminal
      // Deferred differs at exactly one point: a spawned task owns drain_one,
      // not accept. Every other stage is identical.

A failed attempt under this profile is signaled, not retried.

## 8. Lifecycle (CORE-017)

Builder constructs store + runner; runner task spawned only when ≥1 executor is
registered (zero cost otherwise, §20). Drain registered via
`Runtime::register_async_teardown` (builder.rs:311) so it runs before the sync
teardown stack: stop accepting, drain pending until deadline, cancel in-flight
back to `pending`, emit `drain_incomplete` for the remainder (in-memory loss is
loud, never silent). Store init failure fails startup; backlog does not gate readiness.

## 9. Observability (CORE-012A)

Emission points: `accept` (accepted), runner pre-execute (dispatch_started,
attempt), post-`execute` (success | retry_scheduled | terminal_failed),
`reserve`==Duplicate (deduplicated), missing registry key (executor_missing),
per-attempt latency, queue depth (from `mpsc` capacity), oldest-pending age,
drain_incomplete. Correlation: effect id, `effect_type`, destination, tenant,
trace ctx, **hashed** idempotency key. `payload` never logged.

## 10. Threat Matrix

N/A — no shell, subprocess, VCS/PR automation, executable-file classification,
or process-integration boundary. Executor routing is in-process trait dispatch
over an opaque registry key, not command/path routing.

## 11. Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | Registry duplicate fail-closed; `RetryPolicy` backoff+jitter math; dedup `Fresh/Duplicate/Conflict`; state transitions | `#[tokio::test]` in each module |
| Unit | `external_effects` default returns empty (compile-unchanged) | existing-handler compile assertion |
| Integration | accept→queue→runner→executor happy path; retry on `RetryableFailure`; scoped-key dedup; unregistered type → terminal+signal | in-memory store + recording executor |
| Integration | backpressure delays reply not commit; `ImmediateDeliveryPolicy` inline | actor harness |
| E2E | reference-app handler describes effect, delivered post-commit, retried, deduped; kill-process documents in-memory loss boundary | dogfood executor |

## 12. Migration / Rollout

Additive + opt-in. Rollback = remove builder registration + actor field +
delete `effects/`; frozen domain types and existing handlers untouched;
in-memory store only, no data migration.

## 13. Open Questions

None blocking. Non-blocking (roadmap): `EventPublisher` migration (§6.3);
`destination: String` long-term shape if multiple destination shapes emerge.
