# Delta for external-effects

This delta adds a durable, production-grade backend for the reliable-effects
subsystem and the port capabilities a durable, potentially multi-consumer store
requires, while retaining `InMemoryEffectStore` as the reference implementation.
It preserves the at-least-once (never exactly-once) guarantee and the unordered
delivery contract.

## ADDED Requirements

### Requirement: Durable Effect State Survives Crash and Restart

A durable `EffectStateStore` implementation MUST persist every accepted effect's
lifecycle state (`Pending`, `InFlight`, `Succeeded`, `RetryableFailed`,
`TerminalFailed`), its `attempt`, `next_at`, `tenant`, and `description` to
storage that outlives the process, so that after a crash and restart the
subsystem can reconstruct every not-yet-`Succeeded`/`TerminalFailed` effect and
(re-)dispatch it. A `Pending` or `InFlight` effect present before a crash MUST
still be present and dispatchable after restart. The reference in-memory store is
explicitly exempt (it loses state on crash, as already specified).

#### Scenario: A pending effect survives a restart

- GIVEN a durable `EffectStateStore` holding an accepted effect in state
  `Pending`
- WHEN the process crashes and a new process reopens the same durable store
- THEN the effect is still present in state `Pending` and is claimable for
  dispatch

#### Scenario: A succeeded effect is not re-dispatched after a restart

- GIVEN a durable `EffectStateStore` in which an effect reached `Succeeded`
  before the crash
- WHEN a new process reopens the store and asks for due effects
- THEN the `Succeeded` effect is not returned for dispatch

#### Scenario: The in-memory reference store remains exempt

- GIVEN the shipped `InMemoryEffectStore`
- WHEN the process crashes with a `Pending` or `InFlight` effect present
- THEN that effect is lost, exactly as already documented, and this durable
  requirement does not apply to it

### Requirement: Atomic Claim With Lease Prevents Concurrent Double Delivery

A durable store MUST offer an atomic claim operation that, in one step, selects
due effects AND transitions each to `InFlight` under a lease (visibility
timeout), such that a claimed effect is invisible to any other consumer's claim
until its lease is renewed or expires. Two consumers claiming concurrently MUST
receive disjoint sets of effects; the same effect MUST NOT be handed to two
consumers at the same time. This atomic-claim capability is in addition to — and
MUST NOT alter — the existing non-atomic, single-consumer `claim_due`, which
remains valid for the reference store and single-consumer deployments.

#### Scenario: Concurrent consumers receive disjoint claims

- GIVEN a durable store with several due effects and two consumers claiming
  concurrently
- WHEN both issue an atomic claim at the same time
- THEN each receives a disjoint subset and no effect appears in both results

#### Scenario: A leased effect is invisible to another consumer while its lease is live

- GIVEN one consumer holds a live lease on a claimed effect
- WHEN a second consumer issues an atomic claim before that lease expires
- THEN the leased effect is not returned to the second consumer

#### Scenario: Existing non-atomic claim_due is preserved

- GIVEN a single-consumer deployment or the reference `InMemoryEffectStore`
- WHEN it uses the existing `claim_due` (which returns due effects without
  transitioning them, requiring a separate `mark_in_flight`)
- THEN that behavior is unchanged; the atomic-claim capability is additive and
  does not modify `claim_due`'s contract

### Requirement: Lease Renewal and Expiry Reclaim

A consumer holding a lease MUST be able to renew it to extend its exclusive
claim while still delivering an effect. A lease that is not renewed before its
deadline MUST expire, after which the effect becomes claimable again so a dead or
stalled consumer's effect is redelivered rather than stranded. A renewal attempt
for a lease that has already expired (and whose effect may have been reclaimed by
another consumer) MUST fail or return a distinguishable non-success — it MUST NOT
silently succeed, so a consumer that lost its lease cannot later act on an effect
it no longer owns. Reclaiming an expired-lease effect MUST preserve the effect
record; it MUST NOT drop or lose it.

#### Scenario: Renewal extends an exclusive claim

- GIVEN a consumer holding a live lease on an effect
- WHEN it renews the lease before the deadline
- THEN the lease deadline is extended and the effect stays invisible to other
  consumers

#### Scenario: An expired lease makes the effect claimable again

- GIVEN a consumer that claimed an effect but stopped renewing (e.g. it crashed)
- WHEN the lease deadline passes
- THEN the effect becomes claimable by another consumer and is redelivered,
  never stranded, and the effect record is preserved

#### Scenario: Renewing an already-expired lease does not silently succeed

- GIVEN a consumer whose lease already expired and whose effect was reclaimed by
  another consumer
