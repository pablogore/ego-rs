# Proposal: CORE-019 — Reliable External Effects

## Metadata

| Field | Value |
|-------|-------|
| Change ID | CORE-019 |
| Title | Reliable External Effects |
| Type | Core change (write-side effect delivery) |
| Date | 2026-07-12 |
| Parent | — (roadmap successor in the CORE-014 sequence: `openspec/changes/archive/2026-06-25-CORE-014-authorization-providers/proposal.md:95`) |
| Related | CORE-019A (External Data Providers, proposed follow-up — see §Relationship) |
| Enables | Durable `EffectStateStore` implementations (future amendment, e.g. Postgres outbox); CORE-019A vocabulary separation |
| Status | PROPOSING |

## 1. Motivation

`ego-rs` promises a write-side external-effect contract it cannot honor.
`crates/domain/src/effect.rs:3-7` documents that external effects are
"dispatched **after** the atomic commit succeeds. Handlers MUST NOT call
external systems directly." Nothing in the workspace performs that dispatch.
An application that follows the documented model today produces effect
descriptions that go nowhere. CORE-019 turns this incomplete abstraction into
an explicit, honestly-labeled **reliable external effects** capability without
coupling the domain to HTTP, brokers, databases, or e-mail.

## 2. Current Gap (verified against source)

1. **Contract exists, delivery does not.**
   `Effect::ExternalEffects(Vec<ExternalEffectDescription>)`
   (`crates/domain/src/effect.rs:52`) and `ExternalEffectDescription
   { idempotency_key, effect_type: String, payload: Vec<u8>, destination: String }`
   (`effect.rs:17-27`) are frozen value types. The only `EffectInterpreter`
   implementation in the workspace is the `#[cfg(test)] RecordingInterpreter`
   (`crates/runtime/src/interpreter.rs:62`), which counts effects. No
   production delivery worker, executor, retry, backoff, circuit breaker, outbox,
   effect worker, or external adapter exists for the write side.
2. **The production command path never sees `Effect`.**
   `PersistentEntity::handle_command` returns `Result<Vec<E>, _>` (events
   only); the actor's commit point is `persistence.persist_events(...)`
   (`crates/persistent-entity/src/actor.rs:221-229`), and the only post-commit
   side channel is a fire-and-forget `let _ = self.publisher.publish(&events)`
   (`actor.rs:294`) whose error is discarded — at-most-once, unobserved.
   Grep confirms `ExternalEffectDescription`/`EffectInterpreter` are referenced
   only inside `crates/domain` and `crates/runtime`; `examples/reference-app`,
   `crates/service-sdk`, `crates/testkit`, and `crates/persistent-entity`
   never use them. Handlers on the shipping path cannot even *describe* an
   external effect today.
