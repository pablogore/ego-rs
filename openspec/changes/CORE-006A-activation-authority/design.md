# Design: CORE-006A — Activation Authority & Linearizability

## Technical Approach

Make `EntityRegistry` the single routing authority for the `persistent-entity`
runtime. `entity_ref()` stops unconditionally spawning and becomes
**lookup-or-spawn** against a triple-keyed map of live mailbox handles guarded by
a registry-owned `parking_lot::Mutex` (non-poisoning — ADR-001). That mutex's
check-and-insert critical section is the single-flight coordinator — no separate
coordination object.
"Is this entity active" stops being a `HashSet<String>` membership fact and becomes
a lifecycle state **published by the owning actor** and observed by the registry:
one writer (the actor), one read view (the registry). All rollback collapses into a
single actor-owned drop guard.

Net structural change: `SharedActivation` (`activation.rs`) and `Supervisor`
(`supervisor.rs`) are **removed**, not wired in. The guarantees they were meant to
provide are already delivered by the mailbox + per-command oneshot + the actor's
recovery barrier once the mailbox is cached instead of discarded.

This directly instantiates the archived drafts' converged position — *"the Actor is
the sole Execution Authority; exactly one per triple"* (`execution-authority/spec.md`
FR-EA-001/004, `reactivation-safety-spec.md` FR-SF-001..006): the **registry map**
enforces "exactly one per triple"; the **actor** remains the sole authority for
execution, ordering, and its own lifecycle.

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Single activation authority | Registry-owned routing map; `SharedActivation` removed | ADR-001 |
| Registry + mailbox locking primitive | `parking_lot::Mutex` (non-poisoning); no `.unwrap()` on lock | ADR-001 |
| Map value type (registry is non-generic, mailboxes are generic) | Type-erased `Arc<dyn Any + Send + Sync>` mailbox handle | ADR-002 |
| Downcast mismatch on a live entry | `entity_ref()` returns an explicit `Err`; never falls through to spawn | ADR-002 |
| Single source of truth for "active" | Actor-published lifecycle state observed by the registry | ADR-003 |
| State-cell mechanism | `watch::channel<EntityState>`; initial value fixed at construction | ADR-003 |
| Cold vs reactivation | One code path, parameterized only by persisted history | ADR-004 |
| Rollback + guaranteed completion | One actor-owned drop guard; **synchronously drains + terminally answers every enqueued command** on any exit | ADR-005 |
| Teardown identity ("remove only if mine") | Monotonic per-entry `u64` epoch (ABA-safe) | ADR-005 |
| `supervisor.rs` | Removed | ADR-006 |
| `ARCHITECTURE.md` (D4) | Updated in this change | ADR-007 |
| Stale-mailbox send (`MailboxClosed`) | Distinct, retryable terminal error; caller may re-`entity_ref()` | ADR-008 |

---

## Execution Authority — the invariant every ADR below derives from

**Execution Authority** for a triple `(tenant_id, entity_type, aggregate_id)` is the
right to run that entity's actor loop and be the sole writer of its lifecycle state.
At most one authority exists per triple at any instant. Every ADR in this document is
a consequence of fixing exactly when that authority is born, who may observe it, and
how it dies — not a set of independent decisions:

- **Birth.** Authority is created exactly once per activation: when `entity_ref()`'s
  single-flight critical section (ADR-001) finds no live entry, inserts one, and
  spawns the actor. Insert-and-spawn happen under one lock acquisition — there is no
  window where two callers can each believe they created the authority.
- **Visibility.** A live routing entry means "an authority has been (or is being)
  established for this triple" — a routing fact. It does **not** mean "the authority
  is ready to do work" — that is the actor-published `EntityState` (ADR-003). Conflating
  these two questions is the root cause of the original `mark_active()`-before-recovery
  bug; keeping them separate is why fixing it doesn't require a second coordination
  primitive.
- **Death.** Authority ends exactly once per actor lifetime, via the single
  `deactivate()` call owned by that actor's drop guard (ADR-005). No other code path
  may remove or overwrite a routing entry.
- **No silent duplication.** Any code path that could leave two authorities alive for
  one triple, or hand a caller a routing fact for an authority that turns out not to
  exist (a downcast mismatch, ADR-002) and respond by minting a *second* one, violates
  this invariant regardless of how "unlikely" the path is. The fix is always a
  structural error, never a fallback that manufactures a second authority.
- **No unowned mailbox.** A caller may only ever observe a mailbox for which a live
  authority exists or for which the channel is already closed (a terminal, observable
  signal) — never a mailbox that is open but permanently undrained (ADR-008).
- **Guaranteed completion.** Once a command is enqueued, the death of the authority —
  by ANY cause (normal passivation, recovery/command/drain panic, task cancellation,
  runtime shutdown) — MUST terminally answer that command. Draining the queue and
  replying to every remaining `oneshot` is therefore part of `deactivate()` itself, not
  a best-effort in-body step that a panic can skip (ADR-005, spec FR-009).

ADR-001–ADR-007 each satisfy one clause of this invariant; ADR-002, ADR-005, and ADR-008
below were tightened specifically because two rounds of adversarial review proved the
first draft violated "no silent duplication," "guaranteed completion," and the
poison-free birth of authority respectively (see **Judgment Day Resolution**).

---

## ADR-001 — Single activation authority lives in `EntityRegistry`; `SharedActivation` is removed

**Context.** `entity_ref()` (`runtime.rs:129-158`) calls `TokioEntityRef::new()`
(`entity_ref_tokio.rs:76-140`), which *always* creates a mailbox and
`tokio::spawn`s an actor — there is no lookup. N callers for one triple ⇒ N actors
racing `persist_events()` (proposal finding 4). `SharedActivation`
(`activation.rs:12-31`) — a `tokio::sync::Mutex<()>` plus a `watch` channel
broadcasting the recovery outcome — was written to coordinate this but was never
declared in `lib.rs` (finding 3): zero callers, never compiled. Decision D2 assigns
its disposition here.

**Decision.** Move routing authority into `EntityRegistry`. Replace
`active_entities: HashSet<String>` with a map from `aggregate_id` to a live entry
that holds the entity's mailbox handle and its published lifecycle state.
`entity_ref()` becomes:

