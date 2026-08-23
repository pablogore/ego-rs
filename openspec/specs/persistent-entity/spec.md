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

### FR-011 — Handler-Reachable External Data Access

A `PersistentEntity` handler MUST be able to obtain external data during
command handling through a capability that `persistent-entity` exposes to
it, without depending on runtime-internal types or constructing an
external client inline. That capability is backed by whichever provider
the surrounding application has registered for a given key (see
`external-data-providers`); `persistent-entity` is not the registration
owner and does not implement provider logic — it exposes the
handler-reachable surface and obtains its backing from the runtime.

#### Scenario: Handler fetches external data during command handling
- GIVEN a handler's command-handling code needs data from a registered
  external data provider
- WHEN it invokes the fetch capability `persistent-entity` exposes to it
- THEN it receives the provider's response without depending on any
  runtime-internal type or constructing an external client inline

### FR-012 — Missing Registration Fails Closed From the Handler's Perspective

When a handler fetches external data for a key with no registered
provider, `persistent-entity`'s exposed fetch capability MUST surface that
failure to the handler explicitly — never a silent default, empty value,
or no-op result. (Registration and resolution semantics themselves are
`external-data-providers`'s fail-closed resolution requirement; this
requirement only fixes that the failure is observable through the
persistent-entity-owned surface a handler actually uses.)

#### Scenario: Handler observes an explicit error for an unregistered key
- GIVEN no provider is registered for key `K`
- WHEN a handler fetches external data for `K` through `persistent-entity`'s
  exposed fetch capability
- THEN the handler receives an explicit error, never a silent default or
  empty result

### FR-013 — Fetch Attempts Are Observable

Every fetch a handler makes through `persistent-entity`'s exposed
capability MUST be observable through the runtime's existing observability
pipeline (see `external-data-providers`'s observability requirement for
the exact signal set) — `persistent-entity` introduces no separate or
bypassing telemetry path of its own.

#### Scenario: A handler's fetch is observable through the existing pipeline
- GIVEN a handler fetches external data through `persistent-entity`'s
  exposed capability
- WHEN the fetch completes
- THEN a signal is emitted through the runtime's existing observability
  pipeline, never a `persistent-entity`-local or bypassing one

### FR-014 — Existing Handlers Unaffected

An existing `PersistentEntity` implementation that never uses the fetch
capability MUST continue to compile and behave exactly as before this
capability exists — the capability is additive and opt-in from the
handler's point of view.

#### Scenario: Unmodified handler compiles and passes unchanged
- GIVEN an existing handler that never uses the fetch capability
- WHEN the workspace is rebuilt after this capability ships
- THEN it compiles and its existing tests pass without modification

### FR-015 — Receipt Consultation Gates Dispatch and Recovery

Before dispatching a command carrying an `operation_key` to a `PersistentEntity`
handler, the actor MUST consult that aggregate's persisted receipt for
`(tenant_id, aggregate_type, aggregate_id, operation_key)`. If a receipt exists
with a matching fingerprint, the actor MUST no-op (return the receipt's
recorded outcome) rather than re-invoking `handle_command`. If a receipt
exists with a different fingerprint, the actor MUST return a permanent
conflict and MUST NOT invoke `handle_command`.

#### Scenario: Already-applied operation no-ops instead of re-executing
- GIVEN a receipt exists for `(tenant, User, user-7, K)` with fingerprint F
- WHEN a command carrying key K and fingerprint F is dispatched to the actor
  for `user-7`
- THEN `handle_command` is never invoked; the actor returns the receipt's
  recorded outcome

#### Scenario: Fingerprint mismatch is a permanent conflict, not a re-execution
- GIVEN a receipt exists for `(tenant, User, user-7, K)` with fingerprint F
- WHEN a command carrying key K and a different fingerprint F' is dispatched
- THEN the actor returns a permanent conflict; `handle_command` is not invoked

### FR-016 — Zero-Event Branch Opens a Transaction to Confirm a Receipt

The actor's zero-event success branch (today: `CommandResult::NoEvents`,
never opening a transaction) MUST open a transaction to durably confirm the
operation's receipt for that aggregate, even though no event is appended.

#### Scenario: A zero-event success still produces a durable receipt
- GIVEN a command whose `handle_command` returns no events (e.g. an
  already-idempotent domain-level "Ensure")
- WHEN the actor completes the command
- THEN a transaction opens and confirms the receipt for that aggregate and
  operation key, where previously no transaction was opened at all

### FR-017 — CommandContext Carries the Operation Key

`CommandContext` MUST carry the `OperationKey` established at ingress through
to the actor and its receipt-consultation/confirmation logic.

#### Scenario: Operation key reaches the actor unchanged
- GIVEN an `OperationKey` established at HTTP ingress for a command
- WHEN the command reaches `EntityActor::execute_command` via
  `CommandContext`
- THEN the identical `OperationKey` value is available for receipt lookup
  and confirmation

### FR-018 — Aggregate Identity Is Structurally Distinct, Not Concatenated

`EntityTriple::aggregate_id()` (or its replacement) MUST expose
`aggregate_type` and `aggregate_id` as distinct identity components rather
than producing a single concatenated string (e.g. via a hyphen join) for
persistence. Two different `(aggregate_type, aggregate_id)` pairs that would
collide under the previous concatenation scheme MUST resolve to distinct
persisted streams.

#### Scenario: Previously-colliding pairs no longer collide
- GIVEN aggregate type `user-account` with id `7`, and aggregate type `user`
  with id `account-7`
- WHEN both are persisted through `EntityTriple`
- THEN they resolve to two distinct persisted streams, not one shared string

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