3. **`IdempotencyKey` is a validated string, not a guarantee.**
   `crates/domain/src/idempotency.rs` enforces non-emptiness only; its module
   doc delegates duplicate detection entirely to the receiving system ("The
   receiving external system MUST use this key to detect and reject duplicate
   dispatches"). No dedup window, TTL, state, or store exists on our side.
4. **The write side has zero reliability primitives, but the read side has
   precedent.** `DedupStore` (scope `(projection_id, tag, event_id)`,
   `crates/domain/src/read_side/dedup.rs:20-41`) and the
   `Transient | Fatal | PoisonEvent` taxonomy with a documented retry policy
   ("max 3 retries, 100ms base, 10s max",
   `crates/domain/src/read_side/error.rs:9-11`) are established conventions
   CORE-019 should align with rather than reinvent.
5. **No canonical spec exists.** `openspec/specs/` contains seven specs
   (persistent-entity, security-jwt, security-sdk, http-transport,
   reference-service, service-sdk, testkit); none covers effects.

## 3. Proposed Capability

A write-side **effect delivery subsystem** with four cooperating contracts:

1. **Acceptance seam** — the runtime accepts a commit's
   `Vec<ExternalEffectDescription>` only after the successful atomic commit
   (post-commit, best-effort; **not** inside the commit transaction — see §7),
   and gives the persistent-entity path a backward-compatible way to describe
   effects at all (today it cannot).
2. **Delivery contracts (two public ports + one internal mechanism)** — the
   delivery-state responsibility is split at the design level into:
   - **`EffectStateStore`** (public port) — the pending → in-flight →
     succeeded / retryable-failed / terminal-failed state machine plus retry
     bookkeeping;
   - **`EffectDedupStore`** (public port) — scoped idempotency-key dedup
     (named to avoid colliding with the read side's `DedupStore`,
     `crates/domain/src/read_side/dedup.rs:20`);
   - **`EffectQueue`** (internal runtime mechanism, **not** a public trait) —
     admission and ordering of pending work.

   **Why `EffectQueue` is internal, not a swappable port**: the queue is
   purely in-process wake-up/ordering and holds no state, so it is the one
   piece that never needs to vary across implementations — in-memory now and
   a durable outbox later both swap `EffectStateStore` (plus
   `EffectDedupStore`), not the wake-up mechanism. A public `EffectQueue`
   trait would be an SPI with exactly one sensible implementation; it does
   not earn its keep.

   **What `EffectQueue` represents**: the queue is the wake-up/ordering
   mechanism only — `EffectStateStore` is the source of truth for effect
   state. The queue holds no durable state of its own and does not need to
   survive a crash, because state recovery (once a durable store exists)
   comes from `EffectStateStore`/`EffectDedupStore`, never from queue
   replay. A future durable implementation is free to organize its internals
   however it wants; the public contract surface is only the two store ports.

   A single struct MAY implement both public traits — slice 1 ships one
   in-memory composite (`InMemoryEffectStore`) — but
   the contracts MUST be separable so a future durable (outbox)
   implementation can satisfy them independently and enlist in the same unit
   of work as `persist_events`. **Caveat**: the composite is a slice-1
   convenience only, not a recommended pattern — future/production
   implementations (e.g. a durable outbox) are expected to satisfy these
   contracts via separate, independently swappable components.
3. **Delivery runner** — a runtime-owned worker that drains the queue,
   applies retry/backoff policy, enforces single-flight per idempotency key,
   bounds concurrency, and integrates with the CORE-017 lifecycle
   (`Runtime::shutdown/shutdown_async`,
   `crates/service-sdk/src/runtime/builder.rs:285,332`). Naming rationale in
   §11.
4. **`ExternalEffectExecutor` port + registry keyed by `effect_type`** — one
   executor per effect type, registered explicitly by the application. The
   runtime never contains a central `match effect_type { "http" => … }`;
   protocols live entirely behind executors.

## 4. Scope

### In Scope

- Domain-compatible way for persistent-entity handlers to describe external
  effects (exact API shape is a design concern; existing handlers keep
  compiling unchanged).
- Post-commit acceptance seam (runs after `persist_events` returns success,
  **not** inside its transaction — §7/§9); best-effort, so a future durable
  store built on it inherits the dual-write gap rather than closing it.
- `EffectStateStore` port + in-memory implementation.
- `ExternalEffectExecutor` port + explicit registry keyed by `effect_type`,
  wired through `RuntimeBuilder`.
- Retry policy (exponential backoff + jitter, attempt caps, per-effect-type
  override), error taxonomy, poison/terminal handling.
- Idempotency/dedup semantics on our side (states, key scope, window) — see §8.
- Lifecycle integration: startup, bounded queue, backpressure, graceful
  shutdown with draining deadline.
- Observability signals aligned with CORE-012A infrastructure.
- Tenant propagation per CORE-008A contracts.
- Testkit support and reference-app dogfooding of the capability.

### Out of Scope / Non-Goals

- Official adapters for HTTP, Kafka, NATS, Iggy, SMTP, S3, or any concrete
  system (tests/reference-app may use trivial local executors).
- Durable delivery store implementation (Postgres outbox) — enabled, not shipped.
- CDC, Debezium, distributed scheduling, cluster coordination, sharding,
  cross-node leasing.
- Physical exactly-once against arbitrary systems.
- Workflow engine, saga orchestration, temporal-style durable execution.
- Domain event sourcing changes.
- Read-side external providers (CORE-019A).
- HTTP/gRPC/GraphQL middleware; vendor-specific dead-letter queues.
- Circuit breaker (deferred with an extension point — see §7).

## 5. Capabilities

### New Capabilities

- `external-effects`: write-side external effect description, post-commit
  acceptance, delivery-state ownership, retry/idempotency semantics, executor
  registry, lifecycle, and observability contracts.

### Modified Capabilities

- `persistent-entity`: handlers gain a compatible mechanism to describe
  external effects; the actor routes accepted effects to the delivery
  subsystem after `persist_events` succeeds.
- `service-sdk`: `RuntimeBuilder` registers executors/policies; `Runtime`
  lifecycle (CORE-017) gains delivery-runner startup/drain/shutdown ordering.

## 6. Architectural Boundaries

| Layer | Owns | Never does |
|-------|------|------------|
| Domain | Describes effects (`ExternalEffectDescription`); carries the idempotency key | External I/O; delivery state; protocol knowledge |
| Runtime | Accepts effects post-commit; delivery state via `EffectStateStore`; retry/backoff/dedup policy; lifecycle, draining, telemetry | Interpreting `effect_type`/`destination` semantics; protocol I/O |
| Executor (adapter) | One protocol; executes one attempt; classifies protocol errors as retryable/terminal; may declare the destination honors idempotency keys | Retry loops, backoff, dedup, persistence, tenant minting |
| Application | Registers executors and policies; defines concrete effect types; chooses the delivery store | Bypassing the registry; calling executors directly from handlers |

Retry, dedup, and state live **once** in the runtime; executors stay
single-attempt and stateless with respect to reliability. This mirrors the
read-side split where the runtime batch executor owns retries and
`DedupStore` owns dedup, not each projection.

**Normative**: the runtime MUST remain transport-agnostic — it MUST NOT
branch on `effect_type` or `destination` values to select behavior; all
type-specific logic MUST live behind a registered `ExternalEffectExecutor`.
This is a hard requirement (enforced by success criterion §20), not a
design preference.

## 7. Execution Model, Delivery Semantics, Reliability

### Execution model decision

| Model | Verdict |
|-------|---------|
| A. Post-commit in-memory dispatch + in-process retry | **Shipped in CORE-019** as the default `InMemoryEffectStore` |
| B. Persistent outbox in the commit transaction | **Not shipped, and not enabled "for free"**: the acceptance seam is **post-commit** — acceptance runs after `persist_events` returns success (§9, and Design AD-1), never inside its transaction. A durable store built on this seam therefore **inherits the dual-write gap**: a crash between the commit and the durable-store write loses the effect description — the same failure mode as today's in-memory slice, only with a smaller window. Closing that gap (true transactional enlistment) would require moving acceptance *inside* the persistence transaction, which is an explicit non-goal here and a future redesign, not something this architecture already provides |
| C. One contract permitting both | **Chosen shape**: the separable delivery contracts (§3) let a durable `EffectStateStore` replace the in-memory one later (durability), independently of whether acceptance is ever moved inside the commit transaction (transactional enlistment, per B) |

### Wake-up model

The runner learns of new work via a **bounded `tokio::sync::mpsc` channel**
as the admission queue: acceptance is `send().await` (which is also the
backpressure point, §9), the runner loop is `recv().await`. This matches the
workspace's established Tokio conventions — bounded `mpsc` for work handoff
(`crates/runtime/src/runtime/runtime.rs:147`) and `tokio::sync::watch` for
stop signals (`crates/runtime/src/read_side/scheduler.rs:140`), which the
runner reuses for shutdown. Rejected: `Notify` (lost-wakeup bookkeeping
against a separate store), polling/scheduler tick (idle cost violating the
zero-cost-when-unused criterion, added latency), `watch` for work (coalesces
values). Retryable effects re-enter the queue after their backoff timer
fires (runtime-owned `tokio::time` re-enqueue), not via polling.

### Batching

Effects are dispatched **one at a time**: one effect, one executor
invocation, one attempt. The runtime never forms batches — the executor
contract stays "execute one attempt of one effect" (§11), which keeps
failure classification per-effect and avoids partial-batch semantics.
Throughput comes from bounded concurrency, not batching. If a protocol
benefits from batching, that is an executor-internal implementation detail
invisible to the runtime contract.

Honest distinctions this proposal commits to documenting verbatim in the spec:

- The domain *describes* effects — always true after CORE-019.
- The runtime *attempts* them with retries — true after CORE-019.
- Effects *survive a crash* — **false with the shipped in-memory store**.
  Accepted effects survive a crash when stored in a durable
  `EffectStateStore`; effects not yet accepted remain exposed to the
  post-commit dual-write gap. That gap includes any effect where the crash
  falls between the atomic commit and `EffectStateStore::accept` succeeding —
  a durable store narrows the window but does not close it. We never claim
  otherwise.
- Logical deduplication — true within the store's window and process
  lifetime; end-to-end only when the destination honors the key.

### Delivery guarantee

**At-least-once attempted delivery within the lifetime and durability of the
registered `EffectStateStore`**, plus mandatory idempotency-key
propagation to executors. With a cooperating destination this composes to a
logical once-only outcome. With the default in-memory store, the cross-crash
guarantee degrades to at-most-once, and the spec/API docs must say so. The
phrase "exactly once" is banned from the public contract.

### Failure taxonomy and retry

Attempt outcomes: `Success | RetryableFailure | TerminalFailure`, extended by
runtime-derived outcomes: `Timeout` (runtime-enforced per-attempt deadline →
retryable), `Cancelled` (shutdown → remains pending), `ExecutorMissing`
(no registration for `effect_type` → terminal, loud), `InvalidEffect`
(terminal), executor **panic** (caught via task isolation, counted as a
retryable attempt, subject to the same cap). The **executor classifies
protocol errors**; the **runtime classifies everything else and computes
backoff** (exponential + jitter, attempt cap, per-effect-type policy
override; defaults aligned with the read-side precedent in
`crates/domain/src/read_side/error.rs:9`). Terminal effects stay in the
store as `terminal-failed` with a signal — a queryable in-store dead-letter
state, not a vendor DLQ.

### Circuit breaker: deferred

Retry + idempotency are *correctness* foundations — without them the
documented contract is false. A breaker is a *load-protection* optimization
with no in-repo precedent and real tuning surface. CORE-019 keeps the
dispatch policy consultable per effect type (extension point) and defers the
breaker to a later amendment.

## 8. Idempotency Model

- `IdempotencyKey` (frozen, `crates/domain/src/idempotency.rs`) stays the
  domain-visible value. It is **not** treated as a guarantee by itself.
- The store's dedup identity is **scoped**: `(tenant, effect_type, key)` —
  never the bare string — preventing cross-tenant and cross-type collisions.
- Reuse of a scoped key with a **different payload/destination** is rejected
  as `InvalidEffect` (terminal, signaled), never silently deduplicated.
- Delivery states: `pending → in-flight → succeeded | terminal-failed
  (→ pending after backoff) | terminal-failed`.
- Dedup window / retention: TTL-based, store-configurable; the in-memory
  store additionally bounds entries. Exact defaults are a design/spec concern.
- Crash behavior: in-memory store forgets everything (documented); a durable
  store must recover `in-flight` as `pending` (at-least-once re-attempt).
- Concurrency: single-flight per scoped key — the same key is never
  in-flight twice concurrently.
- Executors may declare that the destination honors idempotency keys; this
  affects documentation of end-to-end semantics, not runtime behavior.
- **Effect identity (decided)**: the runtime mints the effect id at
  acceptance time — immediately after the atomic commit succeeds, before the
  effect enters the queue/store. This is stated normatively rather than left
  open because the rest of the document already assumes it: the effect id
  lives in the runtime-owned delivery-metadata envelope (§15, never a field
  on the frozen `ExternalEffectDescription`), and §12 names it the "runtime
  effect identifier". The id therefore exists for every accepted effect
  before dedup/state bookkeeping begins and associates with the scoped
  idempotency key from the first store interaction.
- Port naming: dedup is owned by the `EffectDedupStore` contract (§3);
  `InMemoryEffectStore` names the slice-1 in-memory composite implementing
  both public delivery contracts (`EffectQueue` is internal, §3), not a
  single monolithic port. Rejected
  umbrella names: `IdempotencyStore` (dedup is only one job),
  `EffectJournal` (implies append-only log), `EffectOutboxStore`
  (pre-commits to one implementation strategy). The `-Store` suffix matches
  the read side's `DedupStore`.

## 9. Ordering and Concurrency

The current API carries no ordering key beyond per-commit `Vec` order and an
opaque `destination: String`, so CORE-019 invents no order guarantee it
cannot keep:

- **Guaranteed**: single-flight per scoped idempotency key.
- **No ordering guarantee**: effects carry no ordering contract. Each accepted
  effect is delivered independently, and the store MAY return due effects in
  any order (nothing downstream needs or specifies ordering) — not global,
  per-aggregate, per-destination, per-commit acceptance, or execution order
  (execution is concurrent).
- Concurrency is bounded (configurable). Acceptance of committed effects is
  never refused *outright at intake* (the commit already happened — there is
  no synchronous "effect list rejected" path), so the bounded queue exerts
  **backpressure upstream**: acceptance awaits capacity, slowing command
  throughput rather than dropping effects. Recording an accepted effect MAY
  still fail after a bounded retry of a transient store error, in which case
  the caller receives an explicit post-commit `EffectAcceptanceError` — the
  committed event is never rolled back (commit and acceptance are separate
  concerns; Design AD-9).
- **Exact commit/backpressure boundary** (verified against
  `crates/persistent-entity/src/actor.rs:194-304`): the atomic commit is
  `persist_events` (`actor.rs:221-229`) and is **complete when it returns
  success** — it never waits on queue space. Effect acceptance is a
  separate step in the actor's post-persist sequence, before the command
  reply is sent (`actor.rs:301`): acceptance `await`s queue capacity there.
  So a full queue delays the command **reply**, never the commit, and a
  committed effect always eventually enters the queue. Because the actor
  processes commands serially, this is the upstream backpressure. This
  preserves the `effect.rs:3-7` invariant verbatim: effects are dispatched
  only after the atomic commit succeeds. Acceptance itself can also fail after
  a bounded retry of a transient store error; when it does, the actor
  propagates an explicit post-commit `EffectAcceptanceError` on the command's
  reply path instead of a success reply, and still never rolls back the
  committed event (Design AD-9).

## 10. Runtime Lifecycle Integration (CORE-017)

- **Startup**: delivery runner and store are constructed by `RuntimeBuilder`
  and started with the runtime; zero cost (no worker, no queue) when no
  executor is registered and the capability is unused.
- **Immediate delivery — one pipeline, not a bypass**: there is exactly one
  execution model — accept → queue → delivery runner → executor — and it
  always exists. The simple case is **not** a separate store type or no-queue
  code path (a distinct no-op implementation would create two execution
  models and inevitably grow `if null_store { … }` branches through the
  runtime); it is an **`ImmediateDeliveryPolicy`** configuration profile of
  the same pipeline: queue capacity effectively 1, retry policy of 0
  retries, delivery runner scheduled to run immediately/inline rather than
  deferred. A failed attempt under this profile is signaled, not retried.
  "Runtime owns the store" is therefore not mandatory ceremony — the simple
  case is just the smallest configuration of the one pipeline.
- **Readiness**: a runner with an empty or draining backlog does not gate
  readiness; a store that fails to initialize fails startup.
- **Shutdown** (`Runtime::shutdown/shutdown_async`,
  `crates/service-sdk/src/runtime/builder.rs:285,332`): stop accepting new
  work (command intake already stops first per CORE-017 ordering), drain
  pending effects until a configurable deadline, then stop. In-flight
  attempts get `Cancelled` and remain pending in the store. With the
  in-memory store, undrained effects are lost — emitted as an explicit
  `drain_incomplete` signal, never silent.
- **No executor registered** for a produced `effect_type`: terminal failure,
  loud signal — fail-closed, never silent drop.

## 11. Adapter Extensibility (naming decision)

Chosen: **`ExternalEffectExecutor`**, registered per `effect_type`.

- `*Handler` collides with the pervasive command-handler vocabulary
  (`handle_command` in `crates/persistent-entity/src/persistent_entity.rs:58`).
- `*Adapter` is not used as a trait suffix anywhere in the workspace.
- `Executor` matches the read side's batch executor
  (`crates/runtime/src/read_side/batch_executor.rs`) and describes exactly
  what it does: execute one attempt of one effect. Async trait, `Send + Sync`,
  mirroring `EffectInterpreter` and `KeyResolver` (CORE-011A AD-008) precedent.

**Extension surface (summary)**: CORE-019 exposes exactly three swappable
extension points — the `EffectStateStore` and `EffectDedupStore` public
ports (§3) and the `ExternalEffectExecutor` registry. `EffectQueue` is an
internal runtime wake-up mechanism, not part of the extension surface (§3).

**Duplicate registration (normative)**: registering a second executor for an
`effect_type` that already has a registered executor MUST fail at
registration time. Registration is a startup-time, one-owner-per-type
contract — there is no last-wins, first-wins, or multicast. Fail-closed at
registration is consistent with the proposal's existing bias (missing
executor at dispatch time is a loud terminal failure, §10/§15) and with the
workspace's `RegistryError` fail-fast precedent.

