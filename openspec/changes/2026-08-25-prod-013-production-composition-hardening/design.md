# Design: PROD-013 — Production Composition Hardening

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers).
>
> **Inputs**: `proposal.md` (D-1 … D-8, IS-1 … IS-12, R-1 … R-7, SC-1 … SC-11) and
> `explore.md`. This document decides **how**, never **what** — except where reading
> the real code falsified a premise the *what* rested on. Two such cases exist
> (§Evidence Corrections) and both are surfaced rather than silently implemented
> around, because designing against a premise known to be false ships a broken
> change with a clean paper trail.
>
> **Baseline verified**: `develop` @ `a740d34`.

## Technical Approach

One `Profile` enum lives in the lowest crate that needs it (`persistent-entity`) and
is re-exported upward. One shared predicate — `require_durably_configured` — is the
single place where "declared production + capability not *durably* configured =
refuse" is decided; both builders call it rather than each restating the rule. The
durability signal itself comes from a minimal capability declaration on each store's
own trait (`is_durable()` for event/snapshot store, reusing PROD-002's existing
`EffectStoreCapabilities.durable` for the effect store) — never from whether a store
was merely *present*. Two builders enforce it
because the capabilities live in two crates and the layer boundary is one-way:
`EntityRuntimeBuilder` for the event and snapshot stores, `RuntimeBuilder` for the
effect store. `EntityRuntimeBuilder` gains a `try_build()` sibling mirroring
PROD-012's exact validate-before-delegate shape, and `build()` keeps its infallible
signature and panics — so all 67 existing call sites keep compiling.

The reference app then proves the mechanism is live rather than merely available: its
production composition declares `Profile::Production` **through the type that already
exists to state its store choice**, so the declaration cannot be forgotten
independently of the durable stores it describes.

---

## Evidence Corrections

Both were found by reading the code the proposal points at. Neither is a design
preference; each is a factual correction with file:line evidence, and each changes
what the change must do.

### EC-1 — D-2's cost premise is false: 15 partial-configuration call sites exist, including the reference app's own production composition root

D-2 states the profile-independent partial-configuration check has "zero cost in
blast radius: no current call site does this — every one configures both stores or
neither (explore §4)."

Measured on `develop @ a740d34`: `with_event_store` has 18 call sites,
`with_snapshot_store` has 4. Only three chains configure both. **Fifteen chains
configure exactly one** and would be rejected outright by an unconditional check:

| # | Site | Configured |
|---|---|---|
| 1 | `examples/reference-app/src/lib.rs:502` (`observed_entity_runtime`) | event only |
| 2 | `examples/reference-app/tests/register_user_multi_aggregate_recovery.rs:341` | event only |
| 3 | `examples/reference-app/tests/register_user_multi_aggregate_recovery.rs:347` | event only |
| 4 | `crates/persistent-entity/tests/receipt_written_in_unit_of_work.rs:284` | event only |
| 5 | `crates/persistent-entity/tests/receipt_written_in_unit_of_work.rs:454` | event only |
| 6 | `crates/persistent-entity/tests/real_actor_path_tests.rs:126-130` | event only |
| 7 | `crates/persistent-entity/tests/guaranteed_completion_tests.rs:164` | event only |
| 8 | `crates/persistent-entity/tests/guaranteed_completion_tests.rs:547` | event only |
| 9 | `crates/persistent-entity/tests/guaranteed_completion_tests.rs:1037` | event only |
| 10 | `crates/persistent-entity/tests/receipt_outcome_metric.rs:345` | event only |
| 11 | `crates/persistent-entity/tests/receipt_outcome_metric.rs:367` | event only |
| 12 | `crates/persistent-entity/tests/activation_ordering_tests.rs:44` | event only |
| 13 | `crates/persistent-entity/tests/receipt_gating.rs:265` | event only |
| 14 | `integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs:284` | event only |
| 15 | `crates/persistent-entity/tests/real_actor_path_tests.rs:210-214` | **snapshot only** |

Site 1 is the reference app's production composition root. Site 15 proves the
asymmetry runs both ways. Configuring both: `persistence_failure_tests.rs:172-173`,
`210-211`, `238-239`.

The proposal is therefore internally contradictory as written. IS-5 and SC-6 require
rejection "in every profile"; IS-8 and SC-7 require that "all 67 existing call sites
compile and pass unmodified". With 15 partial sites, exactly one of those pairs can
hold. Resolved in **AD-7**.

### EC-2 — The effect store has a silent in-memory fallback, exactly like the other two

Explore §1.4 states the effect store field is "a plain `Option<...>` with **no**
`unwrap_or_else` fallback anywhere (confirmed by grep — zero in-memory default
construction for the effect store)" and concludes the risk is deferred failure at
first use, not silent volatility. Proposal D-3, IS-4 and SC-3 inherit that framing.

`crates/service-sdk/src/runtime/builder.rs:804-817` says otherwise:

```rust
let effect_acceptor_impl = if self.effect_executors.is_empty() {
    None
} else {
    let (state_store, dedup_store) =
        match (self.effect_state_store, self.effect_dedup_store) {
            (Some(state_store), Some(dedup_store)) => (state_store, dedup_store),
            _ => {
                let store = Arc::new(InMemoryEffectStore::new());   // <- line 811
                ...
```

The fallback is a `match` arm, not an `unwrap_or_else`, which is why the grep missed
it. Its own doc comment says so plainly at `builder.rs:493-495`: "without this call
`build()` keeps constructing `InMemoryEffectStore` exactly as before, whenever an
executor is registered."

This correction makes the design **simpler and more coherent**, not harder: all three
gated capabilities share one identical failure mode — silent substitution of volatile
storage — so "one gate, one rule" (D-3) is more true than the proposal claimed, not
less. It changes two things:

- SC-3's clause "and it fails at **bootstrap** rather than at first use" describes a
  failure mode the effect store does not have. What it actually prevents is the same
  silent volatility as SC-4. Spec wording should follow the code.
