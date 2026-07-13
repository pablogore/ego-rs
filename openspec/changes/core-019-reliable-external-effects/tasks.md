# Tasks: CORE-019 — Reliable External Effects

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~900-1300 (9 new runtime files, 1 new persistent-entity file, 3 modified files, testkit + reference-app wiring, full test suite) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 → PR2 → PR3 → PR4 → PR5 (see Work Units) |
| Delivery strategy | ask-on-risk |
| Chain strategy | stacked-to-main |

Decision needed before apply: Resolved — stacked-to-main
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Store/dedup ports + in-memory composite, executor trait, registry (Phases 1-2) | PR1 | `cargo test -p ego-runtime effects::store:: effects::executor:: effects::registry::` | Unit-only, no wiring | Delete `effects/{mod,store,executor,registry}.rs` — no other file touched |
| 2 | Retry policy, internal queue, delivery runner, immediate-policy config (Phases 3-4,6-7) | PR2 | `cargo test -p ego-runtime effects::runner:: effects::policy:: effects::queue::` | In-memory store + recording executor integration test | Delete `effects/{policy,queue,runner}.rs`; PR1 untouched |
| 3 | Acceptor port/impl, handler API, actor post-persist wiring (Phases 5,8) | PR3 | `cargo test -p ego-persistent-entity actor:: effect_acceptor::` | Actor test harness, post-persist accept scenario | Revert `effect_acceptor.rs`, default method, actor field/call — additive |
| 4 | Lifecycle wiring, observability, tenant isolation (Phases 9-11) | PR4 | `cargo test -p ego-service-sdk runtime::builder::` | Runtime builder startup/shutdown/drain scenario | Revert `builder.rs` additions; nothing downstream depends yet |
| 5 | Test doubles, E2E dogfood, docs (Phase 12) | PR5 | `cargo test --workspace effects` | reference-app describe→deliver→retry→dedup + kill-process crash-loss test | Revert testkit executor + reference-app wiring; PR1-4 unaffected |

## Phase 1: Store & Dedup Ports (`crates/runtime/src/effects/`)

- [x] 1.1 RED: state-transition + dedup Fresh/Duplicate/Conflict tests (store.rs)
- [x] 1.2 GREEN: `EffectState`, `EffectEnvelope`, `EffectStoreError`, `EffectStateStore`, `EffectDedupStore` traits + `InMemoryEffectStore` composite (store.rs)
- [x] 1.3 Create `effects/mod.rs` re-exports

## Phase 2: Executor + Registry

- [x] 2.1 RED: duplicate-registration-fails + one-executor-multi-type tests
- [x] 2.2 GREEN: `ExternalEffectExecutor`, `AttemptOutcome`, `EffectContext` (executor.rs); `ExecutorRegistry`, `DuplicateEffectType` error (registry.rs)