- WHEN it attempts to renew that lease
- THEN the renewal fails or returns a distinguishable non-success, so the
  original consumer does not act on the reclaimed effect

### Requirement: Durable Crash Recovery Reclaims Only Expired-Lease In-Flight Effects

On a durable store, crash recovery MUST return an `InFlight` effect to a
claimable state only when its lease has expired. An `InFlight` effect whose lease
is still live (a peer consumer is actively delivering it) MUST NOT be reclaimed
by recovery, so recovery never double-delivers an effect a live consumer is
mid-delivering. Recovery MUST preserve the effect record in every case. The
in-memory store's blanket in-flight → pending recovery remains valid only because
a single process owns all of its state.

#### Scenario: Expired-lease in-flight effect is recovered to claimable

- GIVEN a durable store with an `InFlight` effect whose lease has expired
- WHEN crash recovery runs at restart
- THEN that effect becomes claimable again and its record is preserved

#### Scenario: Live-lease in-flight effect is not stolen by recovery

- GIVEN a durable store with an `InFlight` effect whose lease is still live
  under a peer consumer
- WHEN crash recovery runs
- THEN that effect is left to its current consumer and is not reclaimed

#### Scenario: In-memory blanket recovery remains valid

- GIVEN the reference `InMemoryEffectStore`, owned by a single process
- WHEN `recover_in_flight` runs at startup
- THEN every `InFlight` effect returns to `Pending`, unchanged from its existing
  behavior

### Requirement: State Transitions Are Transactional With a Documented Boundary

A durable store MUST perform each effect-state transition within a transaction
whose boundary is documented, committing before the corresponding port method
returns `Ok`. When a state transition and a dedup outcome must be consistent
(e.g. marking an effect `Succeeded` and committing its dedup reservation), the
two writes MUST be committed atomically together, so a partial write can never
leave a committed dedup reservation without its corresponding state, or the
reverse. A transaction that cannot commit MUST surface as an `EffectStoreError`,
classified transient vs permanent per the existing error taxonomy: recoverable
contention (serialization failure, deadlock, lock or pool timeout) MUST map to
`TemporarilyUnavailable` (retryable); a permanent backend fault (corruption,
schema mismatch, constraint violation that is not a dedup conflict) MUST map to
`Backend` (permanent); a dedup/optimistic conflict MUST map to `Conflict`.

#### Scenario: A rollback leaves no partial state or dedup write

- GIVEN a durable store transition that fails mid-transaction
- WHEN the transaction rolls back
- THEN neither the state row nor the dedup reservation reflects the failed
  transition — no partial write is observable

#### Scenario: Transient contention is reported as retryable

- GIVEN a transition that fails on a serialization failure, deadlock, or pool
  timeout
- WHEN the error is returned
- THEN it is `EffectStoreError::TemporarilyUnavailable`, which the delivery
  runner may retry

#### Scenario: A permanent fault is reported as non-retryable

- GIVEN a transition that fails on corruption or a schema mismatch
- WHEN the error is returned
- THEN it is `EffectStoreError::Backend`, which the delivery runner MUST NOT
  auto-retry

### Requirement: Durable Dedup Reservations Persist With Unchanged Scope

A durable `EffectDedupStore` implementation MUST persist reservations scoped
`(tenant, effect_type, key)` with the same ownership/status semantics as the
reference store (`Fresh`, `OwnedInProgress`, `OwnedSucceeded`, `OtherInProgress`,
`OtherSucceeded`, `Conflict`), so that reservations survive a restart. A
different fingerprint under the same scope MUST remain a `Conflict`. A durable
store MUST NOT allow two tenants producing the identical `effect_type` and key
to collide.

#### Scenario: A reservation survives a restart

- GIVEN a durable dedup store with a committed `OwnedSucceeded` reservation for a
  scope
- WHEN the process restarts and the same effect reserves the same scope
- THEN it is told `OwnedSucceeded`, not `Fresh` — the reservation persisted

#### Scenario: A different fingerprint under the same scope is a conflict

- GIVEN a durable dedup store with a reservation for a scope under one
  fingerprint
- WHEN a different fingerprint reserves the same scope
- THEN the outcome is `Conflict`, never a silent deduplication

#### Scenario: Cross-tenant reservations never collide durably

- GIVEN two tenants durably reserving the identical `effect_type` and key
- WHEN both reservations are recorded
- THEN each is tracked under its own tenant-scoped identity; neither is a
  duplicate of the other

### Requirement: The Durable Store Preserves At-Least-Once and Never Claims Exactly-Once

