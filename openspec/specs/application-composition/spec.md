# Spec: CORE-028 Stage 1 — Application Composition API (`App` / `AppBuilder`)

> Folder retains its historical `core-026-developer-experience-refinement`
> label; the initiative is CORE-028 (see proposal.md, explore.md). Stage 0's
> read-side spec (`specs/read-side/spec.md`) is unmodified and its
> read-model-ownership decision is a hard constraint on this spec (see
> "Lifecycle" below).

## Capability: application-composition (new)

Purpose: a single composition-root API (`App`/`AppBuilder`) through which an
application developer registers services, adapters, config, and security,
then separately validates/constructs and starts the application — replacing
today's hand-sequenced `RuntimeBuilder` + manual pipeline (kit-config→logger,
read-side spawn, two-phase shutdown) every host currently re-derives
(explore.md #14). `RuntimeBuilder` itself, `Injectable`, testkit's
`FixtureBuilder`, and stage 0's `spawn_projection` are unchanged; this
capability delegates to them and specifies no parallel contract.

### Requirement: Single Composition Root

An application MUST compose its services, adapters, config, and security
through one entry point. The developer MUST NOT need to construct or touch
the framework's internal runtime state directly to assemble an application.
Registration call order MUST NOT silently change the resulting application
unless a specific ordering effect is explicitly documented as intentional.

#### Scenario: Two equivalent registration orders produce the same application
- GIVEN two composition sequences registering the same services, adapters,
  config, and security but in a different call order
- WHEN each is built
- THEN both builds produce an application with the same resolvable
  dependencies, unless an ordering effect is explicitly documented

#### Scenario: No internal runtime type is required to compose
- GIVEN a developer composing an application
- WHEN they use only the composition root's public registration calls
- THEN they never construct or reach into the framework's internal runtime
  state to complete composition

### Requirement: Build Is Separate From Starting And Starts No Process

Validating and constructing an application MUST NOT start any background
task or accept any external effect. Composition errors MUST surface at this
construction step, before any process starts. A constructed-but-not-started
application MUST be usable directly in tests (assertions, resolution)
without starting it. Starting an application MUST only be possible on an
already-constructed application — there is no direct "compose and start" in
one step that skips construction.

#### Scenario: Constructing an application starts nothing
- GIVEN an application with registered services and adapters
- WHEN it is constructed but not started
- THEN no background task is running and no external effect is being
  accepted

#### Scenario: A composition error surfaces before anything starts
- GIVEN an application registration that cannot be satisfied (e.g. a missing
  dependency)
- WHEN the application is constructed
- THEN construction fails with a composition error and no process has
  started

#### Scenario: A constructed-not-started application is assertable in a test
- GIVEN a test that constructs an application without starting it
- WHEN the test resolves a registered service or adapter from it
- THEN resolution succeeds without any process having been started

#### Scenario: Starting requires prior construction
- GIVEN no constructed application exists yet
- WHEN a caller attempts to start
- THEN starting is only defined on an already-constructed application; there
  is no operation that starts an unconstructed registration set

### Requirement: Service Registration Follows The Existing Injectable Contract

Registering a service MUST validate and construct it through the same
service-construction contract the framework already uses in production
(`Injectable`) — no parallel construction path is introduced. A missing
dependency MUST produce an error naming both the missing dependency's type
and the service that required it, matching the attribution `try_build`
already provides today.

The composition root MUST offer exactly two named forms for registering a
service, distinguished by whether the service type carries a
macro-generated link to its resolution Tag:

1. **Macro-linked registration (primary form), `AppBuilder::service::<S>()`.**
   For a service type that carries a macro-generated trait link (produced by
   the companion `service-sdk` delta's optional struct-macro argument), the
   composition root provides a registration call that takes only that one
   service type as a generic parameter — no Tag parameter, and no
   caller-supplied coercion closure. It constructs the service through the
   existing `Injectable` contract and registers it resolvable under its
   linked Tag, with construction and error-attribution behavior identical to
   the two-generic form.
2. **Explicit-Tag registration (renamed, permanent), `AppBuilder::service_with_tag::<S, Tag>(closure)`.**
   The form that takes both the service type and its Tag as separate generic
   parameters, plus a caller-supplied coercion closure, remains available
   under this name for service types with no macro-generated trait link —
   chiefly hand-rolled `Injectable` structs the macro never touched. This
   form MUST NOT be deprecated, time-boxed, or scheduled for removal: it is
   the only route a hand-rolled `Injectable` struct has to registration,
   since such a struct can never carry a macro-generated link.

A service type with no macro-generated trait link MUST NOT be accepted by
the macro-linked registration call — this MUST be rejected at compile time
(an unsatisfied trait/type bound), never accepted and left to fail at
runtime or during `build()`/`try_build()`.

#### Scenario: A registered service with satisfied dependencies resolves
- GIVEN a service registered with all its declared dependencies also
  registered
- WHEN the application is constructed
- THEN the service is constructed successfully and resolvable

#### Scenario: A missing dependency names both the missing type and the requester
- GIVEN a service registered whose declared dependency is not itself
  registered
- WHEN the application is constructed
- THEN construction fails with an error identifying the missing dependency's
  type and the service that requested it

#### Scenario: A bare `#[service]` struct with no trait link is unaffected by this change
- GIVEN a struct annotated only `#[service]` (no macro trait-link argument),
  exactly as before this change
- WHEN it is registered through the existing registration path it already
  used before this change
- THEN it compiles and registers exactly as it did before this change — no
  new required argument, no behavior difference

#### Scenario: A macro-linked service registers with a single type parameter and no closure
- GIVEN a service struct carrying a macro-generated trait link
- WHEN it is registered using the macro-linked registration call, naming
  only that one service type
- THEN registration succeeds with no Tag parameter and no coercion closure
  supplied by the caller, and the service resolves identically to how the
  prior two-generic form would have resolved it

#### Scenario: A service type with no trait link fails to compile against the macro-linked call
- GIVEN a service struct with no macro-generated trait link (e.g. a
  hand-rolled `Injectable` struct, or a bare `#[service]` struct with no
  trait-link argument)
- WHEN that type is passed as the single generic parameter to the
  macro-linked registration call
- THEN the code fails to compile — the missing link surfaces as a compile
  error, never a runtime registration failure or a successfully-built
  application

#### Scenario: A hand-rolled Injectable struct still registers through the renamed explicit-Tag form
- GIVEN a hand-rolled `Injectable` struct with no macro annotation and
  therefore no trait link
- WHEN it is registered using the renamed explicit-Tag registration form,
  naming both the struct's type and its Tag with a coercion closure, exactly
  as the pre-rename two-generic form required
- THEN registration succeeds and the service resolves under that Tag,
  identically to how the pre-rename form resolved it

Registration-to-construction ordering and tag-binding mechanics follow the
same pipeline for both forms: `Injectable::validate`, then
`Injectable::build`, then (for the macro-linked form) the macro-generated
coercion, then registration under the resolved Tag — unchanged from the
pre-Stage-2B form.

### Requirement: Adapter Registration

Registering an adapter MUST make a concrete adapter instance resolvable by
services that depend on it, mirroring today's adapter registration path.
Registering a second adapter of the same type MUST produce one specific,
documented, testable outcome — either rejection or an explicit replace —
never an undocumented silent overwrite.

#### Scenario: A registered adapter is resolvable by a dependent service
- GIVEN an adapter instance registered for a given type
- WHEN a service depending on that type is constructed
- THEN it receives that adapter instance

#### Scenario: Duplicate adapter registration has one documented, testable outcome
- GIVEN an adapter already registered for a type
- WHEN a second adapter of the same type is registered
- THEN the outcome (rejection with an error, or an explicit replace) matches
  whatever design.md settles as the one documented rule — silent,
  undocumented overwrite is not an acceptable outcome

*Open → design.md*: whether duplicate registration is an error or an
explicit replace operation is not decided by this spec.

*Non-goal for this requirement set*: trait-bound or multi-implementation
adapter bindings (registering more than one implementation behind a shared
trait and selecting among them) are not specified or designed here — only
single-concrete-instance-per-type registration is in scope for Stage 1.

### Requirement: Config, Security, Logging, And Observability Reuse Existing Abstractions

Config values, security providers, logging, and observability MUST
integrate through the framework's existing abstractions — pre-constructed
config values, pre-constructed authentication/authorization providers
(both-or-nothing), the existing config-to-logger pipeline, and the existing
observability hook. This capability MUST NOT introduce a second config
system or a second provider-construction path.

#### Scenario: A registered config value is resolvable
- GIVEN a pre-constructed config value registered with the application
- WHEN a service depending on that config type is constructed
- THEN it receives that value

#### Scenario: Security providers are both-or-nothing
- GIVEN only an authentication provider is supplied without an authorization
  provider
- WHEN the application is composed
- THEN this is rejected the same way the existing both-or-nothing pairing
  already rejects it

### Requirement: Start And Shutdown Administer The Runtime Lifecycle

Starting an application MUST begin its background processes and take over
their shutdown ordering. Every process the application started MUST be
subject to one clear, documented shutdown policy when the application is
shut down. Shutdown MUST let in-flight work drain and MUST surface a
component's shutdown error to the caller rather than discarding it — the
first error among all shutdown participants MUST be the one surfaced, after
every participant has been given the chance to shut down (matching the
existing all-hooks-run-then-first-error-surfaces behavior).

#### Scenario: Started processes are stopped on shutdown
- GIVEN an application that has been started and has running background
  processes
- WHEN the application is shut down
- THEN every process it started is stopped as part of that shutdown

#### Scenario: Shutdown drains in-flight work before completing
- GIVEN a background process with work in flight when shutdown begins
- WHEN shutdown is requested
- THEN that in-flight work is allowed to finish before shutdown completes

#### Scenario: One failing shutdown participant does not hide others, and its error surfaces
- GIVEN two shutdown participants, one of which fails to shut down cleanly
- WHEN the application shuts down
- THEN both participants are given the chance to shut down, and the failing
  participant's error is surfaced to the caller rather than swallowed

*Per design.md*: this capability owns no transport startup, draining, or
signal handling — those remain the host's responsibility, sequenced around
starting and shutting down the application. Only starting background
processes and administering their shutdown ordering belong to this
capability.

### Requirement: Read-Model Ownership Is Preserved (Hard Constraint)

Integrating a spawned read-side lifecycle handle (e.g. stage 0's
`spawn_projection` result) into this capability's lifecycle management MUST
NOT change which component owns the queryable read model. The application
that constructed its own read model MUST remain its sole owner, exactly as
stage 0's read-side spec already requires — this capability only takes over
scheduling *when* the handle's stop is invoked as part of shutdown, never
what the read model is or who queries it.

#### Scenario: The application's read model is unaffected by lifecycle integration
- GIVEN an application that owns its own queryable read model and has spawned
  a read-side projection handle
- WHEN the application registers that handle's stop into its shutdown
  sequence
- THEN the application's read model reference is unchanged and remains
  queryable by the application directly, exactly as before this capability
  existed

### Requirement: Errors Are Distinguished By Phase And Identify Their Component

Errors MUST be distinguishable by the phase in which they occurred:
composition (registration/validation), initialization (construction),
execution (post-start), and shutdown. Every error MUST identify the specific
component that failed to construct, initialize, execute, or shut down — a
bare internal type identifier or an internal-string-only message, with no
human-readable component identification, is not an acceptable error shape.

#### Scenario: A composition-phase error is distinguishable from a shutdown-phase error
- GIVEN one failure during registration/validation and one failure during
  shutdown
- WHEN each is reported
- THEN each is identifiable as belonging to its own phase, not conflated with
  the other

#### Scenario: An error names its failing component
- GIVEN a component that fails during any phase
- WHEN the resulting error is inspected
- THEN it identifies that specific component by a human-readable name, not
  only an internal type identifier

### Requirement: An Application Is Testable Without Running

An application MUST be constructible and assertable without ever being
started. It MUST be usable together with the framework's existing test
fixture path. Adapters and providers MUST be explicitly substitutable in
tests using that same existing test-construction path — no second,
parallel test-construction path is introduced.

#### Scenario: A built-not-run application is used directly in a test
- GIVEN a test that constructs an application without starting it
- WHEN the test asserts on a resolved service or adapter
- THEN the assertion succeeds without the application ever having been
  started

#### Scenario: A test substitutes an adapter through the existing fixture path
- GIVEN a test that needs a substitute adapter instead of the production one
- WHEN it registers the substitute through the existing test-fixture
  construction path
- THEN the substitute is what gets resolved, using the same underlying
  construction path production composition uses — not a second, separate
  test-only construction mechanism

### Requirement: Projection Registration Facade

`AppBuilder` MUST provide a `.projection(...)` registration method that is a thin pass-through to the completed `RuntimeBuilder` projection-registration path (Stage 1 facade precedent: `.adapter()`/`.service()`) — no parallel registration or resolution mechanism is introduced. A projection registered through `AppBuilder::projection(...)` MUST be resolvable exactly as if it had been registered directly on `RuntimeBuilder`.

#### Scenario: A projection registered via AppBuilder resolves after build
- GIVEN a projection instance registered via `AppBuilder::projection(...)`
- WHEN the application is built
- THEN a service declaring a dependency on that projection type resolves it successfully

#### Scenario: Registration is equivalent whether performed via RuntimeBuilder or AppBuilder
- GIVEN two otherwise-identical applications, one composed by registering a projection directly on `RuntimeBuilder` and one by registering the same projection via `AppBuilder::projection(...)`
- WHEN each is built
- THEN both expose the same resolvable projection dependency, with no observable difference in outcome

#### Scenario: No internal runtime type is required to register a projection
- GIVEN a developer composing an application through `AppBuilder`
- WHEN they register a projection using only `.projection(...)`
- THEN they never construct or reach into `RuntimeBuilder` or any other internal runtime state to complete that registration

### Requirement: Duplicate Projection Registration Through AppBuilder Fails Closed

Registering a second projection of the same type through `AppBuilder::projection(...)` MUST fail the same way the underlying `RuntimeBuilder` projection registration already fails closed (see the `service-sdk` capability spec's "Duplicate Projection Registration Fails Closed" requirement) — surfaced through `AppBuilder::build()`'s existing composition-error reporting, never a silent replacement.

#### Scenario: Duplicate projection registration surfaces at build, not silently replaced
- GIVEN `AppBuilder::projection(...)` called twice for the same projection type
- WHEN `AppBuilder::build()` is called
- THEN construction fails with a composition error identifying the duplicate registration, and the first-registered projection instance is what would have resolved had construction succeeded

## Non-Goals

- `.entity::<E>()` / any per-aggregate entity registration — no stable
  entity contract exists to delegate to (deferred to CORE-006). Stage 2B
  further specifies: no entity registration coupling to the service
  trait-link mechanism introduced by this stage.
- Unifying `RuntimeBuilder`'s config, the kit-config subtree, and
  `EntityRuntimeBuilder::from_value` into one config object.
- Trait-bound or multi-implementation adapter binding/selection (see
  "Adapter Registration" above).
- Any change to `RuntimeBuilder`'s public contract, `Injectable`, DI
  resolution, or stage 0's `spawn_projection` behavior — all remain exactly
  as specified today.
- New macros, HTTP/gRPC declarative routing, hot reload, plugins, module
  discovery, or a read-side DSL.
- Any runtime or link-time service registry/discovery mechanism (e.g.
  `inventory`, `linkme`, or `ctor`-style discovery) to locate
  macro-linked services — the trait link is resolved entirely at compile
  time; DI resolution stays synchronous.
- Naming-convention–based inference of the implemented trait (e.g. stripping
  an `Impl` suffix) — the trait link is established through the explicit
  macro argument only, never inferred from a struct's name.
- A deprecation window or migration deadline for the renamed explicit-Tag
  registration form — it is a permanent, first-class path.
