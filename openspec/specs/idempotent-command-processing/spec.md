# Idempotent Command Processing Specification

## Purpose

Defines the observable contract for end-to-end idempotent command processing:
a mandatory client-supplied `OperationKey` identifying one complete business
operation, a pre-dispatch reservation with lease/fencing, per-aggregate
receipts confirmed atomically with the event append, and two separately-named
guarantees bounded by different lifetimes. This spec fixes WHAT the guarantee
is; trait shapes, table layout, and the NULL-tenant uniqueness mechanism are
design-phase decisions (see Non-Goals).

## Requirements

### Requirement: Mandatory Key on Every External Mutable Command

Every external mutable command (HTTP today; gRPC/Kafka when those transports
exist) MUST carry a client-supplied `Idempotency-Key`. A missing key MUST be
rejected before the operation is dispatched. `IdempotencyEnforcementMode`
MUST expose exactly one bounded compatibility variant permitting a temporary
transition period; its default MUST be the fail-closed (mandatory-key)
variant.

#### Scenario: Missing key rejected under the default mode
- GIVEN the default `IdempotencyEnforcementMode`
- WHEN a mutable command arrives with no `Idempotency-Key`
- THEN the command is rejected before dispatch; no aggregate is touched

#### Scenario: Compatibility mode is explicit and bounded, never silent
- GIVEN `IdempotencyEnforcementMode` set to its compatibility variant
- WHEN a command arrives with no key
- THEN the command is admitted only because that variant was explicitly
  configured — no undocumented default permits this

### Requirement: No Server-Side Key Generation

The system MUST NOT mint an `OperationKey` on the caller's behalf when one is
absent. A server-generated key is a function of the request as received and
therefore deduplicates nothing on retry.

#### Scenario: Server never fabricates a key for a keyless request
- GIVEN a mutable command with no client-supplied key
- WHEN enforcement is active
- THEN the system rejects the command; it MUST NOT generate a key and proceed

### Requirement: Operation-Scoped Identity, Reserved Before Dispatch

One `OperationKey` MUST identify one complete business operation, potentially
spanning multiple aggregates — not one key per aggregate command. The
reservation MUST be created after `#[authorize]` and `#[tenant_scoped]`
evaluation succeeds, and before the first `EntityRuntime` call. The
reservation's uniqueness namespace MUST be the `CanonicalTenant` produced by
`TenantResolver::resolve`; it MUST NOT be namespaced by the raw client tenant
hint.

#### Scenario: One key covers a multi-aggregate operation
- GIVEN an operation that writes two aggregates under one `OperationKey`
- WHEN the operation is retried after a partial completion
- THEN both aggregates are addressed under the same reservation; no second,
  independent reservation is created for the second aggregate

#### Scenario: Reservation happens after authorization and tenant scoping
- GIVEN a command that fails `#[authorize]` or `#[tenant_scoped]`
- WHEN the guard denies the call
- THEN no reservation is created — the reservation step never runs before a
  passing guard evaluation

#### Scenario: Namespace uses the resolved tenant, never the raw hint
- GIVEN a caller-supplied tenant hint that differs from the `CanonicalTenant`
  resolved by `TenantResolver::resolve`
- WHEN the operation is reserved
- THEN the reservation key is namespaced by the resolved `CanonicalTenant`,
  never by the raw hint

### Requirement: Lease With Owner, Expiry, and Verified Fencing

A reservation in progress MUST be governed by a lease carrying `owner_id`,
`lease_until`, and `fencing_token`. Every renewal, completion, or abandonment
of a reservation MUST perform a conditional update verifying
`operation_id + owner_id + fencing_token` together — storing a fencing token
without verifying it on every mutating call does NOT satisfy this
requirement. An update presented by an owner whose lease has expired MUST be
rejected with `StaleOwner`, and that owner MUST NOT be able to close or renew
the operation afterward. A later caller MUST be able to take over an expired
lease atomically, fencing out the prior owner.

#### Scenario: Conditional update rejects a stale owner
- GIVEN a reservation whose lease expired and was taken over by a new owner
- WHEN the original owner attempts to complete the reservation
- THEN the conditional update fails, `StaleOwner` is returned, and the
  reservation is not modified by the stale caller

