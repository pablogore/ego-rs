# Production Composition Hardening Specification

## Purpose

A composition declared as production MUST never start on volatile storage
because a durable persistent capability was not explicitly wired. This spec
defines `Profile::Production` as an explicit opt-in gate that rejects
bootstrap — with an actionable error — when any of the three
composition-root-observable persistent capabilities (event store, snapshot
store, effect store) lacks an explicitly configured durable implementation.
`Profile::Dev` (the default) preserves today's behavior byte-for-byte.

## Requirements

### Requirement: Explicit Profile Declaration At The Composition Root

The system MUST provide a `Profile` enum with exactly two variants,
`Profile::Dev` and `Profile::Production`, and a builder method at the
composition root to set it. `Profile::Dev` MUST be the default when no
profile is declared.

#### Scenario: No profile declared preserves today's default
- GIVEN a composition that never calls the profile-setting builder method
- WHEN the composition is built
- THEN it behaves as `Profile::Dev`, identical to today's behavior

#### Scenario: Explicit declaration sets Production
- GIVEN a composition that calls the profile-setting builder method with
  `Profile::Production`
- WHEN the composition is built
- THEN it is evaluated under `Profile::Production`'s rules

### Requirement: Event Store Gate Under Production

Under `Profile::Production`, `EntityRuntimeBuilder::build()` MUST reject the
bootstrap when no durable event store was explicitly configured, with an
error naming the event store capability and `EntityRuntimeBuilder::with_event_store()`.

#### Scenario: Missing event store rejected under Production
- GIVEN `Profile::Production` and no call to `.with_event_store()`
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it is rejected with an error naming the event store and
  `EntityRuntimeBuilder::with_event_store()`
- AND `InMemoryEventStore` is never constructed

### Requirement: Snapshot Store Gate Under Production

Under `Profile::Production`, `EntityRuntimeBuilder::build()` MUST reject the
bootstrap when no durable snapshot store was explicitly configured, with an
error naming the snapshot store capability and
`EntityRuntimeBuilder::with_snapshot_store()`.

#### Scenario: Missing snapshot store rejected under Production
- GIVEN `Profile::Production` and no call to `.with_snapshot_store()`
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it is rejected with an error naming the snapshot store and
  `EntityRuntimeBuilder::with_snapshot_store()`
- AND `InMemorySnapshotStore` is never constructed

### Requirement: Effect Store Gate Under Production, Enforced At Bootstrap, Conditional On A Registered Executor

Under `Profile::Production`, when at least one effect executor is
registered, the composition MUST reject the bootstrap when no effect store
was explicitly configured, naming the configuration call for the
composition surface in use (`RuntimeBuilder::with_effect_store()` or
`AppBuilder::effect_store()`). This closes the same silent-volatility
defect the event and snapshot store gates close: the effect store has a
real fallback to `InMemoryEffectStore`
(`crates/service-sdk/src/runtime/builder.rs:811`) that runs at bootstrap,
not a failure deferred to first use. When no effect executor is registered,
no effect store is constructed at all, so there is nothing volatile to
gate.

#### Scenario: Missing effect store rejected at bootstrap when an executor is registered
- GIVEN `Profile::Production`, at least one registered effect executor, and
  no call to `.with_effect_store()` / `.effect_store()`
- WHEN the composition builds
- THEN it is rejected immediately, naming the missing capability and the
  call that fixes it — the same bootstrap-time silent fallback the event
  and snapshot store gates close, never left to fail on first attempted use

#### Scenario: No effect executor registered means nothing to gate
- GIVEN `Profile::Production` and no registered effect executor
- WHEN the composition builds
- THEN it succeeds regardless of whether an effect store was configured —
  no effect store is constructed, so nothing volatile is reachable

### Requirement: Partial Event/Snapshot Configuration Under Production Is Covered By The Per-Capability Gates

Under `Profile::Production`, if exactly one of `{event_store,
snapshot_store}` is configured and the other is not,
`EntityRuntimeBuilder::build()` MUST reject the bootstrap — not through a
separate partial-configuration check, but because the missing capability's
own gate (Event Store Gate / Snapshot Store Gate, above) already rejects
it: exactly one missing is still one missing. Under `Profile::Dev`, partial
configuration remains valid, unchanged from today's behavior — including
the 15 existing call sites that configure exactly one of the two stores
today (`design.md` §Evidence Corrections, EC-1), among them the reference
app's own production composition root, `observed_entity_runtime`
(`lib.rs:502`).