> **Post-review fixes (PR1, before merge)** — maintainer review of PR1 found 3
> architectural findings against Phases 1-2, all fixed in the same PR before
> merge:
> - **F-01 (BLOCKER)**: `EffectStateStore::accept` took the crate-private
>   `EffectEnvelope`, making the trait not actually implementable outside
>   `ego-runtime`. Fixed by introducing the public `AcceptedEffect` DTO;
>   `accept` now takes `AcceptedEffect`, and `EffectEnvelope` stays
>   crate-private, no longer part of any public trait signature.
> - **F-02 (BLOCKER)**: no crash-recovery path, and the in-memory store
>   discarded `tenant`/`description` (unreconstructable), and
>   `mark_retryable`'s `next_at: Instant` was not persistable. Fixed by adding
>   `EffectStateStore::claim_due`/`recover_in_flight`, a new `StoredEffect`
>   DTO, a persistable `Timestamp` newtype (wraps `chrono::DateTime<Utc>`,
>   aligned with `ego_domain::Clock`'s convention) replacing `Instant`, and
>   `InMemoryEffectStore` now retains `tenant`/`description` per effect.
> - **F-03 (HIGH)**: `EffectStoreError` only had `NotFound`/`InvalidTransition`,
>   with no way to express transient vs permanent backend failures for AD-7.
>   Fixed by adding `Conflict`, `TemporarilyUnavailable`, and `Backend`
>   variants.
>
> `design.md` §4 updated to match (trait sketch, `Timestamp`, expanded error
> enum, and the now-corrected "Consequence" paragraph). See
> `crates/runtime/src/effects/store.rs` and
> `crates/runtime/tests/effect_store_public_api.rs`.

## Phase 3: Retry Policy

- [x] 3.1 RED: backoff+jitter math, attempt cap (AD-5), per-`effect_type` override tests
- [x] 3.2 GREEN: `RetryPolicy`, `DeliveryConfig`, `DeliveryConfig::immediate()` (policy.rs)

> **Timestamp conversion (F-02)**: backoff math produces a `Duration`, but
> `EffectStateStore::mark_retryable` takes `next_at: Timestamp`, which currently
> exposes no arithmetic helper. Whoever implements this phase should decide then
> whether `Timestamp` needs a `checked_add(Duration)`-style helper or whether
> call sites do the conversion inline — flagged so it is not silently
> rediscovered in Phase 6.
>
> **Resolved (Phase 6)**: call sites do the conversion inline — a private
> `timestamp_after(Duration) -> Timestamp` helper lives in `runner.rs` (the
> only caller), not on `Timestamp` itself, to avoid touching the already-shipped
> `store.rs` (PR1).
>
> **PR2 round 2 fix — per-`effect_type` retry policy override actually wired
> to the runner.** A single shared `RetryPolicy` per runner instance was the
> only option; nothing consulted a per-type override even though this note's
> own RED test title mentions one. Fixed: `RetryPolicies { default_retry,
> retry_overrides: HashMap<String, RetryPolicy> }` (policy.rs) with
> `policy_for(effect_type) -> RetryPolicy`; `DeliveryRunner` now calls it
> wherever it used to read a single `retry` field. See design.md's "PR2
> round 2 review follow-up" note.

## Phase 4: Internal Queue (not public)

- [x] 4.1 RED: bounded-queue backpressure test (blocks at capacity, never drops)
- [x] 4.2 GREEN: internal `EffectQueue` mpsc wrapper (queue.rs)

## Phase 5: Acceptor Port

- [x] 5.1 RED: `accept()` mints id + attaches tenant before store interaction; awaits capacity; never refused at intake, but returns `Err(EffectAcceptanceError)` when the one retryable `EffectStateStore::accept` error (`TemporarilyUnavailable`) survives the bounded retry policy, or immediately when the store error is permanent, and never rolls back the committed event (AD-9)
- [x] 5.2 GREEN: `EffectAcceptor` trait returning `Result<(), EffectAcceptanceError>` + the `EffectAcceptanceError` type (`crates/persistent-entity/src/effect_acceptor.rs`)
- [x] 5.3 GREEN: `RuntimeEffectAcceptor` impl (runtime/effects/acceptor.rs) — bounded retry via the AD-5 `RetryPolicy` of retryable store errors only (`TemporarilyUnavailable`); every other `EffectStoreError` variant (`Backend`, `InvalidTransition`, `NotFound`, and `Conflict` from `accept`) is permanent and maps immediately to `EffectAcceptanceError` without retry (AD-9 classification table)

> **Implementation notes (this pass)**: `RuntimeEffectAcceptor::new` returns
> `(Self, watch::Sender<bool>)` — the shutdown handle for the `Deferred`
> profile's spawned `DeliveryRunner::run` loop. `Deferred` mode spawns that
> loop internally (concurrency=1 placeholder) so PR3 never needs to leak the
> crate-private `EffectQueueReceiver` across a `pub fn` boundary; Phase 9
> (PR4, builder wiring) is expected to make concurrency configurable and wire
> the shutdown sender into `register_async_teardown`'s drain, and to decide
> *whether* to construct the acceptor at all (zero cost when no executor is
> registered). `Inline` mode holds its own receiver behind a `tokio::sync::Mutex`
> and drives `drain_one` synchronously inside `accept`, per design.md §7 — a
> concurrent second `accept()` call blocks on that mutex rather than refusing,
> which is this phase's proof of "awaits capacity, never refuses" for the
> `Inline` profile (queue_capacity is always 1 there).
> Added `persistent-entity` as an `ego-runtime` dependency (design.md §2's
> `runtime → persistent-entity` direction; no cycle — `persistent-entity`
> depends only on `ego-domain`).

## Phase 6: Delivery Runner

- [x] 6.1 RED: happy-path success; `RetryableFailure` re-enqueue+backoff; `ExecutorMissing` terminal+signal; dedup `Conflict`→`InvalidEffect` terminal; executor panic = one retryable attempt; AD-7 bookkeeping-failure stays in-flight and re-dispatches
- [x] 6.2 GREEN: `DeliveryRunner` drain loop, semaphore, watch-shutdown, backoff re-enqueue, AD-7 bounded-retry bookkeeping (runner.rs)

> **Single-consumer invariant (design.md AD-8)**: this slice instantiates
> exactly **one** `DeliveryRunner`; `claim_due` is deliberately non-atomic and
> is safe only because a single runner consumes it. The runner must be a
> singleton — do not spawn more than one instance.
> **Timestamp conversion (F-02)**: the backoff `Duration` computed here must be
> turned into a `Timestamp` for `mark_retryable`'s `next_at` (see the Phase 3
> note); decide between a `Timestamp` helper and inline conversion when
> implementing.
>
> **Implementation notes (this pass)**:
> - `mark_terminal` only accepts `from: InFlight | RetryableFailed` (already-shipped
>   `store.rs`). So `drain_one` calls `mark_in_flight` **before** the dedup
>   `reserve` check (one step earlier than design.md §5's informal sketch) so
>   every short-circuit path (`Duplicate`, `Conflict`, `ExecutorMissing`,
>   dedup-store error) can still reach a valid terminal transition.
> - AD-7's "re-dispatch" is implemented literally as "bounded-retry the
>   idempotent write" (`commit_success` + `mark_succeeded`, `BOOKKEEPING_RETRY_ATTEMPTS
>   = 3`); if still failing after that bound, the effect is left `InFlight`
>   (never marked `Succeeded`/`TerminalFailed`) and relies on the existing
>   `recover_in_flight`/`claim_due` machinery (Phase 1) for eventual
>   re-delivery, rather than inventing a second synchronous re-dispatch path
>   not required by this phase's RED tests.
> - AD-8 is documented on `DeliveryRunner` as a doc comment and proven honest
>   by test `two_runners_can_share_the_same_store_the_type_system_does_not_prevent_it`
>   (constructs two runners against one shared store and asserts
>   `Arc::strong_count`), not enforced by any type.
> - `EffectQueue`/`DeliveryRunner`/`policy` are still `pub(crate)`/internal
>   only, so `cargo build`/`cargo test -p ego-runtime` reports several
>   "never constructed/used" warnings until PR3/PR4 wire the acceptor and
>   builder around them — expected, matches the same pattern already
>   accepted for `queue.rs` in this same delivery slice.
>
> **Post-review fixes (PR2, before the next PR built on top)** — a code
> review of this PR's diff found 9 findings against `policy.rs`/`queue.rs`/
> `runner.rs`, all fixed in the same PR:
> 1. **`retry_or_give_up`'s `mark_retryable` failure used to abandon the
>    effect** (returning before scheduling redispatch), permanently stranding
>    it `InFlight` with its dedup reservation leaked. Fixed: redispatch is now
>    scheduled unconditionally on the in-memory `effect` value; the
>    bookkeeping write's failure is only logged (`tracing::warn!`).
> 3. **Dedup was released right after `mark_retryable`, before the backoff
>    sleep**, opening a duplicate-delivery window. Fixed: `dedup.release` now
>    happens inside the same spawned redispatch task, immediately before
>    `queue.send`, via a shared `schedule_redispatch` helper.
> 2. **`finish_success`'s exhausted-bookkeeping path only left the effect
>    `InFlight`** with nothing further scheduled, not the "re-dispatched" AD-7
>    promises. Fixed: it now calls the same `schedule_redispatch` helper.
> 4. **Nothing ever re-fed a `Pending` effect whose `mark_in_flight` write
>    failed.** Fixed: `mark_in_flight` gets a bounded, AD-9-classified retry,
>    and `DeliveryRunner::run` gained a periodic `claim_due`-driven reclaim
>    tick (same single-consumer task, third `tokio::select!` branch, 5s
>    default interval) with its own RED tests (reclaims a due `Pending`
>    effect; ignores a not-yet-due one; stops on shutdown).
> 6. **A `dedup.reserve` store error was unconditionally terminal**, including
>    the retryable `TemporarilyUnavailable` case. Fixed: classified the same
>    way AD-9 classifies `accept`'s errors, bounded-retried under the
>    existing `RetryPolicy`.
> 7. **A hand-rolled `Semaphore` duplicated `read_side::backpressure`'s
>    `Backpressure` type.** Fixed: `run`'s concurrency limiter now reuses it.
> 5. **Shutdown didn't wait for detached spawned tasks** (main dispatch or
>    backoff-redispatch). Fixed: both are tracked in a shared
>    `tokio::task::JoinSet`; shutdown stops accepting new work, then awaits
>    the `JoinSet` bounded by a local shutdown-drain deadline (5s default).
> 8. **~6 duplicated `mark_terminal`(+`dedup.release`) call sites.** Fixed:
>    extracted `abandon`/`abandon_and_release` helpers.
> 9. **The full `ExternalEffectDescription` (payload included) was deep-cloned
>    per attempt** just to satisfy `tokio::spawn`'s `'static` bound. Fixed:
>    `AcceptedEffect`/`StoredEffect` (`store.rs`) now wrap `description` in
>    `Arc`, so retries clone a pointer.
>
> See `design.md`'s "PR2 review follow-up" note (AD-6/AD-7/AD-8) for the
> coherent redesign rationale behind fixes 1/2/3/4/5 together.
>
> **PR2 round 2 review follow-up.** A second review pass on this PR's diff
> found the timer added by fix 1/2/3 above raced the reclaim loop added by
> fix 4 for the same effect, and was itself deadlock-prone against fix 5's
> shutdown-drain — both symptoms of one root cause: two competing redispatch
> producers. Fixed by removing the timer entirely: `mark_retryable(next_at)`
> is now the sole source of truth for "when is this effect due", and the
> reclaim loop is the sole way it re-enters the queue. Also fixed in the same
> pass: the dedup reservation's lifetime is now decoupled from "attempt" (held
> for the whole effect lifetime, not released/re-reserved per attempt);
> `finish_success`'s bookkeeping-exhausted path now transitions out of
> `InFlight` via `mark_retryable` instead of leaving the effect permanently
> unreachable; the dedup fingerprint is now a stable `EffectFingerprint`
> (SHA-256) instead of an unstable `DefaultHasher` `u64`; a `DedupOutcome::
> Duplicate` on a fresh submission is a benign `Succeeded`, not a
> `TerminalFailed` error; a cancelled/aborted executor task no longer charges
> a retry attempt; previously-silent bookkeeping-failure discards now log;
> and `timestamp_after`'s duration-conversion fallback saturates instead of
> degrading to zero. Full rationale in design.md's "PR2 round 2 review
> follow-up" note.
>
> **PR2 round 4 review follow-up.** F-02 and F-04 shared one root cause:
> `effect.attempt == 0` was overloaded as both "attempts charged against the
> retry cap" and "has dedup already been reserved" — fixed together as one
> dedup-identity redesign, not two patches. `DedupOutcome` (`store.rs`) grows
> `OwnedInProgress`/`OwnedSucceeded` (reservation now records the owning
> `EffectId` and a `succeeded` flag, not just a fingerprint), so `drain_one`'s
> `effect.attempt == 0` gate is gone — dedup is checked unconditionally every
> attempt, fixing a silent-data-loss BLOCKER where a crash mid the first
> attempt got falsely marked `Succeeded` without ever re-executing (F-02).
> With that gate gone, `requeue_without_charging_attempt` no longer needs to
> bump `attempt` to skip re-reservation, so a shutdown cancellation no longer
> silently eats into the real retry budget (F-04). Separately: `reclaim_due`
> now calls `mark_in_flight` immediately after `claim_due`, before ever
> enqueueing — via a new `QueuedEffect::{Fresh,Reclaimed}` distinction
> (`queue.rs`) so `drain_one`/`drain_reclaimed` don't double-transition —
> fixing `claim_due`'s same-effect double-enqueue race across reclaim ticks
> (F-01); and the main loop's backpressure-permit wait now races
> `shutdown.changed()` too, so a hung executor holding every concurrency
> permit can no longer block shutdown from ever reaching the drain-deadline
> abort logic (F-03). Full rationale in design.md's "PR2 round 4 review
> follow-up" note.
>
> **PR2 round 5 review follow-up.** Two more BLOCKERs, both in the round 4
> fix itself. First, `reclaim_due`'s `QueuedEffect::Reclaimed` +
> `send_reclaimed` (added in round 4) could self-deadlock: `send_reclaimed`
> blocks until `EffectQueue` has capacity, but the only consumer that would
> ever free capacity is this exact reclaim loop — with queue capacity
> smaller than one `claim_due` batch, the loop could get stuck awaiting its
> own queue's capacity forever (F-01). Fixed by removing the queue hop for
> this path entirely: `reclaim_due` now dispatches each claimed, transitioned
> effect directly through a new shared `acquire_permit_and_spawn` helper (the
> same concurrency-permit-gated mechanism the queue-fed path uses) —
> `EffectQueue::send_reclaimed`/`QueuedEffect` are removed. Second,
> `DedupOutcome::Duplicate` still collapsed every DIFFERENT-owner case into
> one flat outcome, treated exactly like `OwnedSucceeded` regardless of
> whether that other owner had actually succeeded yet — a genuine duplicate
> could be marked `Succeeded` while its real owner was still mid-delivery,
> the same silent-data-loss class as F-02 (round 4) for the "different
> submitter" case (F-02, round 5). Fixed: `Duplicate` is split into
> `OtherInProgress` (must not execute or mark succeeded; left reclaim-eligible
> for a later re-check) and `OtherSucceeded` (safe to short-circuit, same as
> `OwnedSucceeded`). Full rationale in design.md's "PR2 round 5 review
> follow-up" note.

