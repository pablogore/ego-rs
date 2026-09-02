# Delta for production-composition-hardening

> Canonical / source of truth. Spanish review companion: `spec.es.md`
> (1:1 identifiers). Base capability spec: PROD-013's own delta,
> `openspec/changes/2026-08-25-prod-013-production-composition-hardening/specs/production-composition-hardening/spec.md`
> (not yet archived to `openspec/specs/`; this delta applies on top of it
> and both apply together at archive time).

Scope: PROD-014A. Extends the Production gate to read-side durable
progress as a fourth governed capability (event store, snapshot store,
effect store, read-side durable progress), reusing the identical
mechanism (`is_durable()` + `require_durably_configured`) and error shape
(`PersistenceCompositionError::NotConfigured { capability, fix }`,
surfaced through `CompositionError::Validation`) PROD-013 established.

## ADDED Requirements

### Requirement: Read-Side Durable Progress Gate Under Production, Enforced At Bootstrap, Conditional On A Registered Projection

Under `Profile::Production`, when at least one projection's durable
progress pair (`OffsetStore` + `DedupStore`) is registered at the
composition root, `AppBuilder::build()` MUST reject the bootstrap when
either store of that pair is not durable, naming the missing/non-durable
capability and the exact registration call that fixes it. This mirrors
the effect store gate's conditionality: when no projection is registered,
no read-side durable progress pair exists to construct, so there is
nothing volatile to gate — a command-only or non-read-side application is
never forced to register a dummy store. The gate travels the same
existing path PROD-013 established (`AppBuilder::build()` ->
`RuntimeBuilder::try_build()` -> `validate_persistence_profile()` ->
`RuntimeError` -> `CompositionError::Validation`), never a second,
parallel validator. Durability is determined solely by `is_durable()` on
each store, fed into `require_durably_configured()` — never from
`.is_some()` or any other heuristic.

#### Scenario: A volatile store in a registered pair is rejected at bootstrap

- GIVEN `Profile::Production` and a registered projection whose
  `OffsetStore` or `DedupStore` is not durable
- WHEN `AppBuilder::build()` runs
- THEN it is rejected, naming the missing/non-durable capability and the
  exact registration call that fixes it — never deferred to
  `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, or the first
  batch

#### Scenario: No projection registered means nothing to gate

- GIVEN `Profile::Production` and no read-side projection progress
  registered at all
- WHEN `AppBuilder::build()` runs
- THEN it succeeds — no read-side durable progress pair is constructed,
  so nothing volatile is reachable

#### Scenario: Both stores durable succeeds

- GIVEN `Profile::Production` and a registered projection whose
  `OffsetStore` and `DedupStore` are both durable
- WHEN `AppBuilder::build()` runs
- THEN it succeeds

#### Scenario: Dev profile with volatile stores is unchanged

- GIVEN `Profile::Dev` (the default) and a registered projection using
  in-memory/volatile `OffsetStore`/`DedupStore`
- WHEN `AppBuilder::build()` runs
- THEN it succeeds, byte-for-byte as before this change

### Requirement: Profile::Production's Doc Comment Reflects The Read-Side Durable Progress Slot

`Profile::Production`'s doc comment (`crates/persistent-entity/src/profile.rs`)
MUST NOT state that read-side/projection persistence "has no such slot
yet and is deliberately not governed here" once this change ships — that
statement becomes false the moment the registration exists. The doc
comment MUST instead name the read-side durable progress pair as a fourth
governed capability alongside the event store, snapshot store, and effect
store, and MUST NOT point to a successor identifier as still-pending work
for a capability this change already governs.

#### Scenario: The doc comment lists the fourth governed capability

- GIVEN `Profile::Production`'s doc comment after this change ships
- WHEN it is read
- THEN it lists the read-side durable progress pair alongside the event
  store, snapshot store, and effect store as governed capabilities, and
  does not claim read-side has no composition-root slot

## MODIFIED Requirements

### Requirement: One Shared Predicate Is The Single Source Of Truth For The Rule

Exactly one shared predicate MUST decide "declared production + capability
not durably configured = refuse" for all four capabilities (event store,
snapshot store, effect store, read-side durable progress). Because the
capabilities live across a one-way crate boundary (`persistent-entity`
cannot see `service-sdk`'s effect-store or read-side types), this
predicate cannot itself inspect either builder directly: each composition
surface (`EntityRuntimeBuilder`'s `validate_persistence()`,
`RuntimeBuilder`'s `validate_persistence_profile()`, including its
read-side branch) MUST compute its own capability's answer locally and
pass it to the one shared predicate — never restate the refuse/allow
decision itself. No second, independently-maintained definition of the
decision MUST exist anywhere in the composition path.

(Previously: scoped to three capabilities, event store/snapshot
store/effect store; now includes read-side durable progress as a fourth,
validated by the same `validate_persistence_profile()` the effect store
already uses.)

#### Scenario: All three capabilities' decision routes through the same predicate

- GIVEN the composition path from `EntityRuntimeBuilder` and
  `RuntimeBuilder`/`AppBuilder`
- WHEN the codebase is inspected for capability-gating logic on event
  store, snapshot store, or effect store
- THEN every gate call site computes its own local answer and passes it
  to the one shared predicate that decides refuse-or-allow; no call site
  reimplements that decision itself

#### Scenario: The fourth capability's decision routes through the same predicate

- GIVEN the read-side durable progress gate added by this change
- WHEN the codebase is inspected for its gating logic
- THEN it computes its own local answer (are both stores of a registered
  pair durable?) and passes it to the same shared predicate the other
  three capabilities already use — no separate, independently-maintained
  read-side-only decision exists

### Requirement: Rejections Are Actionable

Every rejection under this spec MUST name both the missing capability and
the exact configuration call that resolves it.

(Previously: enumerated three capabilities in its scenario; now includes
read-side durable progress as a fourth.)

#### Scenario: Error names the capability and the fix

- GIVEN any rejection produced by this spec's gate
- WHEN the error is inspected
- THEN it names the missing capability (event store, snapshot store,
  effect store, or read-side durable progress) and the exact registration
  or builder call that configures it