1. Lock the registry map (`parking_lot::Mutex`).
2. If a live entry exists for the triple, return a `TokioEntityRef` wrapping the
   **existing** mailbox (a cheap `Arc` clone).
3. Otherwise create the mailbox, create the `watch::channel` (ADR-003), assign
   `epoch = next_epoch()`, and insert the entry (state `Recovering`).
4. **Release the lock**, then construct the teardown guard (capturing `epoch`, `tx`)
   and `tokio::spawn` the actor future — strictly *outside* the critical section.

Steps 1–3 run under the map lock; step 4 runs after it is released. Mailbox creation
is sync, so **no `.await` occurs inside the critical section** — the lock is held only
for the map mutation itself, never across `tokio::spawn` or recovery. That single
critical section is the single-flight coordinator. **Remove `activation.rs`.**

**Why `tokio::spawn` runs after the lock is released (Judgment Day Round 3 CRITICAL).**
The first revision of this ADR called `tokio::spawn` *inside* the critical section,
with the teardown guard (ADR-005) already captured in the future being spawned. Both
Round 3 judges independently traced the same self-deadlock from this: `tokio::spawn`
panics when called outside a runtime context — a real precondition on `entity_ref()`,
not a contrived one. If that panic fires while the future (holding `guard`) is still
owned by `tokio::spawn`'s own stack frame, unwinding drops the future *before*
unwinding reaches the still-held map lock (Rust unwinds LIFO) — so `guard::drop()` →
`deactivate()` tries to re-lock the same, still-held, non-reentrant `parking_lot::Mutex`
on the same thread. `parking_lot::Mutex` has no reentrancy detection: this self-relock
hangs forever, and because it's the one global registry lock, every other triple's
`entity_ref()`/`active_count()` call now blocks too — a worse outage than the
poisoning bug this ADR set out to fix (a silent permanent hang instead of a fail-fast
panic). Releasing the lock *before* `tokio::spawn` closes this: if spawn panics, the
future (and `guard`) drop with the map lock already free, so `deactivate()` correctly
re-locks and removes the zombie `Recovering` entry instead of either deadlocking or
leaking it — satisfying FR-006 on this path too.

**Poison-free critical section (`parking_lot::Mutex`, not `std::sync::Mutex`).** The
first draft justified `std::sync::Mutex` with "no `.await` ⇒ no deadlock." That is
necessary but not sufficient: `std::sync::Mutex` poisons on **any** panic while held,
and the critical section is not panic-free. `tokio::spawn` panics when called outside a
runtime context (a real precondition on `entity_ref()`, not a contrived one), and
mailbox/`watch`-channel construction can panic under allocation failure — both run
*inside* the lock. With `std::sync::Mutex`, a single such panic poisons the map for the
entire process: every subsequent `.lock().unwrap()` — every other triple's
`entity_ref()`, every `active_count()` — panics forever, a global outage triggered by
one triple's fault. This is a risk today's code does not have (it never spawns under any
lock). **Decision: the registry map (and the mailbox queue, ADR-005) use
`parking_lot::Mutex`.** It has no poison concept: `lock()` returns the guard directly
(no `.unwrap()`), and a panic while held simply releases the lock on unwind. The blast
radius of an in-section panic is then exactly the one `entity_ref()` call that hit it —
it unwinds to that caller; every other triple keeps routing. `parking_lot` is a single,
ubiquitous, audited dependency; the rejected alternative — keep `std::sync::Mutex` and
wrap the fallible section in `catch_unwind` plus manual `Mutex::clear_poison` recovery —
is strictly more code, needs `UnwindSafe` gymnastics around the spawn closure, and must
be repeated at every lock site, so it is more fragile for a worse result.

Type-checking the erased handle — the downcast (ADR-002) — happens **after** the lock is
released, so it too is outside the critical section. The section itself only performs map
lookup/insert/remove, clones the erased handle (`Arc<dyn Any>::clone`, infallible),
constructs the mailbox + `watch` channel, and `tokio::spawn`s — and none of those can
poison a non-poisoning lock.