## Phase 7: ImmediateDeliveryPolicy

- [x] 7.1 RED: Inline mode still traverses full pipeline (no bypass); failed attempt signaled, not retried
- [x] 7.2 GREEN: `runner_mode: Inline` drain-one-on-accept wiring (runner.rs / acceptor.rs)

> **Scope note**: the `runner.rs` half of this wiring is `DeliveryRunner::drain_one`
> itself — the one shared entry point both `Deferred`'s spawned `run()` loop
> and an `Inline` caller invoke identically (test
> `immediate_delivery_config_runs_the_same_pipeline_and_signals_failure_without_retry`
> drives `DeliveryConfig::immediate()`'s policy straight through `drain_one`
> and asserts exactly one attempt, then a terminal signal, never a retry).
> The `acceptor.rs` half (the code that actually calls `queue.send` +
> `drain_one`/spawns `run()` based on `config.runner_mode`) is Phase 5/PR3
> scope and intentionally not built here.

## Phase 8: Handler API + Actor Wiring

- [x] 8.1 RED: unmodified handler compiles/passes unchanged (default `external_effects`)
- [x] 8.2 GREEN: default `external_effects(cmd, new_state, events, ctx) -> Vec<ExternalEffectDescription>` (persistent_entity.rs)
- [x] 8.3 RED: actor calls `external_effects`+`accept` after commit, before reply; backpressure delays reply not commit
- [x] 8.4 GREEN: optional `effect_acceptor` field + post-persist call after `publisher.publish` (actor.rs:294)