#### Scenario: Atomic takeover fences out the prior owner
- GIVEN a reservation with an expired lease
- WHEN a new caller takes over the reservation
- THEN the takeover succeeds atomically with a new `fencing_token`, and any
  subsequent call from the prior owner's fencing token fails

#### Scenario: Storing a token without verifying it is insufficient
- GIVEN an implementation that persists `fencing_token` but does not compare
  it on renew/complete/abandon
- WHEN a stale owner issues a renew after takeover
- THEN this requirement is NOT satisfied — the conditional-update comparison
  is mandatory, not merely storing the value

### Requirement: Per-Aggregate Receipts Confirmed Atomically With the Append

Each aggregate an operation reaches MUST record a permanent receipt keyed
`(tenant_id, aggregate_type, aggregate_id, operation_key)`, confirmed in the
same transaction as that aggregate's event append. The receipt MUST be
written even when the command succeeds and produces zero events. The
snapshot or in-memory state MUST NOT be treated as the sole source of truth
for whether an operation already applied to an aggregate — the receipt is
authoritative.

#### Scenario: Zero-event success still writes a receipt
- GIVEN a command that succeeds without producing any event
- WHEN the command completes
- THEN a receipt is written for that aggregate/operation pair inside the
  same transaction as the (empty) commit

#### Scenario: Receipt confirmation is atomic with the append
- GIVEN a command that produces events
- WHEN the append transaction commits
- THEN the receipt for that aggregate/operation pair is confirmed in the
  identical transaction — never as a separate, subsequent write

#### Scenario: Snapshot alone cannot answer "already applied"
- GIVEN an aggregate whose in-memory snapshot has advanced past the point an
  operation would have affected it
- WHEN recovery asks whether that operation already applied
- THEN the answer is determined by the persisted receipt, not by inferring
  it from the snapshot's current state

### Requirement: Two Guarantees, Named Separately

The replay window and domain duplication protection MUST be named and
bounded separately. The replay window — exact prior response returned on
retry — is bounded by the reservation TTL, counted from `completed_at`.
Domain duplication protection — no aggregate re-mutates for an operation
already applied to it — lasts for the life of the stream, ending only on
explicit, definitive deletion of the aggregate or tenant. After the TTL
expires, there is no response replay and no boundary-level detection of a
reused key; for an operation rejected before touching any aggregate, or
successful without reaching one, protection ends with the TTL.

#### Scenario: After TTL, prior response is no longer replayed
- GIVEN a reservation whose TTL has elapsed and been purged
- WHEN the same key is retried
- THEN no stored response is returned — the boundary treats it as a new
  operation for replay purposes

#### Scenario: Receipts still block re-mutation after the TTL
- GIVEN a reservation purged after TTL, for an operation that already
  reached and wrote to an aggregate
- WHEN the same key is retried against that aggregate
- THEN the aggregate's permanent receipt still causes a no-op — the
  aggregate is not re-mutated, even though replay is no longer available

#### Scenario: Zero-aggregate operation loses all protection at TTL
- GIVEN an operation rejected before touching any aggregate, or successful
  without reaching one
- WHEN its reservation TTL elapses
- THEN reusing that key is indistinguishable from a fresh operation — this
  is the documented limit of protection, not a defect

### Requirement: Fingerprint Determines Replay vs. Conflict

Each reservation and receipt MUST record a fingerprint of the operation's
content alongside its key. The same key with the same fingerprint MUST be
treated as already applied (replay or no-op). The same key with a different
fingerprint MUST be treated as a permanent conflict — never a silent dedupe
and never a silently reopened business transaction. This rule MUST hold both
at the reservation boundary and in the per-aggregate receipts table.

#### Scenario: Same key, same fingerprint replays
- GIVEN a completed operation under key K with fingerprint F
- WHEN K is retried with the identical fingerprint F
- THEN the stored outcome is returned (or the aggregate no-ops), never
  re-executed