#### Scenario: Partial configuration rejected under Production via its own capability gate
- GIVEN `Profile::Production` with `.with_event_store()` called and
  `.with_snapshot_store()` never called
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it is rejected by the Snapshot Store Gate (naming the snapshot store
  and `.with_snapshot_store()`), not by a separate partial-configuration
  check

#### Scenario: Partial configuration remains valid under Dev, unchanged
- GIVEN `Profile::Dev` (the default, no profile declared) with
  `.with_event_store()` called and `.with_snapshot_store()` never called
- WHEN `EntityRuntimeBuilder::build()` runs
- THEN it succeeds, falling back to `InMemorySnapshotStore` for the
  unconfigured capability, byte-for-byte as before this change

### Requirement: One Validator Is The Single Source Of Truth

Exactly one private validator MUST implement the rule across all three
capabilities (event store, snapshot store, effect store). No second,
parallel check MUST exist anywhere in the composition path.

#### Scenario: All three capabilities route through the same validator
- GIVEN the composition path from `EntityRuntimeBuilder` and
  `RuntimeBuilder`/`AppBuilder`
- WHEN the codebase is inspected for capability-gating logic
- THEN exactly one validator per composition surface implements the rule;
  no duplicate, independently-maintained check exists

### Requirement: Rejections Are Actionable

Every rejection under this spec MUST name both the missing capability and
the exact configuration call that resolves it.

#### Scenario: Error names the capability and the fix
- GIVEN any rejection produced by this spec's gate
- WHEN the error is inspected
- THEN it names the missing capability (event store, snapshot store, or
  effect store) and the exact builder call that configures it

### Requirement: Non-Production Compositions Compile And Pass Unmodified

Every composition that does not declare `Profile::Production` MUST continue
to compile and pass without modification, including all 67 existing
`EntityRuntimeBuilder::new()` call sites. `cargo test --workspace` MUST show
zero new failures attributable to this change.

#### Scenario: An unmodified call site still builds on in-memory storage
- GIVEN an existing call site that never declares `Profile::Production` and
  never configures event/snapshot/effect store
- WHEN it is rebuilt after this change ships
- THEN it compiles and `EntityRuntimeBuilder::build()` still succeeds on
  in-memory storage, byte-for-byte as before this change

#### Scenario: The full existing test suite shows no new failures
- GIVEN the full workspace test suite before and after this change
- WHEN `cargo test --workspace` is run after the change
- THEN it reports zero new failures caused by this change

### Requirement: Persistence Completeness Rule Is Documented

Architecture documentation MUST state the persistence completeness rule: a
database is not considered supported by `ego-rs` until it implements every
persistent capability a production composition declares it uses; missing
capabilities MUST NOT be completed by falling back to in-memory storage;
backend support is all-or-nothing across the durable capabilities a
composition enables. This is forward-looking guidance, not a report that
today's only backend (PostgreSQL) is non-compliant.

#### Scenario: The rule is documented as forward-looking, not a violation report
- GIVEN the architecture documentation added by this change
- WHEN it is read
- THEN it states the completeness rule explicitly and does not flag
  PostgreSQL, the only backend that exists today, as violating it

### Requirement: The PROD-005 Boundary Is Documented

Documentation MUST state explicitly that this spec rejects the bootstrap
itself, before anything starts, while PROD-005 (Health, Readiness and
Startup) signals the health of an application that has already started,
with degraded mode permitted for optional dependencies — so the two are
never conflated.

#### Scenario: The boundary is readable and unambiguous
- GIVEN the documentation added by this change
- WHEN a reader compares this spec's scope to PROD-005's
- THEN the text states plainly that this spec decides whether the app may
  start at all, and PROD-005 describes an app that already did

### Requirement: Reference App Declares Its Profile Through EntityEventStores

`build_runtime_with` (`lib.rs:567`) is the shared entry point for both
durable and in-memory compositions and MUST NOT hardcode a profile itself
— it is called with in-memory stores from four places today, and
hardcoding `Profile::Production` inside it would break every one of them.
Instead, `EntityEventStores` — the type that already exists so the choice
of backing store is stated, never defaulted — MUST carry the profile:
`EntityEventStores::open(pool)` MUST produce `Profile::Production`, and
`EntityEventStores::in_memory()` MUST produce `Profile::Dev`, through a
private field set only by those two constructors. `main.rs`, which already
calls `EntityEventStores::open()`, becomes a `Profile::Production`
composition with no separate declaration to forget.

#### Scenario: EntityEventStores::open yields Production
- GIVEN `EntityEventStores::open(pool)`
- WHEN the resulting value's profile is inspected
- THEN it reports `Profile::Production`

#### Scenario: EntityEventStores::in_memory yields Dev
- GIVEN `EntityEventStores::in_memory()`
- WHEN the resulting value's profile is inspected
- THEN it reports `Profile::Dev`