**Worker naming — "Delivery Runner" over "Dispatcher".** "Dispatcher"
connotes exactly the central `switch(type)` routing this proposal bans (§6
normative rule), so it is dropped. Of the considered alternatives —
"Delivery Engine", "Delivery Runtime", "Delivery Coordinator" — none has a
suffix precedent in the workspace, and "Delivery Runtime" additionally
collides with `Runtime` and the `crates/runtime` crate. `Runner` is the
established workspace name for a drain-loop worker (`ReadSideRunner`,
`crates/domain/src/read_side/runner.rs:17`), which is precisely this
component's role: drain the queue, drive attempts, own retry timing. The
component is the **delivery runner** throughout this proposal; "dispatch"
survives only as the verb for handing one effect to one executor.

## 12. Observability

Aligned with CORE-012A infrastructure. Conceptual signals: effect accepted,
dispatch started, attempt, success, retry scheduled, terminal failure,
deduplicated, executor missing, per-attempt latency, queue depth, age of
oldest pending effect, drain incomplete. Correlation fields: runtime effect
identifier, `effect_type`, logical destination, tenant (when scoped),
trace/correlation context, and a **redacted/hashed** idempotency key.
Payloads (`payload: Vec<u8>`) are never logged or exported by default.

## 13. Security and Tenant Isolation (CORE-008A)