#### Scenario: Same key, different fingerprint is a permanent conflict
- GIVEN a completed or in-progress operation under key K with fingerprint F
- WHEN K is retried with a different fingerprint F'
- THEN the call fails with a permanent conflict; the original operation is
  never silently reopened or reinterpreted

#### Scenario: Receipt-level fingerprint mismatch is also a conflict
- GIVEN an aggregate holds a receipt for key K with fingerprint F
- WHEN a later command presents K with fingerprint F' during recovery
- THEN the receipt lookup reports a conflict, not a no-op

### Requirement: Split Retention and Safe Purge

Reservations and stored responses MUST be retained for a configurable TTL
counted from `completed_at`, never from `created_at`. A reservation in the
`InProgress` state MUST NOT be purged by TTL; it becomes purge-eligible only
after its lease expires and is resolved through the lease-expiry path.
Per-aggregate receipts MUST be retained permanently for the life of the
stream and MUST be deletable only alongside an explicit, definitive deletion
of the owning aggregate or tenant — never through the ordinary retention job.
The purge job MUST be batched, observable, and safe when run concurrently
from multiple workers.

#### Scenario: TTL is measured from completion, not creation
- GIVEN a reservation created at T0 and completed at T1
- WHEN the configured TTL elapses from T1
- THEN the reservation becomes purge-eligible at `T1 + TTL`, not `T0 + TTL`

#### Scenario: InProgress reservations are never TTL-purged
- GIVEN a reservation still `InProgress` past what would be its TTL if it had
  completed
- WHEN the purge job runs
- THEN it is not purged; only lease expiry and takeover may resolve it

#### Scenario: Receipts survive ordinary retention
- GIVEN receipts for an aggregate older than any configured reservation TTL
- WHEN the ordinary purge job runs
- THEN those receipts are not deleted — only an explicit aggregate/tenant
  deletion removes them

#### Scenario: Concurrent purge workers do not double-purge or deadlock
- GIVEN two purge workers running concurrently against overlapping eligible
  rows
- WHEN both execute a purge pass
- THEN each eligible row is purged exactly once, and neither worker deadlocks
  or errors due to the other's concurrent pass

### Requirement: The Dual-Aggregate Write Is Not Promised Atomic

This capability MUST NOT promise atomicity across multiple aggregates
touched by one operation (e.g. `RegisterUserImpl`'s organization-then-user
write). It promises safe recovery by re-execution only: an aggregate already
holding a receipt for the operation no-ops on retry; an aggregate that never
received the operation executes it. No saga, compensation, or rollback
mechanism is introduced by this capability.

#### Scenario: Partial completion recovers without duplication
- GIVEN an operation whose lease expired after one aggregate's receipt was
  confirmed but before a second aggregate was reached
- WHEN a new owner takes over and re-executes
- THEN the first aggregate no-ops on its existing receipt, the second
  aggregate executes, and the operation completes with zero duplicated
  events — with no atomicity claim between the two writes

### Requirement: OperationKey Is Distinct From IdempotencyKey

`OperationKey` MUST be a distinct newtype from the existing `IdempotencyKey`,
defined in the common domain crate. No `From<OperationKey>` or other implicit
conversion between the two types MUST exist. A future bridge, if ever needed,
MUST be a deliberately named function, never a generic conversion trait
implementation.

#### Scenario: No implicit conversion compiles
- GIVEN the `OperationKey` and `IdempotencyKey` types
- WHEN the workspace is searched for a `From<OperationKey> for IdempotencyKey`
  implementation (or the reverse)
- THEN none exists — a compile-fail test asserts an attempted implicit
  conversion does not compile

#### Scenario: Both validate non-empty strings but remain unrelated types
- GIVEN a value that is valid as both an `OperationKey` and an
  `IdempotencyKey` string
- WHEN it is used to construct one type
- THEN the result cannot be passed anywhere the other type is required
  without an explicit, named derivation function

### Requirement: Cross-Tenant Replay Is Prohibited

A stored response, reservation, or receipt keyed under tenant A MUST NOT ever
be replayed, returned, or treated as already-applied for a request resolved
to tenant B — including when tenant B is the NULL/systemwide tenant. This is
a security requirement: cross-tenant replay is an information-disclosure
vector, not merely a correctness defect.