A durable store MUST preserve the at-least-once attempted-delivery guarantee
across process crash and restart: an effect accepted before a crash MUST still be
attempted after restart. The durable store MUST NOT introduce, imply, or document
exactly-once delivery; the phrase "exactly once" MUST NOT appear in the durable
store's contract or documentation. Idempotency of the delivered effect remains
the handler/executor's responsibility via propagated idempotency keys — the
durable store does not make delivery idempotent on the destination's behalf.

#### Scenario: An accepted effect is still attempted after a crash

- GIVEN a durable store holding an accepted, not-yet-delivered effect
- WHEN the process crashes before delivery and restarts
- THEN the effect is attempted at least once after restart

#### Scenario: A redelivered effect may reach the destination more than once

- GIVEN an effect whose consumer crashed after dispatch but before recording
  success
- WHEN the effect is reclaimed and redelivered after lease expiry
- THEN the destination may receive it more than once, and dedup is the
  destination/executor's responsibility via the propagated idempotency key — the
  store does not promise exactly-once

#### Scenario: The contract never states exactly-once

- GIVEN the durable store's public contract and documentation
- WHEN they are inspected for delivery guarantees
- THEN "exactly once" appears nowhere; only at-least-once is stated

### Requirement: Durable Delivery Ordering Is Not Guaranteed

A durable store MUST NOT promise FIFO or any cross-effect ordering of delivery.
Concurrent consumers, `next_at`-scheduled retries with backoff, and lease-expiry
reclaim all legitimately reorder delivery relative to acceptance order. Callers
requiring ordering MUST NOT rely on the store to provide it.

#### Scenario: Acceptance order is not preserved on delivery

- GIVEN two effects accepted in a known order
- WHEN they are claimed and delivered by one or more consumers
- THEN their delivery order is not guaranteed to match their acceptance order

#### Scenario: A retried effect may be delivered after later-accepted effects

- GIVEN an effect that failed and was rescheduled with a future `next_at`
- WHEN other effects accepted after it are claimed while it waits
- THEN those later effects may be delivered first; ordering is not guaranteed

### Requirement: The In-Memory Store Is Retained as the Reference Implementation

`InMemoryEffectStore` MUST be retained, with its behavior unchanged, as the
reference/test implementation of `EffectStateStore` and `EffectDedupStore`. The
durable store is a separate, independent implementation of the same ports, not a
replacement of the ports' contract; each port MUST remain independently
satisfiable by either implementation. Existing code and tests using
`InMemoryEffectStore` MUST continue to compile and behave unchanged.

#### Scenario: The in-memory store is unchanged

- GIVEN existing code and tests built against `InMemoryEffectStore`
- WHEN the durable store is added to the workspace
- THEN the in-memory store's behavior and API are unchanged and those tests pass
  without modification

#### Scenario: Both implementations satisfy the same port contract

- GIVEN the shared port test-suite for `EffectStateStore`/`EffectDedupStore`
- WHEN it is run against both `InMemoryEffectStore` and the durable store
- THEN both satisfy every port scenario, each independently

## MODIFIED Requirements

### Requirement: Delivery Guarantee Is At-Least-Once, Never Exactly-Once

The capability MUST guarantee at-least-once attempted delivery within the
lifetime/durability of the registered `EffectStateStore`, plus mandatory
idempotency-key propagation to executors. With a cooperating destination this
composes to a logical once-only outcome; "exactly once" MUST NOT appear in the
public contract or its docs. With the shipped in-memory store, the guarantee MUST
be documented as degrading to at-most-once across a crash. **With a durable
`EffectStateStore`, the at-least-once guarantee MUST hold across a process crash
and restart — an effect accepted before the crash MUST still be attempted after
it — and MUST NOT be documented, implied, or upgraded to exactly-once.** The
choice of store therefore determines only whether the guarantee survives a crash,
never whether it becomes exactly-once.

(Previously: the requirement fixed at-least-once and the in-memory at-most-once
-across-crash degradation, but predated any durable store and did not state that
a durable store preserves at-least-once across crash while still never claiming
exactly-once.)

#### Scenario: Logical once-only outcome with a cooperating destination

- GIVEN a destination that rejects a duplicate idempotency key
- WHEN the same effect is attempted more than once due to a retry
- THEN the destination's dedup plus the propagated key yields one logical
  outcome

#### Scenario: In-memory store loses undelivered effects on crash

- GIVEN the shipped in-memory `EffectStateStore`
- WHEN the process crashes before a pending or in-flight effect completes
- THEN that effect is lost, documented explicitly, never hidden

#### Scenario: Durable store preserves at-least-once across a crash

- GIVEN a durable `EffectStateStore` holding an accepted, undelivered effect
- WHEN the process crashes before delivery and restarts
- THEN the effect is attempted at least once after restart, and the guarantee is
  still stated as at-least-once — never exactly-once