`SharedActivation`'s watch-channel result broadcast is redundant once the mailbox is
cached: all concurrent callers for a passivated/new triple receive a ref to the *same*
mailbox and enqueue their own command with its own `oneshot` reply. The actor recovers
exactly once (recovery barrier, `actor.rs:61-68`) before draining the mailbox in FIFO
order. On recovery success every queued command is processed; on recovery failure the
actor drains the mailbox sending `EntityNotActive` to every caller
(`actor.rs:63-66` → `drain_mailbox_with_error`, `:315-321`). This satisfies
`reactivation-safety-spec.md` FR-SAF-002 (all callers observe a consistent outcome)
and FR-SF-004 (mechanism is implementation-defined; only "exactly one activation per
window" is mandatory) **without** a second coordination channel.

**Precedent.** We keep the *mutex* half of `SharedActivation`'s mutex+watch pattern —
now realized as the registry map's `parking_lot::Mutex` — and drop the *watch* half
because the mailbox already carries per-caller outcomes. This is the same "single
mutex guards the shared map, cheap clones hand out access" shape already used by
`BoundedMailbox` (`mailbox.rs:23-52`, all state `Arc`-backed, `Clone` shares it).

**Consequences.**
- Exactly one actor per triple, enforced by a lock the registry already owns; no CAS
  loop (satisfies `reactivation-safety-spec.md` FR-SF-006 / constitution §5).
- One fewer type and one fewer coordination path to keep consistent.
- The critical section must stay `.await`-free and, as of the Round 3 fix,
  `tokio::spawn`-free: only sync map/mailbox construction runs under the lock;
  `tokio::spawn` (the one call in this path with a real, documented panic
  precondition) runs strictly after release, so a spawn-time panic can never
  deadlock or poison the map.

## ADR-002 — Type-erased mailbox handles in the routing map

**Context.** `EntityRegistry` is a single non-generic `Arc<EntityRegistry>` shared by
`EntityRuntime` across every entity type (`runtime.rs:82,149`). Mailboxes are
`BoundedMailbox<ActorEnvelope<C>>`, generic over the command type `C`, which is a
per-call type parameter of `entity_ref::<C, S>()`. The registry cannot name `C`.

**Decision.** Store each entry's mailbox as `Arc<dyn Any + Send + Sync>` holding the
`BoundedMailbox<ActorEnvelope<C>>` clone. On a live-entry hit, the critical section
(ADR-001) only clones the erased `Arc` — an infallible, panic-free operation — and
releases the lock. `entity_ref::<C, S>()` downcasts **after** the lock is released
(`Arc::downcast::<BoundedMailbox<ActorEnvelope<C>>>()`). The triple's `entity_type`
(`&'static str`) is 1:1 with `C` by construction, so downcast is always expected to
succeed.

A mismatch is a programming error, not a routing fact. Per the Execution Authority
invariant's "no silent duplication" clause, and per spec FR-001's *Type-mismatch
resolution fails closed* scenario, **`entity_ref()` fails the call with an explicit
`Err(EntityError::Internal("routing type mismatch for triple …"))` in every build** —
`MailboxClosed` is deliberately NOT reused here so the caller can tell a permanent
programming error from the retryable teardown window (ADR-008). A `debug_assert!(false, …)`
additionally trips in debug/test builds to surface the bug loudly, but the operative
behavior in all builds is a returned error, never a spawn. It is **never** a fallback
that treats the mismatch as "no live entry," because that fallback is precisely the path
that would spawn a second actor for a triple that already has a live one. The first draft
of this ADR proposed exactly that fallback.

**This closes Judgment Day CRITICAL 1.** The corroborated trace: a caller with command
type `C2` hits a live entry actually holding `C1`'s mailbox, the downcast fails, the code
treats it as "no live entry," spawns actor B, and overwrites A's map slot — A is now live
but orphaned from the registry, B is live and routed: two actors for one triple, a direct
FR-001 violation. Returning `Err` on the downcast-fail branch (after the lock is released,
so no poison — ADR-001) means the second `tokio::spawn` never executes: the live entry and
actor A are left exactly as they were, and the mistyped caller gets a hard error.

**Precedent.** The crate already erases types this way: command results cross the
mailbox as `CommandErasedResult = Box<dyn Any + Send>` and are downcast by the caller
(`mailbox.rs:11-16`, `entity_ref_tokio.rs:179-183`). Erasing the mailbox handle in the
registry is the same technique applied one layer up.

**Consequences.**
- The registry stays non-generic and process-wide, matching `runtime.rs` today.
- Downcast is O(1) type-id check; negligible next to spawn/recovery.
- `entity_ref()`'s return type becomes `Result<impl EntityRef<Command = C>, EntityError>`
  (today it is infallible `impl EntityRef`). The only failure it introduces is the
  downcast-mismatch programming error; every test call site adapts with a `?`/`unwrap`.
  The blast radius is contained — there are no non-test callers (see Caller impact).
- Because the downcast runs outside the map lock and now returns `Err` (rather than
  panicking) on mismatch, it can neither poison nor unwind through the critical section —
  the blast radius of a mismatch is exactly the one call that hit it.
- `passivated_entities: HashMap<String, u64>` (version tracking) is unchanged — it is
  advisory, not a routing gate (see ADR-004).

## ADR-003 — Single source of truth: actor-published state, registry-observed

**Context.** Today "active" is `HashSet<String>` membership (`registry.rs:14`),
written eagerly in the sync constructor `mark_active()` (`entity_ref_tokio.rs:119`)
**before** the actor recovers (`actor.rs:132`) — finding 1's early visibility. It is
never reconciled with the per-actor `LifecycleStateMachine` (`actor.rs:35`,
`lifecycle.rs:25-28`) — finding 2's split. Duplicate actors shared one `HashSet` entry,
so `active_count()` could not count actors and any duplicate's `remove_active` deleted
the entry for all. Decision D1: `active_count()` counts only `EntityState == Active`;
early visibility is the bug, not a contract.

**Decision.** The **actor is the only writer of its lifecycle state once activation
ownership is transferred to it.** The inserting call in `entity_ref()` (ADR-001 step 3)
sets the entry's initial value to `Recovering` a single time, as part of the same
critical section that creates the entry — there is no actor yet at that instant, so
this is not a second writer, it is the handoff itself. From the moment `tokio::spawn`
returns, every subsequent transition is written exclusively by the actor: on every
`lifecycle.transition_to(_)` it publishes the new `EntityState` into its entry's cell.
`active_count()` iterates entries and counts those observing `EntityState::Active` —
never `Recovering`, never a bare map entry.

- Delete the eager `mark_active()` call at `entity_ref_tokio.rs:119`. Entries are
  inserted (by `entity_ref()`, ADR-001) in state `Recovering` and are **not counted
  as active** until the actor publishes `Active` after `recover_state()`
  (`actor.rs:132`). This is finding 1's fix by construction.
- The map entry's existence answers "is there a live actor" (routing); the published
  state answers "is it active" (D1 count). The two questions are decoupled, ending the
  split: there is exactly one writer of state (the actor), and the registry is a read
  view — no second source.

**State-cell mechanism — decided: `watch::channel<EntityState>` (resolves the former
Open Question).** `entity_ref()` step 3 calls `watch::channel(EntityState::Recovering)`;
the **`watch::Sender` is moved into the spawned actor**, and only the read-only
`watch::Receiver` is stored in the map entry. `active_count()` reads `*rx.borrow()`
(`watch::borrow` is sync, so it composes with the registry's `parking_lot::Mutex`).

This choice makes ADR-003's "the actor is the ONLY writer" claim **literally true, not
aspirational.** The `Recovering` seed is the *constructor argument* to
`watch::channel(..)`, fixed at channel-construction time — it is **not** a
post-construction `Sender::send()` call. So the `entity_ref()` caller thread never writes
the cell; it only supplies the channel's initial value and then hands the sole `Sender`
to the actor. From that instant the only object in the program that can mutate the state
is the actor's `Sender`. The rejected alternative, `Arc<Mutex<EntityState>>`, would
require the caller thread to `lock().set(Recovering)` *after* construction (a real write
from outside the actor) and would leave a writable handle in the registry — making "only
writer" merely a convention two parties agree to honor rather than a type-enforced fact.
`watch` also gives the registry a genuinely read-only view (`Receiver` cannot `send`),
which the `Arc<Mutex<_>>` form cannot express.

**Precedent.** CORE-010B's two-state model kept a single owner of the authoritative
state and derived all external queries from it rather than maintaining a parallel flag;
same principle here — the actor owns state, every external query is derived.

**Consequences.**
- `active_count()` becomes correct and actor-level: it counts `Active` actors, not ID
  strings.
- Visibility now *trails* recovery instead of leading it — see the caller analysis for
  which tests this touches (none break; all count assertions follow an awaited command).

## ADR-004 — Cold activation and reactivation are one code path (D3)

**Context.** D3 broadens the contract to every transition into `Active`:
`∅ → Recovering → Active` and `Passivated → Recovering → Active` must satisfy the same
authority/visibility/linearizability guarantees. The proposal warns this must not
become two contracts by origin.

**Decision.** There is **one** path; the starting state is not branched on.
`entity_ref()` does lookup-or-spawn (ADR-001) with **no special case for
`Passivated`**: a passivated entity is simply a triple with no live map entry —
identical, from the router's view, to a never-activated one. Both spawn an actor that
runs the same `EntityActor::run()` → `recover_state()` (`actor.rs:61,127`). The only
difference is what `load_for_recovery()` returns:

- Cold: no snapshot, no events ⇒ `initial_state()`, version 0 (`actor.rs:100`), then
  `transition_to(Active)`.
- Reactivation: snapshot + replayed events ⇒ recovered state and version
  (`actor.rs:89-118`), then the **same** `transition_to(Active)`.

The map insert, the `Recovering`-then-`Active` publish (ADR-003), and the rollback
guard (ADR-005) are literally the same lines for both. So the contract "provably holds
on the reactivation path" because it is *the same code executed*, parameterized only by
persisted history — not a second path proven equivalent. The advisory
`passivated_entities` map only records the last version for observability
(`passivated_count()`); it never gates or forks routing.

**Consequences.**
- No new passivation *functionality* is added (D3's constraint) — the existing
  `Passivated → Recovering → Active` transition already flows through this one path.
- The multi-thread probe scenario applies uniformly to a cold entity and a
  reactivated one; tests cover both by seeding history (or not) before the concurrent
  burst.

## ADR-005 — One rollback + guaranteed-completion contract via an actor-owned drop guard

**Context.** Registry mutation on teardown is split across four paths (finding 7):
`SpawnGuard::drop` (`entity_ref_tokio.rs:30-39,125-133`), recovery-failure drain
(`actor.rs:63-66,320`), and two passivation sub-paths (`actor.rs:328,340-341,360-361`).
Each mutates the registry unaware of the others; a duplicate's cleanup could delete a
live entry.

**Additional context (Judgment Day CRITICAL 3 → spec FR-009).** Two review rounds proved
the first draft's `deactivate()` — *remove-if-mine* + *publish terminal state*, two steps,
no mailbox handling — leaves already-enqueued callers hung forever. `BoundedMailbox` is
`Arc`-shared with **no `Drop` that closes it** (`mailbox.rs`), and every caller keeps it
alive via their own clone. Mailbox closing today happens *only* via in-body
`self.mailbox.close()` calls inside `passivate()`/`drain_mailbox_with_error()`
(`actor.rs:316,327`) — ordinary statements that are **skipped entirely** on a panic in
`recover_state()`/`execute_command()`/the drain loop itself, on task cancellation, or on
runtime shutdown while suspended mid-`.await`. The remaining `oneshot::Sender`s are then
never resolved and the callers' `rx.await` never completes. Round 2 sharpened it: even
when `close()` *is* reached, if the drain loop panics partway, commands still queued
behind the failed one are also never answered — closing is necessary but not sufficient;
the drain must survive its own failure.

**Decision.** The actor owns its routing entry for its entire lifetime and is the **only**
code that tears it down, through one **synchronous** operation — `deactivate()` — run from
the `Drop` of a guard moved into the spawned task. `Drop` is the *only* code guaranteed to
run on every exit path — normal return, panic during recovery/command/drain, panic during
panic-handling, task cancellation (future dropped), and runtime shutdown (future dropped) —
and `Drop` cannot run async code, so every step below is synchronous:

1. **Close the mailbox.** `mailbox.close()` — a sync atomic store + `notify_waiters()`
   (`mailbox.rs:69-73`). Stops new sends, wakes any parked `recv()`/`send()`.
2. **Synchronously drain the queue and terminally answer every remaining command.** The
   guard locks the mailbox's queue (`parking_lot::Mutex` per ADR-001 — see below for why
   this *must not* be the current `tokio::sync::Mutex`), `std::mem::take`s the
   `VecDeque<ActorEnvelope<C>>`, and for each `ActorEnvelope { reply, .. }` calls
   `reply.send(Err(EntityError::EntityNotActive))` — `oneshot::Sender::send` is sync. This
   is a new sync method on `BoundedMailbox`, e.g. `close_and_drain() -> VecDeque<T>`.
3. **Remove-if-mine (epoch).** Remove the map entry only if its epoch equals this guard's
   epoch (see epoch decision below), then publish the terminal `EntityState` via the
   `watch::Sender` (ADR-003) — the `Sender` being dropped also marks the channel closed,
   a terminal signal to any observer.

The command **in flight at panic time** (already popped from the queue by `recv()` and
moved into `execute_command`) is answered automatically: its `reply: oneshot::Sender` lives
in the unwinding stack frame, so as the frame unwinds the sender is dropped, closing the
channel; the caller's `rx.await` returns `Err`, which `send_command` already maps to a
terminal `EntityError` (`entity_ref_tokio.rs:175`). So: **queued** commands ← guard drain
(step 2); **in-flight** command ← oneshot-drop-on-unwind. Together, every enqueued command
gets a terminal outcome — satisfying FR-009's four scenarios (panic mid-processing, panic
mid-drain, shutdown while `Recovering`, and the 20-caller recovery-panic probe).

**Independent of the normal drain loop.** `passivate()`/`drain_mailbox_with_error()` keep
their in-body drain, but it is now an **optimization** (it answers queued commands promptly
and, for graceful passivation, lets remaining commands run to a *real* result), **not** the
guaranteed path. The guard's step 2 assumes nothing about whether that loop ran or finished
— it simply takes whatever is still in the `VecDeque`. If `passivate()`'s own loop is the
thing that panicked, unwinding drops the guard, and step 2 answers the undrained remainder.
This is exactly Round 2's sharper sub-case.

**Why the mailbox queue must move off `tokio::sync::Mutex`.** Today `queue` is
`Arc<tokio::sync::Mutex<VecDeque<T>>>` (`mailbox.rs:26`) — its `lock()` is **async** and
cannot be acquired from a sync `Drop`. Step 2 therefore requires a **sync** mutex on the
queue. It is chosen as `parking_lot::Mutex` (not `std::sync::Mutex`) specifically because
the drain runs inside `Drop`, possibly *during panic unwinding*: a `std::sync::Mutex` could
be poisoned by the very panic that triggered teardown, and a `.lock().unwrap()` on a
poisoned lock during unwind is a second panic → `abort`. `parking_lot::Mutex` never poisons,
so the drain always succeeds. The lock is only ever held across sync `push`/`pop` (both
`send`/`recv` `.await` on `Notify` *outside* the lock, `mailbox.rs:79-122`), so switching
to a sync mutex changes no `.await` behavior.

**Teardown identity — decided: monotonic per-entry `u64` epoch (resolves the former Open
Question).** The registry keeps a monotonic counter; each insert stamps its entry (and the
guard captures) a unique, never-reused `epoch`. `deactivate()` removes only if the current
entry's epoch matches. Rationale against the two review findings: **(a) it is redundant
under the single-mutex-per-key invariant** — there is no window where an old and a new entry
for the same triple coexist, so the base case cannot clobber regardless of mechanism — but
FR-009 adds *more* Drop-driven teardown paths (panic/cancel/shutdown), and the epoch keeps
`deactivate()` correct even if that invariant is ever weakened or if a late guard `Drop`
interleaves after a fresh insert; it is near-zero-cost defense in depth, so we keep it rather
than drop it. **(b) We pick the epoch over pointer identity** precisely to avoid the ABA
hazard: a cached raw address of a freed mailbox can be reused by the allocator under normal
reactivation churn, so pointer-identity would need `Arc::ptr_eq` against a still-live `Arc`;
a monotonic `u64` never repeats, so it is ABA-safe without keeping a zombie `Arc` alive.

`deactivate()` collapses all prior teardown paths: normal passivation, recovery failure,
command-loop failure (`run()` returns → guard drops), and the never-polled future (runtime
teardown before first poll → task dropped → guard drops) — the case `SpawnGuard` existed
for, now folded in. The bodies of `drain_mailbox_with_error`/`passivate` stop calling
`remove_active` directly. Graceful passivation still records the final version via
`mark_passivated` (`actor.rs:360-361`) — advisory version bookkeeping, distinct from
routing-entry removal, stays where it is.

**Consequences.**
- Rollback + completion is deterministic and idempotent: one function, one caller, runs
  exactly once per actor regardless of exit path; every enqueued command is terminally
  answered (satisfies proposal "fail-closed" + "one rollback contract" + spec FR-009).
- `SpawnGuard` as a separate type is deleted; its job becomes the guard's `Drop` calling
  the shared `deactivate()`.
- `BoundedMailbox` gains a sync `close_and_drain()` and its queue moves to
  `parking_lot::Mutex`; `mailbox.rs` is no longer UNCHANGED (see module layout).

## ADR-006 — Remove `supervisor.rs`

**Context.** `Supervisor` (`supervisor.rs:15-56`) is absent from `lib.rs`, has zero
callers, and would not compile — it `await`s `registry.remove_active(...)`
(`supervisor.rs:38,54`) which is a sync method (`registry.rs:67`). Its only behavior is
`log::error!` + `remove_active`.

**Decision.** Remove the file. Its two responsibilities are already owned elsewhere:
failure logging is done by the actor at the failure site (`actor.rs:146-150,226-230,
290-294`), and registry removal on failure is now the single `deactivate()` contract
(ADR-005). Reintroducing a supervisor would recreate the fifth cleanup path this change
exists to eliminate.

**Consequences.**
- No behavior lost; broken dead code deleted.
- If supervision trees are wanted later (CORE-007+), they are a separate, designed
  concern — not a resurrected broken stub.

## ADR-007 — `ARCHITECTURE.md` alignment (D4)

**Context.** `ARCHITECTURE.md` documents the aspirational design that was never wired
in: an "active entity actor map" plus `pending_activations → SharedActivation` and a
`Supervisor` node (`ARCHITECTURE.md:117-119,137-138`). D4 requires the doc to land in
this change.

**Decision.** Update these specific sections to the implemented reality:

1. **Registry & Activation subgraph (`:117-120`).** In the `REG` node, replace
   `active: EntityTriple → ActorHandle` with `active: aggregate_id → { mailbox handle,
   published lifecycle state }`; **delete** the `pending_activations → SharedActivation`
   line and the entire `ACT` (`SharedActivation`) node (`:119-120`).
2. **Infrastructure subgraph (`:137-138`).** Delete the `SUP` (`Supervisor`) node.
3. **Edges (`:144-146,153`).** Remove `REG -->|insert_active/remove| ACT`,
   `ACT -->|spawns| EA`, and `EA -->|failure| SUP`. Rewire activation as
   `REF/EntityRuntime -->|entity_ref() lookup-or-spawn| REG` and `REG -->|spawns| EA`.
4. **Activation Ordering sequence diagram (`:167-221`).** Rewrite: drop participant
   `A (SharedActivation)`; single-flight is the registry map mutex. Correct the
   mislabeled note at `:191-192` ("VISIBLE but NOT READY / Existence ≠ Readiness"):
   the entry is inserted in `Recovering` and **is not counted as active** until the
   actor publishes `Active` — existence in the map is decoupled from the active count.
   Replace the passivation steps `:215-219` (`remove_active` by the caller `C`) with the
   actor-owned `deactivate()` on task exit (ADR-005).
5. **State table (`:242-248`).** Keep the five states, but change the semantics column
   so "In Registry?" (a live map entry exists) is distinct from the active *count*
   (only `Active`). `Recovering` = in map, not counted; `Active` = in map, counted.
6. **Key Design Invariants (`:250-257`).** Re-attribute "Exactly one actor per triple"
   to the registry-map single-flight (not a separate `SharedActivation` mutex), and
   "single source of truth" to actor-published state.

**Consequences.** The doc stops describing a component (`SharedActivation`) and a data
shape (`ActorHandle` map, `pending_activations`) that do not exist, and matches the
shipped code. Per Non-Goals, the term "registry" is retained (no rename).

## ADR-008 — `MailboxClosed` is a distinct, caller-retryable terminal (spec FR-010)

**Context.** A caller can hold a `TokioEntityRef` whose mailbox was live at lookup time
but whose actor has since begun teardown — `deactivate()` step 1 closed the mailbox
(ADR-005) — while step 3 has not yet removed the map entry. In that window a concurrent
`entity_ref()` still finds a present entry, clones its (now-closed) mailbox, downcasts OK,
and returns a ref; the subsequent `send_command` observes `MailboxClosed`. The triple will
shortly have a fresh healthy actor (reactivation), so — per Judgment Day WARNING JD-2 and
spec FR-010 — this must be **distinguishable from permanent failure**, not a dead end.

**Decision.** `MailboxClosed` is surfaced as its **own distinct `EntityError` variant**
(it already exists — `error.rs:34`), never collapsed into `Internal`/`EntityNotActive`.
This is the FR-010 distinguishability contract: the caller can tell "the actor I was
routed to is tearing down — retry `entity_ref()` to reach the next activation" apart from
a genuinely permanent error (recovery failure → `EntityNotActive`; type mismatch →
`Internal`, ADR-002). `TokioEntityRef` does **not** auto-retry internally — no hidden loop
that could re-spawn actors if a caller always races the guard; the retry is the *caller's*
explicit re-call of `entity_ref()`, which either finds the still-closing entry (retry
again — bounded, because `deactivate()` removes it synchronously as part of the same
teardown) or finds no entry and spawns a fresh actor per ADR-001.

**Presence-only lookup is sufficient — justified by FR-009.** ADR-001's "live entry?" check
stays **presence-based** (entry exists → route to its mailbox); it does *not* need to
consult published state to reject a closing entry. Round 1's Vector 5 flagged this window,
but the FR-009 guarantee (ADR-005) makes it **safe to retry through**: a send into a closed
mailbox fails *fast and observably* with `MailboxClosed` rather than hanging, and every
already-enqueued command is terminally answered by the guard drain — so no command is ever
stranded in the window, and the only caller-visible effect is one retryable error. Adding
state-aware lookup (spawn-over-a-dying-entry) would buy nothing the epoch + retry don't
already give, at the cost of more routing logic, so it is rejected.

**Consequences.**
- No new retry/backoff logic to design; `MailboxClosed` stays a distinct, already-defined
  variant, and FR-010's retry scenario is satisfied by the caller re-calling `entity_ref()`.
- The teardown-to-removal window is a "your ref went stale, look up again" signal, and
  FR-009 guarantees no enqueued command is lost while it is open.

---

## Crate / module layout after this change

```
crates/persistent-entity/Cargo.toml   MODIFIED  add `parking_lot` dependency (ADR-001, ADR-005)
crates/persistent-entity/src/
  registry.rs          MODIFIED  active: triple → { erased mailbox, watch::Receiver<EntityState>, epoch:u64 };
                                 parking_lot::Mutex (non-poisoning, no .unwrap); monotonic epoch counter;
                                 lookup(), insert-if-absent (single-flight), deactivate_if_mine(epoch),
                                 active_count() over state cells
  runtime.rs           MODIFIED  entity_ref() = lookup-or-spawn, now returns Result<_, EntityError> (ADR-002)
  entity_ref_tokio.rs  MODIFIED  TokioEntityRef::new → split: lookup(+downcast, may Err) vs spawn;
                                 delete eager mark_active (:119) and SpawnGuard type (:30-39);
                                 drop guard's Drop now calls the shared deactivate() (ADR-005)
  actor.rs             MODIFIED  publish state via watch::Sender on each transition; single deactivate()
                                 via drop guard (close→sync-drain→remove-if-mine→publish terminal);
                                 remove direct remove_active calls (:320,328)
  mailbox.rs           MODIFIED  queue → Arc<parking_lot::Mutex<VecDeque<T>>> (was tokio::sync::Mutex,
                                 so it can be locked from a sync Drop); add sync close_and_drain() (ADR-005)
  activation.rs        REMOVED   SharedActivation subsumed by registry map mutex (ADR-001)
  supervisor.rs        REMOVED   dead, broken, redundant (ADR-006)
  lib.rs               MODIFIED  ensure no mod activation/supervisor (already absent)
  lifecycle.rs         UNCHANGED EntityState / transitions reused as-is
  error.rs             UNCHANGED reuse existing MailboxClosed + Internal variants (ADR-002, ADR-008)
tests/
  activation_ordering_tests.rs   MODIFIED  multi_thread flavor; tighten active_count assertions to == 1
  guaranteed_completion_tests.rs NEW       FR-009: panic/cancel/shutdown drains + answers all enqueued
ARCHITECTURE.md        MODIFIED  ADR-007
```

---

## Caller impact analysis (proposal Risks 1 & 2)

There are **no non-test callers** of `entity_ref()` or `active_count()` in the
workspace. `runtime.rs:129/161` are the definitions; `entity_ref_tokio.rs:112-118` are
comments; all invocations live in three test files. Enumerated:

### `entity_ref()` callers

| Location | Same triple called >1×? | Behavior change | Result |
|----------|-------------------------|-----------------|--------|
| `activation_ordering_tests.rs` (`:50,71,101,129,163,201,220,254,287,324,368,403,423,451,452`) | `entity-6` via 10 tasks (`:220`); `entity-5` via helper (`:177`) | Coalesce to **one** actor instead of N | Intended fix; sends still succeed |
| `persistence_failure_tests.rs` (`:107,143,171,196,225,242,265,266`) | `reactivate-1` (`:225` then `:242`) across passivation | r1's entry evicted on passivation (ADR-005); r2 spawns fresh | Unchanged observable result (value 15) |
| `real_actor_path_tests.rs` (`:190`) | No | Single spawn either way | Unchanged |

Every single-call site is behaviorally unchanged (one triple, one actor). Every
multi-call-same-triple site *now* correctly coalesces — the pinned visible contract is:
**callers of `entity_ref()` for the same live triple get the same mailbox**; a call for
a triple with no live actor spawns one. `entity_ref()` stays cheap (a clone on the hot
path; a spawn only on cold/reactivation).

**Signature change (ADR-002).** `entity_ref()` now returns `Result<impl EntityRef, EntityError>`
instead of `impl EntityRef`. Since all call sites are tests and each is followed by a
`send_command().await`, they adapt mechanically with `.unwrap()`/`?` at the `entity_ref()`
call. No production caller exists, so the blast radius is limited to the three test files
above; the only new `Err` path is the never-in-practice downcast mismatch.

### `active_count()` callers

| Location | Expectation | New semantics | Result |
|----------|-------------|---------------|--------|
| `real_actor_path_tests.rs:211` | `0` after failed recovery | Failed actor publishes `Failed` + `deactivate()` ⇒ 0 | PASS (now correct by construction) |
| `persistence_failure_tests.rs:200` | `1` after an awaited command | Actor is `Active` once command reply returns ⇒ 1 | PASS |
| `persistence_failure_tests.rs:214` | `0` after passivation | `deactivate()` on exit ⇒ 0 | PASS |
| `persistence_failure_tests.rs:274` | `2` for two awaited-active entities | Two `Active` publishers ⇒ 2 | PASS |
| `activation_ordering_tests.rs:187,237` | `<= 2` | Single actor ⇒ 1 | PASS; **tighten to `== 1`** per success criteria |
| `activation_ordering_tests.rs:272` | `> 0` | Some `Active` ⇒ > 0 | PASS |

**Risk 2 resolution — no test relies on eager `active_count()`.** Grep confirms every
count assertion is preceded by an *awaited* `send_command` (which returns only after the
actor reached `Active`) or by an explicit passivation wait loop. No test reads
`active_count()` synchronously right after `entity_ref()` expecting pre-recovery
visibility. So moving visibility to trail recovery (ADR-003) breaks none of them. The
one nuance at `:200`: after the awaited command, the fast-passivation timeout is not
set (default 300 s), so no passivation race; where fast timeout is used
(`:190-216`) the assertion is `1` *before* the passivation wait loop — safe.

---

## Data Flow (after)

```
entity_ref::<C,S>(triple) -> Result<impl EntityRef, EntityError>
  └─ registry.map.lock()  (parking_lot)  ── single-flight critical section (sync, no .await, no spawn, no panic-prone call)
       ├─ live entry?  ── yes ─▶ clone erased Arc<dyn Any>            (infallible)
       └─ no ─▶ (tx,rx) = watch::channel(Recovering); mailbox = BoundedMailbox::new()
                insert entry { erased mailbox, rx, epoch = next_epoch() }
     ── lock released ──                                    (Round 3 fix: spawn moved here, out of the lock — see ADR-001)
       ├─ (from live-entry branch) downcast Arc<dyn Any> → BoundedMailbox<ActorEnvelope<C>>  (ADR-002, outside lock)
       │    ├─ ok       ─▶ Ok(TokioEntityRef { same mailbox })
       │    └─ mismatch ─▶ Err(Internal("routing type mismatch"))    (never "no live entry" → never a 2nd spawn)
       └─ (from spawn branch)  tokio::spawn(actor.run())  ── guard{epoch, tx} moved in  ─▶ Ok(TokioEntityRef { new mailbox })
              (spawn-panic here drops guard with the map lock already free — deactivate() cleans up safely, no deadlock)

actor.run()  (identical for cold and reactivation — ADR-004)
  recover_state()  → tx.send(Active)        ── now counted by active_count()
  process_commands()  (FIFO, single writer)
  passivate() / failure / panic / cancel / shutdown
  guard.drop() → deactivate()  (sync, runs on EVERY exit path — ADR-005, FR-009):
       1. mailbox.close()                            (sync atomic + notify)
       2. for env in mailbox.close_and_drain():      (sync parking_lot lock + VecDeque take)
              env.reply.send(Err(EntityNotActive))   (sync oneshot — answers every QUEUED command)
       3. deactivate_if_mine(epoch): remove entry + tx.send(terminal)
     (the IN-FLIGHT command at panic time: its reply Sender drops on unwind → rx.await = Err → terminal)

active_count() = count of entries whose published state == Active   (ADR-003, D1)
```

## Testing Strategy

| Layer | What | How |
|-------|------|-----|
| Concurrency | 20-caller probe, cold triple | `#[tokio::test(flavor="multi_thread")]`; 20 tasks `entity_ref()`+`send_command()` one id; assert 0 optimistic-concurrency conflicts, exactly 1 actor spawned, `active_count()==1` |
| Concurrency | Same probe, reactivation | Seed history, passivate, then the 20-caller burst; same assertions (D3 coverage) |
| Actor-level count | Actor spawns, not ID-set size | Instrument spawn count; assert `spawns == 1` (finding 2 — ID set could not detect duplicates) |
| Visibility | No pre-recovery active | Insert-then-block recovery; assert `active_count()==0` while `Recovering`, `==1` after `Active` |
| Rollback | Fail-closed | Force recovery failure under N concurrent callers; assert no residual entry, `active_count()==0`, all N get the same error |
| Type safety | Downcast mismatch never duplicates | Force an erased-handle type mismatch (test-only second `C` for the same `entity_type`); assert `entity_ref()` returns `Err`, live actor A unchanged, no second spawn (ADR-002, CRITICAL 1) |
| Poison safety | One triple's in-section panic doesn't brick the registry | Force a panic inside the map critical section itself (construction failure); assert `entity_ref()`/`active_count()` for OTHER triples still work (ADR-001, CRITICAL 2) |
| Deadlock safety (Round 3) | `tokio::spawn` panic (no runtime context) doesn't self-deadlock the registry | Force `tokio::spawn` to panic for triple X (e.g. call `entity_ref()` outside a Tokio runtime); assert the call for X fails/cleans up (no zombie entry) AND `entity_ref()`/`active_count()` for OTHER triples, called from OTHER threads concurrently, are not blocked — proves the lock is released before spawn, not just that spawn doesn't poison it |
| Completion (FR-009) | Panic while `Active` answers all N queued | Actor `Active`, enqueue N commands behind one whose handler panics; assert all N callers' `rx.await` resolve to a terminal `Err`, none hangs |
| Completion (FR-009) | Panic while `Recovering` / runtime shutdown | Enqueue a command, then panic in recovery (or drop the runtime) before `Active`; assert the enqueued caller observes a terminal outcome, not a hang |
| Completion (FR-009) | Panic mid-passivation-drain answers the remainder | Close mailbox with M commands queued, panic partway through the drain loop; assert the *undrained* remainder still gets a terminal reply (guard drain, independent of the loop) |
| Completion (FR-009) | 20-caller probe under recovery-time panic | multi_thread; 20 tasks `entity_ref()`+`send_command()`; recovery panics partway; assert all 20 `rx.await` resolve (none hangs on an unresolved `oneshot`) |
| Retry (FR-010) | `MailboxClosed` is distinguishable + retryable | In the close→remove window, `send_command` observes `MailboxClosed` (distinct variant); caller re-calls `entity_ref()` and reaches the next healthy actor |
| Regression | Existing suites | `activation_ordering_tests` retightened to `==1`; `persistence_failure_tests` unchanged expectations pass |

## Open Questions

None. The two former open questions are now firm decisions:

- **State-cell mechanism** — decided `watch::channel<EntityState>` (ADR-003), chosen
  because its initial value is fixed at construction, making ADR-003's "actor is the only
  writer" literally true (the caller supplies the seed, never writes post-construction).
- **Teardown identity** — decided monotonic per-entry `u64` epoch (ADR-005): ABA-safe by
  monotonicity, avoids the freed-address-reuse hazard of raw-pointer identity, and is cheap
  defense-in-depth even though single-flight makes clobbering structurally impossible today.

---

## Judgment Day Resolution

Audit trail: three rounds of adversarial review against the prior drafts. Rounds 1–2
(4 judges, safety + liveness lenses) found 3 CRITICAL architectural gaps and 2
WARNING(real) findings in the original design; the resulting fixes were themselves
verified in Round 3 (2 judges, fix-verification lens), which found one further
CRITICAL — a self-deadlock created by the interaction of two of the Round 1–2 fixes,
not present in either fix alone. Each is mapped to the ADR/section that resolves it.

| # | Finding | Resolved by |
|---|----------------------------------|-------------|
| **CRITICAL (Round 3)** | Combining the Round-2 `parking_lot::Mutex` fix (ADR-001, non-poisoning) with the Round-1/2 guard-in-`Drop` fix (ADR-005) created a NEW self-deadlock: `tokio::spawn` panics on a missing runtime context (a real precondition, not contrived); if that panic fires while the future — already holding the teardown guard — is still under the map lock, LIFO unwind drops the guard *before* the lock is released, and `deactivate()` tries to re-lock the same, still-held, non-reentrant mutex on the same thread. Result: the entire registry hangs (every triple, not just the failing one) — worse than the poisoning bug ADR-001 fixed. Found independently, with matching traces, by both Round 3 judges. | **ADR-001 (amended)** — `tokio::spawn` moved to run strictly *after* the map lock is released; step 3 (insert) and step 4 (spawn) are no longer under the same critical section. A spawn-time panic now drops the guard with the lock already free, so `deactivate()` safely removes the zombie entry instead of deadlocking. Tested: "Deadlock safety (Round 3)." |
| **CRITICAL 1** | ADR-002's downcast-mismatch fallback ("treat as no live entry") spawns a **second** actor for a live triple, overwriting the first's map slot — two actors per triple, violates FR-001. | **ADR-002** — downcast mismatch on a live entry now returns an explicit `Err(EntityError::Internal(..))` after the lock is released; the second `tokio::spawn` never runs. Satisfies spec FR-001 *Type-mismatch resolution fails closed*. Tested: "Downcast mismatch never duplicates." |
| **CRITICAL 2** | Mailbox construction / `tokio::spawn` panic **inside** the `std::sync::Mutex` critical section poisons the registry process-wide — every later `entity_ref()`/`active_count()` `.lock().unwrap()` panics forever. | **ADR-001** — registry map (and mailbox queue) switch to `parking_lot::Mutex` (no poison, no `.unwrap()`); an in-section panic unwinds to the one caller and every other triple keeps routing. Rejected `catch_unwind`+`clear_poison` as more fragile. Tested: "Poison safety." |
| **CRITICAL 3** | `deactivate()` never closes/drains the mailbox; a panic in recovery/command/drain, cancellation, or shutdown skips the in-body `close()`/drain, stranding already-enqueued `oneshot`s forever. Round 2: even when `close()` runs, a panic mid-drain strands the queued remainder. | **ADR-005 (rewritten)** — `deactivate()` runs from the guard's `Drop` (only code guaranteed on every exit) and **synchronously** closes + drains the queue (`parking_lot` lock + `VecDeque` take) replying `Err` to every remaining `oneshot`; the in-flight command is answered by its `Sender` dropping on unwind. Independent of `passivate()`'s loop, so it survives that loop's own panic. Satisfies spec **FR-009**. Tested: 4 Completion rows. |
| **WARNING — ABA / identity** | Open Question left pointer-identity vs epoch undecided; raw cached address is an ABA hazard on allocator reuse; under single-mutex the check is arguably redundant. | **ADR-005** — decided monotonic `u64` epoch: ABA-safe (never reused), kept as defense-in-depth for the FR-009 Drop-driven teardown paths even though single-flight makes the base case moot. |
| **WARNING — single-writer purity** | ADR-003's "actor is the ONLY writer" was aspirational: `entity_ref()` (caller thread) seeds `Recovering`, a write from outside the actor if the cell were `Arc<Mutex<EntityState>>`. | **ADR-003** — decided `watch::channel`: the `Recovering` seed is the channel *constructor argument*, not a post-construction `send()`; the caller never writes the cell and the sole `Sender` is moved into the actor. "Only writer" is now literally true. |

Bonus: JD-2 (`MailboxClosed` indistinguishable from permanent failure during a
passivation→reactivation handoff) is resolved by **ADR-008** — `MailboxClosed` is a
distinct, caller-retryable terminal, and FR-009's guaranteed drain makes the
close→remove window safe to retry through. Satisfies spec **FR-010**.