#### Scenario: Dev-oriented callers stay on Profile::Dev with no edit
- GIVEN `build_runtime_in_memory` (`lib.rs:311`) and
  `build_runtime_observed_in_memory` (`lib.rs:522`)
- WHEN their composition code is inspected
- THEN each flows through `EntityEventStores::in_memory()` and stays on
  `Profile::Dev` with no modification

### Requirement: The Reference App's Production Snapshot Store Is Durable

`EntityEventStores::open(pool)` MUST construct its two snapshot stores as
`PostgreSQLSnapshotStore` instances (already implemented at
`crates/persistence/src/postgres/snapshot.rs:27`) over the same pool,
replacing the in-memory snapshot store the production composition path
uses silently today. `EntityEventStores::in_memory()` MUST continue
constructing `InMemorySnapshotStore` instances, unchanged. This wires an
already-existing backend into an already-existing constructor; it builds
no new storage, so it remains within this spec's Non-Goal of no new
Postgres backend.

Implementation note for `tasks`/`apply`:
`PostgreSQLSnapshotStore::save_snapshot` calls
`tokio::task::block_in_place` (`snapshot.rs:46-48`), which panics on a
current-thread Tokio runtime. Any test that can trigger a real snapshot
save against Postgres MUST run under
`#[tokio::test(flavor = "multi_thread")]` rather than the bare
current-thread default.

#### Scenario: EntityEventStores::open wires durable snapshot stores
- GIVEN `EntityEventStores::open(pool)`
- WHEN the resulting value's snapshot stores are inspected
- THEN both are backed by `PostgreSQLSnapshotStore` over the same pool, not
  `InMemorySnapshotStore`

#### Scenario: EntityEventStores::in_memory keeps volatile snapshot stores
- GIVEN `EntityEventStores::in_memory()`
- WHEN the resulting value's snapshot stores are inspected
- THEN both remain `InMemorySnapshotStore`, unchanged from today

### Requirement: A Regression Check Guards The Reference Declaration

A check (an `xtask` lint or a test — the exact mechanism is a `design.md`
decision) MUST fail the build if `EntityEventStores::in_memory().profile()`
ever stops reporting `Profile::Dev`, or if `main.rs`'s composition — the
one caller that reaches `EntityEventStores::open()` — ever stops resulting
in a `Profile::Production` composition, so the reference composition stays
a live regression guard rather than a one-time example. Asserting the
behavior of `EntityEventStores`'s constructors is sufficient: with the
profile field private and those two constructors its only source, there is
no separate declaration elsewhere that could drift from them.

#### Scenario: Removing the production wiring fails the check
- GIVEN a composition that reaches `EntityEventStores::open()` no longer
  resulting in a `Profile::Production` composition
- WHEN the regression check runs
- THEN the build fails, naming the missing declaration

#### Scenario: The declaration present passes the check
- GIVEN `EntityEventStores::open()` yielding `Profile::Production` and
  `EntityEventStores::in_memory()` yielding `Profile::Dev`, both wired as
  required
- WHEN the regression check runs
- THEN it passes

## Non-Goals

- **No new Postgres backend.** Durable event, snapshot, and effect stores
  already exist; this spec validates configuration and wires the
  already-existing `PostgreSQLSnapshotStore` into `EntityEventStores::open`
  — it does not implement or build new storage.
- **No read-side/projection/checkpoint gate, real or pseudo, anywhere in
  this change.** No generic read-side registration exists at the
  composition root today. Deferred verbatim to **PROD-014 — Read-Side
  Persistence Composition & Durable Store**, which MUST introduce a generic
  read-side/projection persistence registration at the composition root and,
  from its introduction, apply the identical fail-closed policy this spec
  establishes: capability not configured → valid; capability configured
  with a non-durable/in-memory backend → startup rejected; capability
  configured with a durable backend → valid.
- **No observability, HA, migration, or other production-hardening theme.**
- **No second database engine.** The persistence completeness rule is
  forward-looking; nothing to validate against exists today.
- **No decision on Approach C** (flipping the default to fail-closed with a
  named opt-out, mirroring `IdempotencyEnforcementMode`) or its ~32-call-site
  migration. Recorded as an evaluated, deferred alternative in `design.md`.
- **No removal, deprecation, or hiding of in-memory implementations.** They
  remain valid, explicit, and first-class for `Profile::Dev` and tests.
- **No re-opening of PROD-012's idempotency rule or enforcement mode.**
- **No outbox/inbox pattern.** Confirmed absent from the codebase; not
  applicable.