> **Implementation notes (this pass)**: `CommandResult<E, S>` gained a new
> variant, `EventsAcceptanceFailed { new_state, events, error: EffectAcceptanceError }`,
> to satisfy AD-9's REQUIRED constraint — the concrete shape chosen for
> distinguishing "committed but effects unaccepted" from a real command
> failure. The actor's reply stays `Ok(CommandErasedResult)` in both the
> ordinary-success and acceptance-failure cases; only the boxed
> `CommandResult` variant differs. This avoids overloading `EntityError`
> (which callers already read as "not committed, safe to retry") with a
> non-failure, post-commit outcome. `execute_command` calls
> `external_effects` unconditionally (cheap, defaults to `Vec::new()`) and
> only touches `self.effect_acceptor` when the returned `Vec` is non-empty —
> zero acceptor invocation when a handler describes no effects. Constructed
> `TenantId` from `entity_id.tenant_id` (a plain `String`) inline at the call
> site; a conversion failure (never expected in practice since the tenant is
> already established) maps to `EffectAcceptanceError::Permanent` rather than
> panicking or silently dropping the effects. Both pre-existing
> `EntityActor { .. }` struct-literal construction sites (the production spawn
> path in `entity_ref_tokio.rs` and the actor.rs panic test) were updated with
> `effect_acceptor: None` — Phase 9 (PR4) is expected to thread a real
> acceptor through the builder once ≥1 effect executor is registered.

> **Acceptance-failure propagation (AD-9)**: `accept` returns
> `Result<(), EffectAcceptanceError>`; the actor MUST propagate that error
> through to the command's reply path in place of a success reply. It is a
> post-commit error that does NOT mean the command failed or the event was
> rolled back — commit is final. Docs-only note; this phase is not yet
> implemented.
>
> **REQUIRED — unambiguous post-commit reply variant (AD-9)**: the
> command-result/reply type this phase introduces MUST expose an unambiguous
> distinction between "command not committed" (a real failure, safe to retry)
> and "command committed but effects not fully accepted" (commit is final, MUST
> NOT be retried as a command) — conceptually something like a
> `CommittedButEffectsUnaccepted` outcome versus a generic command error. It
> MUST NOT collapse a post-commit `EffectAcceptanceError` into a single generic
> `Err` variant indistinguishable from a real command failure, because a caller
> would then treat it as failure and retry, causing a second commit / duplicate
> command execution of an already-committed command. The exact enum/type shape
> is left to this phase's design; this note only fixes the constraint so it
> cannot be silently skipped.

