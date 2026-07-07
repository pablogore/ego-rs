# Tasks: CORE-006A — Activation Authority & Linearizability

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~950-1150 (6 src files modified, 2 files deleted, 3 test files mechanically adapted, 1 new test file, 1 doc, 1 new Cargo dep) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 → PR2 → PR3 → PR4 (see Work Units) |
| Delivery strategy | ask-on-risk (default; not yet confirmed by user) |
| Chain strategy | pending |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | `parking_lot` dep + `BoundedMailbox` sync queue + `close_and_drain()` | PR 1 | Base: feature/tracker branch. Standalone — public API stays `async fn`, only internal locking changes. ~60-90 lines. |
| 2 | Registry map rewrite + teardown guard + actor wiring + delete `activation.rs`/`supervisor.rs` | PR 2 | Base: PR 1 branch. Coupled ADR core (ADR-001/002/003/005/006/008) — cannot compile if split further. ~500-600 lines, highest risk. |
| 3 | Existing test files' mechanical adaptation + new `guaranteed_completion_tests.rs` | PR 3 | Base: PR 2 branch. Needs the `Result`-returning `entity_ref()` from PR 2. ~450-550 lines. |
| 4 | `ARCHITECTURE.md` alignment (ADR-007) | PR 4 | Base: feature/tracker branch (pure doc, no code dependency) or PR 3 branch if stacking sequentially. ~100-120 lines. |

If `feature-branch-chain` is chosen: PR1 base = tracker; PR2 base = PR1; PR3 base = PR2; PR4 base = tracker (doc-only) or PR3, per team preference.

---

## Phase 1: Foundation — `parking_lot` + sync mailbox queue (ADR-001, ADR-005)

- [x] TASK-001 RED: failing test asserting `BoundedMailbox::close_and_drain()` synchronously drains the queue and replies `Err(EntityNotActive)` to every pending `oneshot` (`mailbox.rs`).
- [x] TASK-002 GREEN: add `parking_lot = "0.12"` to `crates/persistent-entity/Cargo.toml`; migrate `BoundedMailbox.queue` from `Arc<tokio::sync::Mutex<VecDeque<T>>>` to `Arc<parking_lot::Mutex<VecDeque<T>>>`; sync-ify `push`/`pop` sites in `send`/`recv`/`is_empty`/`is_full`/`len` (still `.await` on `Notify` outside the lock); implement sync `close_and_drain() -> VecDeque<T>` to pass TASK-001.

## Phase 2: Registry map rewrite (ADR-001, ADR-002, ADR-003)

- [x] TASK-003 RED: failing test — N concurrent `entity_ref()` calls for one triple resolve to the same mailbox `Arc` (spawn-count instrumentation, not ID-set size — NFR-002).
- [x] TASK-004 RED: failing test — `active_count()` excludes an entry whose published state is `Recovering`; counts it once it's `Active`.
- [x] TASK-005 GREEN: rewrite `EntityRegistry` (`registry.rs`): replace `active_entities: HashSet<String>` with a triple-keyed map `{ mailbox: Arc<dyn Any+Send+Sync>, rx: watch::Receiver<EntityState>, epoch: u64 }`, guarded by `parking_lot::Mutex`; add monotonic epoch counter, `lookup()`, insert-if-absent, `deactivate_if_mine(epoch)`, `active_count()` over state cells. Delete eager `mark_active`; keep `passivated_entities`/`mark_passivated` unchanged (advisory).
- [x] TASK-006 RED: failing test — a downcast mismatch on a live entry returns `Err`, does not spawn a competing actor (ADR-002, CRITICAL 1).
- [x] TASK-007 GREEN: `entity_ref::<C,S>()` downcasts the erased `Arc` *after* the lock is released; mismatch returns `Err(EntityError::Internal("routing type mismatch for triple …"))` plus `debug_assert!(false, ..)` in debug builds — never a spawn fallback.

## Phase 3: Teardown guard + actor wiring (ADR-005, ADR-008)

