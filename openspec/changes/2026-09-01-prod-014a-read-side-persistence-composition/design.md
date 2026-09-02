# Design: PROD-014A — Read-Side Durable Progress Composition

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers).
>
> **Inputs**: `proposal.md` (D-1 … D-8, IS-1 … IS-10, OOS-1 … OOS-9, R-1 … R-8, SC-1 … SC-13)
> and `explore.md` (the `## REVISION` section, which supersedes the original recommendation).
> This document decides **how**, never **what** — except where reading the real code
> falsified a premise the *what* rested on. Two such cases exist (§Evidence Corrections)
> and both are surfaced rather than silently implemented around.
>
> **Baseline read**: `develop` @ `509f65f` (branch
> `opsx/prod-015-real-postgresql-integration-verification`). Every file:line below was
> read on this baseline, not recalled.

## Technical Approach

Read-side durable progress becomes the fourth capability governed by PROD-013's existing
machinery, with no second mechanism invented. The two SPIs declare durability the same
way `EventStore` and `Snapshot` already do (`is_durable() -> bool { false }`). The
composition root accepts a **host-constructed pair** — `OffsetStore` *and* `DedupStore`
together, in one call, keyed by `projection_id` — and stores it type-erased. The one
existing validator, `RuntimeBuilder::validate_persistence_profile()`, gains a read-side
branch that calls the unmodified `require_durably_configured(...)` with
`durably_configured` computed from **both** stores' `is_durable()`.

Two properties fall out of the API shape rather than out of a runtime check:

- **A partial pair is not representable.** One method takes both stores, so there is no
  intermediate state in which one is registered and the other is not, and therefore no
  state a validator could mistake for complete (invariants 1 and 2).
- **The registered pair is the pair that runs.** The host holds one value and hands it
  to two destinations — the registration and `ProjectionSpec` — so a durable
  registration over a volatile projection is not expressible (IS-7 / R-2).

The framework never constructs a store (D-5 / OOS-6 intact). It accepts, classifies, and
refuses.

---

## Evidence Corrections

Both were found by reading the code the proposal points at. Each changes what the change
must do.

### EC-1 — The read-side branch cannot be appended to `validate_persistence_profile()`; the existing early return would skip it

`crates/service-sdk/src/runtime/builder.rs:777-796` opens with an unconditional early
return:

```rust
fn validate_persistence_profile(&self) -> Result<(), RuntimeError> {
    if self.effect_executors.is_empty() {
        return Ok(());                    // <- line 785
    }
    persistent_entity::profile::require_durably_configured(
        self.profile,
        self.effect_state_store.as_ref().is_some_and(|s| s.capabilities().durable),
        "effect store",
        "RuntimeBuilder::with_effect_store(store) (or AppBuilder::effect_store(store))",
    )?;
    Ok(())
}
```

A read-side branch added after that `?` is unreachable for **every composition that
registers no effect executor** — which includes any read-side-only or command-only
service, i.e. a large share of the hosts this gate exists for. The proposal's Affected
Areas row ("read-side branch added to the one existing validator") is correct in intent
and unimplementable as a literal append. Resolved in **AD-6**: the method splits into two
private per-capability helpers, and `validate_persistence_profile` becomes the two-line
sequencer. The effect-store helper keeps its body, its early return, and its message
byte-for-byte, so no existing refusal changes.

### EC-2 — An erased pair cannot reach `ProjectionSpec` today: no forwarding impl of `OffsetStore`/`DedupStore` for `Arc<T>` exists

`TagSchedulerImpl::spawn` (`crates/runtime/src/read_side/scheduler.rs:276-287`) requires
the stores **by value** with a `Clone` bound:

```rust
D: DedupStore + Send + Sync + Clone + 'static,
O: OffsetStore + Send + Sync + Clone + 'static,
```

A registration surface must be type-erased (`Arc<dyn OffsetStore + Send + Sync>`) because
`RuntimeBuilder` is a non-generic struct and `ReadSideHandles`/`BuiltRuntime` cannot
become generic without going viral through `main.rs` and every reference-app test. But a
workspace-wide grep for `impl … OffsetStore for Arc` / `… DedupStore for Arc` returns
**zero matches**: `Arc<dyn OffsetStore + Send + Sync>` does not implement `OffsetStore`,
so the value the composition root would hand back is exactly the value the projection
engine cannot accept.

Without the forwarding impls, IS-2's registration is decorative by construction — the
framework would hand you a pair its own scheduler refuses. **AD-3** adds them, in
`crates/domain`, next to the traits. This is additive machinery the proposal's Affected
Areas table does not list; it is named here rather than smuggled into implementation.

---

## Component Map