- The runtime attaches the **established** tenant from the entity identity at
  acceptance time (the same authoritative source `persist_events` already
  receives at `crates/persistent-entity/src/actor.rs:225`). Effects never
  carry caller-supplied tenant hints; `TenantResolver`/`CanonicalTenant`
  contracts (`crates/service-sdk/src/runtime/tenant.rs`) are reused, not
  redefined.
- Tenant lives in runtime-owned delivery metadata, not in new
  `ExternalEffectDescription` fields.
- Dedup identity is tenant-scoped (§8); store implementations must not leak
  state across tenants.
- Executors receive the established tenant as a fact and cannot substitute
  another; an executor has no API through which to mint or swap tenants.
- Payloads are treated as sensitive (§12).

## 14. Relationship with CORE-019A

- **Out of scope here.** CORE-019 is command/write-side delivery; CORE-019A
  is read-side, I/O-capable ports for handlers that need external data.
  They do not share lifecycle, delivery guarantees, or idempotency, and
  merging them would blow past a reviewable change size.
- **Naming**: CORE-019A is **External Data Providers**, matching the
  dominant `*Provider` suffix (`AuthenticationProvider`,
  `ConfigurationProvider`, CORE-014 authorization providers) and the
  CORE-011A async-`KeyResolver` shape it would generalize. The name also
  keeps clear distance from `Effect` (`crates/domain/src/effect.rs:40`),
  the shipped write-side outcome enum.