- The gate must be conditional on at least one registered executor (**AD-5**), since
  with none registered no store is constructed at all and nothing is volatile.

---

## Component Map

```
crates/domain                                     (traits both stores implement)
├── src/persistence/event_store.rs        MOD  + EventStore::is_durable() (default false)
└── src/persistence/snapshot.rs           MOD  + Snapshot::is_durable() (default false)
                                                   ↑ implemented by
crates/persistence                                (Postgres implementations)
├── src/postgres/event_store.rs           MOD  is_durable() -> true
└── src/postgres/snapshot.rs              MOD  is_durable() -> true
                                                   ↑ read by
crates/persistent-entity                          (lower layer, no service-sdk dep)
├── src/profile.rs                        NEW  Profile { Dev, Production }
│                                              require_durably_configured(...)  ← THE rule
├── src/error.rs                          MOD  + PersistenceCompositionError
└── src/builder.rs                        MOD  + .profile(), validate_persistence(),
                                               try_build(); build() validates+panics
                                               ↑ depends on
crates/service-sdk                                (upper layer)
├── src/runtime/mod.rs                    MOD  pub use persistent_entity::profile::Profile;
├── src/runtime/runtime_builder.rs        MOD  + RuntimeError::PersistenceNotConfigured
│                                               (#[from] PersistenceCompositionError)
├── src/runtime/builder.rs                MOD  + .profile(), validate_persistence_profile()
│                                               called from build()/try_build()
└── src/app/mod.rs                        MOD  + AppBuilder::profile() (thin delegation)
                                               CompositionError::Validation already forwards
                                               ↑ used by
examples/reference-app
├── src/lib.rs                            MOD  EntityEventStores carries profile + snapshot
│                                               stores; observed_entity_runtime threads both
├── tests/production_profile_guard.rs     NEW  Dev-side regression guard (IS-12)
└── (integration-tests/...postgres.rs)    MOD  one Production assertion (IS-12)
```

## Data Flow

```
Host                                  Framework                       Outcome
────                                  ─────────                       ───────
EntityRuntimeBuilder::new()
  .profile(Production)
  .with_event_store(pg)          ──▶  validate_persistence()
  .with_snapshot_store(pg)              is_durable()? × 2, then
  .try_build()                          require_durably_configured × 2   ──▶  Ok  → EntityRuntime
                                                                            Err → PersistenceCompositionError
                                                                            (host handles it; AD-6)

App::builder()
  .profile(Production)
  .effect_executor(...)          ──▶  RuntimeBuilder::validate_persistence_profile()
  .effect_store(pg)                     capabilities().durable?, then
                                         require_durably_configured × 1   ──▶  Ok  → App
  .build()                                                            Err → RuntimeError::PersistenceNotConfigured
                                                                            → CompositionError::Validation
```

---

## Architecture Decisions

### AD-1 — `Profile` lives in `crates/persistent-entity/src/profile.rs`, re-exported from `service-sdk::runtime`

**Decision**: a new, dedicated 20-line module in the lower crate, plus one
re-export line.

```rust
// crates/persistent-entity/src/profile.rs
/// What a composition declares about the deployment it is being built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// Today's behavior, byte-for-byte. Volatile storage by omission is valid
    /// here, because that is what dev and test are for.
    #[default]
    Dev,
    /// Every persistent capability this composition uses must be configured
    /// explicitly. Nothing volatile is reachable by omission.
    Production,
}
```

```rust
// crates/persistent-entity/src/lib.rs
pub mod profile;

// crates/service-sdk/src/runtime/mod.rs  (alongside the existing pub use block)
pub use persistent_entity::profile::Profile;
```

**Criteria**: (a) `service-sdk` depends on `persistent-entity`
(`crates/service-sdk/Cargo.toml:24`) and never the reverse, so the shared type has
exactly one admissible home; (b) one type, not two — a `service_sdk::Profile` distinct
from a `persistent_entity::Profile` would let a host declare Production on the
`AppBuilder` and Dev on its entity runtimes with nothing objecting.

**Why a dedicated module rather than inside `builder.rs`**: exact precedent.
`IdempotencyEnforcementMode` — the type this whole change mirrors — lives in its own
`crates/service-sdk/src/runtime/idempotency.rs`, not inside `builder.rs`, and is
re-exported from `runtime/mod.rs:17`. `Profile` governs two builders in two crates, so
`use persistent_entity::builder::Profile` would misdescribe it.

**Consequence**: hosts write `use ego_service_sdk::runtime::Profile` (or
`persistent_entity::profile::Profile` when composing entity runtimes only). `Default`
is derived rather than hand-written: unlike `IdempotencyEnforcementMode`, whose
default needed a paragraph explaining why it is the *strict* variant, `Profile::Dev`
is the permissive one and D-1 already carries that reasoning.

### AD-2 — The refusal is `PersistenceCompositionError`, in `crates/persistent-entity/src/error.rs`

**Decision**: one new `thiserror` enum in the existing error module, carrying the
capability and the fix hint as `&'static str`.

```rust
// crates/persistent-entity/src/error.rs
/// A composition was declared production and a persistent capability it uses
/// has no explicitly configured implementation.
///
/// Deliberately not an `EntityError` variant: `EntityError` reports what went
/// wrong while an entity was *running* a command. This one reports that the
/// runtime must not be built at all, and nothing that handles a command
/// failure should have to consider it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersistenceCompositionError {
    #[error(
        "Profile::Production is declared but no {capability} is configured — a \
         production composition must never fall back to volatile storage. \
         Configure one with {fix}, or state that this composition is not \
         production with .profile(Profile::Dev)"
    )]
    NotConfigured {
        /// The capability with no configured implementation.
        capability: &'static str,
        /// The exact call that fixes it.
        fix: &'static str,
    },
}
```

