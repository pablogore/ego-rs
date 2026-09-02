# Spec: `persistence-memory-adapter` (New Capability)

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).
> Source of truth for requirement content: CORE-PERSIST-B's `proposal.md` Acceptance
> Requirements R1-R18. This capability's contract is purely structural — a pure move/reexport
> with zero new behavior — so every scenario is phrased in terms of declaration location,
> compile-time resolution, and identity, not new runtime behavior.

## Purpose

The observable contract that the workspace's in-memory implementations of the domain-owned
persistence ports have exactly one owning crate, `ego-persistence-memory`; that every path
resolving one of them today keeps resolving to the same item after this change; that no port
gains, loses, or changes an implementation; and that durability classification is unchanged.
This capability does not cover the Postgres adapters, a conformance-test framework, or any new
port, method, or behavior.

## Requirements

### Requirement: R1 — Canonical Ownership

The system MUST ensure each of the seven relocated implementations (`InMemoryEventStore` +
`InMemoryEventStoreUnitOfWork`, `InMemoryRepository`, `InMemorySnapshotStore`,
`InMemoryReadSideStore` + `paginate`, `InMemoryOffsetStore`, `InMemoryDedupStore`,
`InMemoryOperationReservationStore`) resolves from exactly one declaring crate,
`ego-persistence-memory`, and is declared nowhere else.

#### Scenario: Each implementation has exactly one declaring crate

- GIVEN the seven relocated implementations after this change
- WHEN the workspace is searched for their declarations by name
- THEN each name is declared exactly once, in `ego-persistence-memory`

#### Scenario: The vacated crates no longer declare it

- GIVEN `ego-infrastructure` and `ego-testkit` held the pre-change declarations
- WHEN the move completes
- THEN those crates contain only `pub use` re-exports at the old paths, not declarations

### Requirement: R2 — No Duplicate Canonical Implementation Is Introduced

The move MUST create zero new declarations; the count of `impl <Port> for` blocks per moved port
MUST be unchanged workspace-wide.

#### Scenario: The impl-block count is stable across the move

- GIVEN the workspace-wide count of `impl <Port> for` blocks for each of the eight moved ports
  before this change
- WHEN the same count is taken after this change
- THEN the counts are identical

### Requirement: R3 — Named Test Fakes Are Not Promoted

`FakeDurableOffsetStore` and `FakeDurableDedupStore` MUST remain declared in
`examples/reference-app`, byte-identical, and MUST appear nowhere in `ego-persistence-memory`.

#### Scenario: The fakes stay in the example, unedited

- GIVEN `FakeDurableOffsetStore` and `FakeDurableDedupStore` as declared in
  `examples/reference-app/src/read_side/store.rs` before this change
- WHEN this change completes
- THEN both are still declared there, byte-identical, and neither is declared in
  `ego-persistence-memory`

### Requirement: R4 — Missing Stays Visibly Missing

`ProjectionStateStore` MUST have zero implementations after this change, and no stub,
placeholder, or `todo!()` implementation MUST be added.

#### Scenario: A dead port stays dead

- GIVEN `ProjectionStateStore` has zero implementations before this change
- WHEN the workspace is searched for implementations after this change
- THEN it still has zero implementations, and no stub or placeholder implementation exists in
  `ego-persistence-memory` or elsewhere

### Requirement: R5 — Behavior Preservation

Every moved type's body — including tenant resolution, locking strategy, version-conflict
arithmetic, and fail-closed empty-tenant handling — MUST be textually identical to its
pre-change form, modulo module path and `use` lines.

#### Scenario: A moved body is a diff of only module path and imports

- GIVEN a moved implementation's pre-change source file
- WHEN it is diffed against its post-change location in `ego-persistence-memory`
- THEN the only differences are the module path declaration and `use` lines

#### Scenario: Fail-closed empty-tenant handling survives the move

- GIVEN `InMemoryReadSideStore`'s pre-change fail-closed behavior on an empty tenant
- WHEN the store is exercised post-move with an empty tenant
- THEN it still fails closed, with the identical error path

### Requirement: R6 — Durability And Production Preservation

No moved type MUST declare `is_durable()`; `presence_alone_is_not_durability` and both
`try_build_rejects_explicit_in_memory_*` tests MUST pass unmodified, still rejecting in-memory
stores under `Profile::Production`.

