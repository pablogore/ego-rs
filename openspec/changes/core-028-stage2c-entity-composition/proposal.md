# Proposal: CORE-028 Stage 2C — Entity Composition (`.entity::<E>()`)

> Third Stage 2 slice, after 2A (projection registration) and 2B (service→tag
> macro), both shipped and archived. Every prior CORE-028 document marked
> `.entity::<E>()` "blocked by CORE-006" — that blocker is **stale**: CORE-006
> (persistent-entity runtime) shipped and archived at
> `openspec/changes/archive/2026-06-22-persistent-entity-runtime/`, and
> CORE-006A (activation authority, issue #127, PRs #128/#131) is likewise
> archived. The entity contract this slice delegates to exists on `develop`
> today. This proposal records that correction explicitly so no reader needs
> git-log archaeology.

## Intent

An application author cannot obtain an entity runtime through the composition
API. `DepKey::Entity` exists in the DI vocabulary, but dependency validation
unconditionally rejects it (`RuntimeInner::check_dependency` is a fail-safe
stub, pinned by `check_dependency_entity_is_always_err_regardless_of_table_state`)
— a service declaring an entity dependency can never build, with no recourse.
Meanwhile reference-app proves the need is real, not speculative: it builds
`Arc<EntityRuntime<E>>` by hand and threads it into `RegisterUserImpl` through
the `.service_instance()` escape hatch. Stage 2C closes the gap: register a
host-constructed entity runtime once at composition, resolve it wherever a
service declares it.

## Approach

Mirror `.projection()` (2A) exactly — the closest shipped analog, validated
by exploration:

- `.entity::<E: PersistentEntity>(...)` accepts a host-constructed
  `Arc<EntityRuntime<E::Event>>`. The framework constructs nothing —
  `EntityRuntime` needs no DI-resolved inputs (reference-app precedent),
  preserving Stage 1's "build starts nothing" contract.
- Fail-closed duplicate guard, same observable shape as
  `DuplicateProjection`/`CompositionError::Projection`.
- Services declare a new DI type `EntityRuntimeRef<E>` as their `Injectable`
  dependency and obtain per-request entity handles from it — exactly what
  `RegisterUserImpl` does today with hand-threaded fields. Named
  `EntityRuntimeRef`, **not** `EntityRef`: `persistent-entity` already owns
  `EntityRef` for the per-dispatch handle, and that handle needs a runtime
  entity id, so it cannot be the composition-time dependency.
- No teardown integration: entity actors self-terminate (passivation /
  `TeardownGuard`), same as `.projection()`/`.adapter()`.

### Flagged open question (AD candidate for design — not resolved here)

Which `TypeId` keys the dependency table: the aggregate `E` or `E::Event`?
Recommendation leans toward the aggregate `E` — two distinct aggregates could
share one event type and collide silently if keyed on the event. Design must
settle this with rationale.

## Scope

### In Scope
- `.entity::<E>()` on `AppBuilder` with matching `RuntimeBuilder`
  registration, projection-shaped (registration → validation → resolution).
- `DepKey::Entity` validation becomes satisfiable: a service declaring an
  entity dependency builds when the runtime was registered, and fails before
  startup — naming the missing type — when it wasn't.
- Duplicate registration rejected at build, never a silent replacement.
- `EntityRuntimeRef<E>` resolvable type; stale `entity_sdk`/`EntityRef`
  comment in `di/mod.rs` corrected.
- Spec update: `application-composition` Non-Goals still defers
  `.entity::<E>()` to CORE-006 — the delta spec MUST retire that non-goal
  (flagged here so spec/design don't silently ignore it; the delta itself is
  sdd-spec work).

### Out of Scope (non-goals)
- Migrating `RegisterUserImpl` off `.service_instance()` — still blocked by
  its hand-wired `ReadSideSink`, not by entity resolution.
- Framework-owned `EntityRuntime` construction, config folding, or any
  change to `EntityRuntimeBuilder`, `EntityRegistry`, activation, or
  passivation.
- Entity lifecycle ownership (spawn/stop) in the composition API.
- Any change to 2A/2B surfaces (projection, service-tag) beyond docs.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `application-composition`: composition gains entity-runtime registration;
  the CORE-006-deferral non-goal is retired.
- `service-sdk`: entity dependencies become declarable, validated, and
  resolvable through the public builder.

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `crates/service-sdk/src/app/mod.rs` | Modified | `.entity::<E>()` facade |
| `crates/service-sdk/src/runtime/builder.rs` | Modified | Registration, `with_projection` pattern |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modified | Entity table; retire `check_dependency` always-Err stub |
| `crates/service-sdk/src/di/mod.rs` | Modified | `EntityRuntimeRef<E>`; stale comment fix |
| `crates/service-sdk/src/app/error.rs` | Modified | Duplicate-entity error variant |
| `examples/reference-app` | Modified | Register entity runtime via composition API as production proof |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| TypeId keying choice wrong (aggregate vs event) | Med | Flagged as explicit design AD with collision rationale; not silently resolved |
| `EntityRuntimeRef` vs `EntityRef` confusion | Low | Distinct name chosen deliberately; docs state the split (composition-time vs per-dispatch) |
| Stale spec non-goal survives into merged specs | Low | Retirement listed as in-scope deliverable; archive checklist catches it |

## Rollback Plan

Purely additive: remove the registration methods, `EntityRuntimeRef`, and the
entity table; restore the always-Err stub and its pinning test. Reference-app
reverts to hand-threading. No stored-data or runtime-behavior changes to
unwind.

## Dependencies

- None open. CORE-006/CORE-006A shipped (see header for archive evidence).

## Success Criteria

- [x] A service declaring an entity dependency builds and resolves an
      `EntityRuntimeRef<E>` when the runtime was registered; fails before
      startup, naming the type, when not.
- [x] Registration works equivalently via `RuntimeBuilder` and `App::builder()`.
- [x] Duplicate entity registration rejected at build, not silently replaced.
- [x] Reference-app registers its entity runtime through the composition API;
      existing behavior unchanged.
- [x] `application-composition` spec no longer lists `.entity::<E>()` as a
      CORE-006-deferred non-goal.
- [x] `cargo test --workspace` green.