```
crates/domain                                    (the two SPIs)
├── src/read_side/offset.rs         MOD  + OffsetStore::is_durable() (default false)
│                                        + impl OffsetStore for Arc<T>   (EC-2 / AD-3)
└── src/read_side/dedup.rs          MOD  + DedupStore::is_durable()  (default false)
                                         + impl DedupStore for Arc<T>    (EC-2 / AD-3)
                                              ↑ read by
crates/persistent-entity                         (owns the rule, unchanged)
├── src/profile.rs                  MOD  doc comment only (IS-10 / AD-10);
│                                        require_durably_configured REUSED VERBATIM
└── src/error.rs                    —    PersistenceCompositionError reused verbatim
                                              ↑ called by
crates/service-sdk                               (the composition root)
├── src/runtime/builder.rs          MOD  + read_side_progress field (BTreeMap)
│                                        + with_read_side_progress(...)
│                                        validate_persistence_profile() split (AD-6)
├── src/app/mod.rs                  MOD  + read_side_progress_ids guard
│                                        + AppBuilder::read_side_progress(...)
└── src/app/error.rs                MOD  + CompositionError::DuplicateReadSideProgress
                                              ↑ used by
examples/reference-app
├── src/read_side/store.rs          MOD  + FakeDurableOffsetStore / FakeDurableDedupStore
├── src/read_side/mod.rs            MOD  + ReadSideProgressStores; ReadSideHandles takes
│                                        the pair instead of constructing it
├── src/lib.rs                      MOD  build_runtime_with gains one parameter; the
│                                        registered pair and the spawned pair are one value
└── src/main.rs                     MOD  passes `None` (no durable backend exists — F-1)

crates/runtime/src/read_side/scheduler.rs   UNTOUCHED  (OOS-3 / SC-9)
crates/domain/src/read_side/{session,runner}.rs  UNTOUCHED  (OOS-3)
crates/domain/src/read_side/projection_state_store.rs  UNTOUCHED  (OOS-1 / D-4)
crates/persistence/src/postgres/            UNTOUCHED  (OOS-2 / F-1)
```

## Data Flow

```
Host                                        Framework                              Outcome
────                                        ─────────                              ───────
let pair = ReadSideProgressStores           ┌─ registration ─────────────┐
    ::fake_durable();                       │                            │
                                            ▼                            │
App::builder()                       AppBuilder::read_side_progress      │
  .profile(Production)                 dup guard on projection_id ───────┼─▶ CompositionError::
  .read_side_progress(                 ↓ thin delegation                 │   DuplicateReadSideProgress
     PROJECTION_ID,                  RuntimeBuilder::with_read_side_     │   (latched in pending_error)
     pair.offset.clone(),              progress → BTreeMap insert        │
     pair.dedup.clone())                                                 │
  .build()  ──▶ RuntimeBuilder::try_build()                              │
                  └─ validate_persistence_profile()                      │
                       ├─ validate_effect_store_profile()   (unchanged)  │
                       └─ validate_read_side_progress_profile()          │
                            for each (projection_id, pair):              │
                              offset.is_durable() && dedup.is_durable()  │
                              → require_durably_configured(..) ──────────┼─▶ Ok  → App
                                                                         │   Err → PersistenceComposition
                                                                         │         Error::NotConfigured
                                                                         │       → RuntimeError::Persistence
                                                                         │         NotConfigured
                                                                         │       → CompositionError::Validation
ReadSideHandles::new(store, pair)  ◀────────┴─ same value, second destination ─┘
  .spawn() → ProjectionSpec::new(.., pair.dedup, pair.offset)
             (accepts Arc<dyn ..> only because of AD-3)
```

---

## Architecture Decisions

### AD-1 — One method takes the whole pair; two methods and a framework-owned struct are both rejected

**Decision**: a single registration call carrying `projection_id` plus both stores.

```rust
// crates/service-sdk/src/app/mod.rs
/// Registers the durable progress pair a projection resumes from — its
/// `OffsetStore` and `DedupStore` **together**, because neither alone is
/// progress state. Thin delegation to
/// [`RuntimeBuilder::with_read_side_progress`], mirroring [`Self::effect_store`].
///
/// The pair is one argument list, not two calls, so a half-registered
/// projection is not representable: there is no state in which an offset
/// store is registered and a dedup store is not, and therefore none that
/// `Profile::Production` could mistake for a complete configuration.
///
/// The framework never constructs either store (CORE-026's non-goal, intact):
/// it accepts what the host built, classifies it via `is_durable()`, and
/// refuses at [`Self::build`] under `Profile::Production` if either is
/// volatile. Registering nothing at all is valid and unchanged (IS-5).
///
/// Registering the same `projection_id` twice fails closed at [`Self::build`]
/// with `CompositionError::DuplicateReadSideProgress` — never last-write-wins.
pub fn read_side_progress(
    mut self,
    projection_id: impl Into<String>,
    offset_store: Arc<dyn OffsetStore + Send + Sync>,
    dedup_store: Arc<dyn DedupStore + Send + Sync>,
) -> Self
```

```rust
// crates/service-sdk/src/runtime/builder.rs
pub fn with_read_side_progress(
    mut self,
    projection_id: impl Into<String>,
    offset_store: Arc<dyn OffsetStore + Send + Sync>,
    dedup_store: Arc<dyn DedupStore + Send + Sync>,
) -> Self
```

**Criteria**:

1. **Invariant 1 + 2 are satisfied structurally, not by validation.** Any shape with two
   independent setters (`.offset_store(id, o)` / `.dedup_store(id, d)`) makes a partial
   registration a first-class value that has to be *detected* later. A validator that
   catches it is strictly weaker than a type that cannot express it, and it forces a
   third refusal reason ("pair incomplete") that this design does not need.
2. **It mirrors `.effect_store()` exactly.** `with_effect_store<T: EffectStateStore +
   EffectDedupStore>` (`builder.rs:523-530`) already sets *two* fields from one argument
   for this same reason, and its doc comment already says "a mixed durable/non-durable
   pair is not representable here" (`app/mod.rs:579-581`). AD-1 is that idea with two
   arguments instead of one bound, because `OffsetStore` and `DedupStore` are two
   independent SPIs no single type is required to implement.