- [ ] TASK-008 RED: failing test — actor panics mid-processing with N commands already queued; all N callers eventually resolve to a terminal `Err`, none hangs (FR-009).
- [ ] TASK-009 RED: failing test — `MailboxClosed` observed in the close→remove teardown window is retried via a fresh `entity_ref()` call and reaches the next healthy actor (FR-010, ADR-008).
- [ ] TASK-010 GREEN: `entity_ref()` becomes lookup-or-spawn: lock map → clone existing erased mailbox, or create mailbox + `watch::channel(Recovering)` + `epoch = next_epoch()` + insert → release lock → `tokio::spawn` the actor with a `TeardownGuard { epoch, tx }` moved in, strictly *after* release (Round-3 self-deadlock fix — do not move spawn back under the lock).
- [ ] TASK-011 GREEN: implement `TeardownGuard::drop()` → sync `deactivate()`: `mailbox.close()` → `close_and_drain()` replying `Err(EntityNotActive)` to every queued envelope → `deactivate_if_mine(epoch)` (remove entry + `tx.send(terminal)`). Delete `SpawnGuard` (`entity_ref_tokio.rs:30-39`).
- [ ] TASK-012 GREEN: `actor.rs` — move the `watch::Sender` into the actor; publish `EntityState` on every `lifecycle.transition_to(_)`; remove direct `registry.remove_active` calls in `drain_mailbox_with_error`/`passivate` (`:320,328`) — teardown now flows only through the guard.
- [ ] TASK-013 GREEN: `runtime.rs` — `entity_ref()` signature changes to `Result<impl EntityRef<Command = C>, EntityError>`.

## Phase 4: Remove dead activation/supervisor code (ADR-006)

- [ ] TASK-014 Delete `crates/persistent-entity/src/activation.rs` (`SharedActivation` subsumed by the registry map mutex).
- [ ] TASK-015 Delete `crates/persistent-entity/src/supervisor.rs` (dead, broken — `await`s a sync method, zero callers); confirm `lib.rs` still declares no `mod activation`/`mod supervisor`.

## Phase 5: Adapt existing test call sites (Caller Impact Analysis)

- [ ] TASK-016 `activation_ordering_tests.rs`: adapt every `entity_ref()` call site with `.unwrap()`/`?`; switch concurrency-probe tests to `#[tokio::test(flavor = "multi_thread")]` (NFR-001); tighten `active_count() <= 2` assertions at `:189,238` to `== 1`.
- [ ] TASK-017 `persistence_failure_tests.rs`: adapt `entity_ref()` call sites (`:107,143,171,196,225,242,265,266`) with `.unwrap()`/`?`.
- [ ] TASK-018 `real_actor_path_tests.rs`: adapt the `entity_ref()` call site (`:190`) with `.unwrap()`/`?`.
- [ ] TASK-019 Run `cargo test --workspace`; confirm all three suites' observable expectations pass unchanged (per design's Caller Impact Analysis table).

## Phase 6: New `guaranteed_completion_tests.rs` (FR-009, FR-010, NFR-001/002)

- [ ] TASK-020 RED+GREEN: panic mid-processing while `Active` with N queued commands — assert all N `rx.await` resolve to a terminal `Err`.
- [ ] TASK-021 RED+GREEN: panic/cancellation mid-passivation-drain — assert the undrained remainder still gets a terminal reply, independent of the in-body drain loop.
- [ ] TASK-022 RED+GREEN: runtime shutdown while `Recovering` with a command already enqueued — assert the caller observes a terminal outcome, not a hang.
- [ ] TASK-023 RED+GREEN: 20-caller probe (`multi_thread`) under a recovery-time panic — assert all 20 `rx.await` resolve and exactly 1 actor was spawned (spawn-counter instrumentation, NFR-002).
- [ ] TASK-024 RED+GREEN: force an in-section construction panic and a `tokio::spawn`-outside-runtime panic for one triple — assert other triples' `entity_ref()`/`active_count()` are unaffected (poison + Round-3 deadlock safety).

## Phase 7: `ARCHITECTURE.md` alignment (ADR-007, last — pure doc, no code dependency)

- [ ] TASK-025 Registry & Activation subgraph (`:117-120`): update `REG` node's `active` shape; delete `pending_activations → SharedActivation` and the `ACT` node.
- [ ] TASK-026 Infrastructure subgraph (`:137-138`): delete the `SUP` (`Supervisor`) node.
- [ ] TASK-027 Edges (`:144-146,153`): remove `REG→ACT`, `ACT→spawns→EA`, `EA→failure→SUP`; add `REF/EntityRuntime -->|entity_ref() lookup-or-spawn| REG`, `REG -->|spawns| EA`.
- [ ] TASK-028 Activation Ordering sequence diagram (`:167-221`): drop participant `A`; fix the `:191-192` note (existence ≠ active count); replace caller-driven `remove_active` (`:215-219`) with actor-owned `deactivate()` on exit.
- [ ] TASK-029 State table (`:242-248`): distinguish "in registry" (map entry exists) from active *count* (only `Active`).
- [ ] TASK-030 Key Design Invariants (`:250-257`): re-attribute "one actor per triple" to the registry-map single-flight; "single source of truth" to actor-published state.
