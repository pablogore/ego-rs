# Persistence Stoolap Read-Side Specification

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## Purpose

Stoolap-backed implementations of the framework's three read-side ports — `OffsetStore`,
`DedupStore`, `ReadSideClaimStore` — exist, satisfy every invariant each port's contract already
defines with no weaker semantics, survive a clean close/reopen of the underlying file, remain
correct under real concurrency from multiple tasks inside one process, and are exercised through a
real `Profile::Production` composition that the existing gate accepts for durable stores and
rejects for volatile ones. This capability is scoped to a single ego-rs process owning the Stoolap
file; no evidence exists that these stores are safe when a file is shared by multiple OS processes
or multiple nodes.

## Requirements

### Requirement: Offset Reads And Writes Are Isolated Per (projection_id, tag, tenant)

`read_offset` MUST return `None` for a key never written. `write_offset` MUST scope strictly to
`(projection_id, tag, tenant)`; a write to one key MUST NOT affect any other key. `write_offset`
MUST be last-write-wins — this capability enforces no compare-and-swap or monotonicity, matching
the port's contract as-is.

#### Scenario: A write is isolated to its key

- GIVEN offsets written for two distinct `(projection_id, tag, tenant)` keys
- WHEN either key is read
- THEN each returns only the offset written for that exact key, and a never-written key returns
  `None`

#### Scenario: A repeat write overwrites without ordering enforcement

- GIVEN an offset already written for a key
- WHEN `write_offset` is called again for the identical key with any `Offset` value
- THEN the store accepts it, and `read_offset` returns the value just written

### Requirement: Dedup Identity Is (projection_id, tag, event_id), No Tenant, No Pruning

`seen`/`mark_seen` MUST key by `(projection_id, tag, event_id)` only, with no tenant parameter.
`mark_seen` MUST be idempotent, producing no error on repeat. The store MUST NOT prune, expire, or
evict marks; no TTL or retention exists in this capability.

#### Scenario: mark_seen is idempotent

- GIVEN an event already marked seen for a `(projection_id, tag)`
- WHEN `mark_seen` is called again for the identical triple
- THEN the call succeeds without error and `seen()` still returns `true`

#### Scenario: No dedup entry is ever pruned

- GIVEN a dedup mark written arbitrarily long ago
- WHEN `seen()` is queried for it
- THEN it still returns `true`

### Requirement: Claim Grant And Refusal Are Mutually Exclusive

`try_claim` MUST return `Ok(Some(fence))` exactly when no live (unexpired) lease holds
`claim_id`, whether granting fresh or taking over a lapsed lease, and `Ok(None)` — a refusal, not
an error — when a live lease already holds it. A takeover's `fencing_token` MUST be strictly
greater than any previously issued for that `claim_id`.

#### Scenario: A live claim refuses a second claimant

- GIVEN a claim held with a lease not yet expired
- WHEN a different owner calls `try_claim` for the identical `claim_id`
- THEN it returns `Ok(None)`, and the existing holder's fence remains valid

#### Scenario: Takeover of a lapsed lease mints a strictly greater token

- GIVEN a claim whose `lease_until` has passed
- WHEN a new owner calls `try_claim` for the identical `claim_id`
- THEN it returns `Ok(Some(fence))` with a `fencing_token` strictly greater than the lapsed
  holder's, and the lapsed holder's fence no longer verifies

### Requirement: Renew And Release Verify The Full Fence Against Live State, Atomically

`renew` and `release` MUST verify `claim_id` + `owner_id` + `fencing_token` together against the
currently stored row, not a prior read. Both MUST reject with `ClaimError::StaleOwner` a fence
that no longer matches the live claim, and separately a fence whose lease has already lapsed,
leaving state unmodified either way. `release` MUST NOT delete the claim's row — it MUST set an
already-expired lease, keeping the fencing token monotone and the claim immediately reclaimable.

#### Scenario: renew rejects a stale or lapsed fence without mutating state

- GIVEN a fence that no longer matches the live claim, or whose lease has already lapsed
- WHEN `renew` is called with it
- THEN it fails with `StaleOwner` and the stored claim is unchanged

#### Scenario: release marks the claim expired, not deleted

- GIVEN a claim held under a valid fence
- WHEN `release` is called with that fence
- THEN a subsequent `try_claim` for the identical `claim_id` succeeds immediately, and the
  claim's row still exists with an expired lease

### Requirement: Claim Types And Exhaustion Reuse The Reservation Port

`OwnerId` and `FencingToken` used by this claim store MUST be the identical types
`crate::operation::reservation` already defines and `OperationReservationStore` uses, not parallel
redefinitions. `FencingToken::next()` returning `None` MUST surface as
`ClaimError::FencingExhausted`, never a wrapped or truncated token.

#### Scenario: Exhaustion is reported, not wrapped

