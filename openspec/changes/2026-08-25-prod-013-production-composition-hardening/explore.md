# Exploration: PROD-013 — Production Composition Hardening

**Phase**: `sdd-explore`
**Change**: `prod-013-production-composition-hardening`
**Baseline**: `develop` @ `e860fb6`
**Status**: complete — ready for `sdd-propose`

## Intent

A composition declared as production must never start on volatile storage
because durable persistence was not explicitly wired. Fail closed: an
unconfigured persistent capability rejects startup with an actionable error,
never silently degrades to in-memory.

## Why This Exists Now

PROD-012 (Durable Idempotency, archived) hardened one specific durable
capability — the operation reservation/receipt store — to fail closed when
unconfigured. That audit also surfaced that `EntityRuntimeBuilder::build()`
(`crates/persistent-entity/src/builder.rs:279-286`) does the opposite for two
sibling capabilities: it silently falls back to in-memory storage. A
composition that believes itself production-ready can lose every event and
snapshot on restart with no error anywhere. This spec closes that gap and
generalizes the "fail closed on missing durable persistence" rule PROD-012
already established for one capability, to the rest of the persistent
capabilities a production composition depends on.

## 1. Current State

### 1.1 The two silent fallbacks

```rust
// crates/persistent-entity/src/builder.rs:279-286
let event_store: Arc<dyn EventStore<E> + Send + Sync> = self
    .event_store
    .unwrap_or_else(|| Arc::new(InMemoryEventStore::new()));   // 279-281

let snapshot_store: Arc<Mutex<dyn Snapshot + Send>> = self
    .snapshot_store
    .unwrap_or_else(|| Arc::new(Mutex::new(InMemorySnapshotStore::new())));  // 283-286
```

Both are documented only by a one-line doc comment ("Defaults to in-memory."),
with no warning, no log line, no error path. `EntityRuntimeBuilder::build()`
is infallible (`-> EntityRuntime<E>`), unlike `RuntimeBuilder`, which already
has a fallible `try_build()` alongside its panicking `build()`.

### 1.2 The existing fail-closed precedent (PROD-012) is the template to reuse

`crates/service-sdk/src/runtime/builder.rs:735-771` — `IdempotencyEnforcementMode`:

- Enum with a strict default variant (`MandatoryKey`) and exactly one named
  escape hatch (`Compatibility`).
- One private validator (`validate_idempotency()`) is the single source of
  truth for the rule.
