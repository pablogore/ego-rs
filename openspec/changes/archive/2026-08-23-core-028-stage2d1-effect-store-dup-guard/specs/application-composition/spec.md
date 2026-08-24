# Delta for application-composition

## ADDED Requirements

### Requirement: Duplicate Effect Store Registration Through AppBuilder Fails Closed

Registering a second effect store through `AppBuilder::effect_store(...)` MUST fail the same way `.adapter()`/`.projection()`/`.entity()` already fail closed: latched as a composition error and surfaced only through `AppBuilder::build()`'s existing composition-error reporting, never a silent overwrite. If a composition error has already latched from any prior registration call (including but not limited to a prior duplicate effect store), a subsequent `.effect_store(...)` call MUST NOT further mutate the builder's effect-store registration state, and the pre-existing error remains the one surfaced at `build()`.

#### Scenario: Duplicate effect store registration surfaces at build, not silently replaced
- GIVEN `AppBuilder::effect_store(...)` called twice
- WHEN `AppBuilder::build()` is called
- THEN construction fails with a composition error identifying the duplicated effect store registration, and the first-registered effect store is what would have resolved had construction succeeded

#### Scenario: A pre-existing composition error is not overwritten by a later effect store call
- GIVEN a composition error already latched from an earlier registration failure
- WHEN `.effect_store(...)` is called afterward
- THEN the builder is returned unmodified and the original composition error, not a new one, is what surfaces at `build()`

#### Scenario: Effect store and effect retention store registrations stay independent
- GIVEN `.effect_store(store.clone())` followed by `.effect_retention_store(store)` using the same underlying instance
- WHEN `AppBuilder::build()` is called
- THEN construction succeeds — a call to `.effect_retention_store(...)` never counts as, or triggers, a duplicate-effect-store error

### Requirement: Duplicate Effect Retention Store Registration Through AppBuilder Fails Closed

Registering a second retention store through `AppBuilder::effect_retention_store(...)` MUST fail the same way `.adapter()`/`.projection()`/`.entity()` already fail closed: latched as a composition error and surfaced only through `AppBuilder::build()`'s existing composition-error reporting, never a silent overwrite. If a composition error has already latched from any prior registration call (including but not limited to a prior duplicate retention store), a subsequent `.effect_retention_store(...)` call MUST NOT further mutate the builder's retention-store registration state, and the pre-existing error remains the one surfaced at `build()`.

#### Scenario: Duplicate effect retention store registration surfaces at build, not silently replaced
- GIVEN `AppBuilder::effect_retention_store(...)` called twice
- WHEN `AppBuilder::build()` is called
- THEN construction fails with a composition error identifying the duplicated retention store registration, and the first-registered retention store is what would have resolved had construction succeeded

#### Scenario: A pre-existing composition error is not overwritten by a later effect retention store call
- GIVEN a composition error already latched from an earlier registration failure
- WHEN `.effect_retention_store(...)` is called afterward
- THEN the builder is returned unmodified and the original composition error, not a new one, is what surfaces at `build()`

*Non-goal for these two requirements*: no escape hatch (e.g. a `replace_effect_store`/`replace_effect_retention_store` method) is specified or introduced — fail-closed is unconditional. Duplicate detection keys on slot presence, not on the concrete type passed to `.effect_store(...)`; a second call with a different concrete type is still rejected as a duplicate. `RuntimeBuilder`-level registration behavior (the layer `AppBuilder` delegates to) is unmodified and remains outside this spec's scope, as does any projection-lifecycle or module/bundle composition work.