#### Scenario: Identical key across two tenants never cross-replays
- GIVEN tenant A completes an operation under key K with a stored response
- WHEN tenant B later presents the identical key K
- THEN tenant B's request is evaluated as its own operation; tenant A's
  stored response is never returned to tenant B

#### Scenario: Systemwide (NULL-tenant) requests do not leak into or from a real tenant
- GIVEN an operation completed under key K in the NULL-tenant systemwide
  scope
- WHEN a real tenant later presents the identical key K
- THEN the real tenant's request is evaluated independently; the systemwide
  response is never replayed to it, and vice versa

### Requirement: The Guarantee Is Protocol-Neutral, Demonstrated By Two Key Carriers

Idempotency MUST NOT depend on any protocol type. `OperationKey`,
`OperationFingerprint`, key validation and the missing-key policy MUST live in the
domain and SDK layers and MUST NOT reference a transport type. A transport adapter
MUST contribute only the location a raw value is read from, never a rule about it.

At least **two** transports MUST implement `OperationKeyCarrier` and MUST pass the
identical three-state conformance harness — one adapter can satisfy a contract by
accident, two cannot. HTTP and gRPC are the required minimum for this capability.
Every adapter MUST report the same three states, `Absent`, `Present` and
`Unreadable`, and MUST NOT redefine validation or enforcement for its protocol.

This requirement is about key-carrier conformance, not about a second working
command-dispatch transport: only the HTTP adapter dispatches real commands through
the idempotency-aware path today. The gRPC carrier (`GrpcMetadataCarrier`)
implements `OperationKeyCarrier` and passes the shared harness for
`idempotency-key` metadata extraction, but no gRPC service, socket, or command
dispatch path exists in the workspace — a claim of "two working transports for
commands" would be false.

#### Scenario: Two adapters resolve identically for every input class
- GIVEN an HTTP carrier and a gRPC metadata carrier
- WHEN each is presented with an absent key, a valid key, an invalid key and a value
  that cannot be read as text
- THEN both resolve to the identical outcome for every class, under both the
  fail-closed and the compatibility enforcement modes

#### Scenario: gRPC conformance is extraction-only, not a dispatch claim
- GIVEN the gRPC metadata carrier's `OperationKeyCarrier` implementation
- WHEN its conformance-harness result is cited as evidence
- THEN it establishes only that its key extraction matches HTTP's for every input
  class — it is never read as evidence of a working gRPC command-dispatch path,
  because none exists in the workspace

#### Scenario: No protocol type reaches the core
- GIVEN the domain layer, the entity runtime, and the reservation and receipt surfaces
- WHEN their public and internal surfaces are inspected
- THEN no HTTP or gRPC type appears in any of them, and idempotency behaviour is
  reachable without naming a protocol

#### Scenario: The extraction-to-dispatch path is shared, not duplicated
- GIVEN two transports that each extract a key their own way
- WHEN a resolved key travels onward from `ServiceContext`
- THEN both follow one identical path to the entity, so adding a transport adds an
  extraction step and nothing else

## Non-Goals

- Multi-node activation authority, membership, or distributed contention
  tests (PROD-009).
- Transactional outbox / atomic effect publication (CORE-030).
- Saga orchestration or step checkpointing (CORE-029).
- Reviving `CommandContext.expected_version`.
- Read-side projection dedup (`crates/domain/src/read_side/dedup.rs`).
- Prescribing the `EventStore` trait shape, sync-vs-async, or the physical
  location of the receipt index — see `event-store` spec's Non-Goals; these
  are design-phase decisions.
- Prescribing the NULL-tenant uniqueness mechanism (`NULLS NOT DISTINCT`,
  sentinel, or partial indexes) — design-phase decision, constrained only by
  the "Cross-Tenant Replay Is Prohibited" requirement above.
- Kafka enforcement — the contract is transport-agnostic; a Kafka adapter
  does not exist in the workspace today. HTTP and gRPC adapters already
  exist and are covered by the requirement above.