#### Scenario: Production profile still rejects an in-memory store

- GIVEN `EntityRuntimeBuilder` configured with `Profile::Production` and an explicit moved
  `InMemoryEventStore` or `InMemorySnapshotStore`
- WHEN the runtime attempts to build
- THEN it fails, naming the non-durable capability, exactly as before the move

#### Scenario: No moved type overrides is_durable

- GIVEN the seven relocated implementations
- WHEN each is inspected for a `fn is_durable` override
- THEN none exists, and each defaults to the trait's `false`

### Requirement: R7 — Backend Neutrality

`ego-persistence-memory` MUST contain no reference to any backend — no `sqlx`, Postgres,
Stoolap, HTTP, or Kafka type, dependency, or feature flag — and MUST offer no
backend-selection surface.

#### Scenario: The crate's dependency graph is backend-free

- GIVEN `crates/persistence-memory/Cargo.toml` and its source tree
- WHEN both are inspected
- THEN neither names nor imports `sqlx`, a Postgres/Stoolap client, an HTTP client, or a Kafka
  client, and no feature flag selects a backend

### Requirement: R8 — Read-Side Consolidation

`InMemoryOffsetStore` and `InMemoryDedupStore` MUST be declared in `ego-persistence-memory` and
no longer in `examples/reference-app`; the example MUST consume them as an ordinary dependency.

#### Scenario: The example no longer declares the two read-side stores

- GIVEN `examples/reference-app/src/read_side/store.rs` before this change
- WHEN this change completes
- THEN `InMemoryOffsetStore` and `InMemoryDedupStore` are declared only in
  `ego-persistence-memory`, and the example imports them as a dependency

### Requirement: R9 — Compatibility Re-Exports At Every Old Path

Every path in the COMPATIBILITY REEXPORT MATRIX MUST still resolve, unedited, to the same item —
proven at compile time over the full list, not by sampling. All six confirmed downstream
consumer files MUST compile with byte-identical source.

#### Scenario: An old path resolves to the identical relocated item

- GIVEN `ego_infrastructure::persistence::in_memory::InMemoryEventStore` as a pre-change import
  path
- WHEN the workspace compiles after this change
- THEN it still resolves, and the resolved type identity-coerces with
  `ego_persistence_memory::persistence::event_store::InMemoryEventStore`

#### Scenario: The six confirmed consumer files compile unedited

- GIVEN `crates/infrastructure/tests/in_memory_event_store_conformance.rs`,
  `crates/infrastructure/tests/commit_publishes_atomically.rs`,
  `examples/reference-app/src/lib.rs`, `crates/transport/tests/operation_key_extractor.rs`, and
  the two `crates/service-sdk/tests/{retention_worker_lifecycle,cross_tenant_reservation_isolation}.rs`
  files
- WHEN the workspace is rebuilt after this change
- THEN all compile with byte-identical source to before the change

### Requirement: R10 — Single Implementation Ownership Per Moved Port