- GIVEN a `claim_id` whose fencing token is already at its maximum value
- WHEN a takeover would otherwise be granted
- THEN `try_claim` returns `ClaimError::FencingExhausted` instead of wrapping the token

### Requirement: Lease Expiry Is Always Caller-Computed

The store MUST NEVER read system time to decide whether a lease has expired. Every expiry
decision MUST compare only against the `lease_until` value the caller supplied on `try_claim` or
`renew`.

#### Scenario: Expiry decisions use only the supplied timestamp

- GIVEN a caller-supplied `lease_until` value
- WHEN the store decides whether a claim is still live
- THEN the decision depends only on that value compared to the previously stored `lease_until`,
  never on a clock read performed inside the store

### Requirement: Claim Correctness Holds Under Real Intra-Process Concurrency

When multiple tasks within the same process call `try_claim` concurrently for the identical
`claim_id`, exactly one MUST receive `Ok(Some(fence))`. Every other concurrent caller MUST receive
`Ok(None)` or a `ClaimError::Transient` safe to retry — never a second `Ok(Some(fence))` for the
same live lease.

#### Scenario: Concurrent claimants yield exactly one winner

- GIVEN several tasks in the same process calling `try_claim` concurrently for the identical
  `claim_id`, with no existing live lease
- WHEN all calls complete
- THEN exactly one receives `Ok(Some(fence))` and every other receives `Ok(None)` or a
  retry-safe `Transient` error

### Requirement: Offset And Dedup State Survive Close And Reopen

A value written through the offset or dedup store MUST remain readable and unchanged after the
store is closed and the same underlying Stoolap file is reopened. `is_durable()` MUST return
`true` for either store only once this is demonstrated, and `false` otherwise.

#### Scenario: An offset survives a close/reopen cycle

- GIVEN an offset written for a key
- WHEN the store is closed and the same file reopened
- THEN `read_offset` for that key returns the identical value

#### Scenario: A dedup mark survives a close/reopen cycle

- GIVEN an event marked seen
- WHEN the store is closed and the same file reopened
- THEN `seen()` for that event still returns `true`

### Requirement: Claim Durability Is Drop-And-Reopen, Not Crash Recovery

Where `is_durable()` reports `true` for the claim store, a claim's owner, fencing token, and
lease state MUST survive a clean close and reopen of the same file, and a released or
naturally-lapsed claim MUST reopen as reclaimable. This capability MUST NOT claim protection
against process kill or power loss — only against a clean close/reopen cycle.

#### Scenario: A claim's fence survives a close/reopen cycle

- GIVEN a claim held under a valid fence, with its lease not yet expired
- WHEN the store is closed and the same file reopened
- THEN `try_claim` for a different owner on the identical `claim_id` still returns `Ok(None)`,
  and the held fence still verifies against the reopened state

### Requirement: A Real Profile::Production Composition Exercises The Gate, With A Negative Control

The existing `Profile::Production` read-side durability gate MUST be exercised by a real
composition using real Stoolap-backed offset, dedup, and claim stores, real config, and the real
`try_build()` (or equivalent) entry point — not solely by asserting `is_durable()` on an isolated
store. The identical gate, unmodified, MUST reject a composition where any one of the three stores
is a non-durable (volatile) implementation.

#### Scenario: A real durable Stoolap composition passes Production

- GIVEN `Profile::Production` configured with real Stoolap-backed offset, dedup, and claim
  stores over an on-disk database
- WHEN the composition is built
- THEN it succeeds

#### Scenario: A volatile store is rejected by the unmodified gate

- GIVEN the identical composition with one store replaced by a non-durable implementation
- WHEN the composition is built under `Profile::Production`
- THEN it is rejected by the same gate, unmodified

### Requirement: Scoped To Same-Process Concurrency Only

This capability MUST NOT be described, tested, or documented as safe for concurrent access to
the same Stoolap file from more than one OS process or more than one node. Every concurrency
claim MUST be scoped to tasks within one process.

#### Scenario: No multi-process or multi-node claim is made

- GIVEN this capability's documentation and test suite
- WHEN inspected for a concurrency claim
- THEN every claim is scoped to same-process concurrency, and none asserts safety across
  multiple processes or nodes

## Non-Goals

- `EventStore`, `Snapshot`, `Repository`, `EffectStateStore`, `EffectDedupStore`,
  `OperationReservationStore` (already shipped; reused only as a concurrency-pattern template),
  and any other store.
- Any PostgreSQL change, and any change to the `OffsetStore`, `DedupStore`, or
  `ReadSideClaimStore` trait contracts.
- Multi-process, multi-node, distributed, or Kubernetes coordination; `LISTEN`/`NOTIFY`; brokers;
  event buses.
- Dedup pruning, TTL, or retention.
- Offset monotonicity or compare-and-swap on `write_offset`.
- Verimand-specific or any other product-specific logic.
- Crash/power-loss (`kill -9`) durability guarantees — only clean close/reopen durability is in
  scope.
