# Design: CORE-028 Stage 2C — Entity Composition (`.entity::<E>()`)

> Third Stage 2 slice, after 2A (projection registration, archived
> `2026-07-18-core-028-stage2-projection-registration`) and 2B (service→tag
> macro, archived `2026-07-18-core-028-stage2b-service-tag-macro`). This
> document reuses the AD-1.. numbering namespace fresh and cites the sibling
> slices' ADs by name where they constrain this one. The architecture
> decisions below record the decisions made ahead of and during design; they
> are recorded here as the authoritative contract, not re-derived. Scope is
> one `RuntimeBuilder` method, one `AppBuilder` facade, one new DI type, and
> retiring the always-`Err` `DepKey::Entity` stub.

## Technical Approach

The entity *dependency vocabulary* already exists and is half-wired:
`DepKey::Entity(TypeId, &'static str)` is a live variant (`di/mod.rs:88`), and
`RuntimeInner::check_dependency` already routes it — but its arm is a
fail-safe stub that returns `false` unconditionally (`runtime_builder.rs:374`),
pinned by `check_dependency_entity_is_always_err_regardless_of_table_state`
(`runtime_builder.rs:1088`). No registration path, no resolved-instance table,
no resolvable handle type exists. A service declaring an entity dependency can
never build.

Stage 2C mirrors 2A's projection path *exactly*, because the shapes match:
`EntityRuntime<E::Event>` is a host-constructed, `TypeId`-keyed leaf value that
needs no DI-resolved inputs to build (reference-app precedent, `lib.rs:228-229`)
and has no framework-owned teardown. So the slice adds the *write* end
(`RuntimeBuilder::with_entity`), threads a fourth map into `DependencyTable`,
flips the `check_dependency` stub to a real presence check, adds the
resolvable handle (`EntityRuntimeRef<E>`) and its resolver, and exposes a thin
`AppBuilder::entity()` facade — the same registration → validation →
resolution wiring `.projection()` established.

`service-sdk` already depends on `persistent-entity` (non-dev,
`Cargo.toml:22`), so `EntityRuntimeRef<E>` lands in `di/mod.rs` beside
`ProjectionRef` with **no new crate dependency**.

## Architecture Decisions

### AD-1 — Entity registrations are keyed by the aggregate type `E`, never `E::Event`

**Decision**: the DI key is `DepKey::Entity(TypeId::of::<E>(), type_name::<E>())`
and the resolved-instance table is keyed by `TypeId::of::<E>()`, where `E` is
the `PersistentEntity` aggregate (e.g. `OrderEntity`), not its `E::Event`
(e.g. `OrderEvent`).

**Rationale**: the conceptual dependency a service asks for is "the runtime for
`OrderEntity`", not "the runtime for `OrderEvent`". Two distinct aggregates
could share one event type (or a common event-wrapper) and would collide
silently if identity were keyed on the event. Keying on `E` also makes a
missing-dependency error name the aggregate (`OrderEntity`) — which is what an
application author composing the app actually reasons about; naming the event
type would be less intuitive. Concretely the table lives on `DependencyTable`
as `entities: HashMap<TypeId, Arc<dyn Any + Send + Sync>>`, inserted under
`TypeId::of::<E>()`, storing the erased `Arc<EntityRuntime<E::Event>>`.

### AD-2 — `.entity::<E>()` registers a host-constructed `EntityRuntime`; it does not construct one

**Decision**: the host builds `Arc<EntityRuntime<E::Event>>` via the existing,
unchanged `EntityRuntimeBuilder` and hands it in. The framework constructs
nothing. This preserves the Stage 1 principle that `build()` composes wiring —
it does not start infrastructure or invent ownership (same posture as
`.projection()`/`.adapter()`).

**Exact signature and bound stacking** (verified against
`persistent-entity/src/runtime.rs:129-132` and `persistent_entity.rs:53-61`):

```rust
// RuntimeBuilder
pub fn with_entity<E>(
    self,
    runtime: Arc<EntityRuntime<E::Event>>,
) -> Result<Self, DuplicateEntity>
where
    E: PersistentEntity + 'static,
    E::Event: DomainEvent + Clone + serde::de::DeserializeOwned
        + serde::Serialize + Send + Sync + 'static;
```

