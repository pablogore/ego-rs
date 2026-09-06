# Persistence Stoolap Operation Reservation Specification

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## Purpose

A Stoolap-backed `OperationReservationStore` implementation exists that satisfies every
invariant already required of the framework's `OperationReservationStore` contract, with no
weaker semantics than the existing implementations, and that reservations survive the store
being closed and the underlying database reopened. This capability is scoped to same-process
concurrency only; no evidence exists in this framework today that a Stoolap-backed store is
safe when shared by multiple OS processes or multiple nodes.

## Requirements

### Requirement: Reservation Invariants Match The Existing Implementations Exactly

A Stoolap-backed `OperationReservationStore` MUST satisfy, with no weaker semantics: atomic
reservation of an `OperationId{tenant, operation_key}`; full-triple ownership verification
(`operation_id` + `owner_id` + `fencing_token`) on every mutating call; lease expiration and
atomic takeover; strictly monotonic fencing-token advance on takeover; tenant isolation between
an `OperationId` scoped to one tenant and the same operation key scoped to another tenant or to
the systemwide scope; and retention/purge that never removes an in-progress reservation.

#### Scenario: A fresh reservation is exclusive

- GIVEN no existing reservation for an `OperationId`
- WHEN one caller reserves it
- THEN the reservation is granted to that caller as `Fresh`, and a concurrent caller for the
  identical `OperationId` observes `OtherInProgress`, never a second `Fresh` grant

#### Scenario: A stale fence is rejected without mutating state

- GIVEN a reservation whose lease was taken over by a new owner
- WHEN the prior owner presents its now-stale `OwnerFence` to renew, complete, or abandon
- THEN the call fails with `StaleOwner`, and the reservation is left unmodified

#### Scenario: Takeover mints a strictly greater fencing token

- GIVEN a reservation with an expired lease
- WHEN a new caller takes it over
- THEN the takeover succeeds atomically with a fencing token strictly greater than the one it
  displaces, and the prior owner's fence no longer verifies

#### Scenario: Two tenants with the identical operation key remain isolated

- GIVEN tenant A and tenant B each reserve the identical operation key
- WHEN both reservations are queried
- THEN each is a distinct `OperationId`, and neither tenant's reservation, lease, or stored
  response is observable through the other tenant's key

#### Scenario: An in-progress reservation is never purged

- GIVEN a reservation still in progress, older than any configured retention cutoff
- WHEN the retention/purge routine runs
- THEN that reservation is not removed; only lease expiry and takeover may resolve it

### Requirement: Reservations Survive Close And Reopen

A reservation written to the Stoolap-backed store MUST remain readable, with its owner,
fencing token, lease, and tenant scope intact, after the store is closed and the same
underlying database is reopened.

#### Scenario: Ownership and fencing survive a close/reopen cycle

- GIVEN a reservation held by an owner with a specific fencing token
- WHEN the store is closed and the same underlying database is reopened
- THEN the reservation is still observable with the identical owner, fencing token, lease
  bound, and tenant scope as before the close

### Requirement: Durability Is Reported Only Once Demonstrated

The Stoolap-backed store MUST report itself durable only once the close/reopen survival
requirement above is demonstrated for it; it MUST NOT report durable while relying on a
storage mode that does not guarantee the written state is retained across a reopen.

#### Scenario: The store reports durable once survival is proven

- GIVEN the Stoolap-backed store configured in the mode this capability requires for
  close/reopen survival
- WHEN its durability is queried
- THEN it reports durable

### Requirement: Scoped To Same-Process Concurrency

This capability MUST NOT be described or tested as safe for concurrent access from multiple OS
processes or multiple nodes sharing the same store. Documentation and tests MUST scope every
concurrency claim to callers within the same process.

#### Scenario: No multi-process or multi-node safety claim is made

- GIVEN this capability's documentation and test suite
- WHEN they are inspected for a concurrency claim
- THEN every claim is scoped to same-process concurrency, and none asserts or documents safety
  across multiple OS processes or multiple nodes sharing the same store

## Non-Goals

- Multi-process or multi-node concurrent access to the same Stoolap-backed store.
- Any change to the `OperationReservationStore` contract's semantics, retention policy, or any
  other implementation's behavior.
