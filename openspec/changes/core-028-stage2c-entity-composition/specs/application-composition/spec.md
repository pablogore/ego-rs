# Delta for application-composition

> Base capability spec: `openspec/specs/application-composition/spec.md`
> (Stage 1, archived via PR #189; Stage 2A/2B deltas already merged). This
> delta adds an entity-runtime registration facade, mirroring the shipped
> `.projection()` facade (Stage 2A), and retires that same base spec's stale
> Non-Goals entry deferring `.entity::<E>()` to CORE-006. Adapter, config,
> security, lifecycle, and service registration requirements are unchanged by
> this delta.

Scope: CORE-028 Stage 2C. `DepKey::Entity` already exists in the DI
vocabulary but its dependency validation unconditionally fails — a service
declaring an entity dependency has no way to satisfy it. This delta closes
that gap by giving `AppBuilder` a registration facade for a host-constructed
entity runtime, keyed by the aggregate/entity type, following the exact
delegation pattern Stage 2A already established for projections
(`.projection()` as a thin pass-through to the completed `RuntimeBuilder`
registration path). The entity runtime itself — its construction, activation,
and passivation — is unmodified and out of scope; this delta only makes an
existing, host-constructed entity runtime registrable through the
composition root.

## ADDED Requirements

### Requirement: Entity Runtime Registration Facade

`AppBuilder` MUST provide an `.entity::<E>(...)` registration method that is a
thin pass-through to the completed `RuntimeBuilder` entity-registration path
(Stage 2A facade precedent: `.projection()`) — no parallel registration or
resolution mechanism is introduced. An entity runtime registered through
`AppBuilder::entity::<E>(...)` MUST be resolvable exactly as if it had been
registered directly on `RuntimeBuilder`.

#### Scenario: An entity runtime registered via AppBuilder resolves after build
- GIVEN a host-constructed entity runtime registered for aggregate type `E`
  via `AppBuilder::entity::<E>(...)`
- WHEN the application is built
- THEN a service declaring a dependency on that entity type resolves it
  successfully

#### Scenario: Registration is equivalent whether performed via RuntimeBuilder or AppBuilder
- GIVEN two otherwise-identical applications, one composed by registering an
  entity runtime directly on `RuntimeBuilder` and one by registering the same
  entity runtime via `AppBuilder::entity::<E>(...)`
- WHEN each is built
- THEN both expose the same resolvable entity dependency, with no observable
  difference in outcome

#### Scenario: No internal runtime type is required to register an entity runtime
- GIVEN a developer composing an application through `AppBuilder`
- WHEN they register an entity runtime using only `.entity::<E>(...)`
- THEN they never construct or reach into `RuntimeBuilder` or any other
  internal runtime state to complete that registration

### Requirement: Duplicate Entity Registration Through AppBuilder Fails Closed

Registering a second entity runtime for the same aggregate/entity type
through `AppBuilder::entity::<E>(...)` MUST fail the same way the underlying
`RuntimeBuilder` entity registration already fails closed (see the
`service-sdk` delta's "Duplicate Entity Registration Fails Closed"
requirement) — surfaced through `AppBuilder::build()`'s existing
composition-error reporting, never a silent replacement.

#### Scenario: Duplicate entity registration surfaces at build, not silently replaced
- GIVEN `AppBuilder::entity::<E>(...)` called twice for the same aggregate
  type
- WHEN `AppBuilder::build()` is called
- THEN construction fails with a composition error identifying the duplicate
  registration and the aggregate type involved, and the first-registered
  entity runtime is what would have resolved had construction succeeded

## MODIFIED Non-Goals

The base spec's Non-Goals section currently reads:

> - `.entity::<E>()` / any per-aggregate entity registration — no stable
>   entity contract exists to delegate to (deferred to CORE-006). Stage 2B
>   further specifies: no entity registration coupling to the service
>   trait-link mechanism introduced by this stage.

This entry is stale — CORE-006 (the entity contract it deferred to) shipped
and archived, and this delta delivers `.entity::<E>()`. It MUST be replaced
with:

- Framework-owned construction of the entity runtime itself (activation,
  passivation, config folding, or any change to `EntityRuntimeBuilder` or
  `EntityRegistry`) — this delta only registers a host-constructed runtime;
  it constructs nothing.
- Entity lifecycle ownership (spawn/stop) in the composition API — entity
  actors are unaffected by this delta's registration or teardown handling.
- No entity registration coupling to the service trait-link mechanism
  introduced by Stage 2B — unchanged, this non-goal survives verbatim from
  Stage 2B.
- Migrating any existing hand-threaded entity dependency off
  `.service_instance()` — proof-of-use for this capability is tracked by
  this change's tasks, not a new requirement of this spec.
