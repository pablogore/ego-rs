# Persistent Entity — Activation Authority & Linearizability Specification

## Purpose

Defines observable contracts for entity activation in `persistent-entity`:
single activation authority, single source of truth for "active,"
deterministic visibility, linearizable activation, fail-closed failure, one
rollback contract, and guaranteed completion for already-enqueued commands
regardless of termination cause — applying uniformly to cold activation and
reactivation-from-`Passivated`. The entity activation system is the
authoritative reference for lifecycle state and coordination.

---

## Requirements

### FR-001 — Single Activation Authority Per Triple

Exactly one live actor MUST exist per entity triple `(TenantId, EntityType,
EntityId)` at any time. All callers of `entity_ref()` (or equivalent) for the
same triple MUST resolve to the same mailbox.

#### Scenario: Concurrent callers converge on one actor
- GIVEN no actor exists yet for triple T
- WHEN N concurrent callers request activation of T
- THEN exactly one actor is spawned and all N callers route to its mailbox

#### Scenario: Existing actor is reused, not duplicated
- GIVEN an actor is already `Active` for T
- WHEN another caller requests T
- THEN no new actor is spawned; the caller routes to the existing mailbox

#### Scenario: Type-mismatch resolution fails closed, never spawns a competitor
- GIVEN a live registry entry exists for T
- WHEN entity-type resolution against that entry fails (e.g. a
  type/command-shape mismatch)
- THEN the call fails with an explicit error; this MUST NOT be treated as
  "no live entry," and no competing actor for T MUST be spawned

#### Scenario: Removal is authority-scoped
- GIVEN an actor currently occupies T
- WHEN any exit path attempts to remove T's registry entry
- THEN removal succeeds only if that exit path is the one currently
  occupying T; a stale or superseded exit path's removal attempt MUST NOT
  remove the live entry

### FR-002 — Single Source of Truth for "Active"

`active_count()` (and any equivalent externally-visible query) MUST count
only entities whose state is `EntityState::Active`. `Recovering`,
transitional, or duplicate registry entries MUST NOT be counted or exposed as
active.

#### Scenario: Recovering entity excluded from active_count
- GIVEN T is spawned and currently `Recovering`
- WHEN `active_count()` is queried
- THEN T is not included

#### Scenario: Active entity counted exactly once
- GIVEN T's state is `Active`
- WHEN `active_count()` is queried
- THEN T is included exactly once, regardless of how many handles exist

### FR-003 — Deterministic Activation Visibility, Cold Path

An entity activating from no prior state (`∅ → Recovering → Active`) MUST NOT
be externally observable as active before reaching `Active`.

#### Scenario: Cold activation invisible until Active
- GIVEN T has never been activated
- WHEN activation is triggered and recovery is still in progress
- THEN active queries do not report T; once state is `Active`, they do

### FR-004 — Deterministic Activation Visibility, Reactivation Path

Reactivation (`Passivated → Recovering → Active`) MUST satisfy the identical
guarantee as FR-003. The visibility contract MUST NOT differ by origin.

#### Scenario: Reactivation invisible until Active
- GIVEN T was previously `Active` and is now `Passivated`
- WHEN a command triggers reactivation and recovery is still in progress
- THEN active queries do not report T; once state is `Active`, they do,
  identically to the cold path in FR-003

### FR-005 — Linearizable Activation

Concurrent activation attempts for the same triple — cold or
reactivation-from-`Passivated` — MUST resolve as if sequential: exactly one
attempt wins (spawns/recovers); all other concurrent attempts MUST converge
on that winner's result instead of independently spawning or recovering.

#### Scenario: Concurrent cold activations linearize to one winner
- GIVEN T has never been activated
- WHEN M callers concurrently trigger activation of T
- THEN exactly one activation occurs; all M callers observe its outcome

#### Scenario: Concurrent reactivations linearize to one winner
- GIVEN T is `Passivated`
- WHEN M callers concurrently send commands that would each trigger reactivation
- THEN exactly one reactivation occurs; all M callers observe its outcome

### FR-006 — Fail-Closed Activation Semantics

Activation failure, cold or reactivation, MUST leave no residual actor,
mailbox, or registry entry. The entity returns to "never attempted" (or an
explicit failed terminal state) — never a partially-registered state.

#### Scenario: Failed cold activation leaves no residue
- GIVEN T has never been activated
- WHEN activation is attempted and recovery fails
- THEN no actor, mailbox, or registry entry for T remains; `active_count()`
  excludes T

#### Scenario: Failed reactivation leaves no residue
- GIVEN T is `Passivated`
- WHEN reactivation is attempted and recovery fails
- THEN no duplicate actor, mailbox, or registry entry for T remains;
  `active_count()` excludes T

### FR-007 — One Deterministic Rollback Contract

Activation failure, regardless of cause (recovery error, spawn failure,
cancellation) or origin, MUST be handled by exactly one rollback contract. No
independent cleanup paths whose outcomes can diverge MUST exist.