For `EventStore`, `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `ReadSideStore`,
`OffsetStore`, `DedupStore`, and `OperationReservationStore`, `ego-persistence-memory` MUST be
the sole general-purpose in-memory owner; the only other declarations that survive MUST be the
two named `persistent-entity` duplicates and declared test fakes.

#### Scenario: No third general-purpose implementation exists

- GIVEN the eight moved ports
- WHEN the workspace is searched for general-purpose (non-fake, non-`persistent-entity`)
  implementations of each
- THEN exactly one is found for each, and it is declared in `ego-persistence-memory`

### Requirement: R11 — Dependency Integrity

`ego-persistence-memory`'s `Cargo.toml` MUST name exactly `ego-persistence-api` and `ego-domain`
as workspace path dependencies and nothing else; it MUST NOT name `ego-application`,
`ego-runtime`, `ego-infrastructure`, `ego-persistence`, `ego-testkit`, transport, or any example
dependency. `cargo run -p xtask -- verify-layers` MUST pass with no new violation and no matrix
edit.

#### Scenario: The Cargo.toml names exactly two workspace path dependencies

- GIVEN `crates/persistence-memory/Cargo.toml`
- WHEN it is inspected
- THEN it names exactly `ego-persistence-api` and `ego-domain` as workspace path dependencies,
  and no other workspace crate

#### Scenario: The layer gate passes with no matrix edit

- GIVEN `layers.toml` gains the one `ego-persistence-memory = "foundation"` entry and
  `xtask/src/layers.rs` is untouched
- WHEN `cargo run -p xtask -- verify-layers` runs
- THEN it passes with no new violation

### Requirement: R12 — Effects Scope Integrity

`crates/runtime/` and `crates/effect-store/` MUST be unmodified; `InMemoryEffectStore` and its
three ports MUST be byte-identical; CORE-PERSIST-A's D-9 boundary MUST stay intact.

#### Scenario: The effect-store crates are untouched

- GIVEN `crates/runtime/` and `crates/effect-store/` before this change
- WHEN this change completes
- THEN both are byte-identical to before, including `InMemoryEffectStore` and
  `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance`

### Requirement: R13 — No Postgres Refactor

Zero SQL, migration, schema, or `crates/persistence/` file MUST appear in the diff.

#### Scenario: The diff touches no Postgres-owning file

- GIVEN the full diff of this change
- WHEN it is inspected for SQL, migration, schema, or `crates/persistence/` files
- THEN none appear

### Requirement: R14 — No Conformance Framework Expansion

No conformance harness MUST be added, extended, or generalized; `assert_event_store_conformance`
and the reservation lease tests MUST keep their current shape and home.

#### Scenario: The conformance harness is unchanged

- GIVEN `assert_event_store_conformance` and the reservation lease tests before this change
- WHEN this change completes
- THEN both keep their pre-change shape and crate location

### Requirement: R15 — No Contract Or Trait Redesign

`crates/persistence-api/src/**` MUST be unmodified; no port's method set, bounds, supertraits,
default bodies, or object-safety MUST change.

#### Scenario: The port crate is byte-identical

- GIVEN `crates/persistence-api/src/**` before this change
- WHEN this change completes
- THEN it is byte-identical, and no port's method set, bounds, supertraits, default bodies, or
  object-safety differ

### Requirement: R16 — No Test Double Of Any Kind Is Promoted

`TestClock` MUST stay in `ego-testkit`, and no `#[cfg(test)]`-local or `tests/`-local double MUST
be moved into `ego-persistence-memory`.

#### Scenario: TestClock and local doubles stay put

- GIVEN `TestClock` and every `#[cfg(test)]`-local or `tests/`-local double before this change
- WHEN this change completes
- THEN none of them is declared in `ego-persistence-memory`, and `TestClock` remains in
  `ego-testkit`

### Requirement: R17 — The Two `persistent-entity` Duplicates Are Named Debt, Not Silently Handled

Both the `EventStore`/`EventStoreUnitOfWork` additive-capability duplicate and the
tenant-ignoring `Snapshot` duplicate MUST be recorded as KD-6 and KD-5 respectively, each with a
named follow-up owner (F-6, F-5), and neither MUST be moved, merged, fixed, nor partially
addressed.

#### Scenario: Both duplicates are named, not moved

- GIVEN `persistent-entity`'s `InMemoryEventStore`/`StagingUnitOfWork` and `InMemorySnapshotStore`
- WHEN this change completes
- THEN both remain exactly where they were, and the change's documentation names them as KD-6/F-6
  and KD-5/F-5 respectively

#### Scenario: The tenant-isolation defect is not fixed inside this move

- GIVEN `persistent-entity`'s `InMemorySnapshotStore` ignores `tenant_id` (a confirmed defect)
- WHEN this change completes
- THEN the defect still reproduces identically, and is recorded as named debt (KD-5) rather than
  fixed

### Requirement: R18 — The Effect-Store Boundary Is Named Debt, Not Silently Handled

The future change that would relocate the effect-store ports and consolidate
`InMemoryEffectStore` MUST be named (CORE-PERSIST-E) with its prerequisite stated (port
relocation before implementation consolidation), and nothing in that boundary MUST be touched by
this change.

#### Scenario: The boundary is documented, not crossed

- GIVEN the D-9/D-10 boundary between `ego-persistence-memory` and `ego-runtime`'s effect-store
  ports
- WHEN this change's documentation is inspected
- THEN it names CORE-PERSIST-E as the follow-up and states that port relocation must land first,
  and no effect-store file is touched