> **PR3 review fixes (3 BLOCKERs + 3 non-blocking observations)** — all fixed
> in the same PR, RED-then-GREEN for each blocker:
> 1. **F-01 (BLOCKER)**: the `Deferred`-mode spawned runner task had no
>    `JoinHandle` — shutdown could be signalled but never actually awaited.
>    Fixed: `EffectRuntimeHandle` (`acceptor.rs`) owns both the
>    `watch::Sender<bool>` and the `JoinHandle<()>`;
>    `shutdown_and_wait(deadline)` signals then awaits within the deadline.
>    RED test: `shutdown_and_wait_awaits_the_runner_task_to_actually_finish_its_work`
>    (a gated executor proves the returned future resolves only after the
>    runner's real work finishes, not merely after the signal is sent).
> 2. **F-02 (BLOCKER)**: the acceptance retry loop (`accept_into_store`) had no
>    shutdown/deadline awareness — a mid-backoff sleep ignored shutdown
>    entirely. Fixed: both the store `accept` call and the backoff sleep are
>    raced via `tokio::select!` against the same shutdown signal
>    `RuntimeEffectAcceptor` shares with its `EffectRuntimeHandle`. RED test:
>    `acceptance_retry_mid_backoff_resolves_promptly_when_shutdown_fires` (a
>    30s backoff resolves in well under 1s once shutdown fires).
> 3. **F-03 (BLOCKER)**: described effects were silently discarded when
>    `effect_acceptor` was `None` — the actor used to fall through to a plain
>    successful reply. Fixed: a missing acceptor with ≥1 described effect now
>    fails closed via `EffectAcceptanceError::Permanent` through the same
>    `CommandResult::EffectsAcceptanceFailed` reply path. RED test:
>    `missing_acceptor_with_described_effects_fails_closed_not_silently_discarded`.
> 4. Renamed `CommandResult::EventsAcceptanceFailed` →
>    `CommandResult::EffectsAcceptanceFailed` (less ambiguous — it never meant
>    "committing events failed"). Updated every match site, including the one
>    exhaustive match in `examples/reference-app`.
> 5. Split `RuntimeEffectAcceptor::new` (construct only, panics-outside-runtime
>    -safe) from `start(&self) -> EffectRuntimeHandle` (the one place that
>    calls `tokio::spawn` for `Deferred` mode). RED test:
>    `new_does_not_require_a_tokio_runtime_context` (runs on a plain OS thread,
>    no `#[tokio::test]`).
> 6. `EffectAcceptanceError::{RetriesExhausted, Permanent}` became struct
>    variants carrying `message`, `failed_at_index`, and
>    `failed_idempotency_key` — partial-acceptance context for a multi-effect
>    batch. `failed_at_index` doubles as the count of effects already durably
>    accepted before the failure, since `accept` is strictly sequential.

> **PR3 round 2 review fixes (2 more BLOCKERs in the same lifecycle/shutdown
> design)** — both fixed in the same PR, RED-then-GREEN each:
> 1. **F-01 (BLOCKER)**: `shutdown_and_wait`'s timeout branch dropped the
>    `JoinHandle` on the ground that a timeout had occurred — but dropping a
>    `JoinHandle` only detaches from the task, it does NOT abort it, leaving
>    the runner running forever in the background even after `Timeout` was
>    returned. Fixed: the timeout branch now `abort()`s the still-owned
>    `runner_task` and awaits it once more to drain the cancellation before
>    returning `Timeout`. RED test:
>    `shutdown_and_wait_aborts_the_runner_task_on_timeout_instead_of_merely_detaching`
>    (a manually constructed `EffectRuntimeHandle` wraps a task that
>    increments a shared counter forever; the counter must stop the moment
>    `shutdown_and_wait` returns `Timeout`).
> 2. **F-02 (BLOCKER)**: "shutdown has started" was conflated with "the drain
>    deadline has elapsed" — the single shutdown `watch<bool>` flipped `true`
>    the instant `shutdown_and_wait` began, and `accept_into_store`'s retries
>    raced against that SAME signal, cancelling in-flight acceptance the
>    moment shutdown merely *started* regardless of how generous the caller's
>    deadline was. Fixed: a second signal, `deadline_rx`/`deadline_tx`
>    (`watch::Sender<Option<tokio::time::Instant>>`), now carries the actual
>    deadline *instant* — `None` while no shutdown is in progress, set to
>    `Some(Instant::now() + deadline)` exactly once when `shutdown_and_wait`
>    begins. `accept_into_store`'s retry loop/backoff sleep and the new
>    `send_to_queue` helper (which now also guards `queue.send`, previously
>    uncancellable) all race against `deadline_rx` via a new
>    `wait_for_deadline` helper (`sleep_until` once the instant is `Some`),
>    not the plain shutdown bool — which stays exactly as-is for the runner's
>    own separate "stop admitting new work" concern. RED tests:
>    `acceptance_in_progress_completes_normally_when_shutdown_starts_but_deadline_has_not_elapsed`
>    (a 5s deadline set 20ms after shutdown starts must not cut short a 100ms
>    backoff) and
>    `acceptance_in_progress_is_cancelled_once_the_deadline_instant_actually_elapses`
>    (a 1s deadline genuinely cancels a still-in-progress 30s backoff once it
>    elapses). Replaces the round-1
>    `acceptance_retry_mid_backoff_resolves_promptly_when_shutdown_fires` test,
>    which encoded the exact conflation this fix corrects.

> **PR3 round 3 review fixes (2 more BLOCKERs: lifecycle authority fragmented
> across `EffectRuntimeHandle`/`RuntimeEffectAcceptor`/`DeliveryRunner`)** —
> both fixed in the same PR, RED-then-GREEN each:
> 1. **F-01 (BLOCKER)**: aborting the outer `runner_task` did not guarantee
>    `DeliveryRunner`'s own child tasks (per-effect dispatch tasks in `tasks`,
>    in-flight executor calls in `executor_aborts`) were cancelled — those are
>    owned by the `DeliveryRunner` struct itself, not scoped to `run()`'s
>    future, so an externally-aborted outer task could leave them running
>    forever. Fixed: `DeliveryRunner::shutdown_and_drain(deadline_instant)`
>    (`runner.rs`) — callable directly on `&self`/`Arc<Self>` — reuses the
>    same `drain_tasks` `run_inner`'s own end-of-loop step already calls.
>    `EffectRuntimeHandle` now also holds the shared `Arc<DeliveryRunner>` so
>    `shutdown_and_wait` calls this directly as the authoritative cleanup, in
>    addition to (not instead of) awaiting/aborting the outer `runner_task` as
>    a backstop. RED test (`runner.rs`):
>    `shutdown_and_drain_aborts_runner_owned_child_tasks_even_when_the_outer_run_task_was_hard_aborted_first`
>    (a real `DeliveryRunner` + a real spawned `run_inner` task + a hung
>    counter-incrementing executor; the outer task is hard-aborted FIRST,
>    proving the counter keeps changing — then `shutdown_and_drain` alone must
>    stop it).
> 2. **F-02 (BLOCKER)**: shutdown never closed acceptance admission (a NEW
>    `accept()` call starting after shutdown began had nothing rejecting it),
>    and `shutdown_and_wait` never waited for an `accept()` call already in
>    flight when shutdown began — reproducible even in `Inline` mode, where
>    there is no runner task at all. Fixed: a new `LifecycleGate`
>    (`acceptor.rs`), shared via `Arc` between `RuntimeEffectAcceptor` and
>    `EffectRuntimeHandle` — `LifecycleState::{Running, Draining { deadline },
>    Closed}` plus an `AtomicU64` in-flight counter and a `Notify` fired at
>    zero. `accept()` calls `lifecycle.enter()` at entry (before minting
>    anything): rejected calls get `EffectAcceptanceError::Permanent`, same
>    shape as F-03's missing-acceptor case; admitted calls hold an RAII
>    `InFlightGuard` for the whole batch. `shutdown_and_wait` now runs one
>    sequence: signal shutdown + publish deadline (as before) →
>    `begin_draining` (closes admission) → `wait_until_drained` (bounded by
>    the same deadline instant) → `runner.shutdown_and_drain` (F-01) → await/
>    abort the outer task → `close()`. RED tests:
>    `accept_started_after_draining_is_rejected_immediately_without_touching_the_store`
>    and (both `Inline` and `Deferred` mode)
>    `shutdown_and_wait_awaits_an_already_in_flight_accept_call_in_{inline,deferred}_mode`.