**Criteria**: (a) `persistent-entity` cannot borrow `RuntimeError`/`CompositionError`
(R-4, verified: no `service-sdk` dependency); (b) `thiserror = "1.0"` is already a
dependency (`crates/persistent-entity/Cargo.toml:11`), so no new dependency;
(c) IS-7 requires naming both the missing capability and the exact fixing call —
`&'static str` fields deliver both while keeping one variant, matching how
`RuntimeError::DependencyNotFound` already carries `type_name: &'static str` plus a
fix hint (`runtime_builder.rs:1481-1493`).

**Runner-up, rejected**: a new `EntityError::PersistenceNotConfigured` variant.
`EntityError` is a hand-rolled `Display` enum whose every variant describes a
command-execution outcome (`crates/persistent-entity/src/error.rs:9-50`), and every
`match` over it — including exhaustive ones in actor and recovery code — would gain a
dead arm for a condition that can only occur before any actor exists.

**Rejected outright**: one variant per capability (three variants). The message text
differs by two `&'static str`s; three variants means three `#[error]` strings to keep
consistent, which is the drift SC-8 exists to prevent.

### AD-3 — One shared predicate, `require_durably_configured`, decides refuse-or-allow; each capability supplies its own durability signal (IS-6 / SC-8)

**Decision**: the rule is a free function next to `Profile`, and every gate calls it.
It is the only place in the workspace where the words "Production" and "not
durably configured" meet. The boolean it receives MUST represent durability, never
mere presence — `Some(volatile_store).is_some()` is `true`, and that is exactly the
mistake this predicate's own argument name exists to make hard to make by accident.

```rust
// crates/persistent-entity/src/profile.rs
use crate::error::PersistenceCompositionError;

/// The one definition of PROD-013's rule.
///
/// A free function rather than a method on either builder, because the three
/// gated capabilities live in two crates that cannot share a builder: the event
/// and snapshot stores are `EntityRuntimeBuilder`'s, the effect store is
/// `RuntimeBuilder`'s, and `persistent-entity` cannot see `EffectStateStore`.
/// Restating the rule once per crate would create exactly the second, parallel
/// check SC-8 forbids; passing the three varying facts as arguments keeps one.
///
/// `durably_configured`, not `configured`: every call site MUST compute this
/// from the capability's own durability declaration (below), never from
/// `.is_some()` alone — presence and durability are different properties.
pub fn require_durably_configured(
    profile: Profile,
    durably_configured: bool,
    capability: &'static str,
    fix: &'static str,
) -> Result<(), PersistenceCompositionError> {
    match profile {
        Profile::Production if !durably_configured => {
            Err(PersistenceCompositionError::NotConfigured { capability, fix })
        }
        _ => Ok(()),
    }
}
```

**Criteria**: (a) SC-8 demands one shared predicate as the single source of truth for
the refuse/allow *decision* "across all three capabilities"; (b) it must remain true
across a layer boundary that no single function can straddle if it inspects builder
fields directly — so the function takes the *answer* (`durably_configured: bool`)
rather than the builder, and each composition surface keeps only the one-line
capability check it alone can perform; (c) `persistent-entity` genuinely cannot see
the effect store — `EffectStateStore` and `EffectDedupStore` are `ego-runtime` types
reached through `service-sdk` (`builder.rs:12`), and importing them downward would
invert the dependency `xtask verify-layers` enforces.