The bound stack is settled, not guessed: `PersistentEntity::Event` already
guarantees `Serialize + Send + Sync + 'static` (`persistent_entity.rs:58`); the
`EntityRuntime<E::Event>` *impl block* (`runtime.rs:129-132`) additionally
requires `DomainEvent + Clone + serde::de::DeserializeOwned`. Those three extra
bounds on `E::Event` are exactly what must be added at every entry point so the
stored runtime is actually usable (its `entity_ref` method lives on that impl
block). `E: 'static` is required for `TypeId::of::<E>()`.
`EntityRuntime<E::Event>: Send + Sync + 'static` holds — reference-app already
shares `Arc<EntityRuntime<UserRegistered>>` as a struct field across `async`
await points (`application.rs:128`), which is only sound if the runtime is
`Send + Sync` — so erasing it to `Arc<dyn Any + Send + Sync>` is valid.

### AD-3 — `EntityRuntimeRef<E>` is the DI-facing composition-time dependency; `EntityRef<E>` remains the per-dispatch handle. Deliberately distinct types.

**Decision**: introduce a new resolvable type in `di/mod.rs`:

```rust
pub struct EntityRuntimeRef<E: PersistentEntity> {
    inner: Arc<EntityRuntime<E::Event>>,
}

impl<E: PersistentEntity> EntityRuntimeRef<E> {
    pub fn new(inner: Arc<EntityRuntime<E::Event>>) -> Self { Self { inner } }

    /// Opens a per-request handle to one entity instance — a thin passthrough
    /// to `EntityRuntime::entity_ref`, with `Event` pinned to `E::Event`.
    pub fn entity_ref<C, S>(
        &self,
        entity_type: &'static str,
        entity_id: impl Into<String>,
        handler: Arc<dyn PersistentEntity<Command = C, Event = E::Event, State = S>>,
    ) -> Result<impl EntityRef<Command = C>, EntityError>
    where
        C: Send + Sync + serde::Serialize + 'static,
        S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static;
}
```

`EntityRuntimeRef<E>` wraps `Arc<EntityRuntime<E::Event>>` internally but does
not leak `E::Event` as a *name the author must type*: a service declares
`EntityRuntimeRef<OrderEntity>` as its field, and at the call site `E::Event`
is inferred from the `handler` (`Arc::new(OrderEntity::new())`), so `OrderEvent`
is never spelled. This is the composition-time role: "a shared handle capable
of opening/dispatching entities of type `E`." `persistent-entity`'s existing
`EntityRef<E>` (`entity_ref.rs` / `entity_ref_tokio.rs`) stays the per-request
role: "a handle to one specific entity instance," and it *cannot* be the DI
dependency because obtaining it requires a runtime `entity_id` and a per-call
handler that do not exist at composition time.

**Method shape rationale**: the passthrough mirrors `EntityRuntime::entity_ref`
(`runtime.rs:229`) rather than pinning `C = E::Command` / `S = E::State`,
because `PersistentEntity::State` does *not* guarantee `Clone`
(`persistent_entity.rs:61`) while `entity_ref` requires `S: Clone` — a pinned
convenience method would over-constrain `E`. A one-argument
`self.orders.entity(order_id)` DX (auto-supplying `entity_type` + a
`Default`-constructed handler) is **not** shipped: it would require entity-type
constants and a `Default` bound that no current convention provides (YAGNI —
reopens if a real ergonomic need appears). The AD-3 target snippet's
`self.orders.entity(order_id)` therefore resolves today to the three-argument
`entity_ref(entity_type, id, handler)` passthrough — "the equivalent call the
existing `EntityRuntime` API already supports."

### AD-4 — Duplicate entity registration is fail-closed; no replace/override escape hatch