> **PR3 round 4 review fixes (3 more BLOCKERs: shutdown/drain sequencing and a
> lost-wakeup primitive)** — all fixed in the same PR, RED-then-GREEN each:
> 1. **F-01 (BLOCKER)**: `shutdown_and_wait` still sent `shutdown.send(true)`
>    (telling the runner to stop consuming) BEFORE draining in-flight
>    acceptances, not after — an `accept()` call already mid-persistence when
>    shutdown began could reach `queue.send` only once nothing was left
>    consuming, durably accepting and enqueueing an effect that then sat
>    stranded. Fixed: reordered so admission closes and every already-admitted
>    acceptance finishes enqueueing (or is cut short by the deadline) BEFORE
>    `shutdown.send(true)` is ever sent. RED test:
>    `late_enqueue_after_shutdown_begins_is_still_consumed_by_the_deferred_runner`.
> 2. **F-02 (BLOCKER)**: `run_inner`'s own end-of-loop drain step and an
>    externally-invoked `shutdown_and_drain` call were two independent callers
>    racing for the same `tasks` `JoinSet` mutex — whichever got there first
>    held it for its own deadline, the other blocked on that same mutex with
>    no bound of its own. Fixed: single-flight leader-election coordination
>    (`drain_claimed` + `drain_done_tx/rx: watch<bool>`) — the first caller
>    becomes the sole leader; every follower waits for `drain_done`, bounded
>    by ITS OWN deadline. The mutex acquisition itself is now also
>    timeout-bounded as defense-in-depth. RED test:
>    `external_shutdown_and_drain_is_bounded_by_its_own_deadline_even_while_run_inners_own_internal_drain_is_in_progress`.
> 3. **F-03 (BLOCKER)**: `LifecycleGate`'s `AtomicU64` + `Notify` pairing had a
>    lost-wakeup window — `notify_waiters()` only wakes waiters already
>    registered at the exact moment it's called, so a guard's drop landing in
>    the narrow gap between `wait_until_drained`'s read and its `.notified()`
>    registration could lose the wakeup, burning the full deadline. Fixed:
>    replaced with `watch::Sender<u64>`/`Receiver<u64>`, which always reflects
>    the latest value regardless of when it's borrowed/polled relative to the
>    send. RED/GREEN evidence: the true race proved empirically unreproducible
>    via realistic scheduling (25,000+ trials across several synchronization
>    strategies); `lost_wakeup_pattern_is_reproduced_with_a_widened_race_window`
>    validates the same `Notify`-vs-`watch` contract difference using an
>    artificially widened window instead (50/50 losses with `Notify`, 0/50
>    with `watch`), with
>    `wait_until_drained_does_not_lose_the_last_guards_wakeup_under_concurrent_drop`
>    kept as a best-effort integration-level regression check.

