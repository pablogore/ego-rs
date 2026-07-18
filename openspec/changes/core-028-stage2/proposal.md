# Proposal: CORE-028 Stage 2 — Projection Registration

> Stage 2 is incremental. This change is the first Stage 2 slice toward the
> final developer API — it is not "Stage 2, done." Scoped to
> projection-registration only. `.entity::<E>()` remains blocked by CORE-006
> (same non-goal as Stage 1 AD-5); the service→Tag macro (reducing
> `.service::<S, Tag>(|arc| arc)` to `.service::<S>()`) is the next candidate
> slice, tracked separately. Neither is in this proposal.

## Intent

The SDK ships a projection *resolution* contract with no public way to
satisfy it. A service can declare a projection dependency
(`DepKey::Projection`, resolved as `ProjectionRef<P>` via
`Runtime::resolve_projection`), and build-time validation checks for it —
but neither `RuntimeBuilder` nor `AppBuilder` exposes any projection
registration. The only writes to the projection table today are internal
tests reaching into private fields. In production the contract is
unsatisfiable: any service declaring a projection dependency fails
validation with no recourse. Meanwhile reference-app wires its read model
through a separate path (`ReadSideHandles` + `spawn_projection`) that never
meets DI. Stage 2 closes the registration gap.

## Decision: complete the DI path, do not unify the two mechanisms

Two options were investigated against real code:

- **A (chosen)**: add public projection registration to the existing DI
  path (`RuntimeBuilder` + `AppBuilder` facade, Stage 1 precedent). The
  read-side engine path stays exactly as-is.
- **B (rejected for now)**: unify — have the read-side engine register
  through DI and the composition API own projection spawn/stop lifecycle.

Rationale: the two mechanisms answer different questions. The read-side
engine owns *event delivery* (tag streams, batching, dedup, offsets,
spawn/stop); DI answers *how a consumer obtains a read-model handle*.
Registering the queryable handle bridges them without merging them.
Unification would require a framework-owned abstraction over
`spawn_projection`'s full parameter surface plus a lifecycle-ownership
change — contradicting Stage 0's settled ownership split (reaffirmed as a
Stage 1 non-goal) and Stage 1's "build starts nothing" contract. Per Stage
1's own precedent: ship the thin slice, iterate. Unification stays a
possible Stage 3 once real usage shows the seam.

## Scope

### In Scope
- Public projection registration on `RuntimeBuilder`, mirroring the
  existing adapter/config registration shape.
- `AppBuilder::projection(...)` — thin pass-through, Stage 1 facade
  pattern (`.adapter()`/`.config()` precedent).
- Registered projections satisfy dependency validation and resolution: a
  service declaring a projection dependency builds when the projection was
  registered, and fails before startup — with the missing projection type
  named — when it wasn't.
- Duplicate registration of the same projection type is rejected at build,
  never a silent replacement (adapter precedent; escape-hatch question →
  design).
- Reference-app registers its existing read-model query handle
  (`UsersByTenantStore`) through the new path as production proof — the
  read-side engine keeps producing into it unchanged.

### Out of Scope (non-goals)
- `.entity::<E>()` (blocked by CORE-006, unchanged from Stage 1 AD-5).
- Service→Tag macro work (separate optional change).
- Any change to `ReadSideHandles`, `TagSchedulerImpl`, `spawn_projection`,
  or projection spawn/stop lifecycle ownership.
- Framework-owned read models, projection discovery, read-side DSL.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `service-sdk`: projection dependencies become registrable and resolvable
  through the public builder; validation of a declared projection
  dependency becomes satisfiable.
- `application-composition`: composition facade gains projection
  registration (delta on the Stage 1 spec, archived at
  `openspec/specs/application-composition/spec.md` via PR #189).

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `crates/service-sdk/src/runtime/builder.rs` | Modified | Projection registration alongside adapter/config |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modified | Projection table populated from registrations instead of always empty |
| `crates/service-sdk/src/app/mod.rs` | Modified | `.projection()` facade method |
| `examples/reference-app` | Modified | Query handle registered via composition API |
| `examples/reference-app/src/read_side/` | Unchanged | Engine path untouched |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Two projection mechanisms read as duplication | Med | Document the seam: engine = delivery, DI = handle access; unification deferred deliberately |
| Scope creep toward lifecycle unification | Med | Explicit non-goals; ReadSideHandles untouched |
| Duplicate-handling diverges from adapter semantics | Low | Fail-closed contract stated here; details in design |

## Rollback Plan

Purely additive: remove the registration methods and revert reference-app's
registration call. Resolution contract, read-side path, and all existing
behavior are unchanged throughout.

## Dependencies

- None.

## Success Criteria

- [x] A service declaring a projection dependency builds and resolves it
      when registered; fails before startup, naming the type, when not.
- [x] Registration works equivalently via `RuntimeBuilder` and `App::builder()`.
- [x] Duplicate projection type rejected at build, not silently replaced.
- [x] Reference-app read-side behavior (pipeline tests) unchanged; query
      handle obtainable through the composition API.
