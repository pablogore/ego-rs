# Design: CORE-028 Stage 2 — Projection Registration

> Continues the Stage 1 composition-facade work (archived
> `2026-07-18-core-028-application-composition-api`). Stage 1's ADs are the
> baseline; this document reuses its numbering namespace fresh (AD-1..AD-5)
> and cites Stage 1 ADs by name where they constrain this slice. Scope is one
> `RuntimeBuilder` method plus one `AppBuilder` facade — deliberately small.

## Technical Approach

The projection *resolution* contract already exists and is exercised at
`try_build()`: `DepKey::Projection`, `ProjectionRef<P>`,
`RuntimeInner::resolve_projection`, and `check_dependency`'s
`DependencyTable.projections` lookup are all live. The only missing piece is a
public *registration* path — today `DependencyTable::with_registrations`
hardcodes `projections: HashMap::new()` (runtime_builder.rs:74) and only
in-crate test fixtures reach the map. Stage 2 adds `RuntimeBuilder::with_projection`
(the write end) and threads the collected map into `DependencyTable`, then
exposes `AppBuilder::projection()` as a thin pass-through — the exact Stage 1
facade pattern. No resolution, read-side engine, or `DepKey` change.

## Architecture Decisions

### AD-1 — `with_projection` mirrors the *fail-closed* builder registrations, not `with_adapter`

| Option | Tradeoff |
|---|---|
| **`with_projection<P>(self, Arc<P>) -> Result<Self, DuplicateProjection>` (chosen)** | Matches the two existing *fallible* `RuntimeBuilder` registrations exactly — `register_effect_executor` (`DuplicateEffectType`) and `register_data_provider` (`DuplicateProviderId`). Both, like projections, are `TypeId`/id-keyed leaf values that never touch `ServiceRegistry`, and both fail closed with a dedicated single-purpose duplicate error. |
| Reuse `RegistryError::DuplicateService` | Semantic lie: `RegistryError` is `ServiceRegistry`-specific and keyed by `(name, version)` strings (registry.rs:20-24); a projection is neither a service nor registry-held. |
| Last-write-wins like `with_adapter`/`with_config` | Spec forbids it (duplicate MUST fail closed). Confirmed: `with_adapter`/`with_config` silently replace (builder.rs:162, :170); only `with_service` fails closed. |

