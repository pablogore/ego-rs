# Delta for service-sdk

Scope: CORE-028 Stage 2. Completes the projection *resolution* contract
(`DepKey::Projection`, `ProjectionRef<P>`, `RuntimeInner::resolve_projection`,
already validated at `try_build()`) with a public *registration* path on
`RuntimeBuilder`. Today, no production code path can populate the projection
table — only `RuntimeInner`'s own in-crate test fixtures reach into it
directly. Additive only: `resolve_projection`, `DepKey::Projection`,
`ProjectionRef<P>`, and the existing adapter/config/service registration
methods are unchanged by this delta.

## ADDED Requirements

### Requirement: Projection Registration Completes The Resolution Contract

`RuntimeBuilder` MUST provide a public method to register a projection
instance, making it resolvable via `RuntimeInner::resolve_projection::<P>()`
on the built runtime — the same resolution path a service's
`Injectable::build` already uses to obtain a `ProjectionRef<P>`. Before this
method exists, a service declaring a projection dependency has no production
path to satisfy it; after it exists, that dependency is satisfiable exactly
like an adapter or config dependency is today.

#### Scenario: A registered projection is resolvable by a dependent service
- GIVEN a projection instance registered on `RuntimeBuilder` for a given type
- WHEN a service declaring a dependency on that projection type is
  constructed against the built runtime
- THEN it receives that projection instance as `ProjectionRef<P>`

#### Scenario: Resolving an unregistered projection type fails closed, naming the type
- GIVEN a `RuntimeBuilder` with no registration for a given projection type
- WHEN `resolve_projection::<P>()` is called against the built runtime, or a
  service declaring that dependency is validated or constructed
- THEN the call fails with the existing `DependencyNotFound` error naming
  that projection type — no panic, and no silently-empty or default
  projection is fabricated

### Requirement: Duplicate Projection Registration Fails Closed

Registering a second projection instance for a type that was already
registered MUST be rejected at build, mirroring the fail-closed contract
`RuntimeBuilder::with_service` already applies to a duplicate service
registration (`RegistryError::DuplicateService`) — never a silent
last-write-wins replacement. This is a deliberate departure from
`with_adapter`'s and `with_config`'s existing last-write-wins semantics:
those two registration methods are unchanged by this delta, and projection
registration does not adopt their replace-on-conflict behavior.

#### Scenario: First registration for a projection type succeeds
- GIVEN a fresh `RuntimeBuilder`
- WHEN a projection instance is registered for a type with no prior
  registration
- THEN the registration succeeds and the instance is later resolvable

#### Scenario: A second registration of the same projection type is rejected, not silently replaced
- GIVEN a `RuntimeBuilder` that already has a registered projection instance
  for a given type
- WHEN a second projection instance of the same type is registered
- THEN the registration fails, the originally registered instance remains
  the one resolvable afterward, and no silent overwrite occurs

*Open → design.md*: the exact registration method's name and signature, and
whether a deliberate, explicitly-named override/replace operation is
introduced alongside the fail-closed default, are not decided by this spec —
only the fail-closed default and its testable outcome above are required.

### Requirement: A Declared Projection Dependency Is Satisfiable At Build

A service that declares a projection dependency through
`Injectable::dependencies()` MUST build and resolve it when the projection
was registered, and MUST fail before startup — naming the missing
projection type — when it wasn't, using the same `try_build()` /
`DependencyNotFound` attribution path already used for adapter and config
dependencies.

#### Scenario: try_build succeeds when the declared projection dependency is registered
- GIVEN a service declaring a projection dependency, recorded via
  `with_injectable`, whose projection type was registered on the same
  `RuntimeBuilder`
- WHEN `try_build()` is called
- THEN it succeeds, and the service's declared projection dependency
  resolves during construction

#### Scenario: try_build fails before startup when the declared projection dependency is missing
- GIVEN a service declaring a projection dependency, recorded via
  `with_injectable`, whose projection type was never registered
- WHEN `try_build()` is called
- THEN it fails with `DependencyNotFound` naming both the missing
  projection type and the requesting service, and no `Runtime` is produced

## Non-Goals

- `.entity::<E>()` / any per-aggregate entity registration — still blocked
  by CORE-006, unchanged from Stage 1.
- Any change to `ReadSideHandles`, `TagSchedulerImpl::spawn_projection`, or
  read-side projection spawn/stop lifecycle ownership.
- Framework-owned read models, projection discovery, or a read-side DSL.
