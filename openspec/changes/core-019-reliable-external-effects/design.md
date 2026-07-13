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
| `crates/runtime/src/effects/store.rs` | Create | `EffectStateStore` + `EffectDedupStore` traits; `EffectState`, `TerminalReason`, `EffectStoreError`; public DTOs `EffectId`, `Timestamp`, `AcceptedEffect`, `StoredEffect`, `DedupScope`, `DedupOutcome`; crate-private `EffectEnvelope`; `InMemoryEffectStore` composite |
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
| **Public SPI** | Traits external code implements or depends on, plus the public DTOs their signatures use | `EffectStateStore`, `EffectDedupStore`, `ExternalEffectExecutor`, `ExecutorRegistry`, `DuplicateEffectType` (all `pub use`-exported from `effects::mod`); the public DTOs `AcceptedEffect`, `StoredEffect`, `Timestamp`, `EffectStoreError`, `EffectId`, `TerminalReason`, `DedupScope`, `DedupOutcome`; plus the `EffectAcceptor` port trait in `persistent-entity` |
| **Internal runtime** | Exists and is used across the subsystem, but is not a public extension point | `EffectQueue` (bounded `mpsc` + `watch` wake-up), `DeliveryRunner`, `RuntimeEffectAcceptor` (the port impl) |
| **Private helper** | Implementation detail, not exposed even within the crate's public module tree; free to change | `EffectEnvelope` (metadata wrapper — permanently private, see §4), internal state-transition helpers in `store.rs` |

## 3. Architecture Decisions

| AD | Decision | Rejected | Rationale |
|----|----------|----------|-----------|
| AD-1 Acceptance seam | Hook the **actor post-persist sequence** (`actor.rs:294`, beside the fire-and-forget publish), not the `EffectInterpreter::ExternalEffects` arm | Activate dormant interpreter arm | The actor already owns the established tenant (`entity_id.tenant_id`), entity identity, serial ordering, and the exact post-commit point. Routing through the generic `EffectInterpreter<E,R,S>` adds one indirection and still needs a runtime handle inside it, for one variant. Interpreter keeps its exhaustive-match contract untouched (§15). |
| AD-2 Handler API | New `async fn external_effects(&self, cmd, new_state, events, ctx) -> Vec<ExternalEffectDescription>` with `{ Vec::new() }` default | Change `handle_command` return; context sink mutation | Default body = existing impls compile unchanged + zero cost (§20). Receives committed `events` + `new_state` so effects are derived from what actually persisted. |
| AD-3 Acceptor port location | `EffectAcceptor` trait in `persistent-entity`; impl in `runtime` | Trait in `runtime` | Actor references its own crate only, identical to `EventPublisher`. No new layer edge. |
| AD-4 Ports in runtime | `EffectStateStore` + `EffectDedupStore` in `crates/runtime/src/effects/` | Domain (like read-side `DedupStore`) | Delivery state is a runtime responsibility (§6); domain stays I/O- and state-free. `-Store` suffix still matches the read side. |
| AD-5 Backoff defaults | Ship the read-side precedent as **runtime default constants**, explicitly **not** spec-normative numbers: `DEFAULT_MAX_ATTEMPTS: u32 = 3`, `DEFAULT_BASE_BACKOFF = Duration::from_millis(100)`, `DEFAULT_MAX_BACKOFF = Duration::from_secs(10)`, exponential + full jitter | Retune for external calls; pin the numbers in the spec | The spec's *behavioral* contract (retry with backoff, capped, jittered) is what is normative; these specific values are just this implementation's chosen defaults, overridable per-`effect_type`/per-adapter. The only in-repo retry precedent (`read_side/error.rs:9`) gives ops familiarity. Cap = 3 retries (4 attempts total). |
| AD-6 Wake-up | Bounded `tokio::sync::mpsc` for work + `tokio::sync::watch<bool>` for shutdown + `Semaphore` for concurrency | `Notify`, polling tick | Matches `runtime.rs:147` (mpsc handoff) and `scheduler.rs:140` (watch stop); zero idle cost. **Revised (PR2 round 2, see below): retryable effects re-enter SOLELY via `mark_retryable(next_at)` plus the periodic `claim_due`-driven reclaim loop — never via an in-process sleep-then-`send` timer.** **Revised again (PR2 round 4, see below): the reclaim loop claims-then-transitions (`mark_in_flight`) *before* enqueueing (F-01), and the main loop's backpressure-permit wait races shutdown too (F-03).** **Revised once more (PR2 round 5, see below): the reclaim loop no longer enqueues into `EffectQueue` at all — it dispatches claimed effects directly through the same permit-gated helper the queue-fed path uses, closing a self-deadlock where `send_reclaimed` could block forever on a queue only this same loop could ever drain (F-01).** |
| AD-7 Bookkeeping-failure semantics | On post-success bookkeeping failure (`mark_succeeded`/`commit_success`), bounded-retry the idempotent write; if it still fails, keep the effect `in-flight`/`retryable` and re-dispatch rather than losing it | Swallow the error and mark succeeded anyway; treat the bookkeeping failure as terminal | Preserves the at-least-once guarantee (§7): re-dispatch is safe because idempotency-key propagation is mandatory, so a cooperating destination collapses the duplicate. Consequence: a rare double-attempt on the destination, never a silent loss or a false-terminal. Failure-window detail in §6.5. |
| AD-8 Single-consumer claim invariant | Exactly one `DeliveryRunner` instance calls `claim_due` at a time in this slice; `claim_due` is deliberately **non-atomic** — it returns due effects without transitioning their state (a claimant that intends to dispatch still calls `mark_in_flight`) | Make `claim_due` atomically claim (transition to `InFlight` with compare-and-swap / lease-timeout semantics) | Distributed/multi-consumer coordination (leasing, cross-node claims) is out of scope per the proposal's non-goals (§4). Non-atomic `claim_due` is safe **only because** a single runner consumes it. Atomic claiming would add compare-and-swap semantics and lease timeouts with no current driver; deferred until a real multi-consumer need materializes. |
| AD-9 Acceptance-failure policy | `EffectAcceptor::accept` returns `Result<(), EffectAcceptanceError>`. The committed event is **never** rolled back because acceptance fails (commit and acceptance are separate concerns; commit success is final and unconditional). The caller's *successful* reply is withheld until acceptance of that command's effects completes. Only `EffectStateStore::accept`'s `TemporarilyUnavailable` error is retryable under the AD-5 `RetryPolicy`; every other store error is permanent and surfaces immediately as a post-commit `EffectAcceptanceError` (full mapping in the classification table below). If the retryable error survives the policy, the caller likewise receives an explicit post-commit `EffectAcceptanceError`. | Roll back the commit on acceptance failure; swallow the failure and reply success anyway; block shutdown until acceptance eventually succeeds | Extends the existing "backpressure delays the reply, never the commit" rule (§9) to also cover acceptance *failure*, not just queue-capacity waiting. Honest by construction: the error means "your command succeeded and its event is committed, but we could not durably-enough register at least one of its described effects, and it may be lost to the post-commit dual-write gap" — not that the command failed. Reuses the AD-5 `RetryPolicy` shape rather than a second policy type: acceptance retries the same class of transient store write, so a distinct tuning surface would be speculative. |

**Why the spec leaves the numbers open while the design pins them (AD-5):** the
spec states a *behavioral* contract; this design is **one valid implementation**
of that contract, not a restatement of it. The constants above live in
`crates/runtime/src/effects/policy.rs` as defaults — explicitly **not** a
spec-normative requirement. A durable adapter or a per-`effect_type` override
may choose different values without violating the spec, which is precisely why
the spec does not pin exact numbers.

