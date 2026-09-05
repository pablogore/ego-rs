# Spec: `persistence-stoolap-event-sourcing` (New Capability)

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## Purpose

The observable contract that a Stoolap-backed `EventStore<E>` and `Snapshot` exist, that
`Profile::Production` builds and passes `validate_persistence` on them with zero PostgreSQL
involved, that state committed before shutdown is genuinely recoverable from the same file after
the runtime is destroyed and reopened, and that `is_durable()` reflects real sync behavior rather
than a fixed flag. It does not cover `OperationReservationStore`, `OffsetStore`, `DedupStore`,
`ReadSideClaimStore`, any change to `Repository<A>`/`StoolapRepository`, or `StoolapEffectStore`'s
existing behavior — all out of scope, unaffected.

## Requirements

### Requirement: Production Builds Without PostgreSQL

An `EntityRuntimeBuilder` configured with `Profile::Production`, a Stoolap-backed `EventStore<E>`,
and a Stoolap-backed `Snapshot` MUST succeed at `build()`/`try_build()` and pass
`validate_persistence`'s durability gate, with no PostgreSQL connection, driver, or dependency
involved in producing that success.

#### Scenario: Production build succeeds on Stoolap alone
- GIVEN `Profile::Production` with a Stoolap-backed `EventStore<E>` and `Snapshot` registered, and
  no PostgreSQL configuration present
- WHEN `try_build()` runs
- THEN it succeeds, and no PostgreSQL connection is opened at any point

### Requirement: Committed State Survives Runtime Destruction and File Reopen

Once an entity's events (and any snapshot) are committed to a Stoolap file, that state MUST be
recoverable, unchanged, by a new runtime that reopens the identical file after the original
runtime and its process-level resources are fully dropped.

#### Scenario: Write, drop, reopen, state matches
- GIVEN a `Profile::Production` runtime backed by a Stoolap file, with an entity's commands
  applied and committed
- WHEN the runtime (and its `EventStore`/`Snapshot` handles) is dropped, then a new runtime opens
  the same file path
- THEN recovering the same entity produces state identical to what was committed before the drop

### Requirement: Durability Claims Reflect Real Sync Behavior

`is_durable() == true` on the Stoolap-backed `EventStore<E>` and `Snapshot` MUST correspond to a
genuine durable-sync configuration of the underlying store, not a hardcoded return value. A store
instance not actually configured for durable sync MUST NOT report `is_durable() == true`.

#### Scenario: A durably-configured store's claim is truthful
- GIVEN a Stoolap-backed `EventStore<E>` opened in its durable-sync configuration
- WHEN `is_durable()` is called
- THEN it returns `true`, and a write committed under that configuration survives the reopen
  scenario above

#### Scenario: is_durable is not a fixed constant independent of configuration
- GIVEN two Stoolap-backed store instances differing only in their sync configuration
- WHEN `is_durable()` is called on each
- THEN the results reflect each instance's actual configuration, not an identical hardcoded value

### Requirement: File Ownership Is Single-Process, Single-Node Only

The Stoolap-backed `EventStore<E>` and `Snapshot` MUST guarantee correct concurrent behavior only
among callers within one owning process on one node, matching the non-guarantee already
established for `persistence-stoolap-adapter`. No multi-process or multi-node concurrent-access
guarantee is made.

#### Scenario: Multi-process access is an explicit non-guarantee
- GIVEN two separate operating-system processes opening the same Stoolap file
- WHEN both access it concurrently
- THEN this capability documents no guarantee of correct or safe behavior for that case

### Requirement: Tenant Scoping Is Honored Correctly

The Stoolap-backed implementations MUST thread `EventStore`'s and `Snapshot`'s existing
`tenant_id: Option<&str>` parameter correctly: a named tenant's events/snapshots MUST NOT be
visible under, or confused with, a different named tenant or the systemwide (`None`) scope, under
the single-tenant-per-process model `Profile::Production` already enforces.

#### Scenario: A tenant's events are isolated from another tenant sharing the same aggregate identity
- GIVEN the same aggregate identity written independently under two different `tenant_id` values
- WHEN each is loaded
- THEN each returns only its own tenant's events, with no cross-tenant visibility

## Non-Goals

- `OperationReservationStore`, `OffsetStore`, `DedupStore`, `ReadSideClaimStore` — no requirement
  here implies these gain a Stoolap-backed implementation.
- Any change to `Repository<A>` or `StoolapRepository` (S1) — untouched.
- `StoolapEffectStore` behavior — already satisfied, unaffected by this capability.
- Multi-tenant-per-process sharing of one Stoolap file — remains unsupported, per
  `Profile::Production`'s existing single-tenant constraint.