> **PR3 round 5 review fix (1 more BLOCKER, plus a minor defensive
> addition)** — RED-then-GREEN:
> 1. **F-01 (BLOCKER)**: round 4's leader/follower drain coordination was
>    itself unsafe — `run_inner` runs INSIDE the task `EffectRuntimeHandle`
>    holds as `runner_task`, so if it won the leader race (its own longer
>    internal deadline), an external `shutdown_and_wait` follower's
>    shorter-deadline timeout would call `runner_task.abort()`, killing the
>    LEADER mid-drain before it finished aborting the hung executor attempt
>    it was cleaning up. Fixed ("Option A"): leader/follower election removed
>    entirely — `run_inner` no longer drains anything itself, it only stops
>    consuming and returns; `DeliveryRunner::shutdown_and_drain`, called
>    SOLELY by `shutdown_and_wait`, is now the ONE cleanup authority.
>    `shutdown_and_wait` reordered: await/abort the (now cheap) outer
>    `runner_task` first, THEN run the sole drain step — the caller's
>    deadline is split so awaiting `runner_task` can never consume the whole
>    budget and starve the actual drain. RED test (driving the REAL
>    `shutdown_and_wait`/`shutdown_and_drain`/`run()` path end-to-end):
>    `shutdown_and_wait_stops_a_hung_executor_task_even_when_run_inner_would_have_raced_it_for_drain_leadership`.
> 2. **Minor defensive addition**: `drain_tasks_locked`'s lock-acquisition-
>    timeout branch (now nearly unreachable with a single caller) no longer
>    abandons cleanup outright — it still aborts tracked `executor_aborts`
>    handles via a non-blocking `try_lock` on that separate mutex.
>
> **PR3 round 6 review fix (1 BLOCKER, plus 2 doc-only fixes)** — RED-then-GREEN:
> 1. **F-01 (BLOCKER)**: the drain architecture (round 5, above) was confirmed
>    correct, but `shutdown_and_wait`'s final `Result` was dishonest —
>    `shutdown_and_drain` returned `()`, discarded entirely, so the result came
>    only from awaiting `runner_task` (unconditionally `Ok(())` in `Inline`
>    mode; can finish `Ok` quickly in `Deferred` mode even while the
>    subsequent drain still force-aborts a hung executor). Fixed:
>    `drain_tasks_locked`/`shutdown_and_drain` now return `bool` (drained
>    naturally within the deadline vs. deadline-already-elapsed/forced-abort),
>    combined into `shutdown_and_wait`'s `Result` — `Ok(())` only when both
>    the runner task finished cleanly AND the drain was natural; otherwise
>    `Err(Timeout)`. Also closed a related gap: `Inline` mode's `drain_one`
>    never populates `tasks` (it runs synchronously, not via
>    `spawn_tracked`), so the existing timeout-based force-abort check could
>    never fire for a hung Inline executor on its own; `drain_tasks_locked`
>    now also checks the deadline directly on entry and forces the abort
>    either way. RED tests:
>    `shutdown_and_wait_returns_timeout_when_an_inline_executor_hangs_past_the_deadline`,
>    `shutdown_and_wait_returns_timeout_when_the_runner_task_exits_cleanly_but_a_child_executor_hangs`.
> 2. **Doc-only**: confirmed no in-code comment describes leader/follower
>    drain coordination as current (all mentions already correctly framed as
>    removed history) — no change needed beyond noting the check.
> 3. **Doc-only**: `EffectAcceptor::accept`'s doc comment ("never refused
>    outright at intake") reworded to distinguish a normal in-progress effect
>    (never refused mid-flight once admitted) from a NEW call arriving after
>    shutdown/draining has begun (rejected immediately at intake) —
>    `crates/persistent-entity/src/effect_acceptor.rs`.

## Phase 9: Lifecycle Wiring (`service-sdk`)

- [x] 9.1 RED: zero cost when no executor registered; shutdown drains within deadline, in-flight→`Cancelled`→pending, `drain_incomplete` on remainder
- [x] 9.2 GREEN: `register_effect_executor` + `DeliveryConfig` option, conditional runner spawn, `register_async_teardown` drain hook (builder.rs)

> **Shutdown vs. acceptance retry (AD-9)**: a bounded acceptance retry in
> progress during graceful shutdown MUST respect the same drain deadline as
> the rest of the lifecycle and time out into the same "acceptance ultimately
> failed" (`EffectAcceptanceError`) path, never block shutdown indefinitely.
> Docs-only note; not yet implemented.