- **Relationship label**: *Related, sequenced after CORE-019* — a roadmap
  ordering to settle the effects vocabulary first. There is **no technical
  dependency**; CORE-019A is not "Enabled by" CORE-019 and this proposal does
  not design it.

## 15. Compatibility and Migration

- `ExternalEffectDescription` is preserved unchanged; runtime delivery
  metadata (effect id — minted by the runtime at acceptance time, §8 —
  tenant, attempt state) wraps it in a runtime-owned envelope, keeping the
  frozen `Eq/Hash` value type intact.
- `effect_type: String` stays open (registry key); `destination: String`
  stays opaque (executor-interpreted). No enum of transports.

### Opaque contracts (normative)

- **Effect type**: `effect_type` identifies the logical contract/business
  meaning of the effect (e.g. `invoice.created`, `notification.email.send`)
  — never the transport or protocol. Implementers MUST NOT register effect
  types named after transports (`http`, `grpc`, `kafka`); the transport is
  the executor's private concern.
- **Destination**: the runtime MUST treat `destination` as an opaque
  routing key it never parses or interprets — giving it meaning (URL,
  topic, queue, exchange, bucket, host) is entirely the executor's job.
  More specifically, the destination's semantic meaning belongs entirely to
  the executor: the runtime has no concept of what a destination "means"
  beyond being a value it forwards, and different executors are free to
  interpret the same field shape differently.
  Open question worth flagging: `String` conflates those semantics, so
  whether it is the right long-term contract should be revisited if/when
  multiple destination shapes emerge across executors. CORE-019 keeps the
  frozen field type and changes nothing here.