#### Scenario: Rollback is uniform across failure causes
- GIVEN two activation attempts fail for different reasons (recovery I/O
  error vs. spawn failure)
- WHEN each failure is handled
- THEN both leave the identical observable end-state defined in FR-006; no
  path leaves the registry and actor lifecycle in disagreement

### FR-008 — 20-Caller Concurrent Activation Probe

A 20-caller concurrent activation probe against a single entity triple MUST
produce exactly one actor and zero optimistic-concurrency conflicts.

#### Scenario: 20 concurrent callers, one actor, zero conflicts
- GIVEN T has never been activated
- WHEN 20 concurrent callers each call `entity_ref()` and `send_command()`
  against T under a multi-threaded Tokio runtime
- THEN exactly 1 actor exists for T (actor-level assertion, not ID-set
  cardinality) and 0 optimistic-concurrency conflicts occur across all 20 calls

### FR-009 — Guaranteed Completion for Enqueued Commands

For every command successfully enqueued on a triple's mailbox, the owning
actor's termination — by ANY means (normal passivation, recovery failure,
panic during command handling/recovery/passivation, task cancellation, or
runtime shutdown) — MUST result in that command's caller eventually
observing a terminal outcome: a successful reply, a domain error, or
`MailboxClosed`/an equivalent terminal transport error. No termination path
MUST leave an enqueued command permanently unanswered.

#### Scenario: Panic mid-processing answers every already-enqueued caller
- GIVEN an actor is `Active` for T with N commands already enqueued behind
  the one currently being processed
- WHEN the actor panics while processing that command
- THEN all N queued callers, not only the one being processed at panic
  time, eventually observe a terminal outcome

#### Scenario: Panic or cancellation mid-passivation-drain still answers the undrained remainder
- GIVEN T's mailbox has been marked closed for passivation and some
  pre-closure-enqueued commands remain undrained
- WHEN the actor panics or is cancelled mid-drain
- THEN the undrained commands still eventually receive a terminal outcome,
  not only the ones drained before the failure

#### Scenario: Runtime shutdown during Recovering still answers already-enqueued callers
- GIVEN T is `Recovering` (not yet `Active`) with a caller's command
  already enqueued
- WHEN the runtime shuts down before T reaches `Active`
- THEN that caller eventually observes a terminal outcome rather than
  hanging indefinitely

#### Scenario: 20-caller probe under a recovery-time panic
- GIVEN T has never been activated
- WHEN 20 concurrent callers each call `entity_ref()` and `send_command()`
  against T under a multi-threaded Tokio runtime, and recovery panics
  partway through
- THEN all 20 callers eventually observe a terminal outcome — none hangs on
  a `oneshot` that never resolves

### FR-010 — MailboxClosed Retry Contract

During the window between an actor beginning teardown (mailbox closed) and
its registry entry actually being removed, a concurrent
`entity_ref()`/`send_command()` caller MAY observe `MailboxClosed` even
though the triple will shortly have a fresh, healthy actor. `MailboxClosed`
observed in this window MUST NOT be treated as a terminal "entity
unreachable" signal — the caller MUST be able to distinguish it from
genuine permanent failure and retry `entity_ref()` to reach the next
activation.

#### Scenario: Retry across a passivation-to-reactivation handoff succeeds
- GIVEN T is mid-handoff: its old actor's mailbox is closed but its
  registry entry has not yet been removed
- WHEN a caller sends a command and observes `MailboxClosed`
- THEN the caller retries `entity_ref()` and successfully reaches the
  newly-activated actor for T

---

## Test Coverage Requirements (NFR)

| # | Requirement |
|---|---|
| NFR-001 | Concurrency tests for this capability MUST use Tokio's multi-threaded runtime flavor; `current_thread` MUST NOT be used to validate any requirement above. |
| NFR-002 | "No duplicate actor" MUST be asserted at the actor-task level (spawn counters, task handles, or equivalent instrumentation) — asserting only `active_count()` bounds or an ID-set's cardinality is NOT sufficient coverage. |
| NFR-003 | FR-003, FR-004, and FR-005 MUST each have test coverage for both the cold-activation path and the reactivation-from-`Passivated` path individually. |

---

## Non-Goals

- Runtime bootstrap, dependency injection, transports, schedulers/scheduling
  policy, clustering, persistence protocol, authorization, tenancy, telemetry.
- Renaming existing concepts (e.g. "registry") — misleading terminology is
  documented, not renamed, by design.
- Prescribing synchronization primitives or algorithms — a design-phase
  decision.
- Prescribing the identity/ABA-safety mechanism behind "remove only if I'm
  still the current occupant of T" (pointer identity, epoch, generation
  counter, or equivalent) — FR-001's removal-authority scenario fixes the
  observable contract; the mechanism is a design-phase decision.