- `build()` calls the validator and panics on violation (existing infallible
  signature preserved — no breaking change to `build()`'s type).
- `try_build()` calls the same validator first, then delegates to `build()`,
  returning the error as a `Result` instead of panicking. The ordering is
  load-bearing: validation must run before delegating, or the panic unwinds
  before the `Result` path can return it.
- The error message names both fixes: the registration call to make, and the
  explicit opt-out, if one exists (tested by
  `the_refusal_names_the_registration_and_the_opt_out`).

### 1.3 No production/environment profile concept exists anywhere

`AppBuilder`/`App::builder()` (`crates/service-sdk/src/app/mod.rs`, `build()`
at line 791) is the CORE-028 composition root. Confirmed call chain (via
CodeGraph): `AppBuilder::build()` → `RuntimeBuilder::try_build()` →
`RuntimeBuilder::build()`. Grepped `app/mod.rs` and `RuntimeBuilder` for
`profile|Profile|Environment|environment` — zero matches. This spec
introduces a genuinely new composition-root concept; it does not reuse an
existing one.

`CompositionError` (`crates/service-sdk/src/app/error.rs`) already has a
`Validation(#[from] RuntimeError)` variant, so a new `RuntimeError` variant
one layer up would surface through `CompositionError` for free. But
`persistent-entity` (where the two silent fallbacks actually live) has **no
dependency on `service-sdk`**, so any new gate error type for
`EntityRuntimeBuilder` must be defined locally in `persistent-entity` and
cross the layer boundary the same way
`RuntimeError::OperationReservationStoreNotRegistered` already does one layer
up.

### 1.4 The effect store's failure mode is structurally different

`crates/service-sdk/src/runtime/builder.rs`, `.with_effect_store()`: the field
is a plain `Option<...>` with **no** `unwrap_or_else` fallback anywhere
(confirmed by grep — zero in-memory default construction for the effect
store). There is nothing to silently fall back to. The risk is not silent
volatility; it is a *deferred* failure — if a production deployment never
calls `.effect_store()`, nothing fails until the first attempted use, not at
bootstrap. Whether this needs the same gate, for symmetry with event/snapshot
store, is a design-phase decision, not a code fix (there is no fallback to
remove).

**Decided** (post-exploration, confirmed with the architect): yes, in scope,
same `Profile::Production` gate as event/snapshot store. One unconfigured-
persistence gate checking all three capabilities together is simpler than
three separate mechanisms, and the risk this closes — a production
deployment that forgot to wire it, discovered only at first use — is the
same class of failure this whole spec exists to move earlier.

### 1.5 Read-side/projection store: a different-shaped, larger, separate gap

`crates/infrastructure/src/persistence/in_memory/read_side_store.rs`
(`InMemoryReadSideStore`) exists. `crates/persistence/src/postgres/` has
`event_store.rs`, `snapshot.rs`, `reservation.rs`, and migrations for
`events`, `aggregates`, `snapshots`, `operation_reservations`,
`operation_receipts` — **no `read_side` / `checkpoint` / `projection` table
or Postgres module anywhere**. There is no durable implementation to gate
against. This is a missing capability, not a fallback bug.

There is a second, independent reason PROD-013 cannot touch this: unlike
event/snapshot/effect store, **read-side has no generic composition-root
registration slot at all today**. `AppBuilder::projection()` (CORE-028 Stage
2, `crates/service-sdk/src/app/mod.rs:362-391`) registers arbitrary
already-built projection *instances* for DI resolution — it has no concept
of a "read-side persistence backend" to inspect. The actual read-side wiring
(`SharedReadSideStore` / `ReadSideSink` / `.with_read_side_sink()`) is
entirely bespoke to `examples/reference-app` (`application.rs:218-220`,
`lib.rs:670`), not something `AppBuilder`/`RuntimeBuilder` knows about.
PROD-013's mechanism can only validate a slot that exists; it will not
fabricate one just to have something to gate.

**Confirmed out of scope for PROD-013, with an explicit forward constraint
on its successor** (confirmed with the architect): PROD-013 gates only what
the composition root can observe and validate today — event store, snapshot
store, effect store. It does not introduce any read-side gate, real or
pseudo, because doing so would mean inventing the very registration surface
this spec is supposed to be validating, not hardening one that exists.

The successor spec is renamed accordingly: **PROD-014 — Read-Side Persistence
Composition & Durable Store** (not merely "Durable Read-Side Projection
Store") — its scope is inseparably two things: (a) the durable contract,
checkpoint semantics, consistency model, and schema, and (b) introducing
read-side's first-ever generic registration point at the composition root.
PROD-013's proposal must carry this literal constraint forward as a
requirement on PROD-014, so PROD-014 is born already honoring the contract
PROD-013 establishes, not chasing it retroactively:

> **Read-side follow-up constraint**: PROD-014 must introduce a generic
> read-side/projection persistence registration at the composition root.
> From its introduction, Production must apply the same fail-closed policy
> PROD-013 established: capability not configured → valid; capability
> configured with a non-durable/in-memory backend → startup rejected;
> capability configured with a durable backend → valid.

Outbox/inbox: confirmed absent from the codebase entirely. Not applicable.

### 1.6 Boundary with PROD-005 (Health, Readiness and Startup)

PROD-005 signals the health of an application that has *already* started,
with degraded-mode semantics permitted for optional dependencies. PROD-013 is
about rejecting the composition/bootstrap step itself, before the app is
running at all. No overlap in mechanism, but the proposal/spec for PROD-013
should state this boundary explicitly so the two are never conflated.

### 1.7 Boundary with CORE-027

`xtask verify-layers` / `verify-isolation` / `verify-hygiene` lint
inter-crate dependency direction (architectural layering), not persistence
backend completeness. Confirmed this gap is not covered by anything existing.

### 1.8 Backends actually supported today

Only PostgreSQL. No trace of Oracle, MySQL, or SQLite in any crate. Any
mention of a second backend in prior discussion was illustrative/hypothetical,
not an existing partial implementation.

## 2. Blast Radius

**67 call sites of `EntityRuntimeBuilder::new()` across 25 real `.rs` files**
(excluding archived openspec docs). Of those, **at least 14 files / ~32 call
sites never call `.with_event_store()` or `.with_snapshot_store()` anywhere in
the file**, and therefore rely entirely on today's silent in-memory default:

- `crates/service-sdk/src/runtime/builder.rs` (6 — unrelated internal unit
  tests, e.g. entity-registry-conflict tests)
- `crates/service-sdk/src/app/mod.rs` (6)
- `crates/persistent-entity/tests/runtime_verification_suite.rs` (1)
- `crates/service-sdk/tests/effect_acceptor_entity_wiring.rs` (2)
- `crates/service-sdk/tests/effect_store_composition.rs` (1)
- `examples/reference-app/tests/pipeline.rs` (3), `effects_e2e.rs` (1),
  `support/mod.rs` (2), `ingress_trace_wiring.rs` (2), `providers_e2e.rs` (1)
- `integration-tests/tests/infrastructure/conflict_from_postgres.rs` (2 —
  despite the file name, these exercise unrelated registry-conflict behavior
  and never override the store), `effect_store_composition_postgres.rs` (1),
  `replay_from_postgres.rs` (2), `concurrent_replicas_postgres.rs` (2)

This is a materially larger and more diffuse blast radius than PROD-012's:
that migration only touched tests that had opted into (or defaulted into)
`MandatoryKey` idempotency specifically, a narrower surface.

Caveat: this list was derived by grepping for store-override calls
co-located in the *same file* as the `new()` call. A call site that
configures its store via a shared fixture in another file would not surface
here and needs re-checking during design/apply.

## 3. Approaches Considered

### A. Explicit `.profile(Profile::Production)` opt-in gate

New enum (`Profile::Dev` default / `Profile::Production`), builder method;
validation only runs when `Profile::Production` is set. Unset stays exactly
today's behavior.

- **Pros**: zero blast radius on all 67 existing call sites; explicit and
  discoverable at the call site; matches "in-memory stays valid for dev/test"
  exactly; composable independently on `EntityRuntimeBuilder` and
  `AppBuilder`.
- **Cons**: still relies on every production host remembering to set the
  flag — the same "someone forgot" failure class this spec exists to close,
  just moved one level up (from "forgot the store" to "forgot the profile").
  Needs a second layer (docs/CI/example enforcement) to be a genuine
  guarantee rather than a discoverable convention.
- **Effort**: Low.

### B. Infer strictness from partial configuration (no new enum)

`build()` stays permissive when nothing was configured (today's dev/test
case, unchanged), but fails closed the instant exactly one of
event/snapshot store was configured and the other was not.

- **Pros**: no new public API surface; catches the most realistic real
  mistake (partial wiring — event store configured, snapshot store
  forgotten); zero cost for the fully-unconfigured case.
- **Cons**: does **not** catch the fully-unconfigured production case —
  exactly the scenario this spec exists to close; inference from partial
  state is a weaker, less explicit contract than a named mode.
- **Effort**: Low–Medium.

### C. Flip the default to fail-closed, mirroring `IdempotencyEnforcementMode`

Same shape as PROD-012: strict by default, one named
`.allow_in_memory_persistence()`-style opt-out.

- **Pros**: strongest guarantee — production-safe by default with no extra
  call needed; reviewers already know this exact pattern from PROD-012.
- **Cons**: breaks ~14 files / ~32 call sites immediately unless every one is
  migrated to the opt-out first — a much larger, more invasive migration than
  PROD-012's own rollout. Requires introducing and rolling out an equivalent
  `compat()`-style test helper across `persistent-entity` and
  `service-sdk`/reference-app tests in the same change, inflating scope well
  beyond "add a gate."
- **Effort**: High.

## 4. Recommendation

Approach A, informed by Approach B as a free, zero-blast-radius addition:
even without an explicit `Profile::Production`, a **partial** configuration
(one store set, the other not) is unambiguous evidence of a mistake and can
fail closed unconditionally — no existing call site does this today
(confirmed: every call site either configures both stores or neither).
Approach C remains the strongest end-state contract but should only be
adopted by `sdd-design` against an explicit, sized migration plan for the
~32 call sites — not decided in this phase.

The effect store's already-100%-opt-in shape means it needs no
`unwrap_or_else` removal (there is nothing to remove), but it gains the same
`Profile::Production` gate for symmetry (**decided**, see §1.4) — the risk
profile differs (deferred failure vs. silent volatility) but the
actionable-error-at-bootstrap goal is identical, and one unconfigured-
persistence gate checking all three capabilities in one place is simpler
than three separate mechanisms.

## 5. Risks

- The blast-radius list (§2) may be incomplete for call sites using shared
  fixtures across files.
- `persistent-entity` has no dependency on `service-sdk`; any new gate error
  type must be defined locally in `persistent-entity`, not borrowed from
  `RuntimeError`/`CompositionError`.
- If a stricter default-flip (Approach C) is chosen later, the required
  `compat()`-style test-helper rollout across ~14 files is itself a
  nontrivial, separate unit of work that must be sized explicitly, never
  folded silently into "add the gate."

## 6. Explicitly Out of Scope

- Read-side/projection/checkpoint durable store, and any read-side gate at
  all, pseudo or real (§1.5) — no registration surface exists to validate
  today, and PROD-013 will not invent one just to have something to gate.
  Carried forward as an explicit, binding constraint on **PROD-014 —
  Read-Side Persistence Composition & Durable Store** (see the boxed
  constraint in §1.5), not a vague backlog note.
- Outbox/inbox — do not exist in the codebase.
- Observability, HA, migrations, and any other hardening theme the user
  explicitly asked not to mix into this atomic spec.
- Multi-backend (e.g. a second database engine) parity — only PostgreSQL
  exists today; the "backend support is all-or-nothing" principle is
  forward-looking and has nothing to validate against yet.

## Ready for Proposal

Yes. Final scope for `sdd-propose`: introduce `Profile::Production` as an
explicit opt-in gate (Approach A) covering exactly three capabilities that
have a real, generic composition-root slot today — event store, snapshot
store, effect store — informed by Approach B's zero-cost, profile-independent
partial-configuration check. Carry the read-side follow-up constraint (§1.5)
into the proposal verbatim, as a requirement on PROD-014, not as PROD-013
code. State the PROD-005 boundary explicitly.