**This is the honest answer to the question the proposal deferred** ("is there a way
to unify it in a single point despite the layering?"): yes, but only by extracting the
*decision* rather than the *inspection*. The decision is the part that could drift and
the part SC-8 is about. The inspection is the two-part answer below.

**Where the durability signal itself comes from** (the property this predicate can
receive but cannot manufacture): a minimal capability declaration on each store's own
trait, mirroring the pattern PROD-002 already established for the effect store.

```rust
// crates/domain/src/persistence/event_store.rs — EventStore<E>, and the
// structurally identical method on crates/domain/src/persistence/snapshot.rs's Snapshot
fn is_durable(&self) -> bool {
    false   // default: honest for every existing and third-party implementation
}
```

`InMemoryEventStore`/`InMemorySnapshotStore`/`NoopEventStore`/`NoopSnapshotStore`
inherit this default untouched — nothing about them changes. `PostgreSQLEventStore`/
`PostgreSQLSnapshotStore` override it to `true`. The effect store needs no new trait
method at all: `EffectStateStore::capabilities() -> EffectStoreCapabilities { durable,
... }` already exists (PROD-002 AD-3, `crates/runtime/src/effects/store.rs:238-244`),
defaults `durable: false`, and every Postgres effect store implementation already
overrides it to `true` (`crates/effect-store/src/postgres/mod.rs:379-386`, `:690-697`)
— AD-5 below reuses `capabilities().durable` directly.

**Why a trait method, not a downcast, a marker type, or a separate registry**: (a) a
downcast to a concrete `InMemoryEventStore`/`PostgreSQLEventStore` type would make the
gate unable to recognize any third-party durable implementation — it would have to
enumerate every concrete type in the workspace by name, forever; a trait method is
answered by the implementation itself, so an external crate's durable store simply
overrides it and is recognized with no gate-side change. (b) No existing trait method
needed to change signature or behavior — this is additive, so nothing that already
implements `EventStore`/`Snapshot` breaks by inheriting the default. (c) A single
`bool` is proportionate: neither trait declares any other capability today, so a full
`Capabilities` struct (as the effect store already has, for four *different*
concerns — durability, concurrency-safety, multi-node-safety, lease support) would be
one-field ceremony copied for no reason; if a second capability concern is ever needed
here, that is the moment to introduce one, not before.

**Consequence**: the per-capability call sites become declarations of fact — computed
from durability, never presence — and the capability/fix strings live at the site that
owns the call being recommended:

```rust
// crates/persistent-entity/src/builder.rs
fn validate_persistence(&self) -> Result<(), PersistenceCompositionError> {
    require_durably_configured(
        self.profile,
        self.event_store.as_ref().is_some_and(|s| s.is_durable()),
        "event store", "EntityRuntimeBuilder::with_event_store(store)",
    )?;
    require_durably_configured(
        self.profile,
        self.snapshot_store.as_ref().is_some_and(|s| s.is_durable()),
        "snapshot store", "EntityRuntimeBuilder::with_snapshot_store(store)",
    )
}
```

Event store is checked first, deliberately: when both are missing the caller sees the
one they are far more likely to have meant to configure, and PROD-012 established that
a refusal reports the first violation rather than a list
(`try_build_fails_before_startup_when_declared_entity_dependency_is_missing`).

**Revision note**: an earlier draft of this section computed the argument from
`self.event_store.is_some()` — presence, not durability — which a reviewer correctly
flagged before any implementation shipped: `Some(InMemoryEventStore::new())` and
`Some(PostgreSQLEventStore::open(pool))` are indistinguishable under `.is_some()`, so
`Profile::Production` would have accepted an explicitly-wired volatile store. Closed
here, in this same decision, before WU2 implements it — not as a later patch.

### AD-4 — `EntityRuntimeBuilder` gains `try_build()`, mirroring PROD-012 exactly (resolves R-3)

**Decision**: mirror `crates/service-sdk/src/runtime/builder.rs:740-771` and
`:1088-1092` shape-for-shape. No deviation, because none is justified.

```rust
// crates/persistent-entity/src/builder.rs

/// Consumes the builder and produces an [`EntityRuntime`].
///
/// # Panics
///
/// Panics when [`Profile::Production`] is declared and a gated persistent
/// capability has no configured implementation.
///
/// A panic rather than a `Result`, because this signature is what all 67
/// existing call sites already call, and because the alternative is worse
/// than a loud stop: a runtime that declares production and silently writes
/// every event into process memory loses them on the next restart, and
/// reports nothing. Bootstrap is the cheapest moment to refuse.
///
/// [`Self::try_build`] returns the same condition as a structured error.
pub fn build(self) -> EntityRuntime<E> {
    if let Err(err) = self.validate_persistence() {
        panic!("{err}");
    }
    /* ...existing body, unchanged... */
}

/// Consumes the builder and produces an [`EntityRuntime`], returning the
/// profile gate's refusal instead of panicking.
pub fn try_build(self) -> Result<EntityRuntime<E>, PersistenceCompositionError> {
    // Before delegating, not after. `build` panics on this condition, so
    // checking afterwards would mean this method could never return the
    // error it exists to return — the panic would already have unwound.
    self.validate_persistence()?;
    Ok(self.build())
}
```

**Criteria**: (a) the proposal names this template the reference and R-3 calls the
ordering load-bearing — it is, and for the identical reason, so the comment says so in
the same words; (b) `build()`'s signature is load-bearing across 67 call sites in 25
files (re-verified, R-2 discharged: `EntityRuntimeBuilder::new()` → 67 occurrences /
25 files on `a740d34`); (c) a reviewer who knows PROD-012 needs no second pattern.

**Reasons to deviate, considered and found absent**:

- *Change `build()` to return `Result` instead of adding a sibling.* Breaks all 67
  sites and violates IS-8/SC-7. This is Approach C's migration in disguise (AD-11).
- *Only add `try_build()` and leave `build()` unvalidated.* Then `Profile::Production`
  on a `build()` call is silently accepted and the gate is decorative — the exact
  fail-open the change exists to close. PROD-012 rejected this for the same reason.
- *Deprecate `build()`.* A deprecation warning across 67 sites is noise on a signature
  that is correct for `Profile::Dev`, which is most of them.

**Difference from PROD-012, stated because it is real**: `RuntimeBuilder::try_build`
takes `mut self` and does more than validate — it runs `Injectable` validators after
delegating. `EntityRuntimeBuilder::try_build` takes `self` and only validates, so it
is a strictly smaller version of the same shape. It does not need `mut`.

### AD-5 — The effect store gate lives in `RuntimeBuilder`, is conditional on a registered executor, and crosses upward through `RuntimeError`

**Decision**: a second validator in `RuntimeBuilder`, called from both `build()` and
`try_build()` in the same order `validate_idempotency` already is, gated on at least
one registered executor.

```rust
// crates/service-sdk/src/runtime/builder.rs
fn validate_persistence_profile(&self) -> Result<(), RuntimeError> {
    // Conditional on a registered executor because with none registered no
    // effect store is constructed at all (see `build()`'s `effect_acceptor_impl`
    // gate) — there is no volatile storage to refuse. Requiring one anyway
    // would force every production host that describes no external effects to
    // register a store it never reads or writes.
    if self.effect_executors.is_empty() {
        return Ok(());
    }
    require_durably_configured(
        self.profile,
        self.effect_state_store
            .as_ref()
            .is_some_and(|s| s.capabilities().durable),
        "effect store",
        "RuntimeBuilder::with_effect_store(store) (or AppBuilder::effect_store(store))",
    )?;
    Ok(())
}
```

```rust
// crates/service-sdk/src/runtime/runtime_builder.rs — RuntimeError
/// A production composition left a persistent capability unconfigured
/// (PROD-013). Wraps the lower crate's refusal rather than restating it:
/// `persistent-entity` owns the rule (AD-3) and this layer owns only the
/// crossing, exactly as `CompositionError::Validation` owns the crossing for
/// this enum.
#[error("production composition validation failed: {0}")]
PersistenceNotConfigured(#[from] persistent_entity::error::PersistenceCompositionError),
```

**Criteria**: (a) EC-2 — the fallback at `builder.rs:811` is real, so this gate closes
real silent volatility, not merely a deferred failure; (b) `build()` and `try_build()`
must agree, which the existing single-validator shape already guarantees;
(c) `CompositionError::Validation(#[from] RuntimeError)` already exists
(`app/error.rs:59`) and `AppBuilder::build()` already maps through it
(`app/mod.rs:807`), so the `AppBuilder` surface costs zero new plumbing — the
proposal's "surfaces through `AppBuilder` for free" holds, verified.

**Checks `effect_state_store` only, not both**: `with_effect_store` is the only way to
populate either field and always sets both from the same `Arc`
(`builder.rs:501-508`), an invariant `build()` already asserts at
`builder.rs:797-803`. Checking both would imply a mixed state the public API cannot
express.

**Consequence**: `AppBuilder` gains one thin delegating method, matching how
`effect_store` itself delegates (`app/mod.rs:562-578`):

```rust
pub fn profile(mut self, profile: Profile) -> Self {
    if self.pending_error.is_some() { return self; }
    self.runtime_builder = self.runtime_builder.profile(profile);
    self
}
```

`AppBuilder::profile` does **not** propagate the profile to registered entity
runtimes. It cannot: `AppBuilder::entity()` receives an already-built
`Arc<EntityRuntime<E>>`, so the entity runtime's own gate ran before `AppBuilder` ever
saw it. Its doc comment must say so, or a host will reasonably assume one call covers
all three capabilities.

### AD-6 — No cross-layer bridge for the event/snapshot refusal (corrects a proposal assumption)

**Decision**: `PersistenceCompositionError` reaches `RuntimeError` only via the effect
store path (AD-5). The event/snapshot refusal is returned to whoever called
`EntityRuntimeBuilder::try_build()` and goes no further. No
`From<PersistenceCompositionError>` is added for that path.

**Evidence**: the proposal's Approach section anticipates the event/snapshot error
"crossing the layer boundary exactly as
`RuntimeError::OperationReservationStoreNotRegistered` already does one layer up."
It does not need to, because there is no path for it to cross. `EntityRuntime`
instances are built by the **host**, not by `RuntimeBuilder`:
`AppBuilder`/`RuntimeBuilder` only ever receive a finished
`Arc<EntityRuntime<E>>` through `with_entity` / `entity`. Confirmed at every call
site, e.g. `crates/service-sdk/src/runtime/builder.rs:3175-3177`
(`Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build())` then
`.with_entity::<TestEntity>(entity_runtime)`), and in the reference app at
`lib.rs:649-658`.

`PersistenceCompositionError` implements `std::error::Error` via `thiserror`, so the
reference app's `build_runtime_with(...) -> Result<BuiltRuntime, Box<dyn Error>>`
absorbs it with a bare `?` and no conversion at all.

**Criteria**: (a) rung 1 of the ladder — a bridge nothing traverses is speculative
surface; (b) adding it invites a future reader to believe `AppBuilder::build()` can
report an entity runtime's refusal, which it structurally cannot; (c) R-4 is thereby
narrower than assessed: one crossing, not two, and the one that exists uses the exact
cited precedent.

### AD-7 — D-2's unconditional partial-configuration check is folded into the Production gate

**Decision**: no separate profile-independent check ships. Under
`Profile::Production` a missing store is refused (which subsumes every partial case);
under `Profile::Dev` nothing changes.

**This changes the proposal and needs the architect's confirmation.** It is recorded
here rather than absorbed silently, per D-7's own standard: a migration must be sized
explicitly, never folded into "add a gate."

**Criteria**:

1. **Its stated cost is false.** EC-1: 15 partial call sites exist across 8 files, not
   zero. The check's entire justification in D-2 is "zero cost in blast radius: no
   current call site does this."
2. **Site 1 is the reference app's production composition root.**
   `observed_entity_runtime` (`examples/reference-app/src/lib.rs:502`) configures the
   event store and not the snapshot store, and *every* entity runtime the reference
   app builds — production, in-memory, and observed — flows through it. An
   unconditional check makes the reference app's own composition unbuildable in every
   profile, including the dev variants IS-8 exists to protect.
3. **The proposal cannot satisfy both halves of its own contract.** IS-5/SC-6 ("both
   profiles") against IS-8/SC-7 ("all 67 sites compile and pass unmodified"). One must
   yield, and IS-8/SC-7 is the one D-1 was chosen for, the one the rollback plan rests
   on, and the one the architect approved as the change's zero-blast-radius property.
4. **Profile-gating it instead would make it redundant, not cheaper.** Under
   `Profile::Production`, "exactly one configured" is already refused by the
   missing-one rule (AD-3), because the missing one is missing. A profile-gated partial
   check is dead code by construction.
5. **The one real defect D-2 was aimed at is caught anyway, in this change.** Site 1 —
   Postgres event store wired, snapshot store silently in-memory — is precisely the
   "event store wired, snapshot store forgotten" mistake D-2 describes, and AD-9 fixes
   it because AD-8 puts that composition under `Profile::Production`.

**Alternatives considered**:

- *Keep it unconditional and migrate all 15 sites.* Adds
  `.with_snapshot_store(Arc::new(Mutex::new(InMemorySnapshotStore::new())))` to 14 test
  chains that never snapshot (most already use `NoSnapshot` as their strategy), buys
  nothing over criterion 5, and breaks SC-7 outright.
- *Warn instead of refuse.* `tracing` is available in `persistent-entity`
  (`Cargo.toml:12`), so it is cheap. Rejected: a warning that fires 15 times on a clean
  workspace test run is noise that trains readers to ignore it, and "log and continue"
  is the weak contract this change replaces.

**Consequence**: IS-5, SC-6, and D-2 need amending, and the acceptance criterion's
third `Given` block ("any composition, with or without `Profile::Production`") narrows
to Production. Nothing else in the proposal depends on D-2.

### AD-8 — The reference app declares `Profile::Production` through `EntityEventStores`, not through a `build_runtime_with` argument

**Decision**: `EntityEventStores` — the type that already exists so "the choice of
backing store is **stated**, never defaulted" (`lib.rs:338`) — carries the profile.
`EntityEventStores::open(pool)` yields `Profile::Production`;
`EntityEventStores::in_memory()` yields `Profile::Dev`. The profile field is private,
with those two constructors the only way in.

```rust
pub struct EntityEventStores {
    pub org: Arc<dyn EventStore<OrganizationEnsured> + Send + Sync>,
    pub user: Arc<dyn EventStore<UserRegistered> + Send + Sync>,
    /// See AD-9.
    pub org_snapshot: Arc<Mutex<dyn Snapshot + Send>>,
    pub user_snapshot: Arc<Mutex<dyn Snapshot + Send>>,
    /// Private, and set only by the two constructors: durable stores and a
    /// production declaration are one decision in this host, so they cannot
    /// drift apart. A `pub` field would let a caller assemble Production over
    /// `in_memory()` stores, which is the state this change exists to refuse.
    profile: Profile,
}

impl EntityEventStores {
    pub fn profile(&self) -> Profile { self.profile }
}
```

**Why not IS-11 as literally worded.** IS-11 asks that `build_runtime_with` declare
`Profile::Production`. It cannot: `build_runtime_with` is not the production-only
entry point the proposal takes it for. It is the *shared* entry point, called with
in-memory stores from four places today —
`build_runtime_observed_in_memory` (`lib.rs:526-528`, which
`build_runtime_in_memory` at `:311-315` delegates to),
`examples/reference-app/tests/stoolap_restart_persistence.rs:86` and `:134`, and
`examples/reference-app/tests/idempotency_wiring.rs:86`. Hardcoding Production inside
it breaks all four and every dev entry point the app has.

**Criteria**:

1. **It answers R-1 structurally rather than conventionally.** R-1 — "the flag must be
   *remembered*" — is closed for this host not because a check watches for the
   declaration, but because there is no separate declaration to forget:
   `EntityEventStores::open` is the only way to obtain durable stores, and it is the
   only thing that produces Production. `main.rs:78` already calls it. This is a
   strictly stronger guarantee than D-8's "one composition call plus one regression
   check", and it needs neither.
2. **Zero call-site churn.** `main.rs` (which calls `open`) becomes Production with no
   edit. All four in-memory `build_runtime_with` callers stay Dev with no edit. The
   four Postgres integration tests calling `EntityEventStores::open`
   (`durable_entity_progress_postgres.rs:94`, `:390`, `:428`;
   `dual_aggregate_crash_recovery_postgres.rs:237`;
   `concurrent_replicas_postgres.rs:279`) become Production with no edit and remain
   valid, because `open` supplies the durable snapshot stores too (AD-9).
3. **It matches the type's existing purpose exactly.** `EntityEventStores` was
   introduced for this precise class of defect; its own doc comment says the in-memory
   default "meant every event and every receipt lived in process memory — and a restart
   lost the durable progress the receipts exist to record" (`lib.rs:338-342`).

**Alternative considered — a `profile: Profile` parameter on `build_runtime_with`.**
More literal to IS-11, roughly the same line count. Rejected because the regression
guard it permits is strictly weaker: `main.rs` is a binary, so no test can call it, and
the only way to prove `main.rs` passes `Profile::Production` is to grep its source
text. AD-8 removes the thing that could regress instead of watching it.

**Scope note**: `EntityEventStores` is reference-app-local sugar, not framework API.
`Profile` remains a first-class `EntityRuntimeBuilder`/`AppBuilder` method (AD-1,
AD-5), so a host with a durable store this type does not know about declares its
profile directly. AD-8 constrains one example host, not the framework.

**Consequence**: `observed_entity_runtime` (`lib.rs:488-510`) takes the snapshot store
and the profile, and returns `Result` because it now calls `try_build()`.
`compose_entity_runtimes` (`lib.rs:452-471`) — public, and called with `in_memory()`
from `tests/metrics_reach_one_backend.rs:209` — also becomes fallible, or keeps
`build()`; with the profile private and `open()` always supplying every store, no
constructible input can make it refuse, so keeping `build()` there is defensible and
smaller. Tasks should pick one and say which; the design's requirement is only that
whichever is chosen, the Dev path stays infallible for existing callers.

### AD-9 — The reference app's production path gains `PostgreSQLSnapshotStore`, closing a live durability defect the gate exposes

**Decision**: `EntityEventStores::open(pool)` also constructs two
`PostgreSQLSnapshotStore` instances over the same pool;
`EntityEventStores::in_memory()` constructs two `InMemorySnapshotStore`s.

```rust
// in open(pool), alongside the two PostgreSQLEventStore::open calls
org_snapshot: Arc::new(Mutex::new(PostgreSQLSnapshotStore::new(pool.clone()))),
user_snapshot: Arc::new(Mutex::new(PostgreSQLSnapshotStore::new(pool))),
```

**Why this is required, not optional**: without it, AD-8 makes the reference app's
production composition declare `Profile::Production` with no configured snapshot
store, and the gate refuses its own reference host. The alternative — exempting the
snapshot store from the gate — would delete IS-3 and SC-2.

**Why this is in scope**: OOS-1 excludes *building* a durable backend.
`PostgreSQLSnapshotStore` already exists (`crates/persistence/src/postgres/snapshot.rs:27`,
exported at `crates/persistence/src/lib.rs:11`) and the `snapshots` table already
ships in the migrations. Nothing is implemented here; two constructor calls are wired.
The reference app already depends on `ego-persistence` for
`PostgreSQLEventStore::open`.

**What this reveals**: the reference app's production deployment today writes events to
Postgres and snapshots to process memory, silently. It is the exact defect class
PROD-013 exists to close, found in the change's own reference host, and it is the
strongest available evidence that the gate does real work rather than documenting
compliance.

**Two typed instances over one pool, not one shared instance**: mirrors the existing
comment on `EntityEventStores` ("same pool, same tables, same transactions"). One
shared `Arc<Mutex<...>>` would serialize all snapshot I/O for both aggregates behind a
single lock, which the current per-runtime `InMemorySnapshotStore` default does not.

**Landmine for the task phase** — `PostgreSQLSnapshotStore::save_snapshot` calls
`tokio::task::block_in_place` (`snapshot.rs:46-48`), which **panics on a
current-thread runtime**. `main.rs` is `#[tokio::main]` (multi-thread by default), so
production is fine. The Postgres integration tests use bare `#[tokio::test]`
(current-thread) — e.g. `durable_entity_progress_postgres.rs:187`, `:233`, `:289`,
`:374`. They only panic if a save is actually triggered, and the default
`PeriodicSnapshotStrategy::new(100)` (`crates/persistent-entity/src/builder.rs:265`)
means fewer than 100 events per aggregate never triggers one — which is why this is
latent today rather than already broken. Whichever of those tests can cross the
threshold must move to `#[tokio::test(flavor = "multi_thread")]`. Tracked as a risk,
not assumed away.

### AD-10 — The IS-12 regression check is two test assertions, not an `xtask` lint

**Decision**: no new `xtask` subcommand. Two behavioral assertions:

1. `examples/reference-app/tests/production_profile_guard.rs` (new) — asserts
   `EntityEventStores::in_memory().profile() == Profile::Dev` and that
   `build_runtime_with` over in-memory stores still builds. Guards the Dev path and
   SC-5 at the composition root.
2. One added assertion in the existing
   `integration-tests/tests/infrastructure/durable_entity_progress_postgres.rs` (which
   already opens a real pool and already calls `EntityEventStores::open` at `:94` then
   `build_runtime_with` at `:112`) —
   `assert_eq!(stores.profile(), Profile::Production)`. Guards the production
   declaration where the pool it needs already exists.

**Criteria**:

1. **An `xtask` lint here would be run by nobody.** There is no CI:
   `.github/workflows/` does not exist, and the `Makefile` has no `xtask` target
   (grepped for `xtask`/`verify-layers`/`verify-isolation`/`verify-hygiene` — zero
   matches). The existing lints are invoked manually. A guard whose whole purpose is
   to fail a build that never runs it is a guard in name only, which is the failure
   mode R-1 already describes one level up.
2. **The existing `xtask` lints exist because their subject has no runtime
   observation point.** `verify-layers` reads `cargo metadata` and `layers.toml`;
   `verify-isolation` compiles each crate under its own feature set;
   `verify-hygiene` walks `openspec/changes/`. None of those facts is observable from
   a test. A profile declaration is: it is a value a function returns.
3. **Smallest surface that resolves the problem** — the project's stated bar. AD-10 is
   one new small test file plus one line in a test that already exists. An `xtask`
   lint is a new module, a new `main.rs` arm, a new usage string, a source-text parser
   for `lib.rs`, and a `Makefile`/CI hook to make it run at all.
4. **It asserts behavior, not source text.** A text lint searching `lib.rs` for the
   literal `Profile::Production` passes on a declaration inside dead code, a comment,
   or a `#[cfg(test)]` block, and fails on a correct refactor that moves the string.
   `stores.profile()` cannot be satisfied by anything but the real value.

**Consequence, stated rather than glossed**: assertion 2 lives in a Docker-dependent
suite, so it does not fire on a plain `cargo test --workspace`. That is acceptable
because AD-8 already removed the failure mode a cheap always-on check would have
guarded — with the profile field private and `open()` its only Production producer,
there is no way to reach a production composition without going through the code
assertion 2 covers. If a Postgres-free assertion is wanted anyway, the honest option
is a `#[cfg(test)]`-only constructor on `EntityEventStores`, and that is a testing
seam in production code to compensate for a check that has no gap to close — rejected
on the same "smallest surface" grounds.

### AD-11 — Approach C: evaluated, sized, deferred (D-7 / OOS-5)

Recorded for traceability only. **Not implemented and not designed here.**

**What it is**: flip the default so `Profile::Production` (or an unnamed equivalent) is
what a bare `EntityRuntimeBuilder::new()` gets, with one named opt-out — the exact
shape `IdempotencyEnforcementMode::MandatoryKey` + `Compatibility` already has.

**Measured cost on `develop @ a740d34`**:

| Item | Count | Evidence |
|---|---|---|
| `EntityRuntimeBuilder::new()` call sites | 67, in 25 files | grepped, R-2 discharged |
| …that configure neither store | ~32, in 14 files | explore §2 |
| …that configure exactly one | 15, in 8 files | EC-1 |
| **Sites needing the opt-out** | **~47, in ~20 files** | sum of the two rows above |
| `compat()`-style helper definitions needed | ~12 | see below |

The helper count is the part D-7 flags and the part that is easy to underestimate.
PROD-012's own helper is `#[cfg(test)]`-local and four lines
(`crates/service-sdk/src/runtime/builder.rs:1715-1718`), and PROD-012 already needed a
**second** copy for the `AppBuilder` surface (`compat_app()`,
`crates/service-sdk/src/app/mod.rs:822-824`) because a `#[cfg(test)]` item cannot
cross a module, let alone a crate or an integration-test binary. PROD-013's affected
sites span `persistent-entity`'s own `src` tests, its **8** separate files under
`crates/persistent-entity/tests/` (each an independent binary needing its own copy or
a shared `tests/support` module that does not exist there today), `service-sdk`'s `src`
tests and 2 files under its `tests/`, `examples/reference-app/tests/`, and
`integration-tests/tests/infrastructure/`. Either ~12 duplicated definitions or one new
`ego-testkit` export plus ~12 imports — and `ego-testkit` is dev-only by deliberate
layering policy (`crates/persistent-entity/Cargo.toml:19-21`), so that route needs its
own sign-off.

**Estimated total**: ~47 call-site edits plus ~12 helper definitions across ~20 files;
roughly 250–350 lines of pure migration, touching six crates, with zero behavioral gain
over Approach A once AD-8 lands — because AD-8 already makes the reference host's
production path unable to reach volatile storage by omission.

**Why deferred rather than rejected**: it remains the strongest end-state contract for
a *third-party* host that never reads the reference app, which AD-8 does not protect
(the residual risk R-1 explicitly accepts). If it is ever revisited, the numbers above
are the starting inventory, and it should be its own change with its own migration
plan — never folded into a change whose approved property is zero blast radius.

---

## Integration Points

| Boundary | Direction | Mechanism | Verified at |
|---|---|---|---|
| `persistent-entity` → `service-sdk` | up | `pub use persistent_entity::profile::Profile` | `crates/service-sdk/Cargo.toml:24`; re-export style at `runtime/mod.rs:13-27` |
| `PersistenceCompositionError` → `RuntimeError` | up, effect store only | `#[from]` | new variant; AD-5 |
| `RuntimeError` → `CompositionError` | up | `Validation(#[from] RuntimeError)`, already present | `app/error.rs:59`; mapped at `app/mod.rs:807` |
| event/snapshot refusal → host | out, no crossing | `Result` from `try_build()`, absorbed by `?` | AD-6 |
| `AppBuilder::profile` → `RuntimeBuilder::profile` | down, thin | delegation, mirroring `effect_store` | `app/mod.rs:562-578` |
| entity runtimes → `AppBuilder` | in, already built | `with_entity(Arc<EntityRuntime<E>>)` — profile gate already ran | `runtime/builder.rs:3175-3177` |

## Testing Strategy

Per `ego-rs-testing-strategy`: unit tests where the rule lives, integration tests at
the crate boundary, end-to-end only for the composition root. Strict TDD — RED before
GREEN, each error path asserting the specific error rather than `is_err()`.

| Level | Location | What it proves |
|---|---|---|
| Unit | `crates/persistent-entity/src/profile.rs` `#[cfg(test)]` | `require_durably_configured` over the full matrix: {Dev, Production} × {durably configured, not} — four cases, one table-driven test; plus a pinned regression test proving presence alone (`is_some()`) is never accepted as durability |
| Unit | `crates/persistent-entity/src/builder.rs` `#[cfg(test)]` | SC-1, SC-2 (`try_build` refuses, naming capability **and** fix call), SC-5 (Dev + nothing configured still builds), that `build()` panics on the same input `try_build()` refuses, and that an explicit in-memory store under `Profile::Production` is refused exactly like a missing one |
| Unit | `crates/persistent-entity/src/error.rs` `#[cfg(test)]` | the message names both the capability and the exact fixing call, mirroring PROD-012's `the_refusal_names_the_registration_and_the_opt_out` (IS-7) |
| Unit | `crates/service-sdk/src/runtime/builder.rs` `#[cfg(test)]` | SC-3; that no executor registered means no refusal (AD-5); that `build()`/`try_build()` agree |
| Integration | `crates/service-sdk/tests/` | the refusal surfaces as `CompositionError::Validation` through `AppBuilder::build()` |
| E2E | `examples/reference-app/tests/production_profile_guard.rs` | SC-11 Dev half, SC-5 at the composition root (AD-10) |
| Integration (Docker) | existing `durable_entity_progress_postgres.rs` | SC-11 Production half (AD-10) |

Two negative properties need explicit tests, because they are what SC-4 and SC-7
actually assert and neither is provable by a passing happy path:

- **SC-4** — under `Profile::Production`, no path reaches `InMemoryEventStore` or
  `InMemorySnapshotStore`. Assert the refusal happens *before* construction, not that
  the constructed runtime looks right; the current `unwrap_or_else` arms
  (`builder.rs:279-286`) must be unreachable, and a test that inspects the built
  runtime cannot distinguish "unreachable" from "reached and then overwritten".
- **SC-7** — `cargo test --workspace` with zero new failures across all 67 call sites.
  This is the whole of IS-8 and is checked by the suite as a whole, not by a new test.

## Traceability

| Proposal item | Resolved by | Note |
|---|---|---|
| IS-1, D-1 | AD-1 | |
| IS-2, IS-3, SC-1, SC-2 | AD-3, AD-4 | |
| IS-4, SC-3 | AD-5 | SC-3's "at bootstrap rather than first use" needs rewording per EC-2 |
| IS-5, SC-6, D-2 | **AD-7** | premise falsified (EC-1) — needs proposal amendment |
| IS-6, SC-8 | AD-3 | one predicate, honest across the layer boundary |
| IS-7 | AD-2 | |
| IS-8, SC-7 | AD-4 | 67 sites / 25 files re-verified; R-2 discharged |
| IS-9, IS-10, SC-9, SC-10 | documentation-only; no design decision required | |
| IS-11, SC-11 | AD-8, AD-9 | literal wording impossible (`build_runtime_with` is shared) |
| IS-12 | AD-10 | test, not `xtask` |
| D-7, OOS-5, R-7 | AD-11 | sized, deferred |
| R-3 | AD-4 | |
| R-4 | AD-2, AD-6 | narrower than assessed: one crossing, not two |
| R-2 | discharged | 67/25 re-measured; EC-1 found the gap it warned about |

## Items Needing the Architect's Confirmation

1. **AD-7 / EC-1** — D-2, IS-5, SC-6 and the acceptance criterion's third `Given`
   block need amending. The unconditional partial-configuration check costs 15
   call-site migrations including the reference app's production composition root, not
   zero, and contradicts IS-8/SC-7.
2. **EC-2** — SC-3's "fails at bootstrap rather than at first use" describes a failure
   mode the effect store does not have. The real defect is the silent
   `InMemoryEffectStore` fallback at `crates/service-sdk/src/runtime/builder.rs:811`.
   Spec wording should follow the code.
3. **AD-8 / AD-9** — IS-11 as worded is not implementable. Confirm the profile
   travelling on `EntityEventStores`, and confirm that wiring the already-existing
   `PostgreSQLSnapshotStore` into the reference app's production path is in scope
   (it is wiring, not implementing, so OOS-1 permits it — but it is new behavior in
   the reference host and it exposes a live durability defect).