**PR2 review follow-up (AD-6/AD-7/AD-8, `runner.rs`).** A post-merge review of
PR2 found three related gaps in the first `DeliveryRunner` cut, all fixed in
the same PR before the next one built on top:

- **AD-7 redispatch is now unconditional on the in-memory path.** Both
  `retry_or_give_up` (a delivery failure) and `finish_success` (bookkeeping
  exhausted after a real success) now always schedule the backoff-sleep-then-
  `queue.send` redispatch, regardless of whether the corresponding bookkeeping
  write (`mark_retryable` / `commit_success`+`mark_succeeded`) itself
  succeeded — that write is for durability/observability of the retry count,
  never a precondition for the in-process retry. The dedup reservation for
  the effect's scope is held for the *entire* backoff sleep and released only
  immediately before the redispatch re-enters the queue, closing a window
  where an early release could let a racing duplicate submission slip through.
  **(Superseded by the PR2 round 2 redesign immediately below — this
  sleep-then-`send` timer no longer exists.)**
- **A periodic reclaim loop closes the `mark_in_flight`-failure gap.**
  `claim_due` already existed for crash recovery (AD-8) but nothing ever
  drove it during normal operation, so an effect whose `mark_in_flight` write
  failed at drain time (safely still `Pending`, no side effect, no dedup
  reserved) had no path back into the pipeline. `DeliveryRunner::run`'s drain
  loop now also ticks a `claim_due(now, limit)` call on a fixed interval
  (default 5s, a middle ground between prompt recovery and not hammering the
  state store) as a third `tokio::select!` branch on the *same* single-
  consumer task — not a second consumer — re-feeding whatever comes back into
  the internal queue for another attempt.
- **Shutdown now actually waits for outstanding work.** Every per-effect
  dispatch task and every backoff-redispatch task is tracked in a shared
  `tokio::task::JoinSet` instead of a bare, discarded `tokio::spawn` handle.
  On the shutdown signal, the drain loop stops accepting new work (queue and
  reclaim tick alike) and then awaits the `JoinSet` draining, bounded by a
  local shutdown-drain deadline (5s default) so a stuck task can't block
  shutdown forever. This deadline is a local constant for now; once PR4's
  `RuntimeEffectAcceptor::drain(deadline, ..)` lands, its caller-supplied
  deadline should flow down into this runner instead of duplicating the idea.

**PR2 round 2 review follow-up (AD-6 revision, F-01 through F-04, dedup
lifetime).** A second review pass on this PR's diff found a real
double-dispatch race the fixes above introduced, a `JoinSet` self-deadlock,
and three other BLOCKERs, all fixed in the same PR:

- **AD-6 revision — one redispatch mechanism, not two.** The round-1 fix
  above gave every retryable effect *two* competing producers that could both
  end up enqueueing it around the same time once `next_at` passed: the
  in-process sleep-then-`queue.send` timer, and the `claim_due`-based reclaim
  loop (added in the same round, for a different gap). Whichever copy lost
  the race hit `InvalidTransition` and was silently discarded — accidental
  behavior, not a contractual one. Separately, the timer task's own need to
  self-register in the shared, shutdown-drained `JoinSet` was deadlock-prone
  against a shutdown already waiting on that very `JoinSet` (F-01). Fixed by
  picking exactly one mechanism: `EffectStateStore::mark_retryable(next_at)`
  is now the sole source of truth for "when is this effect due," and the
  periodic reclaim loop is the sole way it re-enters the queue.
  `retry_or_give_up` and `finish_success`'s exhausted-bookkeeping path now
  only call `mark_retryable` and return — no task is spawned for the backoff
  itself. This also resolves the `JoinSet` deadlock as a side effect: with
  the timer gone, only the main drain loop (queue-fed effects) and the
  reclaim loop (`claim_due`-fed effects) ever spawn into the tracked
  `JoinSet`, and neither is itself a tracked task recursively spawning
  another — no self-referential lock wait is possible.
