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

## Phase 4: Internal Queue (not public)

- [x] 4.1 RED: bounded-queue backpressure test (blocks at capacity, never drops)
- [x] 4.2 GREEN: internal `EffectQueue` mpsc wrapper (queue.rs)

## Phase 5: Acceptor Port

- [ ] 5.1 RED: `accept()` mints id + attaches tenant before store interaction; awaits capacity; never refused at intake, but returns `Err(EffectAcceptanceError)` when the one retryable `EffectStateStore::accept` error (`TemporarilyUnavailable`) survives the bounded retry policy, or immediately when the store error is permanent, and never rolls back the committed event (AD-9)
- [ ] 5.2 GREEN: `EffectAcceptor` trait returning `Result<(), EffectAcceptanceError>` + the `EffectAcceptanceError` type (`crates/persistent-entity/src/effect_acceptor.rs`)
- [ ] 5.3 GREEN: `RuntimeEffectAcceptor` impl (runtime/effects/acceptor.rs) — bounded retry via the AD-5 `RetryPolicy` of retryable store errors only (`TemporarilyUnavailable`); every other `EffectStoreError` variant (`Backend`, `InvalidTransition`, `NotFound`, and `Conflict` from `accept`) is permanent and maps immediately to `EffectAcceptanceError` without retry (AD-9 classification table)

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

- [ ] 8.1 RED: unmodified handler compiles/passes unchanged (default `external_effects`)
- [ ] 8.2 GREEN: default `external_effects(cmd, new_state, events, ctx) -> Vec<ExternalEffectDescription>` (persistent_entity.rs)
- [ ] 8.3 RED: actor calls `external_effects`+`accept` after commit, before reply; backpressure delays reply not commit
- [ ] 8.4 GREEN: optional `effect_acceptor` field + post-persist call after `publisher.publish` (actor.rs:294)

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

## Phase 9: Lifecycle Wiring (`service-sdk`)

- [ ] 9.1 RED: zero cost when no executor registered; shutdown drains within deadline, in-flight→`Cancelled`→pending, `drain_incomplete` on remainder
- [ ] 9.2 GREEN: `register_effect_executor` + `DeliveryConfig` option, conditional runner spawn, `register_async_teardown` drain hook (builder.rs)

> **Shutdown vs. acceptance retry (AD-9)**: a bounded acceptance retry in
> progress during graceful shutdown MUST respect the same drain deadline as
> the rest of the lifecycle and time out into the same "acceptance ultimately
> failed" (`EffectAcceptanceError`) path, never block shutdown indefinitely.
> Docs-only note; not yet implemented.

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