- **Payload**: the runtime MUST treat `payload: Vec<u8>` as opaque bytes —
  it MUST NOT deserialize, inspect, or version payloads. Serialization
  format, versioning, and schema evolution are entirely the producing
  handler's and consuming executor's concern.
- `EffectInterpreter` keeps its exhaustive-match contract
  (`crates/runtime/src/interpreter.rs:26`); CORE-019 adds the delivery
  subsystem beside it rather than widening the trait. Exact wiring between
  the interpreter seam and the persistent-entity actor is a design concern.
- Existing `PersistentEntity` implementations compile and behave unchanged;
  describing external effects is additive and opt-in.
- Applications that want simple immediate delivery configure the one
  pipeline via `ImmediateDeliveryPolicy` (§10) — there is no bypass
  implementation and no second execution model to opt into.
- An application with no external effects pays zero runtime cost.
- Producing an effect with no registered executor is **fail-closed**
  (terminal failure + signal), never a silent drop.

## 16. Affected Areas

| Area | Impact | Description |
|------|--------|--------------|
| `crates/domain/src/effect.rs`, `idempotency.rs` | Referenced | Frozen value types; contract docs corrected to stop overpromising |
| `crates/runtime` (new module(s)) | New | Delivery ports (state/dedup), internal queue mechanism, delivery runner, executor port/registry, policies |
| `crates/persistent-entity/src/actor.rs`, `persistent_entity.rs` | Modified | Compatible effect description + post-commit routing |
| `crates/service-sdk/src/runtime/builder.rs` | Modified | Executor/policy registration; lifecycle ordering |
| `crates/testkit` | Modified | Recording executor / delivery assertions |
| `examples/reference-app` | Modified | Dogfoods one trivial executor (per CORE-018/026 convention) |
| `openspec/specs/external-effects/` | New | First canonical spec for this capability |