- **Dedup-reservation lifetime, decoupled from "attempt".** Once a
  reservation is made `Fresh` for an effect's `(tenant, effect_type,
  idempotency_key)` scope, it now stays held for that effect's entire
  lifetime — released only on a genuinely terminal outcome (`Succeeded` via
  `commit_success`, or `TerminalFailed` via `abandon_and_release`) — never
  released and re-reserved on every attempt. `drain_one` only ever calls
  `EffectDedupStore::reserve` when `effect.attempt == 0` (a genuinely fresh,
  never-before-dispatched submission); every redispatch path in this file
  always bumps `attempt` to at least 1 before the effect can be reclaimed, so
  a retry of the *same* effect re-entering `drain_one` reliably skips
  `reserve` and can never be `Duplicate`'d against its own still-held
  reservation.
- **F-02 — per-`effect_type` retry policy override.** `DeliveryConfig`/
  `RetryPolicy` only ever supported one global policy per runner instance.
  Added `RetryPolicies { default_retry: RetryPolicy, retry_overrides:
  HashMap<String, RetryPolicy> }` (`policy.rs`) with `policy_for(effect_type)
  -> RetryPolicy`; `DeliveryRunner` now holds a `RetryPolicies` (constructed
  via `impl Into<RetryPolicies>`, so every existing call site passing a bare
  `RetryPolicy` keeps compiling unchanged) and calls `policy_for` everywhere
  it used to read one shared policy field directly — the retry-decision
  (`allows_retry`) and the backoff computation alike.
- **F-03 — AD-7's redispatch used to re-enter a state `mark_in_flight`
  rejects.** `finish_success`'s bookkeeping-exhausted path left the effect's
  stored state `InFlight` while (pre-redesign) scheduling a redispatch — but
  `mark_in_flight` only accepts `from: Pending | RetryableFailed`, and
  `claim_due` explicitly excludes `InFlight`, so that effect could never be
  picked back up by anything. Fixed: the exhausted path now calls
  `mark_retryable(id, attempt, next_at)` — its `allowed_from: [InFlight]`
  fits this "succeeded but bookkeeping didn't confirm" case exactly, no new
  store operation needed. Combined with the dedup-lifetime fix above, the
  effect's own reservation stays held and its later redispatch (attempt ≥ 1)
  skips `reserve` entirely, so it is never mistaken for its own duplicate.
- **F-04 — stable, portable dedup fingerprint.** `EffectDedupStore::reserve`'s
  `fingerprint: u64` (computed via `std::collections::hash_map::
  DefaultHasher`, which carries no cross-version/cross-build stability
  guarantee) is replaced by a public `EffectFingerprint([u8; 32])`
  (`store.rs`), computed via SHA-256 (already a transitive workspace
  dependency, reused rather than adding a new hashing crate) over a
  length-prefixed framing of payload and destination — so
  `payload=b"ab"+destination="cd"` never collides with
  `payload=b"a"+destination="bcd"` the way naive concatenation would.
  `EffectDedupStore::reserve`'s signature and every call site were updated;
  this changes already-merged (`develop`) public API, acceptable here since
  nothing outside this stack consumes it yet.
- **Previously-existing HIGH findings, fixed in the same pass:**
  `DedupOutcome::Duplicate` on a fresh submission is now marked `Succeeded`
  (a benign already-satisfied outcome), never `TerminalFailed` with an
  error-sounding reason; a cancelled/aborted executor task
  (`tokio::task::JoinError::is_cancelled()`, as opposed to `is_panic()`) is
  requeued without charging the effect's retry-attempt cap, instead of being
  charged as an ordinary `RetryableFailure`; `abandon`/`abandon_and_release`/
  `reclaim_due`'s previously-silent `let _ = ...` bookkeeping-failure
  discards now log via `tracing::warn!`; and `timestamp_after`'s
  `chrono::Duration` conversion now saturates to a large, safe fallback
  instead of silently degrading to zero on overflow (which would have
  caused a retry storm instead of backoff).

**PR2 round 3 review follow-up (two residual gaps closed).** The round 2
fix's own report honestly flagged two gaps it left unresolved; both are now
closed:

- **AD-6/shutdown — drain-deadline expiry now actually aborts outstanding
  work.** Before this fix, `DeliveryRunner::drain_tasks` simply gave up once
  its bounded deadline elapsed, leaving any still-running dispatch task (and
  its inner executor attempt) running, untracked, in the background forever
  — the exact concern the very first PR2 review round raised about this
  `JoinSet` design. Fixed: on deadline expiry, every in-flight executor
  attempt's `AbortHandle` (tracked separately from the outer per-effect
  `JoinSet`, precisely so classification stays intact) is aborted, giving
  `classify_join_result`'s `is_cancelled()` branch its first real production
  caller — the owning dispatch task then runs its already-existing
  `CancelledForShutdown` handling (`requeue_without_charging_attempt`) to
  completion and drains out normally. Only if a task is *still* stuck after a
  second bounded window (e.g. hung somewhere other than the executor await)
  does shutdown fall back to `JoinSet::abort_all()` as a last resort. Either
  way, shutdown now provably returns with zero background tasks left running
  past the deadline.
- **AD-7 — `mark_retryable` exhaustion now abandons instead of sticking.**
  Same "stuck and undiscoverable" class of bug F-03 already fixed for
  `finish_success`'s exhausted bookkeeping path, applied to
  `retry_or_give_up`'s own `mark_retryable` write: if its bounded retry is
  still failing after the bound, the effect used to be left silently
  `InFlight` forever, invisible until a future crash-recovery pass that isn't
  wired anywhere. Fixed: this exhaustion now falls back to
  `abandon_and_release` with `TerminalReason::Other("retry bookkeeping
  exhausted: store unavailable")`, and logs via `tracing::warn!` like every
  other abandon path — an operator sees a persistent store outage instead of
  a silent stall. (`finish_success`'s own separate exhaustion fallback is
  unchanged — out of this fix's scope.) The two fixes are independent: the
  drain-deadline abort only ever targets an executor attempt, never the flat,
  no-backoff `mark_retryable` bookkeeping loop itself, so aborting one cannot
  interrupt the other mid-retry.

**PR2 round 4 review follow-up (4 new BLOCKERs, unified dedup-identity
redesign).** A third review round found F-02 and F-04 shared one root cause
— the runner used `effect.attempt == 0` as a dual-purpose signal (both "how
many delivery attempts have been charged against the retry cap" and "has
dedup already been reserved for this effect") — plus two independent
BLOCKERs (F-01, F-03), all fixed in the same PR:

- **Dedup store gains real identity/status awareness (F-02 + F-04, unified
  fix, `store.rs`).** `DedupOutcome` grows two new variants alongside
  `Fresh`/`Duplicate`/`Conflict`: `OwnedInProgress` (this exact `EffectId`
  already owns the scope's reservation, not yet `Succeeded` — legitimately
  the SAME effect recovering/retrying) and `OwnedSucceeded` (this exact
  `EffectId`'s reservation is already recorded `Succeeded` — the ONLY case
  allowed to short-circuit without re-executing). `EffectDedupStore::reserve`
  gained an `effect_id: EffectId` parameter so a reservation records
  ownership, not just occupancy; `InMemoryEffectStore`'s `HashMap<DedupScope,
  EffectFingerprint>` became `HashMap<DedupScope, ReservationRecord>`
  (`effect_id`, `fingerprint`, `succeeded: bool`). `commit_success` now flips
  `succeeded` in place instead of being a no-op — a later crash-recovery
  re-attempt by the SAME effect must still find `OwnedSucceeded`, while a
  genuinely different future submission under the same scope must still be
  told it's settled (`Duplicate`), never `Fresh`. `release` is unchanged
  (still fully clears the scope). Before this fix, a plain `Duplicate` could
  mean either "a different submission already handled this" or "this exact
  effect's own reservation, still legitimately in-flight" — indistinguishable
  — and the runner treated every `Duplicate` as the former, silently marking
  a never-actually-executed, crash-recovered effect `Succeeded` (F-02, a
  silent-data-loss BLOCKER).
- **`drain_one`'s `effect.attempt == 0` dedup gate is gone (F-02 + F-04,
  `runner.rs`).** `dispatch_in_flight` (the post-`mark_in_flight` body
  `drain_one`/`drain_reclaimed` share) now calls `reserve_with_retry`
  unconditionally on every attempt — fresh, retried, or crash-recovered —
  and branches on the richer outcome: `Fresh`/`OwnedInProgress` → proceed to
  (re-)execute; `Duplicate`/`OwnedSucceeded` → short-circuit to success via
  `finish_already_satisfied`; `Conflict`/store-error → unchanged terminal
  handling. This also refines AD-7's redispatch-after-bookkeeping-failure
  tradeoff: previously *every* reclaim-eligible-after-success effect was
  unconditionally re-executed (a coarse, always-safe-but-sometimes-wasteful
  choice); now, when the dedup store can prove this exact effect already
  succeeded (`commit_success` landed even though `mark_succeeded` kept
  failing), the redispatch skips re-executing entirely.
- **`requeue_without_charging_attempt` no longer bumps `attempt` (F-04,
  `runner.rs`).** With dedup reservation no longer attempt-gated, the
  shutdown-cancellation path no longer needs to inflate `attempt` to skip
  re-reservation — it now leaves the stored attempt count exactly as it was
  before the cancellation. Before this fix, the bump (added purely for the
  now-removed dedup-gating trick) silently ate into the effect's real retry
  budget under `RetryPolicy::allows_retry`, directly contradicting the
  documented "cancellation can never exhaust the retry budget" guarantee.
- **`claim_due` can no longer enqueue the same effect multiple times across
  reclaim ticks (F-01, `runner.rs`/`queue.rs`).** `claim_due` itself still
  doesn't transition state (AD-8, unchanged) — but `reclaim_due` now calls
  `mark_in_flight` on each claimed `StoredEffect` immediately, before ever
  enqueueing it, and only enqueues the ones where that transition succeeds
  (an expected, harmless race — e.g. the direct accept-path queue entry for
  the same effect already transitioned it first — is logged at `debug` and
  skipped, not treated as an error). A new `QueuedEffect` enum
  (`Fresh`/`Reclaimed`, `queue.rs`) distinguishes a freshly-accepted effect
  (still needs `mark_in_flight`) from a reclaim-fed one (already `InFlight`
  — must NOT be transitioned again, which would immediately fail with
  `InvalidTransition`); `DeliveryRunner::run_inner`'s receive loop branches
  on it to call `drain_one` or the new `drain_reclaimed` accordingly. Before
  this fix, an effect stayed `Pending`/`RetryableFailed` until `drain_one`
  eventually reached `mark_in_flight` after being dequeued, so the SAME
  effect could be claimed and re-enqueued on every reclaim tick while the
  queue had backlog, inflating it with duplicate entries.
- **Shutdown no longer blocks on a saturated backpressure permit (F-03,
  `runner.rs`).** `run_inner`'s main loop previously received an effect from
  the queue, then did a bare `backpressure.acquire().await` *outside* the
  `tokio::select!` watching shutdown — a hung/slow executor holding every
  concurrency permit could prevent the loop from ever reaching
  `drain_tasks`'s abort-on-deadline logic at all. Fixed: permit acquisition
  now races `shutdown.changed()` in its own nested `select!`/loop; on
  shutdown, the already-dequeued effect is dropped un-attempted (safely
  `Pending`/`RetryableFailed`/`InFlight` in the store, recoverable by a
  future reclaim/`recover_in_flight`) and the loop proceeds straight to
  `drain_tasks`.
- **`reclaim_due`'s previously-silent `queue.send` failure now logs.** A
  reclaim-driven `send_reclaimed` failure emits `tracing::warn!`, matching
  this file's existing bookkeeping-failure logging conventions.

**PR2 round 5 review follow-up (2 new BLOCKERs).** A fourth review round
found the round 4 reclaim-loop redesign and dedup-identity redesign each had
one remaining gap, both fixed in the same PR:

- **AD-6 revision, again — the reclaim loop no longer feeds itself through
  `EffectQueue` at all (F-01, `queue.rs`/`runner.rs`).** Round 4's fix made
  `reclaim_due` claim-then-transition an effect before enqueueing it as
  `QueuedEffect::Reclaimed` via `send_reclaimed` — but `send_reclaimed`
  blocks until the bounded queue has capacity, and the ONLY consumer that
  would ever free that capacity (`EffectQueueReceiver::recv`) is the very
  same reclaim loop. With queue capacity smaller than one `claim_due` batch,
  this was a guaranteed self-deadlock: the first reclaimed effect fills the
  queue, the second blocks forever on `send_reclaimed`, and the loop can
  never get back to `recv()` to drain the first entry and free capacity —
  the runner hangs, shutdown is never even observed. Fixed by removing the
  queue hop entirely for this path: `reclaim_due` now claims, transitions,
  and dispatches each due effect directly, acquiring a concurrency permit
  through a new shared helper (`DeliveryRunner::acquire_permit_and_spawn`)
  that both the queue-fed (fresh) branch and the reclaim-fed branch of
  `run_inner`'s `select!` call — one dispatch mechanism, not two. Since
  nothing routes reclaimed effects through `EffectQueue` anymore,
  `EffectQueue::send_reclaimed` and the `QueuedEffect` enum (`Fresh`/
  `Reclaimed`) are removed outright; the queue now carries plain
  `AcceptedEffect`s, always freshly-accepted.
- **Dedup outcome splits `Duplicate` into `OtherInProgress`/`OtherSucceeded`
  (F-02, `store.rs`/`runner.rs`).** Round 4 correctly distinguished
  `OwnedInProgress`/`OwnedSucceeded` for the SAME `EffectId`, but still
  collapsed every DIFFERENT-owner case into one flat `Duplicate`, which the
  runner treated exactly like `OwnedSucceeded` — mark succeeded, never
  execute — regardless of whether that other owner had actually reached
  `Succeeded` yet. Concretely: effect A reserves a scope and starts
  executing; effect B arrives with the same scope/fingerprint, gets
  `Duplicate`, and is marked `Succeeded` WITHOUT EVER EXECUTING; if A later
  fails terminally and releases its reservation, the only recorded outcome
  for that idempotency key is B's false `Succeeded` — silent data loss, the
  same class of bug as F-02 (round 4) but for the "different submitter"
  case instead of "same effect recovering." Fixed: `DedupOutcome::Duplicate`
  is replaced by `OtherInProgress` (a different owner holds the reservation,
  not yet succeeded) and `OtherSucceeded` (a different owner's reservation is
  already recorded `Succeeded`). The runner's `dispatch_in_flight` now
  branches on the richer outcome: `OtherSucceeded`/`OwnedSucceeded` →
  short-circuit to success, same as before; `OtherInProgress` → must NOT
  execute or mark succeeded — reuses the same "leave it reclaim-eligible, no
  attempt charged, no dedup release" shape `requeue_without_charging_attempt`
  already gives shutdown-cancelled attempts, so the effect is simply
  re-evaluated on a future reclaim tick, by which point the other owner has
  likely resolved to `OtherSucceeded` (mark succeeded then) or released the
  reservation on terminal failure (this effect then sees `Fresh` and executes
  normally).

**Acceptance-failure policy (AD-9) in plain terms:** the whole CORE-019 effort
optimizes for honesty about what survives, so acceptance is honest too. A
command's event commits atomically and irrevocably; recording that command's
effects into the `EffectStateStore` happens *after* that commit and can hit a
store error. Only a genuinely transient store error is retried under the AD-5
`RetryPolicy` (same bounded-attempt + jittered-backoff shape, not a new policy
type). If the retryable error is exhausted, or the store error is permanent,
the caller's reply carries an explicit `EffectAcceptanceError`. This does
**not** mean the command failed or the event was rolled back — the event is
committed for good. It means exactly one thing: at least one described effect
could not be durably-enough registered and may be lost to the post-commit
dual-write gap (§7 Model B). The successful reply is only ever sent once
acceptance completes.

**AD-9 store-error classification (for `EffectStateStore::accept`).** Which
`EffectStoreError` variants the acceptor retries versus surfaces immediately is
pinned to the *already-shipped* `EffectStoreError` contract
(`crates/runtime/src/effects/store.rs`), not re-invented here:

| `EffectStoreError` variant | Acceptance classification |
|----------------------------|---------------------------|
| `TemporarilyUnavailable` | **Retryable** under the AD-5 `RetryPolicy` — the backend is reachable but momentarily unable to serve (pool exhausted, timeout, lock contention). The retry loop applies only here. |
| `Backend` | **Permanent** — a general/permanent backend failure (corruption, serialization failure, schema mismatch). NOT automatically retried; surfaces immediately as the post-commit `EffectAcceptanceError`. |
| `InvalidTransition` | **Permanent** — a logic error, not retried. |
| `NotFound` | **Permanent** — a logic error, not retried. |
| `Conflict` (from `accept` specifically) | **Permanent / invariant violation** — `Conflict` from `accept` is treated as a permanent invariant or data conflict (e.g. an `EffectId` collision or another persistence-level conflict — not necessarily the dedup-scope/fingerprint mismatch that `EffectDedupStore::reserve` reports), unless a concrete recoverable optimistic-concurrency case is specified. NOT retried automatically. (This classification is scoped to `accept`; the generic `Conflict` doc on `EffectStoreError` leaves retryability to the call site, and other call sites may classify it differently.) |

Only `TemporarilyUnavailable` drives the retry loop; every other variant is a
permanent acceptance failure that surfaces at once.

**Shutdown interaction (AD-9).** A bounded acceptance retry that is still in
flight during graceful shutdown/draining does not retry indefinitely. It
respects the same drain deadline as the rest of the lifecycle (§8): when the
deadline passes it stops retrying and times out into the same "acceptance
ultimately failed" (`EffectAcceptanceError`) path, rather than blocking
shutdown forever. Actor and lifecycle wiring are PR3/PR4/PR5 scope (still
unbuilt), so this is a contract-level decision now, not an implementation.

**REQUIRED constraint on the future command-result/reply type (PR3 actor
wiring, AD-9).** A post-commit `EffectAcceptanceError` means "your command
succeeded and its event is committed, but at least one described effect could
not be durably-enough registered" — it is *not* a command failure. If PR3
collapses this into a single generic `Result<CommandReply, Error>` where any
`Err` is indistinguishable from a real command failure, a caller will treat it
as command failure and likely retry, causing a **second commit / duplicate
command execution** of an already-committed command. Therefore the future
command-result/reply type PR3 introduces MUST expose an unambiguous distinction
between "command not committed" (a real failure, safe to retry) and "command
committed but effects not fully accepted" (commit is final, MUST NOT be retried
as a command) — conceptually something like a `CommittedButEffectsUnaccepted`
outcome versus a generic command error. PR3 MUST NOT collapse the two into one
generic `Err` variant. The exact enum/type shape is deliberately left to PR3's
design; this AD only fixes the constraint so it cannot be silently skipped.

**PR3 review follow-up (3 BLOCKERs + 3 non-blocking observations, `acceptor.rs`/
`actor.rs`).** A review of PR3's diff found the following, all fixed in the
same PR:

1. **F-01 (BLOCKER): the spawned `Deferred`-mode runner task had no
   ownership/`JoinHandle`** — `RuntimeEffectAcceptor::new` used to
   `tokio::spawn` the runner internally and discard the handle, so nothing
   could ever await the runner actually finishing, distinguish a clean finish
   from a panic, or block teardown until draining was truly done. Fixed:
   `EffectRuntimeHandle` (`acceptor.rs`) now owns both the `watch::Sender<bool>`
   shutdown signal AND the spawned task's `JoinHandle<()>`.
   `EffectRuntimeHandle::shutdown_and_wait(deadline)` signals shutdown, then
   awaits the task within `deadline`, returning `EffectRuntimeShutdownError`
   (`Timeout` / `RunnerPanicked` / `RunnerCancelled`) when it does not finish
   cleanly. `Inline` mode never spawns a task, so its handle's `runner_task` is
   `None` and `shutdown_and_wait` resolves immediately after signalling.
2. **F-02 (BLOCKER): acceptance retry didn't observe shutdown/deadline** — the
   bounded `TemporarilyUnavailable` retry loop (`accept_into_store`) used to
   `sleep(backoff)` with no cancellation awareness at all, violating AD-9's
   shutdown-interaction requirement. Fixed: both the `state.accept(...)` call
   itself and the backoff sleep are raced via `tokio::select!` against the
   SAME shutdown signal `EffectRuntimeHandle` shares with the spawned runner
   (`self.shutdown_rx`, a `watch::Receiver<bool>` field on
   `RuntimeEffectAcceptor`) — a retry mid-backoff resolves promptly to
   `EffectAcceptanceError::RetriesExhausted` instead of sleeping past the
   drain deadline. Implementation note: racing directly against
   `watch::Receiver::wait_for` inside `tokio::select!` made the whole
   `#[async_trait]` future `!Send` (its `Ref` guard isn't `Send`); a small
   `wait_for_shutdown` helper built from `borrow()` (never held across an
   `.await`) + `changed()` (no guard in its `Output`) avoids that without
   pulling in a new dependency.
3. **F-03 (BLOCKER): described effects were silently dropped when no acceptor
   was configured** — the actor's `if let Some(acceptor) = &self.effect_acceptor
   { .. }` used to fall through to a plain successful reply when the field was
   `None`, discarding any described effects with zero signal to the caller.
   Fixed: a missing acceptor with ≥1 described effect now maps to
   `EffectAcceptanceError::Permanent` and routes through the same
   `CommandResult::EffectsAcceptanceFailed` reply the `Some` branch's failure
   path already used — fail closed, never silently discarded. The commit
   itself is unaffected either way (AD-9: commit is always final).
4. **Observation: renamed `CommandResult::EventsAcceptanceFailed` →
   `CommandResult::EffectsAcceptanceFailed`** (`persistent_entity.rs`) — the
   old name read like "committing events failed," which is not what it means.
   Updated every match site in `persistent-entity` and the one exhaustive
   match in `examples/reference-app/tests/register_user_partial_failure.rs`.
5. **Observation: split `RuntimeEffectAcceptor::new`'s spawn out of
   construction.** `new` used to `tokio::spawn` the `Deferred` drain loop
   internally, which panics outside a Tokio runtime context. `new` now only
   constructs (stashing the `Deferred` queue receiver in a
   `std::sync::Mutex<Option<EffectQueueReceiver>>` it never touches itself);
   `start(&self) -> EffectRuntimeHandle` is the one place that calls
   `tokio::spawn`, taking the stashed receiver and returning the F-01 handle.
   `Inline` mode's `start()` is a no-op returning a handle whose
   `runner_task` is `None`.
6. **Observation: `EffectAcceptanceError` gained partial-acceptance context.**
   `RetriesExhausted`/`Permanent` are now struct variants carrying `message`,
   `failed_at_index`, and `failed_idempotency_key`. Since `EffectAcceptor::accept`
   processes a batch strictly sequentially and returns on the first failure,
   `failed_at_index` doubles as both "which effect in the batch failed" and
   "how many of the batch's effects were already durably accepted before it" —
   no separate counter field needed.

**PR3 round 2 review follow-up (2 more BLOCKERs, `acceptor.rs`).** A second
review of the same lifecycle/shutdown design found:

1. **F-01 (BLOCKER): `shutdown_and_wait`'s timeout branch dropped the
   `JoinHandle` instead of aborting it.** Dropping a `tokio::task::JoinHandle`
   only detaches from the underlying task — it does NOT cancel/abort it. The
   round-1 fix gave `EffectRuntimeHandle` real ownership of the `JoinHandle`
   specifically so shutdown could be awaited, but the timeout arm still threw
   that ownership away on the ground that time was up, silently leaving the
   runner task executing in the background forever, defeating the whole
   point of owning it. Fixed: `shutdown_and_wait` now takes `runner_task` by
   mutable reference across `tokio::time::timeout_at`, and on timeout calls
   `runner_task.abort()` then awaits it once more (draining the resulting
   cancellation) before returning `Timeout` — the caller's `Timeout` now
   means the runner has genuinely stopped, not merely that it was let go of.
2. **F-02 (BLOCKER): "shutdown has started" was conflated with "the drain
   deadline has elapsed."** The single shutdown `watch<bool>` used to flip to
   `true` at the very start of `shutdown_and_wait(deadline)`, and
   `accept_into_store`'s retry loop (both the `state.accept` call and the
   backoff sleep) raced against that SAME signal — cancelling any acceptance
   already in progress the instant shutdown merely *began*, no matter how
   generous the caller's `deadline` actually was. This violated AD-9's
   graceful-shutdown intent: acceptance work in flight should be allowed to
   continue naturally during the drain window and only be cancelled once the
   deadline is genuinely hit. Separately, `queue.send` (the `Deferred`/
   `Inline` enqueue step) raced against nothing at all — not even the old
   conflated signal — so it could never be cancelled during shutdown. Fixed:
   a second, distinct signal — `deadline_rx`/`deadline_tx`
   (`watch::Sender<Option<tokio::time::Instant>>` on `RuntimeEffectAcceptor`/
   `EffectRuntimeHandle`) — now carries the actual *deadline instant*: `None`
   while no shutdown is in progress, set to `Some(Instant::now() + deadline)`
   exactly once, when `shutdown_and_wait(deadline)` begins. A new
   `wait_for_deadline` helper resolves only once that instant is reached
   (`sleep_until`), never merely on the `None` → `Some` transition.
   `accept_into_store`'s retry loop/backoff sleep, and the new
   `send_to_queue` helper (factored out of `accept_one` so both `Deferred`
   and `Inline` mode's `queue.send` are now covered too), race against
   `deadline_rx`/`wait_for_deadline` instead of the plain shutdown bool. The
   original `shutdown_rx`/`shutdown_tx` `watch<bool>` is unchanged and keeps
   its own, genuinely separate job: telling the spawned `Deferred` runner's
   own drain loop to stop admitting new work immediately, which is safe to do
   right away and was never the part of the design F-02 was about.

**PR3 round 3 review follow-up (2 more BLOCKERs, lifecycle authority fragmented
across `EffectRuntimeHandle`/`RuntimeEffectAcceptor`/`DeliveryRunner`,
`acceptor.rs`/`runner.rs`).** A third review found the previous two rounds had
unified the *deadline* signal but not the *lifecycle authority* itself — two
more gaps where child-task/child-call ownership crossed a struct boundary
nothing actually drained:

1. **F-01 (BLOCKER): aborting the outer `runner_task` did not guarantee its
   own child tasks were gone.** `shutdown_and_wait`'s timeout path aborted the
   OUTER spawned `run()` task, but `DeliveryRunner` spawns its OWN per-effect
   dispatch tasks (tracked in its `tasks: Mutex<JoinSet<()>>`) and in-flight
   executor calls (tracked in `executor_aborts: Mutex<Vec<AbortHandle>>`) —
   both owned by the `DeliveryRunner` struct itself, not scoped to `run()`'s
   future. `run_inner`'s own existing drain-on-shutdown logic
   (`Self::drain_tasks`, PR2 rounds 3-4) is what actually aborts/drains those,
   but only runs if `run_inner`'s own loop reaches its end-of-loop step —
   which an externally-aborted outer task never does. Fixed: a new
   `DeliveryRunner::shutdown_and_drain(deadline_instant)` method — callable
   directly on `&self`/`Arc<Self>`, independent of `run()`'s own task —
   converts the instant to a remaining duration and calls the SAME
   `drain_tasks` `run_inner` already uses (one drain implementation, two entry
   points, per the constraint to avoid duplicating slightly-different drain
   logic). `EffectRuntimeHandle` now also holds the shared `Arc<DeliveryRunner>`
   so `shutdown_and_wait` can call `shutdown_and_drain` directly as the
   authoritative cleanup step, in addition to (not instead of) still
   awaiting/aborting the outer `runner_task` as a backstop.
2. **F-02 (BLOCKER): shutdown did not close acceptance admission, and
   `shutdown_and_wait` did not wait for in-flight `accept()` calls.** Two
   distinct gaps: (a) `accept()` never checked any lifecycle signal at entry —
   a brand-new call could start after shutdown had already begun, with
   nothing rejecting it outright; (b) `shutdown_and_wait` only ever awaited
   the runner task, with no way to know about or wait for an `accept()` call
   already running inside `accept_into_store`/`send_to_queue` — reproducible
   even in `Inline` mode, where there is no runner task at all to (accidentally)
   cover for this. Fixed: a new `LifecycleGate` (`acceptor.rs`), shared via
   `Arc` between `RuntimeEffectAcceptor` and `EffectRuntimeHandle`, holding
   `LifecycleState::{Running, Draining { deadline }, Closed}` plus an
   in-flight call counter (`AtomicU64`) and a `tokio::sync::Notify` fired when
   the counter reaches zero. `accept()` now calls `lifecycle.enter()` at
   entry, before minting anything: `Err` (already `Draining`/`Closed`) returns
   immediately with `EffectAcceptanceError::Permanent` — no store call, no
   enqueue — routed through the same post-commit-acceptance-failure shape
   F-03 (round 1) already established for a missing acceptor. `Ok` returns an
   RAII `InFlightGuard` held for the whole batch, so an early `?` return
   mid-loop can never leak the in-flight count. `shutdown_and_wait` now runs
   one coherent sequence: signal shutdown + publish the deadline instant (as
   before) → `lifecycle.begin_draining(deadline_instant)` (closes admission
   immediately) → `lifecycle.wait_until_drained()` (blocks until in-flight
   calls finish naturally or the SAME deadline instant elapses — the existing
   `deadline_rx`-raced retry/enqueue loops are exactly what makes those calls
   actually finish) → `runner.shutdown_and_drain` (F-01's fix) → await/abort
   the outer `runner_task` → `lifecycle.close()`. `enter()`/`begin_draining`
   share one `std::sync::Mutex<LifecycleState>`, so the admit-vs-reject
   decision can never race the transition to `Draining`.

Both fixes close the same class of gap: lifecycle authority previously lived
partly on `EffectRuntimeHandle` (the deadline/shutdown signals), partly on
`RuntimeEffectAcceptor` (admission), and partly on `DeliveryRunner` (child
tasks) — with nothing tying all three together into one sequence. After this
round, `EffectRuntimeHandle::shutdown_and_wait` is the single entry point that
drives all three in the same, deadline-bounded order.

**PR3 round 4 review follow-up (3 more BLOCKERs: shutdown/drain sequencing and
a lost-wakeup primitive, `acceptor.rs`/`runner.rs`).** A fourth review found
the previous round's unification still had an ordering bug, a genuine
concurrency gap, and a race-prone primitive underneath it:

1. **F-01 (BLOCKER): `shutdown_and_wait` told the runner to stop consuming
   BEFORE draining in-flight acceptances, not after.** Round 3 unified the
   sequence, but `shutdown.send(true)` still fired FIRST — before
   `lifecycle.begin_draining`/`wait_until_drained` even ran — so
   `DeliveryRunner::run_inner` could abandon its receive loop while an
   `accept()` call already mid-persistence/backoff was still in flight. Once
   that call finally reached `queue.send`, the runner had already stopped
   consuming: the effect was durably accepted and successfully enqueued
   (`queue.send` itself doesn't fail just because nothing is draining it) yet
   never actually dispatched — `accept()` returned `Ok` for an effect now
   silently stuck. Fixed: the sequence is reordered so admission closes and
   every already-admitted acceptance finishes enqueueing (or is cut short by
   the deadline) BEFORE `shutdown.send(true)` is ever sent — the runner is
   now guaranteed to still be consuming for as long as any admitted
   acceptance could still be enqueueing. RED test (`acceptor.rs`):
   `late_enqueue_after_shutdown_begins_is_still_consumed_by_the_deferred_runner`
   (a gated store call released well after `shutdown_and_wait` begins must
   still reach the registered executor, not fail with "queue closed").
2. **F-02 (BLOCKER): two independent callers could race for the same
   `tasks` `JoinSet` mutex during drain, with no bound on the lock
   acquisition itself.** `run_inner`'s own end-of-loop drain step and an
   externally-invoked `DeliveryRunner::shutdown_and_drain` call (from
   `shutdown_and_wait`) were two entirely separate callers of the same drain
   logic — whichever reached `self.tasks.lock().await` first held it for its
   own deadline (`run_inner`'s own internal deadline, independent of whatever
   a `shutdown_and_wait` caller actually asked for), and the other blocked on
   that SAME mutex with no bound of its own at all, so a short caller-supplied
   deadline could silently balloon to however long the other drain took.
   Fixed: `DeliveryRunner` gained single-flight leader-election coordination
   (`drain_claimed: StdMutex<bool>` + `drain_done_tx/rx: watch<bool>`) —
   whichever call reaches `coordinated_drain` first becomes the sole leader
   that actually locks `tasks` and performs the drain; every other (follower)
   call only waits for `drain_done` to fire, bounded by ITS OWN deadline,
   never the leader's. As defense-in-depth, the mutex acquisition itself is
   now also bounded (`tokio::time::timeout` around `self.tasks.lock()`) so an
   unanticipated stuck lock can't silently blow through a deadline either.
   Both `drain_tasks` (`run_inner`'s own entry point) and `shutdown_and_drain`
   (the external entry point) now delegate to this same coordination — one
   drain implementation, one leader per shutdown. RED test (`runner.rs`):
   `external_shutdown_and_drain_is_bounded_by_its_own_deadline_even_while_run_inners_own_internal_drain_is_in_progress`
   (drives both real entry points concurrently — `run_inner`'s own 3s
   internal drain against a hanging executor, and a directly-invoked 150ms
   external `shutdown_and_drain` — the external call must return near its own
   150ms bound, not block ~3s).
3. **F-03 (BLOCKER): `LifecycleGate`'s `Notify`-based drain signal had a
   lost-wakeup window.** `wait_until_drained` read `in_flight == 0`? no, then
   constructed/polled `self.drained.notified()`; `InFlightGuard::drop`
   decrementing to zero called `self.gate.drained.notify_waiters()` —
   but `notify_waiters()` only wakes waiters ALREADY registered at the exact
   moment it's called; unlike a state-carrying primitive, it stores nothing
   for a later `.notified()` call. A guard's drop landing in the narrow
   window between the read and the `.notified()` registration would lose the
   wakeup entirely, burning the whole deadline despite the count already
   being genuinely zero. Fixed: replaced the `AtomicU64` + `Notify` pair with
   a single `watch::Sender<u64>`/`Receiver<u64>` — `InFlightGuard::drop` and
   `enter()` use `send_modify` to decrement/increment; `wait_until_drained`
   loops on `*rx.borrow() == 0` / `rx.changed()`. `watch::Receiver` always
   reflects the latest sent value regardless of exactly when it's
   borrowed/polled relative to the send, so there is no equivalent
   lost-wakeup window. RED/GREEN evidence (`acceptor.rs`): the true
   integration-level race (a handful of CPU instructions with no intervening
   yield point) proved empirically unreproducible via realistic scheduling
   even across 25,000+ trials (plain `tokio::spawn`, a pre-spun
   busy-waiting `std::thread`, and single-`yield_now` alignment on a
   current-thread runtime all converged to zero reproductions — a
   cooperatively-scheduled task's synchronous check-then-register prefix
   always completes before any other task/thread gets a chance to run).
   `lost_wakeup_pattern_is_reproduced_with_a_widened_race_window` instead
   validates the exact `Notify`-vs-`watch` contract difference using an
   artificially widened window (a `tokio::time::sleep` inserted purely to
   make the race land reliably): the `Notify`-based shape loses the wakeup
   50/50 times under that widening, the `watch`-based shape (mirroring
   `LifecycleGate`'s fix) loses it 0/50 times.
   `wait_until_drained_does_not_lose_the_last_guards_wakeup_under_concurrent_drop`
   remains as a best-effort integration-level regression/stress check against
   the real, fixed `LifecycleGate`.

**PR3 round 5 review follow-up (1 more BLOCKER, plus a minor defensive
addition; `acceptor.rs`/`runner.rs`).** A fifth review found round 4's
leader/follower drain coordination was itself unsafe in the real shutdown
path:

1. **F-01 (BLOCKER): the drain follower could abort the drain leader
   mid-cleanup, since the leader ran inside the very task the follower
   aborted on timeout.** `run_inner` runs INSIDE the task `EffectRuntimeHandle`
   holds as `runner_task`. If `run_inner` observed shutdown and reached
   `coordinated_drain` first, it became the LEADER using its own (longer,
   production `SHUTDOWN_DRAIN_DEADLINE`) internal deadline — all executing
   inside `runner_task`. Meanwhile an external `shutdown_and_wait` call
   (with a caller-supplied, possibly much SHORTER deadline) became the
   FOLLOWER, bounded only by its own deadline. Once that shorter deadline
   elapsed without `drain_done` firing (because the leader's own, longer
   drain hadn't finished), `shutdown_and_wait` proceeded to
   `runner_task.abort()` on its now-elapsed timeout — but that outer task
   WAS the leader, still mid `drain_tasks_locked`, having not yet aborted
   the hung executor attempt it was cleaning up. The leader's cleanup was
   abandoned mid-flight, leaving the hung executor task (owned by
   `DeliveryRunner`'s own fields, not the aborted `runner_task`) running
   forever — the exact "background work survives shutdown" class of bug the
   whole `EffectRuntimeHandle` redesign was meant to eliminate. The round 4
   test only proved the follower itself returned within its own deadline; it
   never drove the real `EffectRuntimeHandle::shutdown_and_wait` path (with
   its own `runner_task.abort()` on timeout) at all, so it could not catch
   this.
   Fixed ("Option A", the maintainer's explicit recommendation): the
   leader/follower election is removed entirely. `run_inner` no longer
   drains anything of its own — on shutdown it only stops consuming new work
   and returns quickly. `DeliveryRunner::shutdown_and_drain`, called SOLELY
   by `EffectRuntimeHandle::shutdown_and_wait`, is now the ONE cleanup
   authority (aborting executor attempts, draining the tracked `JoinSet`).
   With exactly one caller, `drain_claimed`/`drain_done_tx`/`drain_done_rx`
   and `coordinated_drain` are deleted outright — there is nothing left to
   race. `shutdown_and_wait`'s sequence is reordered accordingly: publish
   deadline → `begin_draining` → `wait_until_drained` → `shutdown.send(true)`
   → await/abort the outer `runner_task` (now cheap, since it no longer
   drains) → `runner.shutdown_and_drain` as the sole cleanup step → close the
   lifecycle. The caller's deadline is honestly split: awaiting `runner_task`
   is bounded by at most half of whatever remains, so a slow-to-return
   `runner_task` can never silently consume the entire budget and leave
   nothing for the actual drain; the drain step still gets the full
   remainder of the ORIGINAL deadline. RED test (`acceptor.rs`, driving the
   REAL `shutdown_and_wait`/`shutdown_and_drain`/`run()` path end-to-end, not
   a synthetic harness):
   `shutdown_and_wait_stops_a_hung_executor_task_even_when_run_inner_would_have_raced_it_for_drain_leadership`.
2. **Minor defensive addition:** `drain_tasks_locked`'s branch where it can't
   acquire the `tasks` mutex before its deadline used to just log a warning
   and abandon cleanup entirely for whatever it couldn't reach. Now that
   external drain is the sole authority (removing the concurrent-caller
   scenario that made lock contention likely in the first place), this
   branch should be nearly unreachable in practice — but it now still aborts
   whatever `AbortHandle`s are tracked in `executor_aborts` via a
   non-blocking `try_lock` on that separate mutex (which doesn't require the
   contended `tasks` lock), instead of leaving them running.

**PR3 round 6 review follow-up (1 BLOCKER, plus 2 doc-only fixes;
`acceptor.rs`/`runner.rs`/`effect_acceptor.rs`).** The drain architecture
itself (single external drain authority, no leader/follower — round 5 above)
was confirmed correct; a sixth review found the final `Result`
`shutdown_and_wait` returns was still dishonest about it:

1. **F-01 (BLOCKER): `shutdown_and_wait`'s final `Result` came only from
   awaiting the outer `runner_task`, never from the drain step itself.**
   `DeliveryRunner::shutdown_and_drain` returned `()`, so its outcome was
   discarded entirely — `Ok(())` even when the deadline had already elapsed
   going into the drain, or cleanup had to force-abort a dispatch task/
   executor attempt. Two concrete cases: in `Inline` mode there is no
   `runner_task` at all, so the result was unconditionally `Ok(())` even if
   the inline-executing acceptance itself was blocked on a hung executor that
   `shutdown_and_drain` had to forcibly abort; in `Deferred` mode the receive
   loop can exit (and `runner_task` finish `Ok`) quickly while the subsequent
   child-task drain still exhausts the deadline and force-aborts executors.
   Fixed: `drain_tasks_locked`/`shutdown_and_drain` now return `bool` (`true`
   = drained naturally within the deadline; `false` = the deadline had
   already elapsed entering the drain, the `tasks` lock itself timed out, or
   any dispatch task/executor attempt had to be forced out) instead of `()`.
   `shutdown_and_wait` folds this into the final `Result`: `Ok(())` only when
   the runner task finished cleanly AND the drain reports a natural, on-time
   finish; `Err(EffectRuntimeShutdownError::Timeout)` otherwise (reusing the
   existing variant — a richer per-cause enum was judged unnecessary for this
   fix). A related gap closed in the same pass: `Inline` mode's `drain_one`
   runs synchronously on the caller's own task, never through
   `spawn_tracked`, so `tasks` stays empty and the existing timeout-based
   "did we have to force anything" check could never fire on its own for a
   hung Inline executor. `drain_tasks_locked` now also checks whether
   `deadline_instant` had already elapsed on entry, independent of whether
   `tasks` itself timed out, and routes through the same forced-abort branch
   either way — so a hung Inline-mode executor (tracked only in
   `executor_aborts`) is both actually aborted and correctly reported as a
   non-clean drain. RED tests (`acceptor.rs`):
   `shutdown_and_wait_returns_timeout_when_an_inline_executor_hangs_past_the_deadline`
   (Inline mode) and
   `shutdown_and_wait_returns_timeout_when_the_runner_task_exits_cleanly_but_a_child_executor_hangs`
   (Deferred mode, outer runner task finishes `Ok` while a child dispatch task
   is force-aborted during the subsequent drain).
2. **Doc-only fix:** confirmed no code comment in `runner.rs`/`acceptor.rs`
   describes leader/follower drain coordination as the CURRENT architecture —
   every remaining mention (including this document's own round 4/5 entries
   above) already correctly frames it as removed history. No change needed
   there beyond this note.
3. **Doc-only fix:** `EffectAcceptor::accept`'s doc comment (here and in
   `crates/persistent-entity/src/effect_acceptor.rs`) said "never refused
   outright at intake", which read as contradicting the admission-gating
   added in round 3 (a NEW `accept()` call arriving after shutdown/draining
   has begun IS rejected immediately). Reworded to distinguish the two cases
   explicitly: a NORMAL in-progress effect is never refused mid-flight once
   admitted, but a NEW call arriving after draining has begun is rejected
   immediately at intake — see §4's updated contract below.

## 4. Public Contracts (Rust)

```rust
// crates/persistent-entity/src/effect_acceptor.rs
#[async_trait]
pub trait EffectAcceptor: Send + Sync {
    /// Post-commit acceptance. Mints the effect id, attaches `tenant`, and
    /// records the effect as `Pending` in the configured `EffectStateStore`
    /// (via `EffectStateStore::accept(AcceptedEffect { .. })`) before awaiting
    /// queue capacity (backpressure, §9) and enqueuing it.
    ///
    /// A NORMAL in-progress effect, once admitted, is never refused outright
    /// mid-flight (there is no "your effect list is rejected" path once
    /// `accept` has actually started processing it), but MAY still ultimately
    /// fail: a transient store error is retried under a bounded policy and, if
    /// that policy is exhausted (or the store error is non-retryable), returns
    /// `Err(EffectAcceptanceError)`. That error NEVER implies the
    /// already-committed event was rolled back — commit is final; it means
    /// the effect could not be durably-enough registered and may be lost to
    /// the post-commit dual-write gap (AD-9). Distinct case: a NEW `accept()`
    /// call arriving after shutdown/draining has already begun IS rejected
    /// immediately at intake, before touching the store at all (`LifecycleGate`).
    async fn accept(
        &self,
        tenant: &TenantId,
        effects: Vec<ExternalEffectDescription>,
    ) -> Result<(), EffectAcceptanceError>;
}

/// Layer-neutral acceptance-failure classification (AD-9). Lives beside the
/// port in `persistent-entity`, so it does NOT reference runtime's
/// `EffectStoreError`; the `RuntimeEffectAcceptor` impl maps the underlying
/// `EffectStoreError` into these variants after the bounded retry.
/// (PR3 review, observation 6: both variants carry partial-acceptance
/// context — `failed_at_index` doubles as both "which effect in this
/// `accept` batch failed" and "how many were already durably accepted
/// before it," since `accept` is strictly sequential and stops at the first
/// failure.)
pub enum EffectAcceptanceError {
    /// The one retryable store error (`TemporarilyUnavailable`) survived the
    /// bounded acceptance retry policy — or a shutdown deadline interrupted
    /// an in-progress retry (AD-9's shutdown interaction). Commit is final;
    /// the effect may be lost to the post-commit dual-write gap.
    RetriesExhausted { message: String, failed_at_index: usize, failed_idempotency_key: IdempotencyKey },
    /// A permanent store failure — `Backend`, `InvalidTransition`, `NotFound`,
    /// or a `Conflict` from `accept` (a permanent invariant or data conflict,
    /// e.g. an `EffectId` collision — not necessarily a dedup-scope/fingerprint
    /// mismatch) — surfaced without retry. Same commit-is-final, no-rollback
    /// semantics.
    Permanent { message: String, failed_at_index: usize, failed_idempotency_key: IdempotencyKey },
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
         │   EffectStateStore::accept(AcceptedEffect{..})  ── record Pending in
         │                                                     the configured store
         │   send().await  ◄── BACKPRESSURE (delays REPLY, never commit)
         ▼
    [ bounded mpsc EffectQueue ]  ──recv().await──►  DeliveryRunner
         ▲                                               │ reserve(scope) single-flight
         │ re-enqueue after backoff (tokio::time)        │ mark_in_flight
         │                                               ▼
         └──── RetryableFailure ◄── ExternalEffectExecutor.execute(effect, ctx)
                                          │ Success ─► commit_success + mark_succeeded
                                          │ Terminal/Missing/Invalid ─► mark_terminal + signal

For slice 1 the acceptor records the effect in whatever `EffectStateStore` is
configured **at acceptance time, before the queue ever sees it**. With the
in-memory store this ordering offers no crash protection (the store is lost on
crash, §8), but the accept-then-enqueue sequencing is real and matters for a
future durable store, whose `claim_due`/`recover_in_flight` — not queue replay
— become the source of truth for pending work.

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
   `DuplicateEffectType::AlreadyRegistered` surfaced at `.build()` (fail-closed, §11).
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