**Decision**: `pub fn with_projection<P: Send + Sync + 'static>(mut self, projection: Arc<P>) -> Result<Self, DuplicateProjection>`. It checks `self.projections.contains_key(&TypeId::of::<P>())`; if present, returns `Err(DuplicateProjection { type_name })` leaving the first instance untouched; else inserts and returns `Ok(self)`. `DuplicateProjection { type_name: &'static str }` is a new dedicated error (mirroring `DuplicateEffectType`/`DuplicateProviderId`'s shape), placed beside the projection primitives in `di/mod.rs` and re-exported from `lib.rs`.

**Rationale**: the spec mirrors `with_service`'s *contract* (fail-closed, named); the *structure* that fits best is the effect-executor/data-provider precedent, because projections share their storage model (TypeId-keyed `HashMap` leaf, not `ServiceRegistry`). This satisfies the spec's "mirror `DuplicateService`, not `with_adapter`" instruction while staying consistent with the two nearest siblings.

### AD-2 — No `replace_projection` escape hatch; strictly fail-closed

**Decision**: this slice ships **no** override method. Duplicate registration is terminal.

**Rationale (evidence)**: the only replace sibling in the entire SDK is
`.replace_adapter()` (app/mod.rs:264), and it exists *solely* because
`with_adapter` is last-write-wins — the facade dup-guards the silent replace
while preserving the underlying override the infra API already had. Every
*fail-closed* registration ships without a replace: `with_service` has no
`replace_service`, `register_effect_executor` has no replace, `register_data_provider`
has no replace (grep-confirmed: zero `replace_service`/`replace_config` in the
tree). Projections are specified fail-closed, so they join that group.
Shipping the codebase's first fail-closed-with-override would invent public
surface with no precedent and no demonstrated need (YAGNI), and would
contradict Stage 1 AD-3's L2 discipline against speculative method-surface
growth. Composition is single-pass startup wiring; a type registered twice is
a bootstrap bug, not an override intent. If a real rebind need ever appears,
it reopens this decision rather than pre-building for it.

### AD-3 — `AppBuilder::projection()` follows the effect-executor facade pattern, not the adapter one

**Decision**: `pub fn projection<P: Send + Sync + 'static>(mut self, projection: Arc<P>) -> Self`, using the clone-then-call + `pending_error` shape `.effect_executor()`/`.data_provider()` already use (app/mod.rs:334-357): clone `runtime_builder`, call `with_projection`, on `Ok` swap it in, on `Err` record `CompositionError::Projection(err)` surfaced at `build()`.

**Rationale**: `.adapter()`'s facade keeps its *own* `TypeId` set (app/mod.rs:250) because the underlying `with_adapter` can't fail — the guard has to live in the facade. Here the fail-closed already lives in `RuntimeBuilder` (AD-1), so the facade must **not** duplicate a second guard; it just propagates, exactly like the other two fallible delegations. This preserves Stage 1 AD-1's invariant G3 (one registration, no parallel bookkeeping).

### AD-4 — `CompositionError::Projection(#[from] DuplicateProjection)`

**Decision**: add one variant wrapping the RuntimeBuilder-level error via `#[from]`, mirroring `EffectExecutor(#[from] DuplicateEffectType)` / `DataProvider(#[from] DuplicateProviderId)` (error.rs:37-42). Preserves Stage 1 AD-8's L1 invariant: exactly one wrapping layer, error is the single source of truth.

### AD-5 — Reference-app registers the query handle; no fabricated `Injectable` consumer

**Decision**: reference-app registers its `UsersByTenantStore` query handle through `.projection()` in `build_runtime` (lib.rs:234-255). It does **not** add a service that declares the dependency via `Injectable`; end-to-end resolution is proved by SDK-level tests with purpose-built fixtures instead.

**Rationale**: the one reference-app service, `RegisterUserImpl`, is registered via `.service_instance()` *because it cannot cheaply be `Injectable`* (Stage 1 AD-3 FLAG, confirmed lib.rs:241-250: its `EntityRuntime`s and hand-wired sink aren't DI-resolvable). It is the *write* service and has no reason to hold a *read* query handle. Injecting `UsersByTenantStore` into it would misrepresent the architecture just to trigger resolution. Registration through the public facade proves the path is real and reachable in production wiring; resolution is proved where it belongs — in-crate tests mirroring today's `NeedsAdapter`/`NeedsConfig` fixtures (builder.rs:982-1017). This is the same proof split Stage 1 used. The registered value is a clone of `ReadSideHandles.query` that shares the engine-fed store, so the read-side engine keeps producing into it unchanged (out of scope, untouched).

### AD-6 — `App::resolve_projection()` is deliberate public API, added during implementation

**Decision**: `App` gains `pub fn resolve_projection<P: Send + Sync + 'static>(&self) -> Result<ProjectionRef<P>, RuntimeError>`, symmetric with the existing `App::resolve_adapter()`/`App::resolve_config()`. This was not in this document's original File Changes/Interfaces (below, now updated) — it surfaced during apply because the external `reference-app` test crate has no other way to reach projection resolution (its `RuntimeResolver` only exposes `resolve`/`logger`, and `App`'s `runtime` field is private). Retroactively reviewed post-implementation (PR #190 review) and kept rather than removed.

**Rationale**: the two options were (A) keep it and document it as deliberate public surface, or (B) strip it and prove reachability only through the pre-existing `RuntimeResolver`/internal path. (A) was chosen: `App::resolve_adapter()`/`App::resolve_config()` already establish the precedent that `App` exposes read-only resolution accessors for registered dependencies, so `resolve_projection()` completes that symmetry rather than introducing a new kind of surface. It is a 2-line, read-only, `Result`-returning accessor with no side effects — it does not widen the write/registration surface this slice deliberately keeps narrow (AD-1/AD-2).

## Data Flow

    RuntimeBuilder::with_projection::<P>(Arc<P>)  ──fail-closed on dup──▶ projections: HashMap<TypeId, Arc<dyn Any>>
        │  (AppBuilder::projection delegates here; clone-then-call, AD-3)
        │  build()
        ▼
    DependencyTable::with_registrations(adapters, configs, projections)   ← was: projections always empty
        │
        ▼
    try_build()  ──▶ Injectable::validate → check_dependency(DepKey::Projection) ──▶ present? Ok : DependencyNotFound{type, service}
        │
        ▼
    Injectable::build → RuntimeInner::resolve_projection::<P>() ──▶ ProjectionRef<P>   (unchanged path)

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk/src/di/mod.rs` | Modify | New `DuplicateProjection { type_name }` error beside `ProjectionRef` (AD-1) |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `projections` field + `with_projection` method; thread map into `build()` (AD-1) |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modify | `DependencyTable::with_registrations` accepts `projections` instead of hardcoding empty |
| `crates/service-sdk/src/app/mod.rs` | Modify | `.projection()` facade, effect-executor pattern (AD-3); `App::resolve_projection()` accessor (AD-6) |
| `crates/service-sdk/src/app/error.rs` | Modify | `CompositionError::Projection(#[from] DuplicateProjection)` (AD-4) |
| `crates/service-sdk/src/lib.rs` | Modify | Re-export `DuplicateProjection` (public error surface) |
| `examples/reference-app/src/lib.rs` | Modify | Register `UsersByTenantStore` via `.projection()` (AD-5) |
| `examples/reference-app/src/read_side/**` | Unchanged | Engine path untouched (non-goal) |

## Interfaces / Contracts

```rust
// di/mod.rs
pub struct DuplicateProjection { pub type_name: &'static str }

// RuntimeBuilder
pub fn with_projection<P: Send + Sync + 'static>(
    self, projection: Arc<P>,
) -> Result<Self, DuplicateProjection>;

// AppBuilder
pub fn projection<P: Send + Sync + 'static>(self, projection: Arc<P>) -> Self; // dup → CompositionError::Projection at build()

// App (AD-6)
pub fn resolve_projection<P: Send + Sync + 'static>(&self) -> Result<ProjectionRef<P>, RuntimeError>;
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit (RuntimeBuilder) | Register→resolvable; dup→`DuplicateProjection`, first retained; unregistered→`DependencyNotFound` naming type | Direct `with_projection` + `resolve_projection`, mirror builder.rs:843-923 |
| Integration (Injectable) | Fixture declaring `DepKey::Projection`: `try_build` succeeds when registered, fails naming type+service when missing | New `NeedsProjection` fixture mirroring `NeedsAdapter` (builder.rs:982-1017) |
| Unit (AppBuilder) | `.projection()` resolvable equivalently; dup→`CompositionError::Projection` at build; RuntimeBuilder≡AppBuilder equivalence | Direct facade calls, assert variant |
| E2E (reference-app) | Existing pipeline tests unchanged; query handle registered + resolvable via composition | Run `tests/pipeline.rs` green; assert `resolve_projection::<UsersByTenantStore>` after build |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. This adds an in-memory
`HashMap` insertion guarded by a `TypeId` presence check; nothing spawns,
serves, or executes.

## Migration / Rollout

Purely additive (proposal rollback plan). No data migration, no feature flag.
Removing `with_projection`/`projection` and reverting the reference-app
registration call restores prior behavior exactly; `DependencyTable`'s new
`projections` parameter defaults to the same empty map it hardcodes today.

## Open Questions

None. Placement of `DuplicateProjection` — resolved: `di/mod.rs`, beside `ProjectionRef` (see AD-1, and Affected Files above).

## Future Direction (non-blocking, not this stage)

**Unification (possible Stage 3).** The read-side engine (`ReadSideHandles` +
`TagSchedulerImpl::spawn_projection`) still owns event delivery while DI owns
handle access — the seam the proposal deliberately preserves. If real usage
shows the two mechanisms should merge (framework-owned projection
spawn/stop + registration), that is a separate stage requiring a
lifecycle-ownership change; it stays out of scope here.

**(O1 continuation) Vocabulary review.** Stage 1's O1 flagged that
`AppBuilder`'s surface should not grow unbounded. `.projection()` is the next
method; it is justified (completes an existing contract) and stays a thin
wrapper. Before any *further* method (e.g. a future `.entity::<E>()`,
AD-5-blocked), revisit whether the vocabulary can be consolidated.
