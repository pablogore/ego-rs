# Event Store Specification

## Purpose

Canonical spec for the write-side event persistence contract (`EventStore`
and its implementations). No prior canonical spec existed under
`openspec/specs/` before this change — `EventStore` had zero occurrences
there, and the archived `2026-06-22-persistence-spi` change was never merged
into `specs/`. This document fixes WHAT the contract guarantees for
uniqueness, atomicity, and metadata; the trait's exact shape (sync/async,
caller-owned transaction handle vs. unit-of-work closure) and the physical
schema for the receipt index are design-phase decisions (see Non-Goals).

## Requirements

### Requirement: Effective Uniqueness on the Event Stream Identity

The event store MUST reject a second event written for the same
`(tenant_id, aggregate_type, aggregate_id, version)` tuple — including when
`tenant_id` represents the NULL/systemwide tenant. A duplicate MUST be
rejected by the store itself, not merely by application-level discipline.

#### Scenario: Duplicate version for the same tenant-scoped aggregate is rejected
- GIVEN an event already stored for `(tenant-a, User, user-7, version=3)`
- WHEN a second event is appended for the identical tuple
- THEN the store rejects the second append as a uniqueness violation

#### Scenario: Duplicate version under the NULL-tenant systemwide mode is also rejected
- GIVEN an event already stored for `(NULL, TenantOrganization, org-1,
  version=1)` in the systemwide tenant-less mode
- WHEN a second event is appended for the identical systemwide tuple
- THEN the store rejects the second append — NULL tenant identity does not
  exempt the tuple from uniqueness enforcement

### Requirement: Aggregate Type Is a Distinct Identity Component

`aggregate_type` MUST be stored and compared as a component of the identity
tuple distinct from `aggregate_id`, never produced by concatenating the two
into a single string. Two different `(aggregate_type, aggregate_id)` pairs
that would produce an identical concatenated string under any prior
delimiter scheme MUST be stored and resolved as distinct streams.

#### Scenario: Ambiguous concatenation cases resolve distinctly
- GIVEN aggregate type `user-account` with id `7`, and aggregate type `user`
  with id `account-7`
- WHEN both are persisted and later looked up
- THEN each resolves to its own distinct stream; neither is confused with
  the other

### Requirement: Append and Receipt Confirmation Share One Transaction

When a caller confirms a per-aggregate operation receipt alongside an event
append, both writes MUST commit atomically in a single transaction: either
both the append and the receipt are durably visible, or neither is. This
requirement holds whether the command produced events or none.

#### Scenario: Append and receipt commit together
- GIVEN a command that appends one or more events and confirms a receipt
- WHEN the transaction commits
- THEN both the appended events and the receipt are visible to a subsequent
  read; a crash before commit leaves neither visible

#### Scenario: Zero-event commands still transactionally confirm a receipt
- GIVEN a command that succeeds with no events to append
- WHEN the command completes
- THEN the receipt confirmation still occurs inside a real transaction — it
  is not skipped merely because there is no event to append alongside it

### Requirement: Event Metadata Carries the Operation Key

The stored event's metadata MUST be able to carry an `operation_key`
alongside any other declared metadata fields, and that field MUST actually be
persisted and retrievable — not merely declared on the in-memory type without
a corresponding persisted column or serialized representation.

#### Scenario: Operation key round-trips through storage
- GIVEN an event appended with `operation_key` set in its metadata
- WHEN the event is later read back from the store
- THEN the retrieved event's metadata includes the identical `operation_key`

### Requirement: The In-Memory Store Does Not Silently Diverge on Tenant Scoping

Any in-memory `EventStore` implementation used for testing MUST enforce the
same tenant-scoped uniqueness behavior as the durable implementation. A test
suite exercising uniqueness or cross-tenant isolation against the in-memory
store MUST observe the identical pass/fail outcome it would observe against
the durable backend.

#### Scenario: In-memory store rejects the same duplicate a durable store would reject
- GIVEN an in-memory `EventStore` and the identical duplicate scenario from
  "Effective Uniqueness on the Event Stream Identity"
- WHEN the duplicate append is attempted against the in-memory store
- THEN it is rejected identically to the durable store — the in-memory store
  does not silently accept what the durable store would reject

## Non-Goals

- The exact trait signature for admitting a co-transactional receipt
  (caller-owned transaction handle, unit-of-work closure, or widened
  `append` parameter) and whether the trait becomes async — a design-phase
  decision (proposal §12.2).
- The physical schema location of the receipt index relative to `events`
  (dedicated table, columns + partial index, or a derived index table) — a
  design-phase decision (proposal §12.3).
- The specific mechanism enforcing NULL-tenant uniqueness (`NULLS NOT
  DISTINCT`, sentinel value, or partial unique indexes) and the minimum
  Postgres version this requires — a design-phase decision (proposal §12.1).
- Migration ordering and rollout mechanics — design and tasks concerns.
- Outbox/atomic effect publication (CORE-030) — this spec defines command
  storage, not effect publication.
