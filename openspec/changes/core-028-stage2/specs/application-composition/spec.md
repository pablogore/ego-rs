# Delta for application-composition

> The base capability spec this delta applies to is
> `openspec/specs/application-composition/spec.md` (Stage 1, archived via
> PR #189). This delta's requirements apply on top of that spec's "Adapter
> Registration" and "Single Composition Root" requirements.

Scope: CORE-028 Stage 2. Extends the composition-root facade with
projection registration, following the exact delegation pattern Stage 1
already established for adapters and services (`.adapter()`/`.service()` as
thin pass-throughs to `RuntimeBuilder`). The read-side engine
(`ReadSideHandles`, `TagSchedulerImpl::spawn_projection`) is unmodified and
out of scope — this delta only makes an existing, already-produced-into
queryable handle registrable through the composition root.

## ADDED Requirements

### Requirement: Projection Registration Facade

`AppBuilder` MUST provide a `.projection(...)` registration method that is a
thin pass-through to the completed `RuntimeBuilder` projection-registration
path (Stage 1 facade precedent: `.adapter()`/`.service()`) — no parallel
registration or resolution mechanism is introduced. A projection registered
through `AppBuilder::projection(...)` MUST be resolvable exactly as if it had
been registered directly on `RuntimeBuilder`.

#### Scenario: A projection registered via AppBuilder resolves after build
- GIVEN a projection instance registered via `AppBuilder::projection(...)`
- WHEN the application is built
- THEN a service declaring a dependency on that projection type resolves it
  successfully

#### Scenario: Registration is equivalent whether performed via RuntimeBuilder or AppBuilder
- GIVEN two otherwise-identical applications, one composed by registering a
  projection directly on `RuntimeBuilder` and one by registering the same
  projection via `AppBuilder::projection(...)`
- WHEN each is built
- THEN both expose the same resolvable projection dependency, with no
  observable difference in outcome

#### Scenario: No internal runtime type is required to register a projection
- GIVEN a developer composing an application through `AppBuilder`
- WHEN they register a projection using only `.projection(...)`
- THEN they never construct or reach into `RuntimeBuilder` or any other
  internal runtime state to complete that registration

### Requirement: Duplicate Projection Registration Through AppBuilder Fails Closed

Registering a second projection of the same type through
`AppBuilder::projection(...)` MUST fail the same way the underlying
`RuntimeBuilder` projection registration already fails closed (see the
`service-sdk` delta's "Duplicate Projection Registration Fails Closed"
requirement) — surfaced through `AppBuilder::build()`'s existing
composition-error reporting, never a silent replacement.

#### Scenario: Duplicate projection registration surfaces at build, not silently replaced
- GIVEN `AppBuilder::projection(...)` called twice for the same projection
  type
- WHEN `AppBuilder::build()` is called
- THEN construction fails with a composition error identifying the
  duplicate registration, and the first-registered projection instance is
  what would have resolved had construction succeeded

## Non-Goals

- Any change to `ReadSideHandles`, `TagSchedulerImpl`, `spawn_projection`,
  or read-side projection spawn/stop lifecycle ownership — unchanged by
  this delta.
- Unifying the read-side engine's delivery path with this DI registration
  path — the read-side engine keeps producing into the registered
  projection unchanged; this delta only makes the queryable handle
  registrable.
- `.entity::<E>()` — still blocked by CORE-006, unchanged from Stage 1.
- Reference-app's own registration of `UsersByTenantStore` through this
  path — that is proof-of-use for this capability, tracked by
  `core-028-stage2`'s tasks, not a new requirement of this spec.