**Decision**: registering the same aggregate type `E` twice is rejected at
`build()`, never silently replaced. Same observable semantics as 2A's
`DuplicateProjection` / `CompositionError::Projection`. New dedicated error in
`di/mod.rs` beside `DuplicateProjection`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("entity runtime already registered for type `{type_name}`")]
pub struct DuplicateEntity { pub type_name: &'static str }
```

`with_entity::<E>` checks `entities.contains_key(&TypeId::of::<E>())`; if
present, returns `Err(DuplicateEntity { type_name })` leaving the first
registration untouched. No `replace_entity` ships — every fail-closed
registration in the SDK (`with_service`, `register_effect_executor`,
`register_data_provider`, `with_projection`) ships without a replace; a type
registered twice at composition is a bootstrap bug, not an override intent
(2A AD-2 precedent, verbatim posture).

### AD-5 — `AppBuilder::entity()` is a thin facade over `RuntimeBuilder::with_entity()`

**Decision**: no behavior beyond delegation + the `pending_error` composition
pattern `.projection()` already uses (`app/mod.rs:305-318`):

```rust
pub fn entity<E>(mut self, runtime: Arc<EntityRuntime<E::Event>>) -> Self
where
    E: PersistentEntity + 'static,
    E::Event: DomainEvent + Clone + serde::de::DeserializeOwned
        + serde::Serialize + Send + Sync + 'static,
{
    if self.pending_error.is_some() { return self; }
    match self.runtime_builder.clone().with_entity::<E>(runtime) {
        Ok(builder) => { self.runtime_builder = builder; self }
        Err(err)    => { self.pending_error = Some(CompositionError::Entity(err)); self }
    }
}
```

The fail-closed guard lives in `RuntimeBuilder` (AD-4), so the facade must not
add a second guard — it just propagates, exactly like `.projection()`. Preserves
Stage 1 AD-1's invariant G3 (one registration, no parallel bookkeeping — note
`.entity()` keeps **no** facade-side `TypeId` set, unlike `.adapter()`, whose
set exists only because `with_adapter` can't fail).

### AD-6 — Entity lifecycle stays owned by `persistent-entity`; this slice adds no lifecycle management

**Decision**: no changes to `EntityRuntimeBuilder`, `EntityRegistry`,
activation, passivation, or actor spawn/stop. No teardown-stack integration —
entity actors self-terminate via existing passivation timeout /
`TeardownGuard::drop`, exactly like `.projection()`/`.adapter()` today (neither
is torn down by `RuntimeInner` either). `EntityRuntime`/`EntityRegistry` expose
no shutdown method to wire, so there is nothing to register (exploration
finding 3, confirmed against `runtime.rs` — the runtime has no `stop`/`shutdown`
surface).

### AD-7 — Test coverage must prove BOTH registration and consumption, as two separate work items

The near-miss from Stage 2A (proving registration without proving consumption)
must not repeat. Design mandates two distinct proofs:

1. **SDK-level integration test (consumption)**: a real `impl Injectable`
   fixture (`NeedsEntity`, mirroring `NeedsProjection` at
   `builder.rs:1118-1143`) whose `dependencies()` returns
   `vec![DepKey::Entity(TypeId::of::<E>(), type_name::<E>())]` and whose
   `build()` calls `rt.resolve_entity::<E>()`. Assert `try_build` **succeeds**
   when the entity was registered, and **fails naming the missing aggregate
   type** (`E`) when it wasn't. Plus a `RuntimeBuilder`-level registration test
   (register → `resolve_entity` returns the handle; duplicate → `DuplicateEntity`,
   first retained), mirroring `builder.rs:914-947`.
2. **Reference-app proof (production wiring)**: register a real
   `EntityRuntime` (the existing `user_runtime`, `Arc<EntityRuntime<UserRegistered>>`)
   through `App::builder().entity::<UserEntity>(...)`, then resolve
   `EntityRuntimeRef<UserEntity>` through `App::resolve_entity`. **Explicitly
   WITHOUT** migrating `RegisterUserImpl` off its `.service_instance()`
   hand-threading — that migration is out of scope (AD-9 / proposal non-goal,
   blocked by `ReadSideSink`'s hand-wiring, not by entity resolution).

The `check_dependency` pinning test
`check_dependency_entity_is_always_err_regardless_of_table_state`
(`runtime_builder.rs:1088`) is **retired and replaced** by present→`Ok` /
missing→`Err`-named pair, mirroring the two projection tests at
`runtime_builder.rs:1070-1086`.

### AD-8 — `App::resolve_entity()` is deliberate public API (mirrors 2A AD-6)

**Decision**: `App` gains
`pub fn resolve_entity<E>(&self) -> Result<EntityRuntimeRef<E>, RuntimeError>`
(same bounds as AD-2), symmetric with the existing
`App::resolve_adapter`/`resolve_config`/`resolve_projection`
(`app/mod.rs:177-181`). Unlike 2A — where `resolve_projection` surfaced during
apply — this accessor is planned up front, because AD-7's reference-app proof
resolves through `App` and the external `reference-app` crate has no other
reach into entity resolution (`App.runtime` is private). It is a read-only,
`Result`-returning accessor with no side effects; it completes the resolution
symmetry rather than widening the registration surface.

### AD-9 — `#[service]` macro field-recognition for `EntityRuntimeRef` is deferred; DI path proven via `Injectable` + `App::resolve_entity`

**Decision**: this slice does **not** teach `service-sdk-macros` to map an
`EntityRuntimeRef<E>` field to `DepKey::Entity` / `rt.resolve_entity::<E>()`.
The AD-3 target snippet's `#[service(impl_of = OrderService)] struct { orders:
EntityRuntimeRef<OrderEntity> }` sugar therefore lands in a follow-on slice; in
2C the entity DI contract is proven via a hand-written `impl Injectable`
fixture (AD-7 item 1) and the `App::resolve_entity` accessor (AD-8) — the exact
proof split 2A used.

**Rationale**: the user-approved proposal's Affected Areas
(`proposal.md:91-98`) lists `di/mod.rs`, `app/mod.rs`, `builder.rs`,
`runtime_builder.rs`, `error.rs`, and `reference-app` — and deliberately **not**
`service-sdk-macros`. Adding the macro arm now would silently widen approved
scope. The macro mapping is a two-line mirror of the existing `ProjectionRef`
arms (`service-sdk-macros/src/lib.rs:679,711`) and is a clean, low-risk
follow-on; recording it here as an explicit non-goal (rather than an oversight)
keeps a reader from expecting the `#[service]` field form to compile in this
slice. If the orchestrator judges the `#[service]` sugar in-scope, it is an
additive amendment to the proposal, not a design change.

