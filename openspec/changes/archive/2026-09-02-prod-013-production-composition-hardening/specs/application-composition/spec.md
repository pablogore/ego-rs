# Delta for Application Composition

## ADDED Requirements

### Requirement: Profile Declaration At The Composition Root

`RuntimeBuilder` and `AppBuilder` MUST accept a `Profile` declaration
(`Profile::Dev` default, re-exported upward from `persistent-entity`,
`Profile::Production`). This is the same `Profile` type gating
`EntityRuntimeBuilder`, not a second, parallel concept.

#### Scenario: AppBuilder without a declared profile defaults to Dev
- GIVEN an `AppBuilder` composition that never declares a profile
- WHEN it is built
- THEN it behaves as `Profile::Dev`, identical to today's behavior

### Requirement: Effect Store Gate Under Production, Conditional On A Registered Executor, Surfaced Through CompositionError

Under `Profile::Production`, when at least one effect executor is
registered, the composition MUST reject `build()` when no effect store was
explicitly configured via `RuntimeBuilder::with_effect_store()` or
`AppBuilder::effect_store()`, naming the missing capability and the
configuration call for the surface in use. This rejection MUST reuse
PROD-012's validator/error template (`validate_idempotency()`,
`crates/service-sdk/src/runtime/builder.rs:735-771`) and MUST surface
through the existing `CompositionError::Validation(#[from] RuntimeError)`
path — no new error-reporting mechanism is introduced. Rejection MUST
happen at bootstrap: the effect store has a real silent fallback to
`InMemoryEffectStore` (`crates/service-sdk/src/runtime/builder.rs:811`),
the same bootstrap-time volatility the event and snapshot store gates
close — not a failure deferred to first use. When no effect executor is
registered, no effect store is constructed at all, so there is nothing
volatile to gate.

#### Scenario: Missing effect store under Production rejects at build when an executor is registered
- GIVEN `Profile::Production`, at least one registered effect executor, and
  no call to `.with_effect_store()` / `.effect_store()`
- WHEN `AppBuilder::build()` / `RuntimeBuilder::build()` runs
- THEN it rejects through `CompositionError::Validation`, naming the
  effect store and the exact configuration call — the same bootstrap-time
  fallback the event and snapshot store gates close, never surfacing later
  at first attempted use

#### Scenario: No effect executor registered means nothing to gate under Production
- GIVEN `Profile::Production` and no registered effect executor
- WHEN the composition builds
- THEN it succeeds regardless of whether an effect store was configured —
  no effect store is constructed, so nothing volatile is reachable

#### Scenario: Dev profile with no effect store keeps today's silent fallback, unchanged
- GIVEN `Profile::Dev` (the default) and no effect store configured
- WHEN the composition builds
- THEN it succeeds, silently falling back to `InMemoryEffectStore` exactly
  as today — not a failure deferred to first use, and not a rejection

### Requirement: Reference App Propagates Its Profile From EntityEventStores, Guarded By A Regression Check

`examples/reference-app`'s `build_runtime_with` (`lib.rs:567`) MUST declare
its profile on the `AppBuilder`/`RuntimeBuilder` chain it composes
(`App::builder()...`) by passing through the profile already carried on
the `EntityEventStores` value it was given — via that value's
`.profile()` accessor — rather than a hardcoded literal, because
`build_runtime_with` is the shared entry point called with both
`EntityEventStores::open()` (Production) and `EntityEventStores::in_memory()`
(Dev) stores; hardcoding `Profile::Production` inside it would break every
in-memory caller. `build_runtime_in_memory` (`lib.rs:311`) and
`build_runtime_observed_in_memory` (`lib.rs:522`) MUST continue to reach
`Profile::Dev` through `EntityEventStores::in_memory()`, with no separate
declaration. A check (an `xtask` lint or a test — the exact mechanism is a
`design.md` decision) MUST fail the build if the composition reached
through `EntityEventStores::open()` (`main.rs`) ever stops resulting in a
`Profile::Production` `AppBuilder`/`RuntimeBuilder` composition.

#### Scenario: build_runtime_with propagates Production when given durable stores
- GIVEN `build_runtime_with` called with `EntityEventStores::open(pool)`
- WHEN the resulting `AppBuilder`/`RuntimeBuilder` composition's profile is
  inspected
- THEN it is `Profile::Production`, and durable event/snapshot/effect store
  configuration satisfies that gate

#### Scenario: build_runtime_with propagates Dev when given in-memory stores
- GIVEN `build_runtime_with` called with `EntityEventStores::in_memory()`
- WHEN the resulting `AppBuilder`/`RuntimeBuilder` composition's profile is
  inspected
- THEN it is `Profile::Dev`, unchanged from today

#### Scenario: Removing the production wiring fails the regression check
- GIVEN the composition reached through `EntityEventStores::open()`
  (`main.rs`) no longer resulting in a `Profile::Production` composition
- WHEN the regression check runs
- THEN it fails, naming the missing declaration