3. **`impl Into<String>` matches `ProjectionSpec::new`'s own `projection_id` parameter**
   (`scheduler.rs:191-198`), so a host passes the same `PROJECTION_ID` constant to both
   without a conversion at either site.

**Rejected — a framework-owned `ReadSideProgressStores` struct as the parameter.** It
would be a two-field tuple with no behavior, adding a public framework type and one
`use` line at every call site to buy nothing the three-argument call does not already
buy. The host *does* want such a type (AD-8), but it wants it to carry its own
constructors, and a host-local type does that without widening the framework surface.

**Rejected — `AppBuilder`-only, with no `RuntimeBuilder` method.** `AppBuilder` delegates
every registration and never reimplements assembly (`app/mod.rs:115-117`, stated as G3).
The field must live where the validator reads it.

### AD-2 — The name is `read_side_progress`, not `read_side_store` or `read_side_persistence`

**Decision**: `read_side_progress` / `with_read_side_progress`.

**Criteria**: `ReadSideStore` is an **existing, different trait** — the event view the
projection polls, explicitly out of scope (OOS-8 / D-7). A method named
`read_side_store(...)` would read as registering that trait, in a workspace where it is
the one read-side type this change is required not to govern. `read_side_persistence` is
accurate but names a category rather than a subject, which is the exact ambiguity D-1
renamed the change to remove. `read_side_progress` names what is registered: the state a
projection resumes from.

### AD-3 — `crates/domain` gains forwarding impls of both SPIs for `Arc<T>`, and they MUST forward `is_durable()`

**Decision**: two blanket impls, next to the traits they forward.

```rust
// crates/domain/src/read_side/offset.rs
/// Forwards through a shared handle, so a composition root can hold the
/// pair as `Arc<dyn OffsetStore + Send + Sync>` and still hand that exact
/// value to `TagSchedulerImpl::spawn`, whose `O` parameter is taken by
/// value with a `Clone` bound. Without this, the registered pair and the
/// spawned pair could never be the same value (PROD-014A EC-2).
#[async_trait::async_trait]
impl<T: OffsetStore + Send + Sync + ?Sized> OffsetStore for std::sync::Arc<T> {
    async fn read_offset(&self, projection_id: &str, tag: &EventTag, tenant: &str)
        -> Result<Option<Offset>, OffsetStoreError> {
        (**self).read_offset(projection_id, tag, tenant).await
    }
    async fn write_offset(&self, projection_id: &str, tag: &EventTag, tenant: &str, offset: &Offset)
        -> Result<(), OffsetStoreError> {
        (**self).write_offset(projection_id, tag, tenant, offset).await
    }
    /// **Load-bearing.** Omitting this silently inherits the trait's `false`
    /// default, and every registered pair would be classified volatile no
    /// matter what the host wrapped — the gate would refuse a correct
    /// durable composition and pass nothing.
    fn is_durable(&self) -> bool {
        (**self).is_durable()
    }
}
```

The structurally identical impl goes on `DedupStore` (`crates/domain/src/read_side/dedup.rs`).

**Criteria**: (a) EC-2 — without them the registration cannot be non-decorative;
(b) they belong beside the trait, not in each host, because every host that registers
receives an `Arc<dyn …>` and would otherwise write the same newtype wrapper; (c) they are
purely additive: no existing implementation, signature, or call site changes, and no
existing type gains or loses a trait it previously had.

**Landmine for the task phase**: `#[async_trait]` generates `Self: 'async_trait` bounds,
so the `?Sized` + lifetime interaction is the part to verify first in RED. If `?Sized`
proves impractical, `T: OffsetStore + Send + Sync + 'static` (sized) still covers the
`Arc<dyn Trait>` case via the unsized coercion at the call site — check the narrower form
before widening.

### AD-4 — `is_durable()` defaults to `false` on both SPIs, mirroring `EventStore`/`Snapshot`

**Decision**:

```rust
// crates/domain/src/read_side/offset.rs — inside `pub trait OffsetStore`
/// Whether offsets written through this store survive a process restart.
///
/// Defaults to `false`: honest for every implementation in this workspace
/// today, none of which is durable, and for every third-party implementation
/// that has not considered the question. `Profile::Production` reads this
/// (PROD-014A); a durable implementation overrides it to `true`.
fn is_durable(&self) -> bool {
    false
}
```

Structurally identical on `DedupStore`.

**Criteria**: (a) PROD-013 AD-3 established this exact idiom for `EventStore`/`Snapshot`
and D-8 requires reuse, not a second mechanism; (b) defaulting to `false` keeps
`InMemoryOffsetStore`/`InMemoryDedupStore` compiling untouched and correctly classified
(SC-5, IS-6, OOS-9); (c) a plain `bool` is proportionate — neither trait declares any
other capability, so the effect store's four-axis `EffectStoreCapabilities` struct would
be one-field ceremony copied for no reason; (d) it is object-safe, which the erased
registration requires.

**Why not a downcast, marker type, or type-name match**: PROD-013 AD-3's reasoning applies
unchanged, and SC-8 forbids all three. A trait method is answered by the implementation,
so F-1's future `PostgreSQLOffsetStore` is recognized with no gate-side edit.

