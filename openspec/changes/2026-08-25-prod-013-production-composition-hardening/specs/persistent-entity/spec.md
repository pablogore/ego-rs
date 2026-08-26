# Delta for Persistent Entity

## ADDED Requirements

### Requirement: EntityRuntimeBuilder Gates In-Memory Fallback By Profile

`EntityRuntimeBuilder` MUST accept a `Profile` declaration (`Profile::Dev`
default, `Profile::Production`). `EntityRuntimeBuilder::build()`'s two
`unwrap_or_else` fallbacks to `InMemoryEventStore` and
`InMemorySnapshotStore` MUST only execute under `Profile::Dev`. Under
`Profile::Production`, a missing event store or missing snapshot store
MUST reject the bootstrap with a local error naming the capability and
the exact configuration call (`.with_event_store()` /
`.with_snapshot_store()`) that fixes it, instead of silently falling back.
Because `persistent-entity` has no dependency on `service-sdk`, this error
type MUST be defined locally in `persistent-entity` and cross the layer
boundary the same way `RuntimeError::OperationReservationStoreNotRegistered`
already does one layer up.

#### Scenario: Production with no event store rejects instead of falling back
- GIVEN `Profile::Production` and no call to `.with_event_store()`
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it rejects with an error naming the event store and
  `.with_event_store()`; `InMemoryEventStore` is never constructed

#### Scenario: Production with no snapshot store rejects instead of falling back
- GIVEN `Profile::Production` and no call to `.with_snapshot_store()`
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it rejects with an error naming the snapshot store and
  `.with_snapshot_store()`; `InMemorySnapshotStore` is never constructed

#### Scenario: Dev profile preserves today's silent fallback unchanged
- GIVEN `Profile::Dev` (the default) and neither store configured
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it succeeds on `InMemoryEventStore` and `InMemorySnapshotStore`,
  byte-for-byte as before this change

### Requirement: Partial Event/Snapshot Configuration Under Production Is Covered By The Per-Capability Gates

`EntityRuntimeBuilder::build()` does not need a separate
partial-configuration check. Under `Profile::Production`, if exactly one of
`{event_store, snapshot_store}` was explicitly configured and the other was
not, the profile-gated fallback above already rejects it — the missing
capability's own gate fires because it is, in fact, missing. Under
`Profile::Dev`, partial configuration remains valid, unchanged from today's
behavior; this is not a new exemption but the same behavior every existing
partial call site already relies on, including 14 test chains under
`crates/persistent-entity/tests/` and
`examples/reference-app/src/lib.rs:502` (`design.md` §Evidence Corrections,
EC-1).

#### Scenario: One store configured, the other missing, rejected under Production via its own gate
- GIVEN `Profile::Production` with `.with_event_store()` called and
  `.with_snapshot_store()` never called
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it rejects via the profile-gated snapshot-store fallback above
  (naming the snapshot store and the fix), not via a separate
  partial-configuration check

#### Scenario: One store configured, the other missing, remains valid under Dev
- GIVEN `Profile::Dev` (the default) with `.with_event_store()` called and
  `.with_snapshot_store()` never called
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it succeeds, falling back to `InMemorySnapshotStore` for the
  unconfigured capability, unchanged from today

### Requirement: Existing EntityRuntimeBuilder Call Sites Are Unaffected

All 67 existing `EntityRuntimeBuilder::new()` call sites, none of which
declare `Profile::Production`, MUST continue to compile and pass without
modification after this change ships.

#### Scenario: An unmodified call site keeps compiling and passing
- GIVEN any of the 67 existing call sites, none declaring
  `Profile::Production`
- WHEN the workspace is rebuilt after this change
- THEN it compiles and its tests pass without any modification