## 17. Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| "Reliable" read as crash-durable while only in-memory ships | High | Guarantee labeled per store in every doc/spec; §7 wording is normative |
| Persistent-entity handler API extension breaks compat | Med | Additive opt-in mechanism; compile-unchanged is a success criterion |
| Durable store built on the post-commit seam mistaken for transactional (dual-write gap assumed closed) | Med | Seam is explicitly post-commit/best-effort (§7 Model B, §9); docs state a durable store inherits the dual-write gap with a smaller window, and that true transactional enlistment is a separate future redesign, not provided here |
| Backpressure at acceptance throttles command throughput | Med | Bounded queue sizing configurable; queue-depth/oldest-age signals |
| Scope creep toward brokers/outbox/breaker | Med | Non-goals §4; breaker deferred with extension point |
| Spec-phase load: capability spec never existed | Med | §5 names the new spec explicitly for sdd-spec |

## 18. Rollback Plan

All new contracts live in new modules behind opt-in registration. Rollback =
remove registration wiring from `RuntimeBuilder`/actor and delete the new
modules; frozen domain types and existing handlers are untouched, so revert
is a clean commit-range revert with no data migration (in-memory store only).

## 19. Dependencies

- CORE-017 lifecycle (archived, shipped) — the delivery runner hooks into it.
- CORE-008A tenant contracts (archived, shipped) — reused as-is.
- CORE-012A observability integration (archived, shipped) — signal plumbing.
- No new external crates anticipated beyond the existing async stack (Tokio,
  `async_trait`, `thiserror`).

## 20. Success Criteria

- [ ] A reference-app handler describes an external effect; it is executed
      after commit by a registered executor, observably retried on transient
      failure, and deduplicated on scoped-key replay.
- [ ] Kill-the-process test documents the in-memory loss boundary explicitly
      (guarantee honesty is asserted, not hidden).
- [ ] No `match` on `effect_type`/`destination` string literals anywhere in
      `crates/runtime`/`crates/service-sdk` (registry only).