### AD-5 — Storage is a `BTreeMap<String, ReadSideProgressPair>` on `RuntimeBuilder`

**Decision**:

```rust
// crates/service-sdk/src/runtime/builder.rs
/// The durable progress pair each registered projection resumes from
/// (PROD-014A). Empty by default — a composition that registers none has no
/// read-side to govern, exactly as zero registered effect executors means no
/// effect store to refuse (IS-5).
///
/// `BTreeMap`, not `HashMap`: with two volatile projections registered the
/// refusal must be the same one on every run, and `HashMap` iteration order
/// is not stable across runs.
read_side_progress: BTreeMap<String, ReadSideProgressPair>,
```

```rust
/// One projection's progress pair. Private and constructible only through
/// `with_read_side_progress`, so the two fields cannot be populated
/// independently (AD-1's invariant, enforced at the storage layer too).
#[derive(Clone)]
struct ReadSideProgressPair {
    offset: Arc<dyn OffsetStore + Send + Sync>,
    dedup: Arc<dyn DedupStore + Send + Sync>,
}
```

**Criteria**: (a) keyed multiplicity is D-3, already decided and not re-litigated here;
(b) `RuntimeBuilder` is cloned during composition (`app/mod.rs:822`
`builder.clone().build()`, and `:376` `runtime_builder.clone().with_projection(...)`), so
every field must be `Clone` — `Arc<dyn …>` is, and a map of them is;
(c) `BTreeMap` costs nothing at N=1 (reference-app's actual count) and removes a
nondeterminism the design would otherwise have to explain away.

**Duplicate handling at this layer is last-write-wins**, deliberately: `with_effect_store`
behaves identically when called twice directly on `RuntimeBuilder`, and IS-3 places the
fail-closed guard at `AppBuilder` (AD-7). Restating it here would create the second,
parallel check the change exists to avoid.

### AD-6 — `validate_persistence_profile()` splits into two per-capability helpers (resolves EC-1)

**Decision**:

```rust
// crates/service-sdk/src/runtime/builder.rs
/// Whether this configuration can honour the production posture it declares
/// for every capability this layer owns (PROD-013 AD-5, PROD-014A AD-6).
/// Mirrors [`Self::validate_idempotency`]: one definition, checked from both
/// `build()` and `try_build()`, so the two paths cannot disagree.
///
/// A sequencer, not a body: the effect-store check returns early when no
/// executor is registered, and a read-side check appended after it would be
/// unreachable for every composition that registers no effect executor —
/// including every read-side-only service (PROD-014A EC-1).
fn validate_persistence_profile(&self) -> Result<(), RuntimeError> {
    self.validate_effect_store_profile()?;
    self.validate_read_side_progress_profile()?;
    Ok(())
}

/// Unchanged from PROD-013 AD-5 — body, early return, capability and fix
/// strings all byte-for-byte identical; only the enclosing function name is new.
fn validate_effect_store_profile(&self) -> Result<(), RuntimeError> { /* existing body */ }

/// Under `Profile::Production`, every **registered** projection's progress
/// pair must be durable. Not conditional on anything else: registration is
/// itself the composition-visible signal that this projection has a progress
/// pair worth governing, exactly as executor registration is for the effect
/// store. Zero registered means the loop body never runs, which is IS-5 —
/// falling out of the same reasoning, not a special case.
fn validate_read_side_progress_profile(&self) -> Result<(), RuntimeError> {
    for pair in self.read_side_progress.values() {
        persistent_entity::profile::require_durably_configured(
            self.profile,
            pair.offset.is_durable() && pair.dedup.is_durable(),
            "durable read-side progress store (OffsetStore + DedupStore)",
            "AppBuilder::read_side_progress(projection_id, offset_store, dedup_store) \
             (or RuntimeBuilder::with_read_side_progress(..)), passing stores whose \
             is_durable() returns true",
        )?;
    }
    Ok(())
}
```

**Criteria**:

1. **EC-1 makes the append unimplementable.** This is the smallest restructuring that
   makes the read-side branch structurally unskippable while leaving the effect-store
   semantics provably untouched.
2. **`validate_persistence_profile` remains the single entry point.** Both existing
   callers — `build()` at `builder.rs:822` (panics) and `try_build()` at `:1146`
   (returns) — are unchanged, so `build()`/`try_build()` still cannot disagree.
3. **Effect store is checked first**, deliberately: no composition that is refused today
   gets a different message tomorrow.
4. **`&&`, never `.is_some()`.** `require_durably_configured`'s own doc comment
   (`profile.rs:39-46`) forbids computing the argument from presence. Both stores must
   report durable; either one volatile refuses the pair, which is invariant 1 restated at
   the check.
5. **Complexity stays well under 10 per function** — the split is also what keeps it
   there as capabilities accumulate.

### AD-7 — Duplicate registration fails closed at `AppBuilder`, keyed by `projection_id`

**Decision**: an `AppBuilder`-local key set plus a new `CompositionError` variant, exactly
mirroring `adapter_types: HashSet<TypeId>` (the keyed precedent) and `.effect_store()`'s
latched `pending_error` (the fail-closed precedent).

```rust
// crates/service-sdk/src/app/mod.rs — new AppBuilder field
/// Keys already registered through `.read_side_progress()`. Keyed, not a
/// `bool`: registration is per-projection (D-3), so a second projection is
/// valid and only a second registration of the *same* `projection_id` is not.
read_side_progress_ids: HashSet<String>,
```

```rust
// inside read_side_progress(...)
if self.pending_error.is_some() { return self; }
let projection_id = projection_id.into();
if !self.read_side_progress_ids.insert(projection_id.clone()) {
    self.pending_error = Some(CompositionError::DuplicateReadSideProgress { projection_id });
    return self;
}
self.runtime_builder = self.runtime_builder
    .with_read_side_progress(projection_id, offset_store, dedup_store);
self
```

```rust
// crates/service-sdk/src/app/error.rs
/// A second progress pair was registered for the same `projection_id`
/// through `AppBuilder::read_side_progress(...)` (PROD-014A). Rejected even
/// when the second pair is durable and the first was not: silently replacing
/// a projection's resume state is not a composition a reader can verify.
/// Deliberately has no replace escape hatch — the message must not invent one.
#[error(
    "read-side progress stores already registered for projection `{projection_id}`; \
     second registration rejected — register exactly one progress pair per projection"
)]
DuplicateReadSideProgress {
    /// The `projection_id` whose second registration was rejected.
    projection_id: String,
},
```

**Criteria**: (a) `String`, not `&'static str`, because the key is a runtime value, unlike
`DuplicateEffectStore`'s `type_name`; (b) the latch means the first registration is what
would have resolved had construction succeeded (SC-6's second clause) — the guard runs
before the delegation, so `RuntimeBuilder`'s map never sees the rejected value;
(c) `CompositionError` already carries four `Duplicate*` variants with this exact shape
and message style, including the "must not suggest a non-existent replace API" property
its own tests pin (`app/error.rs:217-234`).

### AD-8 — IS-7: one host-local `ReadSideProgressStores` value reaches both the registration and `ProjectionSpec`

**Decision**: `ReadSideHandles` stops constructing stores and receives the pair.
`build_runtime_with` gains one parameter. The value passed there is the value registered
*and* the value spawned.

```rust
// examples/reference-app/src/read_side/mod.rs
/// The progress pair a projection resumes from — offset and dedup together,
/// because neither alone is resume state (PROD-014A IS-2).
///
/// This type exists for the reason `EntityEventStores` exists one layer over:
/// so the choice of progress storage is **stated** at the composition root,
/// never defaulted inside the thing that uses it. Before it,
/// `ReadSideHandles::new` constructed `InMemoryOffsetStore`/`InMemoryDedupStore`
/// with no parameter and no composition-visible decision at all.
#[derive(Clone)]
pub struct ReadSideProgressStores {
    pub offset: Arc<dyn OffsetStore + Send + Sync>,
    pub dedup: Arc<dyn DedupStore + Send + Sync>,
}

impl ReadSideProgressStores {
    /// Volatile. First-class and unchanged for Dev and tests (IS-6/OOS-9);
    /// refused by `Profile::Production` once registered, which is the point.
    pub fn in_memory() -> Self { /* InMemoryOffsetStore / InMemoryDedupStore */ }

    /// See `store::FakeDurableOffsetStore` (AD-9).
    pub fn fake_durable() -> Self { /* FakeDurable* pair */ }
}
```

```rust
// ReadSideHandles: the two fields become the erased pair, and `new` takes it.
pub fn new(store: SharedReadSideStore, progress: ReadSideProgressStores) -> Self
```

```rust
// examples/reference-app/src/lib.rs — build_runtime_with, new parameter
/// The progress pair this composition states at the composition root, or
/// `None` for "this host has not adopted durable read-side progress".
///
/// `None` uses `ReadSideProgressStores::in_memory()` and registers **nothing**,
/// so `Profile::Production` has nothing to refuse (IS-5). That is what
/// `main.rs` passes today, and it is honest rather than convenient: no durable
/// `OffsetStore`/`DedupStore` exists anywhere in this workspace (OOS-2/F-1), so
/// there is no pair a production deployment could register and satisfy.
///
/// `Some(pair)` registers that pair AND runs the projection on it — the same
/// value, cloned into two destinations, so a durable registration over a
/// volatile projection is not expressible.
read_side_progress: Option<ReadSideProgressStores>,
```

```rust
// in the body, replacing `ReadSideHandles::new(read_side_store)` at lib.rs:775
let progress = read_side_progress.unwrap_or_else(ReadSideProgressStores::in_memory);
let read_side_handles =
    ReadSideHandles::new(read_side_store, progress.clone()).with_logger(logger.clone());
// ... and, only when the host stated one, in the App::builder() chain:
//     .read_side_progress(PROJECTION_ID, progress.offset.clone(), progress.dedup.clone())
```

**Criteria**:

1. **It closes the divergence IS-7 names, structurally.** The Atomicity Gate states the
   defect precisely: "a host can register a durable pair and hand a volatile one to
   `ProjectionSpec`". One binding, cloned into both destinations, makes that
   unrepresentable in this host — the same shape `EntityEventStores` uses to keep the
   profile and the stores from drifting apart (PROD-013 AD-8).
2. **`ReadSideHandles::new` can never fabricate again.** Taking the pair as a required
   argument removes the default, rather than adding a second method the composition root
   must remember to call. "Must be remembered" is the weakness PROD-013 AD-8 removed
   structurally and R-1 accepts only where no mechanism exists.
3. **SC-10 is met literally.** `ReadSideHandles::new()` no longer constructs either
   in-memory store, and the reference host's pair originates at the composition root in
   both variants — `None` constructs it in `build_runtime_with`, not inside the handles.

**`Option`, not a `ReadSideProgressWiring` enum.** `IdempotencyWiring` and
`ExternalEffectsWiring` are enums in this same file because their variants carry
*different* field sets. Here both arms carry the same pair and differ only in whether it
is stated, which is what `Option` means. A bespoke two-variant enum would be a rename of
`Option` with a `use` line.

**`main.rs` passes `None`, and this is the design's one interpretive call.** The
alternative — registering the in-memory pair from `main.rs` — would make the reference
app's `Profile::Production` binary refuse to start, with no in-tree fix until F-1 ships a
durable backend (OOS-2). This does **not** soften the gate that R-3 warns against
softening: every registered volatile pair is refused unconditionally. It exercises IS-5,
which is a first-class in-scope requirement. What changes for `main.rs` is that its
volatile choice is now made and visible at the composition root instead of hidden inside
`ReadSideHandles::new()` — which is exactly the invisibility the Intent section requires
to disappear. Recorded for confirmation below.

**Blast radius, measured on the baseline**: `ReadSideHandles::new` has 5 call sites
(`lib.rs:775`, `tests/read_side_error_logging.rs:35`,
`tests/users_by_tenant_projection.rs:78, :112, :183`) — four are one-token test edits
adding `ReadSideProgressStores::in_memory()`. `build_runtime_with` has 8
(`lib.rs:616`, `main.rs:95`, `tests/stoolap_restart_persistence.rs:84, :132`,
`tests/idempotency_wiring.rs:84`, `tests/production_profile_guard.rs:27`,
`integration-tests/…/dual_aggregate_crash_recovery_postgres.rs:247`,
`…/durable_entity_progress_postgres.rs:118`) — each a one-line `None` addition. Thirteen
mechanical edits, no behavior change at any of them.

### AD-9 — The IS-8 fake durable pair is plain `pub`, named for what it is, with no cargo feature

**Decision**: two thin newtypes beside the in-memory pair in
`examples/reference-app/src/read_side/store.rs`, delegating storage and overriding only
the one bit.

```rust
/// A **fake** durable `OffsetStore`: stores exactly like
/// `InMemoryOffsetStore` and loses everything on restart, and declares
/// `is_durable() -> true` anyway.
///
/// It exists to exercise `Profile::Production`'s accept path, which no real
/// implementation can exercise today — this workspace ships no durable
/// `OffsetStore` or `DedupStore` at all (PROD-014A D-6/OOS-2; the durable
/// backend is F-1). Never wire this into a deployment: it makes a production
/// composition build and then lose every offset on the next restart, which is
/// the exact failure PROD-014A exists to refuse.
#[derive(Clone, Default)]
pub struct FakeDurableOffsetStore(InMemoryOffsetStore);

#[async_trait]
impl OffsetStore for FakeDurableOffsetStore {
    /* read_offset / write_offset delegate to self.0 */
    fn is_durable(&self) -> bool { true }
}
```

Structurally identical `FakeDurableDedupStore`.

**Criteria**: (a) `#[cfg(test)]` is not an option — `examples/reference-app/tests/*` are
separate binaries and cannot see a `#[cfg(test)]` item in the library; (b) a cargo feature
adds a `Cargo.toml` entry, a `--features` flag on every affected test invocation, and a
second compilation configuration, to guard a type inside an example crate that is not
published API; (c) the established convention in this host is honesty by naming and doc,
not by cfg — `EntityEventStores::in_memory()` is plain `pub` and carries its warning in
prose ("Spelled out at every call site rather than reachable by omission",
`lib.rs:422-427`). `FakeDurable` in the type name travels to every call site the way a
feature flag does not.

**Delegation, not reimplementation**: a hand-written second `HashMap` body would be a
parallel implementation that can drift from the one under test, which
`ego-rs-testing-tdd`'s same-contract principle exists to prevent. The newtype guarantees
identical storage semantics, so the only observable difference is the classification bit
— which is precisely what the test is about.

**`crates/service-sdk`'s own tests need their own doubles.** reference-app's types are not
reachable from `service-sdk`, and the layering forbids the dependency. The framework tests
(AD-6/AD-7's matrix) get two four-line `#[cfg(test)]` stubs in
`crates/service-sdk/src/runtime/builder.rs` — a durable one and a volatile one, both
minimal `OffsetStore`/`DedupStore` impls with unreachable bodies, since the gate reads
only `is_durable()`. That is the hand-rolled-stub rung of the double ladder, correct for
a two-method trait where `mockall` is overkill.

### AD-10 — `Profile::Production`'s doc comment (IS-10)

**Decision**: replace `crates/persistent-entity/src/profile.rs:17-23` verbatim.

```rust
    /// Today that means the event store, snapshot store, and effect store
    /// (PROD-013), plus the durable progress pair — `OffsetStore` and
    /// `DedupStore` together — of every projection registered through
    /// `AppBuilder::read_side_progress` (PROD-014A). A projection whose pair
    /// is never registered is not governed here, by design: a command-only or
    /// non-read-side application is never forced to register storage it does
    /// not use, and a projection spawned directly through
    /// `ProjectionSpec`/`TagSchedulerImpl` without passing the composition
    /// root is ungoverned by construction (PROD-014A OOS-7).
    /// See PROD-013/PROD-014A.
```

**Criteria**: the current text makes three claims that IS-2 falsifies ("has no such slot
yet", "deliberately not governed here", "PROD-014 introduces it"), and the exploration
found the third also names the wrong subject. The replacement states the new fact and,
more usefully, states the two *boundaries* a reader will otherwise assume away: no
registration means no gate, and the direct-spawn path is outside it.

`require_durably_configured`'s signature, body, and doc comment are **unmodified** (D-8,
SC-8). This file's only change is these seven doc lines.

### AD-11 — The refusal cannot name the offending `projection_id`; accepted, not worked around

**Decision**: the error names the capability and the fix, not the projection.

**Why**: `require_durably_configured(profile, durably_configured, capability: &'static str,
fix: &'static str)` is reused verbatim by explicit constraint (D-8, SC-8), and a
`projection_id` is a runtime `String`. Reaching a runtime value into a `&'static str`
requires either a signature change (forbidden here) or `Box::leak` (an unbounded leak on
a path a host can call in a loop — never for a diagnostic).

**Why it is acceptable**: SC-4 asks for the missing capability and the exact fixing call;
both are present, and the `fix` string names the parameter (`projection_id`) that
identifies the subject. `BTreeMap` ordering (AD-5) makes the reported failure the
lexicographically-first offender, deterministically, so a host with several volatile
projections fixes them in a stable order rather than chasing a shuffling message.
reference-app registers one projection, so the gap is invisible there today.

**Follow-up (not folded in)**: **F-5** — if per-projection attribution is ever wanted, it
is a `require_durably_configured` signature change affecting all four capabilities, which
is its own change with its own blast radius. It is not softened or pre-empted here.

---

## Integration Points

| Boundary | Direction | Mechanism | Verified at |
|---|---|---|---|
| `ego-domain` → `service-sdk` | up | `service-sdk` already depends on `ego-domain` | `crates/service-sdk/Cargo.toml:19` |
| `OffsetStore`/`DedupStore` → the gate | in | `is_durable()`, forwarded through `Arc<T>` | AD-3, AD-4 |
| `AppBuilder::read_side_progress` → `RuntimeBuilder` | down, thin | delegation, mirroring `effect_store` | `app/mod.rs:582-598` |
| `PersistenceCompositionError` → `RuntimeError` | up | `PersistenceNotConfigured(#[from] …)`, already present | `runtime/runtime_builder.rs:1510-1516` |
| `RuntimeError` → `CompositionError` | up | `Validation(#[from] RuntimeError)`, already present | `app/error.rs:58-59`; mapped at `app/mod.rs:827` |
| duplicate key → host | out | `CompositionError::DuplicateReadSideProgress` via `pending_error` | AD-7; latch pattern at `app/mod.rs:812-814` |
| registered pair → `ProjectionSpec` | out, host-side | same `Arc` clone, enabled by AD-3 | AD-8 |
| `Profile` → scheduler layer | **none** | no path added, none exists | OOS-3 / SC-9 |

Zero new plumbing: every crossing above either already exists or is one delegation.

## Testing Strategy

Per `ego-rs-testing-strategy`: the rule is tested where it lives, the crossing at the
crate boundary, the composition root end-to-end. Strict TDD — RED before GREEN, each error
path asserting the specific error rather than `is_err()`.

| Level | Location | What it proves |
|---|---|---|
| Unit | `crates/domain/src/read_side/{offset,dedup}.rs` `#[cfg(test)]` | `is_durable()` defaults to `false` on a bare impl; **`Arc<T>` forwards it** — the AD-3 landmine, asserted directly (`Arc::new(durable).is_durable() == true`), plus one round-trip proving `read_offset`/`write_offset` forward through the `Arc` |
| Unit | `crates/service-sdk/src/runtime/builder.rs` `#[cfg(test)]` | the full matrix: {Dev, Production} × {none registered, durable pair, volatile offset, volatile dedup, both volatile}. SC-1 (none registered under Production builds), SC-2 (durable pair builds), SC-3 (either store volatile refuses), SC-4 (the message names capability **and** fix); that `build()` panics on the same input `try_build()` refuses; and **EC-1's regression: a composition with zero effect executors and a volatile registered pair is still refused** — the test that would have passed against a naive append |
| Unit | `crates/service-sdk/src/app/error.rs` `#[cfg(test)]` | `DuplicateReadSideProgress` carries `projection_id`, and its message names the projection and suggests no replace API — mirroring `duplicate_effect_store_message_states_the_contract_without_a_replace_api` |
| Unit | `crates/service-sdk/src/app/mod.rs` `#[cfg(test)]` | SC-6: the same `projection_id` twice fails closed at `build()` with the first registration intact; two *different* `projection_id`s both register (D-3's multiplicity is real, not decorative) |
| Integration | `crates/service-sdk/tests/` | the refusal surfaces through `AppBuilder::build()` as `CompositionError::Validation(RuntimeError::PersistenceNotConfigured(..))` — the full documented path, not just the innermost error |
| E2E | `examples/reference-app/tests/users_by_tenant_projection.rs` (existing) | the projection still delivers batches when its pair arrives erased through `ReadSideProgressStores` — proves AD-3 works against the real scheduler, not just in isolation |
| E2E | `examples/reference-app/tests/production_profile_guard.rs` (existing) | SC-10 / SC-5: `build_runtime_with(.., None, ..)` still builds under the Dev path, and `Some(ReadSideProgressStores::fake_durable())` registers and builds |

Three properties need explicit tests because no happy path proves them:

- **SC-7 (a partial pair is not representable)** is a *compile-time* property, so it is
  proved by the absence of an API that could express it, not by a runtime assertion. The
  honest check is a doc-level one: `read_side_progress` is the only registration entry
  point and takes both stores. A `compile_fail` doctest would only pin that a
  nonexistent method does not exist, which any typo also satisfies.
- **SC-8 (no heuristic)** — `TypeId`, `downcast`, and type-name matching appear nowhere in
  the change, and `require_durably_configured`'s signature is byte-identical. Checked by
  reading the diff, not by a test.
- **SC-9 (the scheduler layer is untouched)** — `Profile` appears nowhere in
  `crates/runtime/src/read_side/` or `crates/domain/src/read_side/{session,runner}.rs`.
  Same: a diff property.

## Threat Matrix

N/A — no routing, shell command, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. This change adds two trait default
methods, two forwarding impls, one builder registration, one validator branch, and one
host rewiring. No input crosses a trust boundary, no external process is invoked, and no
file is executed or classified.

## Migration / Rollout

No migration. Additive and default-inert: `Profile::Dev` is the default, the registration
map is empty by default, and an empty map makes the new validator a no-op loop. No schema,
migration, data, persistence format, or runtime storage behavior is touched.

The thirteen call-site edits in AD-8 are mechanical and behavior-preserving — every
existing composition keeps the pair it has and, passing `None`, registers nothing, which
is the state it is in today.

Rollback is the proposal's, unchanged, plus one line: remove the two `Arc<T>` forwarding
impls (AD-3), which nothing outside this change consumes.

## Traceability

| Proposal item | Resolved by | Note |
|---|---|---|
| IS-1, D-8 | AD-4 | PROD-013's idiom, reused |
| IS-2, invariants 1–3 | AD-1, AD-2, AD-5 | one call, both stores, keyed by `projection_id` |
| IS-3, SC-6 | AD-7 | new `CompositionError` variant; latch before delegation |
| IS-4, SC-3, SC-4 | AD-6 | + **EC-1**: the validator splits, it does not append |
| IS-5, invariant 4, SC-1 | AD-6 | empty map ⇒ empty loop; not a special case |
| IS-6, SC-5, OOS-9 | AD-4, AD-8 | `false` default + `ReadSideProgressStores::in_memory()` stays first-class |
| IS-7, R-2, SC-10 | AD-8 | + **EC-2/AD-3**: forwarding impls are what make it non-decorative |
| IS-8, D-6 | AD-9 | plain `pub`, delegating newtype, no cargo feature |
| IS-9 | `sdd-spec`'s scope; no design decision required | R-5's two axes are AD-1's doc comment, verbatim |
| IS-10, SC-13 | AD-10 | exact replacement text given |
| SC-7 | AD-1 | compile-time property; see Testing Strategy |
| SC-8 | AD-4, AD-6 | no `TypeId`/downcast/type-name; signature untouched |
| SC-9, OOS-3 | — | no path added; scheduler files absent from the Component Map |
| SC-11 | `sdd-spec` | D-5's two axes; not a design decision |
| SC-12, OOS-1, OOS-2, D-4 | — | absent from every file list above |
| R-1, OOS-7 | accepted | named in AD-10's doc text so a reader meets the boundary |
| R-3 | AD-8 | gate not softened; `None` exercises IS-5, it does not exempt anything |
| R-4 | AD-5 | `BTreeMap` at N=1 costs nothing and is deterministic |
| R-6 | flagged | see Risks below — `sdd-tasks` owns the forecast |
| R-7 | — | `App` gains a registration and a check, never a lifecycle |
| — | AD-11 | new: the refusal cannot name `projection_id`; F-5 |

## Items Needing the Architect's Confirmation

1. **AD-8 — `main.rs` passes `None` and therefore registers nothing.** The alternative
   makes the reference app's Production binary refuse to start, with no in-tree fix until
   F-1. This is IS-5 being exercised, not the gate being softened, but it is the one place
   this design interprets IS-7 rather than following it literally, and SC-10's second
   clause deserves a deliberate read against AD-8's three criteria.
2. **EC-2 / AD-3 — two forwarding impls land in `crates/domain`.** They are additive and
   load-bearing, but they widen a framework crate's public surface in a change whose
   Affected Areas table listed `offset.rs`/`dedup.rs` as receiving only `is_durable()`.
3. **AD-11 / F-5 — the refusal names the capability, not the projection.** A consequence
   of reusing `require_durably_configured` verbatim, which is itself a binding constraint.
   Confirm F-5 is the right home for per-projection attribution.

## Open Questions

- [ ] AD-3's `?Sized` bound under `#[async_trait]` — verify in RED before the rest of the
      slice depends on it; the sized fallback is noted in AD-3 and covers the `Arc<dyn …>`
      case either way.
- [ ] R-6: the framework half (IS-1 … IS-5, IS-8) and the host half (IS-7, IS-9, IS-10)
      are the natural slice boundary, and AD-3 belongs in the first. `sdd-tasks` owns the
      400-line forecast; this design does not pre-empt it.