> **Implementation notes (this pass, PR4)**:
> - `RuntimeBuilder::register_effect_executor(effect_types, executor) ->
>   Result<Self, DuplicateEffectType>` (service-sdk `builder.rs`) accumulates
>   into a new `effect_executors: ExecutorRegistry` builder field and fails
>   closed immediately on a duplicate `effect_type`, mirroring the existing
>   `with_service`'s Result-returning pattern in this same file — **not**
>   design.md §6.4's literal wording ("surfaced at `.build()`"), because
>   `.build()` is documented and relied upon as infallible ("Always
>   succeeds") everywhere else in this builder; making it fallible would be a
>   breaking change to every existing caller for one new feature. Deviation
>   recorded here rather than silently diverging from the design doc.
> - `RuntimeBuilder::with_delivery_config(DeliveryConfig)` and
>   `RuntimeBuilder::with_effect_drain_deadline(Duration)` (default 5s) added
>   alongside it.
> - **Zero-cost gate**: `build()` checks `effect_executors.is_empty()`
>   (new additive method on `ego-runtime`'s `ExecutorRegistry`) before doing
>   anything else. Empty → no `InMemoryEffectStore`, no `EffectQueue`, no
>   `RuntimeEffectAcceptor`, no spawned task, no `register_async_teardown`
>   hook — `Runtime::effect_acceptor()` returns `None`. Non-empty → a real
>   `RuntimeEffectAcceptor` is constructed via the already-shipped
>   `RuntimeEffectAcceptor::new`, exposed through `Runtime::effect_acceptor()
>   -> Option<&Arc<dyn EffectAcceptor>>` (new field on `RuntimeInner`,
>   threaded through `RuntimeInner::new_with_logger`'s now-9th parameter; all
>   4 existing call sites — 3 test fixtures + 1 production — updated with a
>   trailing `None`).
> - **Drain-on-shutdown**: new additive `RuntimeEffectAcceptor::drain(deadline,
>   shutdown_tx) -> u64` method (`ego-runtime`'s `acceptor.rs`) lets the
>   `Deferred` loop keep consuming normally for up to `deadline` (`Inline` has
>   no loop, so it skips the sleep), then signals shutdown and calls the
>   already-shipped `EffectStateStore::recover_in_flight` — the same
>   crash-recovery mechanism (Phase 1), driven deliberately here instead of by
>   a crash — to reset any still-`InFlight` effect back to `Pending` for a
>   future `claim_due` run. Returns the recovered count. `build()` registers
>   this as a `register_async_teardown` hook only in the non-empty branch;
>   a non-zero recovered count makes the hook return
>   `Err(RuntimeInfraError::Teardown{..})` — reusing the existing "a failing
>   hook surfaces through `shutdown_async`" contract (Finding 6/F-02) as this
>   phase's `drain_incomplete` signal, rather than inventing a new error path.
>   Phase 11 (observability) is expected to route this count to a real
>   `Observability` event; for now it only fails the teardown hook, which is
>   still honest (never silently discarded) and matches AD-9's "never block
>   shutdown forever" (proven by a test asserting elapsed time stays well
>   under the deadline-plus-margin even when an effect is permanently stuck).
> - ponytail: `drain()` sleeps the *full* `deadline` for `Deferred` mode
>   rather than polling for early completion — neither `EffectQueue` nor
>   `DeliveryRunner` expose a queue-depth/in-flight-count accessor to detect
>   "already done" sooner without a broader (out-of-scope) change to those
>   already-shipped files. Upgrade path: add such an accessor and poll it if a
>   flat multi-second shutdown delay ever proves costly in practice.
> - **"Wire a real acceptor into the actor(s)" — scope resolution**:
>   design.md's own Phase 9 file table lists only `service-sdk/builder.rs`
>   as modified; it does **not** list `persistent-entity/{builder,runtime,
>   entity_ref_tokio}.rs`. Per that file list, this PR closes the gap only as
>   far as making a real, working `RuntimeEffectAcceptor` constructible and
>   retrievable via `Runtime::effect_acceptor()` — proven end-to-end in this
>   PR's own tests (`accepted_effects_are_actually_delivered_through_the_
>   wired_acceptor`: an effect accepted through the builder-constructed
>   acceptor really reaches the registered executor). Actually plumbing that
>   acceptor into `persistent_entity::builder::EntityRuntimeBuilder` /
>   `EntityRuntime` / `TokioEntityRef::new` (so a spawned production
>   `EntityActor` picks it up) is host-integration wiring outside
>   `ego-service-sdk`'s own crate boundary and is left to whichever host
>   constructs both runtimes — realistically `examples/reference-app`,
>   Phase 12/PR5's explicit scope ("Wire one trivial executor + handler in
>   examples/reference-app"). Not implemented in this PR; called out here so
>   it is not silently assumed done.
> - `ego-runtime` and `persistent-entity` added as `ego-service-sdk`
>   dependencies (previously absent, despite design.md §2 describing this
>   shape) — no cycle: neither depends back on `ego-service-sdk`.

## Phase 10: Tenant Isolation & Transport-Agnosticism

- [ ] 10.1 RED: cross-tenant dedup never collides; no `effect_type`/`destination` branch outside registry lookup; payload passed through unexamined
- [ ] 10.2 GREEN: finalize tenant-scoped dedup key wiring; fix any violations found

## Phase 11: Observability

- [ ] 11.1 RED: each signal carries id/effect_type/destination/tenant/hashed-key; payload absent by default
- [ ] 11.2 GREEN: emit accepted/dispatch_started/attempt/success/retry_scheduled/terminal_failed/deduplicated/executor_missing/queue_depth/oldest_pending_age/drain_incomplete signals

## Phase 12: Test Doubles, E2E, Docs

- [ ] 12.1 Add recording `ExternalEffectExecutor` test double (`crates/testkit`)
- [ ] 12.2 Wire one trivial executor + handler in `examples/reference-app`; E2E describe→deliver→retry→dedup
- [ ] 12.3 E2E: kill-process test asserts in-memory store loses undelivered effects on crash (explicit, never hidden)
- [ ] 12.4 Update `effects/` module docs; note `EventPublisher` migration deferred to roadmap
