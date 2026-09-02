# Delta for application-composition

> Canonical / source of truth. Spanish review companion: `spec.es.md` (1:1
> identifiers). Base capability spec:
> `openspec/specs/application-composition/spec.md`. This delta follows the
> exact shape of that spec's "Duplicate Effect Store Registration Through
> AppBuilder Fails Closed" requirement.

Scope: PROD-014A. Adds a composition-root registration point for a
projection's durable progress pair, keyed by `projection_id` (D-3). The
exact public surface (two methods, one combined method, a registration
struct) is a `design.md` decision; this delta specifies only observable
registration, validation, and refusal behavior.

## ADDED Requirements

### Requirement: Read-Side Durable Progress Pair Registration, Keyed By Projection ID, The Pair As The Unit

`AppBuilder` MUST provide a composition-root registration point for a
projection's durable progress pair — its `OffsetStore` and `DedupStore`
together — keyed by `projection_id`. The pair MUST be the unit of
registration: a registration covering only one of the two stores MUST NOT
be representable through the public surface, so a partial configuration
can never pass validation as if both were covered. Two different
`projection_id`s MAY register distinct store instances, and MAY also
share the same store instance across projections without that sharing
being treated as a conflict.

#### Scenario: Two projections register distinct pairs independently

- GIVEN two different `projection_id`s
- WHEN each registers its own `OffsetStore`/`DedupStore` pair
- THEN both registrations succeed and remain distinct at `build()`

#### Scenario: Partial registration of only one store is not representable

- GIVEN the public registration surface
- WHEN an application attempts to supply only an `OffsetStore` or only a
  `DedupStore` for a `projection_id` without the other
- THEN the surface offers no way to do so — the pair is always the unit
  supplied together

#### Scenario: The same store instance may be shared across projection_ids

- GIVEN one `OffsetStore`/`DedupStore` instance pair
- WHEN it is registered for two different `projection_id`s
- THEN both registrations succeed — sharing an instance across
  projections is not a conflict

### Requirement: Duplicate Read-Side Durable Progress Registration Through AppBuilder Fails Closed

Registering a second durable progress pair for the same `projection_id`
MUST fail the same way `.adapter()`/`.projection()`/`.entity()`/
`.effect_store()` already fail closed: latched as a composition error and
surfaced only through `AppBuilder::build()`'s existing composition-error
reporting, never a silent overwrite. If a composition error has already
latched from any prior registration call, a subsequent durable-progress
registration call MUST NOT further mutate registration state, and the
pre-existing error remains what surfaces at `build()`.

#### Scenario: Duplicate registration for the same projection_id surfaces at build, not silently replaced

- GIVEN a durable progress pair registered twice for the same
  `projection_id`
- WHEN `AppBuilder::build()` is called
- THEN construction fails with a composition error identifying the
  duplicate `projection_id`, and the first-registered pair is what would
  have resolved had construction succeeded

#### Scenario: A pre-existing composition error is not overwritten by a later registration call

- GIVEN a composition error already latched from an earlier registration
  failure
- WHEN a durable-progress registration call is made afterward
- THEN the builder is returned unmodified and the original composition
  error, not a new one, is what surfaces at `build()`

### Requirement: A Registered Durable Progress Pair Is The Pair The Projection Actually Uses

Registering a projection's durable progress pair at the composition root
MUST supply the actual `OffsetStore`/`DedupStore` instances that
projection's execution uses — not a declaration parallel to, and
potentially divergent from, the pair passed to `ProjectionSpec`/
`TagSchedulerImpl::spawn`. A composition MUST NOT be able to register a
durable pair at the composition root while a different, volatile pair is
what the projection actually spawns with.

#### Scenario: The registered pair is the pair the projection spawns with

- GIVEN a projection registered with a durable pair at the composition
  root
- WHEN that projection's read-side execution is composed
- THEN the `OffsetStore`/`DedupStore` instances it spawns with are the
  same instances registered at the composition root

#### Scenario: The reference host's Production path obtains its pair from the composition root

- GIVEN `examples/reference-app`'s Production composition path
- WHEN its read-side handles are constructed
- THEN the `OffsetStore`/`DedupStore` pair originates at the composition
  root, rather than being unconditionally constructed as
  `InMemoryOffsetStore`/`InMemoryDedupStore` inside
  `ReadSideHandles::new()`
