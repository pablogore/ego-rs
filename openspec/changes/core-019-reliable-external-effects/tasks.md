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

- [ ] 3.1 RED: backoff+jitter math, attempt cap (AD-5), per-`effect_type` override tests
- [ ] 3.2 GREEN: `RetryPolicy`, `DeliveryConfig`, `DeliveryConfig::immediate()` (policy.rs)

## Phase 4: Internal Queue (not public)

- [ ] 4.1 RED: bounded-queue backpressure test (blocks at capacity, never drops)
- [ ] 4.2 GREEN: internal `EffectQueue` mpsc wrapper (queue.rs)

## Phase 5: Acceptor Port

- [ ] 5.1 RED: `accept()` mints id + attaches tenant before store interaction; awaits capacity, never refuses
- [ ] 5.2 GREEN: `EffectAcceptor` trait (`crates/persistent-entity/src/effect_acceptor.rs`)
- [ ] 5.3 GREEN: `RuntimeEffectAcceptor` impl (runtime/effects/acceptor.rs)

## Phase 6: Delivery Runner

- [ ] 6.1 RED: happy-path success; `RetryableFailure` re-enqueue+backoff; `ExecutorMissing` terminal+signal; dedup `Conflict`→`InvalidEffect` terminal; executor panic = one retryable attempt; AD-7 bookkeeping-failure stays in-flight and re-dispatches
- [ ] 6.2 GREEN: `DeliveryRunner` drain loop, semaphore, watch-shutdown, backoff re-enqueue, AD-7 bounded-retry bookkeeping (runner.rs)

## Phase 7: ImmediateDeliveryPolicy

- [ ] 7.1 RED: Inline mode still traverses full pipeline (no bypass); failed attempt signaled, not retried
- [ ] 7.2 GREEN: `runner_mode: Inline` drain-one-on-accept wiring (runner.rs / acceptor.rs)

## Phase 8: Handler API + Actor Wiring

- [ ] 8.1 RED: unmodified handler compiles/passes unchanged (default `external_effects`)
- [ ] 8.2 GREEN: default `external_effects(cmd, new_state, events, ctx) -> Vec<ExternalEffectDescription>` (persistent_entity.rs)
- [ ] 8.3 RED: actor calls `external_effects`+`accept` after commit, before reply; backpressure delays reply not commit
- [ ] 8.4 GREEN: optional `effect_acceptor` field + post-persist call after `publisher.publish` (actor.rs:294)

## Phase 9: Lifecycle Wiring (`service-sdk`)

- [ ] 9.1 RED: zero cost when no executor registered; shutdown drains within deadline, in-flight→`Cancelled`→pending, `drain_incomplete` on remainder
- [ ] 9.2 GREEN: `register_effect_executor` + `DeliveryConfig` option, conditional runner spawn, `register_async_teardown` drain hook (builder.rs)

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