- [ ] Existing `PersistentEntity` implementations compile unchanged.
- [ ] Effect with unregistered type produces a terminal failure and signal,
      never a silent drop.
- [ ] Shutdown drains within the deadline or emits `drain_incomplete`.
- [ ] Zero measurable overhead when the capability is unused.

## 21. Decision Summary (explicit answers)

| # | Question | Answer |
|---|----------|--------|
| 1 | Do effects survive a process crash? | No, with the shipped in-memory store. With a future durable `EffectStateStore`, only effects already accepted (recorded in the store) before the crash survive; effects not yet accepted — including any where the crash falls between commit and `EffectStateStore::accept` succeeding — stay exposed to the post-commit dual-write gap. The port is shaped for durability but does not close that gap. |
| 2 | Delivery guarantee? | At-least-once attempted delivery within store lifetime/durability + idempotency-key propagation; logical once-only when the destination cooperates; at-most-once across crashes with the default store. |
| 3 | Who stores delivery state? | The runtime-owned, separable public delivery ports — `EffectStateStore`/`EffectDedupStore` (§3), fed by the internal `EffectQueue` wake-up mechanism; one in-memory composite ships. |
| 4 | What does idempotency really mean? | Scoped-key `(tenant, effect_type, key)` dedup within the store's window, single-flight per key, key forwarded to executors; end-to-end only if the destination honors it. |
| 5 | No adapter registered? | Terminal failure + loud signal — fail-closed, never dropped silently. |
| 6 | During shutdown? | No new acceptance; drain until deadline; in-flight cancelled back to pending; `drain_incomplete` signal for remainder. |
| 7 | Who decides retryable? | Executor classifies protocol errors; runtime classifies timeout/panic/missing-executor/invalid and computes backoff. |
| 8 | Guaranteed order? | None across effects — single-flight per scoped key only. The store MAY return due effects in any order; no global, per-aggregate, per-destination, per-commit acceptance, or execution order. |
| 9 | Does the runtime know protocols? | No. `effect_type` is an opaque registry key; protocols live in `ExternalEffectExecutor` implementations. |
| 10 | What remains for CORE-019A? | Read-side External Data Providers — related, sequenced after CORE-019, no technical dependency, not designed here. |
| 11 | Who mints the effect id, and when? | The runtime, at acceptance time — immediately after the atomic commit succeeds, before the effect enters the queue/store (§8). |
| 12 | Two executors for one `effect_type`? | Error at registration time — fail-closed, one owner per type; no last-wins/first-wins/multicast (§11). |

## 22. Open Questions

1. ~~Should the acceptance seam live in the `EffectInterpreter` `ExternalEffects`
   arm (activating the dormant trait) or directly in the actor's post-persist
   sequence (`actor.rs:230-301`)? Both satisfy this proposal; the tradeoff
   (trait activation vs. one fewer indirection) is a design-phase decision.~~
   **Resolved in Design AD-1** (post-persist seam).
2. ~~Default backoff/attempt-cap numbers: adopt the read-side precedent
   (3 retries, 100ms base, 10s max) verbatim or retune for external calls?
   Needs a product/ops preference; proposal defaults to the precedent.~~
   **Resolved in Design AD-5** (runtime default constants, not spec-normative).
3. ~~Should CORE-019 also route the existing fire-and-forget
   `EventPublisher.publish` (`actor.rs:294`) through the new delivery
   subsystem, or is that a separate hardening change? Proposal keeps it out
   of scope to bound size, but the inconsistency (two post-commit channels,
   one unreliable) should be acknowledged on the roadmap.~~
   **Deferred — see Design §6.3** (EventPublisher migration out of scope for
   this slice).
4. ~~Can one executor support multiple `effect_type`s? Likely direction:
   **yes** — the registry is keyed by `effect_type` and nothing in §11
   prevents registering the same executor instance under several keys (e.g.
   an S3-family executor handling both `s3.put` and `s3.delete`). Whether
   the registration API sugars this or the application simply registers the
   same instance twice is a design-phase detail.~~
   **Resolved in Design §6.4** (registry keyed by `effect_type`, one executor
   may register multiple keys).
5. ~~What happens if an executor reports success but the runtime's own
   state-update fails?~~ **Resolved in Design AD-7** (bookkeeping failures
   preserve at-least-once semantics via idempotent bounded retry of
   `mark_succeeded`).
