# Delta for service-sdk

> Base capability spec: `openspec/specs/service-sdk/spec.md`. This delta adds
> entity-runtime registration and resolution, mirroring the shipped
> projection-registration requirements ("Projection Registration Completes
> The Resolution Contract", "Duplicate Projection Registration Fails
> Closed", "A Declared Projection Dependency Is Satisfiable At Build", "App
> Exposes A Projection Resolution Accessor" — Stage 2A). It also retires the
> stale Stage 2B Non-Goals entry deferring any entity/aggregate counterpart
> to CORE-006. All context-propagation, tenant-enforcement, and existing
> service-registration requirements are unchanged by this delta.

Scope: CORE-028 Stage 2C. `DepKey::Entity` already exists in the DI
vocabulary, but dependency validation for it unconditionally fails — a
service declaring an entity dependency can never build today, with no
recourse. This delta makes that dependency satisfiable: a host constructs an
entity runtime for a given aggregate/entity type and registers it once;
services declaring a dependency on that aggregate type then resolve a handle
capable of dispatching to any entity instance of that type. This
composition-time, multi-entity-capable handle is distinct from
`persistent-entity`'s existing per-request handle to one specific entity
instance — both exist and this delta introduces only the former; the latter
is unchanged.

## ADDED Requirements

### Requirement: Entity Runtime Registration Completes The Resolution Contract

`RuntimeBuilder` MUST provide a public method to register a host-constructed
entity runtime for a given aggregate/entity type `E`, making it resolvable as
`EntityRuntimeRef<E>` — the same resolution shape `ProjectionRef<P>` already
provides for projections. `EntityRuntimeRef<E>` is a handle capable of
dispatching to any entity instance of type `E`; it is distinct from, and does
not replace, `persistent-entity`'s existing per-request handle to one
specific entity instance, which is obtained separately and unchanged by this
delta. Before this method exists, a service declaring an entity dependency
has no production path to satisfy it; after it exists, that dependency is
satisfiable exactly like an adapter, config, or projection dependency is
today.

#### Scenario: A registered entity runtime is resolvable by a dependent service
- GIVEN a host-constructed entity runtime registered on `RuntimeBuilder` for
  aggregate type `E`
- WHEN a service declaring a dependency on that aggregate type is constructed
  against the built runtime
- THEN it receives that entity runtime as `EntityRuntimeRef<E>`

#### Scenario: Resolving an unregistered entity type fails closed, naming the aggregate type
- GIVEN a `RuntimeBuilder` with no registration for a given aggregate type
- WHEN a service declaring that dependency is validated or constructed
  against the built runtime
- THEN the call fails with the existing `DependencyNotFound` error naming
  that aggregate type — no panic, and no default or empty entity runtime is
  fabricated

#### Scenario: A resolved entity runtime handle is distinct from a per-request entity handle
- GIVEN a service holding an `EntityRuntimeRef<E>` resolved through this
  registration path
- WHEN the service dispatches to one specific entity instance of type `E`
- THEN it does so through `persistent-entity`'s existing per-request handle,
  obtained from the entity runtime, unchanged by this delta — the
  composition-time handle and the per-request handle remain two distinct,
  coexisting concepts

### Requirement: Entity Identity Is Keyed By The Aggregate Type, Not Its Event Type

Entity runtime registration, resolution, and duplicate detection MUST all be
keyed by the aggregate/entity type `E` a service declares as its dependency —
never by `E`'s associated event type. Two distinct aggregate types that share
the same associated event type MUST register and resolve independently, with
no collision between them.

#### Scenario: A missing entity dependency names the aggregate type, not its event type
- GIVEN a service declaring a dependency on aggregate type `OrderEntity`,
  whose entity runtime was never registered
- WHEN construction is attempted
- THEN the resulting `DependencyNotFound` error names `OrderEntity` — never
  `OrderEntity`'s associated event type

#### Scenario: Two aggregates sharing an event type register and resolve without collision
- GIVEN two distinct aggregate types that share the same associated event
  type, each with its own entity runtime registered
- WHEN a service declares a dependency on one of the two aggregate types
- THEN it resolves that aggregate's own registered entity runtime, unaffected
  by the other aggregate's registration despite the shared event type

### Requirement: Duplicate Entity Registration Fails Closed

Registering a second entity runtime for an aggregate type that was already
registered MUST be rejected at build, mirroring the fail-closed contract
`RuntimeBuilder::with_service` and projection registration already apply to a
duplicate registration — never a silent last-write-wins replacement.

#### Scenario: First registration for an aggregate type succeeds
- GIVEN a fresh `RuntimeBuilder`
- WHEN an entity runtime is registered for an aggregate type with no prior
  registration
- THEN the registration succeeds and the runtime is later resolvable

#### Scenario: A second registration of the same aggregate type is rejected, not silently replaced
- GIVEN a `RuntimeBuilder` that already has a registered entity runtime for a
  given aggregate type
- WHEN a second entity runtime for the same aggregate type is registered
- THEN the registration fails, the originally registered entity runtime
  remains the one resolvable afterward, and no silent overwrite occurs

### Requirement: A Declared Entity Dependency Is Satisfiable At Build

A service that declares an entity dependency through
`Injectable::dependencies()` MUST build and resolve it when the matching
entity runtime was registered, and MUST fail before startup — naming the
missing aggregate type — when it wasn't, using the same `try_build()` /
`DependencyNotFound` attribution path already used for adapter, config, and
projection dependencies.

#### Scenario: try_build succeeds when the declared entity dependency is registered
- GIVEN a service declaring an entity dependency, recorded via
  `with_injectable`, whose aggregate type's entity runtime was registered on
  the same `RuntimeBuilder`
- WHEN `try_build()` is called
- THEN it succeeds, and the service's declared entity dependency resolves
  during construction

#### Scenario: try_build fails before startup when the declared entity dependency is missing
- GIVEN a service declaring an entity dependency, recorded via
  `with_injectable`, whose aggregate type's entity runtime was never
  registered
- WHEN `try_build()` is called
- THEN it fails with `DependencyNotFound` naming both the missing aggregate
  type and the requesting service, and no `Runtime` is produced

### Requirement: App Exposes An Entity Resolution Accessor

`App` MUST provide a read-only `resolve_entity::<E>()` accessor, symmetric
with the existing `App::resolve_adapter()`/`App::resolve_config()`/
`App::resolve_projection()` accessors, so a caller holding a built `App` (not
just a service resolved through it) can resolve a registered entity runtime.

#### Scenario: A built App resolves a registered entity runtime through the accessor
- GIVEN an `App` built with an entity runtime registered via
  `AppBuilder::entity::<E>(...)`
- WHEN `App::resolve_entity::<E>()` is called for that aggregate type
- THEN it returns the registered runtime as `EntityRuntimeRef<E>`

## MODIFIED Non-Goals

The Stage 2B delta's Non-Goals section currently reads:

> - Any entity/aggregate-facing counterpart to this trait-link mechanism
>   (`.entity::<E>()` or equivalent) — still blocked by CORE-006, unchanged
>   from Stage 1 and Stage 2A.

This entry is stale — CORE-006 shipped and archived, and this delta delivers
`.entity::<E>()`. It MUST be replaced with:

- Coupling entity registration to the Stage 2B service trait-link mechanism
  (`impl_of`) — entity registration follows the projection-registration
  pattern (a plain generic parameter naming the aggregate type), not the
  macro-generated trait-link pattern services use.
- Framework-owned construction of the entity runtime (activation,
  passivation, config folding, or any change to `EntityRuntimeBuilder` or
  `EntityRegistry`) — this delta only registers and resolves a
  host-constructed runtime.
- Entity lifecycle ownership (spawn/stop) in this capability — entity actors
  are unaffected by this delta.