## Data Flow

    RuntimeBuilder::with_entity::<E>(Arc<EntityRuntime<E::Event>>)
        │  fail-closed on dup (AD-4) → entities: HashMap<TypeId::of::<E>(), Arc<dyn Any>>
        │  (AppBuilder::entity delegates here; clone-then-call, AD-5)
        │  build()
        ▼
    DependencyTable::with_registrations(adapters, configs, projections, entities)  ← entities was: absent
        │
        ▼
    try_build() ──▶ Injectable::validate → check_dependency(DepKey::Entity)
        │              present? Ok : DependencyNotFound{ type=E, service }   ← was: always Err (stub retired)
        ▼
    Injectable::build → RuntimeInner::resolve_entity::<E>()
        │   entities.get(TypeId::of::<E>()).downcast::<EntityRuntime<E::Event>>() → EntityRuntimeRef<E>
        ▼
    service holds EntityRuntimeRef<OrderEntity>; per request:
        ref.entity_ref("order", id, Arc::new(OrderEntity::new())) → impl EntityRef

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk/src/di/mod.rs` | Modify | New `EntityRuntimeRef<E>` + `DuplicateEntity` beside `ProjectionRef`/`DuplicateProjection` (AD-1/AD-3/AD-4); **correct stale `entity_sdk::EntityRef` comment** (lines 6-8 → point at `persistent_entity` and describe `EntityRuntimeRef<E>`) |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `entities` field + `with_entity::<E>` method; thread map into `build()` (AD-1/AD-2/AD-4) |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modify | `DependencyTable.entities` + fourth `with_registrations` param; `resolve_entity::<E>`; flip `check_dependency` `DepKey::Entity` arm to real presence check; retire+replace the always-`Err` pinning test (AD-1/AD-7) |
| `crates/service-sdk/src/app/mod.rs` | Modify | `.entity::<E>()` facade (AD-5); `App::resolve_entity::<E>()` accessor (AD-8) |
| `crates/service-sdk/src/app/error.rs` | Modify | `CompositionError::Entity(#[from] DuplicateEntity)` (AD-4) |
| `crates/service-sdk/src/lib.rs` | Modify | Re-export `EntityRuntimeRef`, `DuplicateEntity` (public surface) |
| `examples/reference-app/src/lib.rs` | Modify | Register `user_runtime` via `.entity::<UserEntity>(...)`; resolve `EntityRuntimeRef<UserEntity>` as production proof (AD-7 item 2) |
| `crates/service-sdk-macros/**` | Unchanged | Macro field-recognition for `EntityRuntimeRef` deferred (AD-9) |
| `crates/persistent-entity/**` | Unchanged | Lifecycle owned here; no changes (AD-6) |

## Interfaces / Contracts

```rust
// di/mod.rs
pub struct DuplicateEntity { pub type_name: &'static str }
pub struct EntityRuntimeRef<E: PersistentEntity> { /* inner: Arc<EntityRuntime<E::Event>> */ }
impl<E: PersistentEntity> EntityRuntimeRef<E> {
    pub fn new(inner: Arc<EntityRuntime<E::Event>>) -> Self;
    pub fn entity_ref<C, S>(&self, entity_type: &'static str, entity_id: impl Into<String>,
        handler: Arc<dyn PersistentEntity<Command = C, Event = E::Event, State = S>>)
        -> Result<impl EntityRef<Command = C>, EntityError>
    where C: Send + Sync + serde::Serialize + 'static,
          S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static;
}

// RuntimeBuilder
pub fn with_entity<E>(self, runtime: Arc<EntityRuntime<E::Event>>) -> Result<Self, DuplicateEntity>
where E: PersistentEntity + 'static,
      E::Event: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static;

// RuntimeInner
pub fn resolve_entity<E>(&self) -> Result<EntityRuntimeRef<E>, RuntimeError> where /* same bounds */;

// AppBuilder
pub fn entity<E>(self, runtime: Arc<EntityRuntime<E::Event>>) -> Self where /* same bounds */; // dup → CompositionError::Entity at build()

// App
pub fn resolve_entity<E>(&self) -> Result<EntityRuntimeRef<E>, RuntimeError> where /* same bounds */; // AD-8
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit (di) | `DuplicateEntity` carries type name; `EntityRuntimeRef::new`/deref-or-`entity_ref` shape | Mirror `duplicate_projection_carries_type_name` (`di/mod.rs:153`) |
| Unit (RuntimeBuilder) | Register → `resolve_entity` returns handle; dup → `DuplicateEntity`, first retained | Mirror `with_projection_*` (`builder.rs:914-947`) |
| Integration (Injectable) | `NeedsEntity` fixture: `try_build` succeeds when registered, fails naming aggregate `E`+service when missing (AD-7 item 1) | New fixture mirroring `NeedsProjection` (`builder.rs:1118-1143`) |
| Unit (check_dependency) | `DepKey::Entity` present → `Ok`; missing → `Err` named. **Replaces** always-`Err` pin | Retire `check_dependency_entity_is_always_err_...`; mirror projection pair (`runtime_builder.rs:1070-1086`) |
| Unit (AppBuilder) | `.entity()` resolvable equivalently; dup → `CompositionError::Entity` at build; `RuntimeBuilder`≡`AppBuilder` equivalence | Mirror `projection_*` (`app/mod.rs:750-795`) |
| E2E (reference-app) | Existing pipeline unchanged; `user_runtime` registered via `.entity()` + `resolve_entity::<UserEntity>` succeeds (AD-7 item 2) | Run `tests/pipeline.rs` green; assert resolution after `build()` |

Strict TDD: RED behavioral tests (method/type absent) first — including
rewriting the retired pinning test to assert the new present/missing contract —
then implement type → builder → resolver → facade.

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. This adds one in-memory
`HashMap` insertion guarded by a `TypeId` presence check plus a downcast on
resolve; nothing spawns, serves, or executes at composition.

## Migration / Rollout

Purely additive (proposal rollback plan). No data migration, no feature flag.
Removing `with_entity`/`entity`/`EntityRuntimeRef`/`DuplicateEntity`, dropping
the `entities` map, and restoring the always-`Err` `check_dependency` arm + its
pinning test returns prior behavior exactly; reference-app reverts to
hand-threading. `DependencyTable`'s new `entities` parameter defaults to the
same empty map the stub effectively assumed.

## Open Questions

None blocking. TypeId keying (aggregate vs event) — resolved by AD-1.
`EntityRuntimeRef` placement — resolved: `di/mod.rs`, `service-sdk` already
depends on `persistent-entity`.

## Future Direction (non-blocking, not this stage)

- **`#[service]` field sugar for `EntityRuntimeRef` (AD-9)**: a two-line mirror
  of the `ProjectionRef` macro arms, making the AD-3 `#[service]` snippet
  compile. Deferred to keep 2C aligned with the approved proposal's Affected
  Areas.
- **`RegisterUserImpl` → `Injectable` migration**: still blocked by its
  hand-wired `ReadSideSink`, not by entity resolution (proposal non-goal). Once
  the sink is DI-resolvable, `RegisterUserImpl` can drop `.service_instance()`
  and consume `EntityRuntimeRef<UserEntity>`/`EntityRuntimeRef<TenantOrganizationEntity>`
  directly.
- **Framework-owned entity lifecycle**: spawn/stop ownership in the composition
  API (like a future projection-spawn unification) is a separate stage
  requiring a lifecycle-ownership change; out of scope here (AD-6).
