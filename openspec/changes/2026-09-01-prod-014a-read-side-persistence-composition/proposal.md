# Proposal: PROD-014A — Read-Side Durable Progress Composition

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers).

## Objective

A composition declared `Profile::Production` must not be able to start a read-side projection
whose durable progress state — its `OffsetStore` and `DedupStore`, the pair that decides
whether a projection can resume correctly after a restart — is volatile. Give read-side
progress its first composition-root registration point, classify it with PROD-013's existing
`is_durable()` mechanism, and refuse the bootstrap at `AppBuilder::build()` when Production is
declared and that pair is not durable.

## Intent

Read-side progress is today the **only** persistent capability in this workspace with no
composition-time visibility whatsoever. It is not "unguarded"; it is unobservable.

- `AppBuilder`, `RuntimeBuilder`, and `App` hold no reference to `OffsetStore`, `DedupStore`,
  or any read-side wiring at all. `RuntimeBuilder::validate_persistence_profile()`
  (`crates/service-sdk/src/runtime/builder.rs:777`) checks the event, snapshot, and effect
  stores and nothing else. There is no code path that could even observe the mismatch.
- `ReadSideHandles::new()` (`examples/reference-app/src/read_side/mod.rs:103-113`)
  unconditionally constructs `InMemoryOffsetStore` / `InMemoryDedupStore` with no parameter,
  no injection point, and no composition-visible decision for the host. This is the exact
  invisibility that must disappear from the Production path.
- A composition can therefore declare `Profile::Production`, pass every PROD-013 gate, and
  spawn a fully volatile read-side pipeline with no refusal, no warning, and no log line.
  On restart every projection resumes from `read_offset() -> Ok(None)` and replays the whole
  stream with no dedup memory.

PROD-013 closed this failure class for the event, snapshot, and effect stores and recorded a
binding constraint on its successor: *"PROD-014 must introduce a generic read-side/projection
persistence registration at the composition root. From its introduction, Production must apply
the same fail-closed policy PROD-013 established."* PROD-014A discharges exactly that
constraint and nothing more.

In-memory offset/dedup stores are not the problem and are not being removed. The problem is a
production composition receiving volatile resume state **by silence** rather than by
declaration.

## Active Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | The change is titled **PROD-014A — Read-Side Durable Progress Composition**, not "Read-Side Persistence Composition" as reserved in ROADMAP.md §7.13 / PROD-013 D-5. The change-folder slug and topic key stay `prod-014a-read-side-persistence-composition` (unchanged, so no artifact churn) | The exploration proved the capability actually in scope is the **durable progress state** — `OffsetStore` + `DedupStore`, the pair that lets a projection resume correctly — not the orphaned generic `ProjectionStateStore` SPI the original brief assumed (D-4). "Persistence Composition" invites governing whatever read-side type is nearest; "Durable Progress Composition" names the real thing being composed and governed, and makes OOS-8 (`ReadSideStore` itself) legible as a boundary rather than an omission. This mirrors PROD-015 D-1, which likewise recorded a deliberate naming departure from ROADMAP.md rather than silently renaming |
| D-2 | **Approach A2 (composition-root registration + gate at `build()`) is adopted; A1 (thread `Profile` into `ProjectionSpec::new` / `TagSchedulerImpl::spawn` and refuse at spawn) is rejected.** The architect's reasoning, recorded in full: (a) `ProjectionSpec` and `TagSchedulerImpl::spawn()` are read-side **execution** surfaces, not composition/deployment-safety surfaces — introducing `Profile` there would leak a deployment/composition-safety concern into the scheduler purely to minimize the diff; (b) under A1, `AppBuilder::build()` could succeed under `Profile::Production` and the read-side could only be discovered invalid later, at `spawn()` time, which would weaken the meaning PROD-013 already established for `Profile::Production` — an incorrect composition must never be allowed to start, so the rejection must happen at composition/bootstrap (`build()`), never deferred to `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, or the first batch; (c) A2 also fixes the defect the exploration surfaced — today `ReadSideHandles::new()` silently constructs `InMemoryOffsetStore`/`InMemoryDedupStore` with zero composition-visible decision for the host, and that invisibility is exactly what must disappear from the Production path; (d) `AppBuilder` is already the explicit application-facing composition root that delegates registrations to `RuntimeBuilder` — the exact precedent being `.effect_store()` → `RuntimeBuilder::with_effect_store()` — so A2 follows the existing shape rather than inventing one; (e) critically, **A2 does not mean `AppBuilder`/`ego-rs` constructs the stores** — CORE-026's non-goal remains fully intact (D-5) | A1 buys a smaller diff by relocating the guarantee to the wrong layer. A gate that fires after `build()` has already succeeded is a different, weaker contract than the one PROD-013 shipped, and the two would then disagree about what `Profile::Production` means |
| D-3 | **Registration is per-projection, keyed by `projection_id` — not a single global slot.** Investigated before deciding, against the real execution model: (1) `TagSchedulerImpl::spawn(self, spec)` (`crates/runtime/src/read_side/scheduler.rs:276-277`) consumes the scheduler by value, so one `TagSchedulerImpl` yields exactly one poll loop and N projections require N scheduler instances — nothing caps N; (2) `ProjectionSpec<F, H, S, D, O, R>` (`:175`) carries `dedup_store: D` and `offset_store: O` as **per-instance generic type parameters**, so two projections may legitimately use different concrete store *types*, which a single erased global slot could not represent; (3) both key spaces are already namespaced by projection: `OffsetKey = (projection_id, tag, tenant)` and `DedupKey = (projection_id, tag, event_id)` (`examples/reference-app/src/read_side/store.rs:163-167, 209-213`), so per-projection registration also *permits* one shared instance across projections without collision — it is strictly the more permissive shape, and costs nothing when N=1; (4) `AppBuilder` already has keyed-multiplicity registration precedent in `.projection()` and `.entity()` (dup-guarded per key), so this is not a novel builder shape; (5) reference-app runs exactly one projection today (`PROJECTION_ID = "users-by-tenant"`), so N=1 is the degenerate case of the chosen shape, not a special case needing its own design | The decisive argument is the gate's **subject**. `validate_persistence_profile` can skip the effect store when `effect_executors.is_empty()` because executor registration makes the capability's existence composition-visible. Read-side has no such signal today. A single global slot would give the gate one anonymous pair and still no answer to "does *this* projection run on durable progress"; keying on `projection_id` gives the gate a real subject per projection and makes "zero registered = no read-side = valid" a fact rather than an assumption. `.effect_store()`'s single-slot shape is therefore reused only as the **validation and fail-closed-duplicate mechanism**, never as the structural shape |
| D-4 | **`ProjectionStateStore` is excluded from this change entirely** and is documented as a disconnected/abandoned CORE-005 fragment | Exploration evidence: zero implementations and zero callers workspace-wide; its only plausible consumer `ReadSideProcessor` also has zero implementations; a workspace-wide OpenSpec grep for the literal string returns no hit outside the exploration itself; CORE-005's own spec, tasks.md, data-model.md and contracts/README.md define read-side state persistence purely as `OffsetStore` + `DedupStore`, never as a dedicated `ProjectionState` store. Governing it would harden a dead port and close no real production gap. Removing or relocating it is separate hygiene work (F-3), not this change |
| D-5 | **The CORE-026 delta is a boundary clarification, not a renegotiation.** Two orthogonal axes are stated explicitly in the delta: "the framework constructs or defaults read-side stores" — **still a non-goal, unaffected**; "the composition root accepts, classifies, and validates a host-constructed pair" — **new, in scope** | CORE-026's Non-Goals (`openspec/specs/read-side/spec.md:160-171`) reject a framework convenience that *internally constructs* dedup/offset stores, because handler and tag-discovery closures are irreducibly application-specific. Nothing in that reasoning addresses inspecting a durability property of a store the application already built and handed over. That non-goal also predates `Profile::Production` (PROD-013), so it never addressed this axis — it is not stale, it is orthogonal. The delta is still required, because the same Non-Goals text states this capability "wraps that engine's existing contract, it does not renegotiate any part of it", and a new composition-time refusal is a real (narrow, additive) change to observable behavior that must be specified rather than silently added |
| D-6 | A **test-only fake durable pair** (`is_durable() -> true`) is sufficient to demonstrate the Production accept path. No real durable backend is built | No durable `OffsetStore`, `DedupStore`, or `ReadSideStore` exists anywhere in the workspace (`crates/persistence/src/postgres/` has event store, snapshot, repository, reservation — nothing read-side). Building one is a separate implementation gap of its own size (F-1). Folding it in would break the Atomicity Gate, and the governance capability is testable and useful without it |
| D-7 | **`ReadSideStore` (the event source the projection polls) is not gated by this change** (OOS-8) | It is a read view of the event stream, not resume state. Its content is derived from the upstream event store, whose durability PROD-013 already governs. Gating it would require deciding what a durable read-side event view even is — a materially different question from "can this projection resume". Named as a boundary, with F-4 as the follow-up |
| D-8 | Durability is declared **only** via `is_durable()` on the two SPIs plus `require_durably_configured(...)`, reused verbatim with its existing signature | PROD-013's mechanism, unchanged. No `TypeId`, no downcasting, no type-name matching, no heuristic. `require_durably_configured`'s own doc comment already forbids computing its `durably_configured` argument from `.is_some()`; this change's call sites compute it from `is_durable()` on both stores |

## Atomicity Gate

**Run, and it cut scope twice.** A real Postgres durable read-side backend was considered and
removed (D-6 → F-1): it is an independently shippable implementation with its own schema,
migration, and conformance obligations, and this proposal's capability is testable without it.
Removing the abandoned `ProjectionStateStore` / `ReadSideProcessor` fragment was considered and
removed (D-4 → F-3): it is deletion hygiene with no dependency on this gate in either direction.

What remains is one indivisible capability, because no in-scope item is independently
shippable with value:

- IS-1 alone is an `is_durable()` nobody calls — dead code.
- IS-2 alone is a registration slot nothing validates — worse than absent, because it *looks*
  like governance.
- IS-3/IS-4/IS-5 cannot exist without both IS-1 (the fact) and IS-2 (the subject).
- IS-7 is what makes IS-2 non-decorative: unless the reference host's Production path obtains
  its pair from the composition, a host can register a durable pair and hand a volatile one to
  `ProjectionSpec`, and the gate would pass over a volatile projection.
- IS-8 is the only way to exercise the accept branch at all, given D-6.

Every item names the same mechanism (`is_durable()` + `require_durably_configured`), the same
error shape (`PersistenceCompositionError::NotConfigured { capability, fix }` surfaced through
`CompositionError::Validation`), and the same acceptance criterion.

**ATOMICITY: PASS**

## Scope

### In Scope

- **IS-1** — Add `is_durable(&self) -> bool { false }` as a default method on `OffsetStore`
  (`crates/domain/src/read_side/offset.rs`) and `DedupStore`
  (`crates/domain/src/read_side/dedup.rs`), mirroring the `EventStore` / `Snapshot` idiom
  PROD-013 established. Defaulting to `false` keeps every existing implementation compiling
  and honest.
- **IS-2** — A composition-root registration point for a projection's durable progress pair
  (`OffsetStore` + `DedupStore` together), keyed by `projection_id` (D-3). The pair is the
  unit: a registration that covers only one of the two MUST NOT be representable, so a partial
  configuration can never pass validation as if both were covered. The exact public surface —
  two methods, one `read_side_store(...)`/`read_side_persistence(...)`, a registration struct,
  or something else — is a `design.md` decision derived from these invariants, not fixed here.
- **IS-3** — Duplicate registration for the same `projection_id` fails closed at `build()` with
  a composition error naming the duplicate, never last-write-wins — following
  `AppBuilder::effect_store()`'s latched-`pending_error` shape and the
  "Duplicate Effect Store Registration Through AppBuilder Fails Closed" requirement.
- **IS-4** — Under `Profile::Production`, `AppBuilder::build()` (through
  `RuntimeBuilder::try_build()` → `validate_persistence_profile()` →
  `CompositionError::Validation`, the exact path PROD-013 already uses) refuses the bootstrap
  when a registered projection's `OffsetStore` or `DedupStore` is not durable. The error names
  the missing capability and the exact call that fixes it.
- **IS-5** — A `Profile::Production` composition with **no** read-side registered at all builds
  successfully. Command-only and non-read-side applications are never forced to register a
  dummy store, mirroring the effect store's "no executor registered, so nothing volatile to
  refuse" conditionality.
- **IS-6** — `Profile::Dev` is unchanged: volatile in-memory offset/dedup stores stay valid,
  explicit, and first-class. Every existing call site compiles and passes unmodified.
- **IS-7** — `examples/reference-app`'s Production composition path obtains its offset/dedup
  pair from the composition root instead of `ReadSideHandles::new()` constructing
  `InMemoryOffsetStore` / `InMemoryDedupStore` itself. Dev and test paths may keep constructing
  them explicitly. The mechanism that keeps the reference host from silently drifting back is a
  `design.md` decision; PROD-013's `EntityEventStores::open()` / `::in_memory()` structural
  coupling (IS-11/IS-12) is the reference precedent, not a mandate.
- **IS-8** — A test-only fake durable `OffsetStore` / `DedupStore` pair (`is_durable() -> true`)
  demonstrating the Production accept path (D-6).
- **IS-9** — Spec deltas: a boundary clarification plus the new composition requirement on
  `read-side` (D-5), the registration and fail-closed-duplicate requirement on
  `application-composition`, and the Production gate's extension on
  `production-composition-hardening`.
- **IS-10** — Correct `Profile::Production`'s doc comment
  (`crates/persistent-entity/src/profile.rs:17-23`), which currently states read-side
  "has no such slot yet and is deliberately not governed here". Once IS-2 lands that sentence
  is false, and it names the wrong successor scope.

### Out of Scope

- **OOS-1** — `ProjectionStateStore` and `ReadSideProcessor`. Untouched, in either direction
  (D-4). Their removal or relocation is F-3.
- **OOS-2** — Any real durable backend: `PostgreSQLOffsetStore`, `PostgreSQLDedupStore`, or a
  durable `ReadSideStore`. Reserved for **a future PROD-014B or equivalent postgres durable
  read-side store change** — the identifier is deliberately left open here rather than
  hard-committed (F-1). That change must exist; PROD-014A's gate is otherwise a refusal with
  nothing to satisfy it in-tree.
- **OOS-3** — Introducing `Profile` into `ProjectionSpec`, `TagSchedulerImpl`, `ReadSideSession`,
  or `ReadSideRunner` (D-2). No change to polling, dedup, offset, or ordering semantics.
- **OOS-4** — Multi-worker ownership, fencing, partition leasing, HA, brokers/Kafka,
  exactly-once delivery, and projection rebuild orchestration.
- **OOS-5** — Any change to CORE-007's or CORE-028's existing contracts beyond the additive
  registration surface itself.
- **OOS-6** — The framework constructing or defaulting read-side stores. CORE-026's non-goal,
  intact and unaffected (D-5).
- **OOS-7** — Governing a projection spawned entirely outside the composition root. A host may
  still call `ProjectionSpec::new(...)` and `spawn(...)` directly with its own stores; that path
  is ungoverned by construction (D-2 rejects the only mechanism that would close it). Residual
  risk R-1.
- **OOS-8** — `ReadSideStore` durability (D-7). Follow-up F-4.
- **OOS-9** — Removing, deprecating, or hiding `InMemoryOffsetStore` / `InMemoryDedupStore`.
  They remain valid and explicit for Dev and tests (IS-6).

## Capabilities

### New Capabilities

- None. This extends three existing capabilities rather than introducing a fourth surface for
  the same rule.

### Modified Capabilities

- `read-side`: the boundary clarification required by D-5 (framework construction remains out
  of scope; composition-root acceptance and validation of a host-constructed pair is new), plus
  the statement that a projection's durable progress pair may be composed at the composition
  root and refused there under Production.
- `application-composition`: registration of a projection's durable progress pair keyed by
  `projection_id`, and duplicate registration failing closed at `build()`.
- `production-composition-hardening`: the Production gate extends to read-side durable progress
  as a fourth governed capability, using the same validator and the same error shape.

If the spec phase finds an existing requirement already implies one of these, it folds rather
than manufacturing a delta.

## Approach

Reuse PROD-013's proven shape end to end rather than inventing a second mechanism: the two SPIs
declare durability with `is_durable()`, the composition root holds the host-constructed pair,
and one validator calls the existing `require_durably_configured(profile, durably_configured,
capability, fix)` with `durably_configured` computed from both stores' `is_durable()` — never
from `.is_some()`, which that function's own doc comment explicitly forbids.

The refusal travels a path that already exists and needs no new plumbing:
`AppBuilder::build()` → `RuntimeBuilder::try_build()` (`builder.rs:1146`) →
`validate_persistence_profile()` (`:777`) → `RuntimeError` →
`CompositionError::Validation(#[from] RuntimeError)`.

The gate's conditionality mirrors the effect store's exactly. `validate_persistence_profile`
already returns `Ok(())` early when `effect_executors.is_empty()`, because with no executor
registered no effect store is constructed and there is nothing volatile to refuse. Read-side's
equivalent is "no projection registered": IS-5 falls out of the same reasoning, not a special
case.

What is genuinely new is only the subject of the check. Registration is keyed by
`projection_id` (D-3), which is not a new concept — it is already the identity `ProjectionSpec`
carries and already the leading component of both the offset and dedup key tuples. The
framework registers, classifies, and validates; it never constructs (D-5).

## Required Semantics

```
Given a composition declaring Profile::Production
When no read-side projection progress is registered at all
Then build() succeeds — a command-only or non-read-side application is never
     forced to register a dummy store.

Given a composition declaring Profile::Production
When a projection's OffsetStore and DedupStore are both registered and both durable
Then build() succeeds.

Given a composition declaring Profile::Production
When either store of a registered projection's pair is volatile
Then build() is refused at composition/bootstrap time — never deferred to
     ProjectionSpec::new(), TagSchedulerImpl::spawn(), or the first batch — with an
     error naming the missing/non-durable capability and the exact call that fixes it.

Given a composition declaring Profile::Dev (the default)
When in-memory/volatile offset and dedup stores are used
Then behavior is unchanged, byte-for-byte with today.

Given a projection's progress pair registered twice for the same projection_id
When build() is called
Then construction fails with a composition error identifying the duplicate, and the
     first registration is what would have resolved had construction succeeded.
```

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/domain/src/read_side/offset.rs`, `dedup.rs` | Modified | `is_durable(&self) -> bool { false }` default method on each SPI (IS-1) |
| `crates/service-sdk/src/app/mod.rs` (`AppBuilder`, `build()` at :811) | Modified | Registration surface keyed by `projection_id`, dup-guarded via the existing `pending_error` latch (IS-2, IS-3) |
| `crates/service-sdk/src/runtime/builder.rs` (`validate_persistence_profile` at :777, `try_build` at :1146) | Modified | Read-side branch added to the one existing validator — never a second, parallel check (IS-4, IS-5) |
| `crates/persistent-entity/src/profile.rs` | Modified (doc only) | `require_durably_configured` reused verbatim, signature unchanged; only `Profile::Production`'s stale doc comment is corrected (IS-10, D-8) |
| `examples/reference-app/src/read_side/mod.rs` (`ReadSideHandles::new` at :103) | Modified | Production path no longer hardcodes `InMemoryOffsetStore` / `InMemoryDedupStore` (IS-7) |
| `examples/reference-app/src/read_side/store.rs` | Modified | Test-only fake durable pair added; in-memory pair kept for Dev/tests (IS-8, OOS-9) |
| `crates/runtime/src/read_side/scheduler.rs` (`ProjectionSpec` :175, `spawn` :276) | Untouched | No `Profile`, no signature change (OOS-3) |
| `crates/domain/src/read_side/{session,runner}.rs` | Untouched | No polling/dedup/offset semantic change (OOS-3) |
| `crates/domain/src/read_side/projection_state_store.rs`, `ReadSideProcessor` | Untouched | OOS-1 / D-4 |
| `crates/persistence/src/postgres/` | Untouched | No durable read-side backend built (OOS-2) |
| `openspec/specs/{read-side,application-composition,production-composition-hardening}/spec.md` | Modified | Deltas per IS-9 |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| R-1 | The direct `ProjectionSpec::new` + `spawn` path stays ungoverned: a host that never registers can still run a volatile projection under Production (OOS-7) | High | Accepted by design (D-2 rejects the only mechanism that closes it). IS-7 makes the reference host's Production path go through the composition, so the reference stays a live example rather than a counter-example. This is the same residual class as PROD-013 R-1 ("the profile must be remembered"), not a new one |
| R-2 | The registration could become decorative: a host registers a durable pair at the composition root and hands a *different*, volatile pair to `ProjectionSpec` | Med | IS-2's invariant is that the registration is the projection's progress pair, not a parallel declaration about it. `design.md` MUST state how the registered pair reaches `ProjectionSpec` — IS-7's reference rewiring is the proof that it does |
| R-3 | No durable read-side backend exists in-tree, so a Production host adopting this must supply its own implementation or be refused | High | Named explicitly (OOS-2, F-1). A refusal is strictly better than today's silent volatility, and F-1 is the named successor. `design.md` should not soften the gate to compensate |
| R-4 | Per-projection registration designs multiplicity against a single-projection reality (reference-app runs one) | Med | D-3's evidence: the key is `projection_id`, which already exists in both store key tuples and in `ProjectionSpec`; no new concept is invented, and N=1 is the degenerate case. A global slot would be the riskier choice — it structurally forbids what `ProjectionSpec`'s per-instance `D`/`O` already allows |
| R-5 | The `read-side` spec delta reads as reversing CORE-026's non-goal | Med | D-5 fixes the wording on two named axes. `sdd-spec` MUST state both — "framework constructs/defaults" unaffected, "composition root accepts and validates" new — not one |
| R-6 | Review budget: reference-app rewiring plus three spec deltas plus the gate plausibly exceeds 400 changed lines | Med | `sdd-tasks` forecasts it. Natural first slice: IS-1 + IS-2 + IS-3 + IS-4 + IS-5 + IS-8 (framework + its tests) with IS-7 + IS-9 + IS-10 as the second |
| R-7 | `AppBuilder` gaining its first read-side awareness invites scope creep toward `App` owning the read-side lifecycle | Med | OOS-3 / OOS-7: the composition root registers and validates; it never spawns, owns, or stops the poll loop. `ReadSideHandles::spawn()` stays outside `App` |
| R-8 | PROD-013 is still unarchived on `develop`, so this change builds on an in-flight predecessor | Low | Verified non-blocking during exploration: `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` both exit 0 on PROD-013's verify report; its unchecked boxes are administrative. `require_durably_configured` is on `develop` and reused unmodified |

## Named Follow-Ups (deliberately not folded in)

- **F-1** — A durable Postgres read-side progress store (`PostgreSQLOffsetStore` +
  `PostgreSQLDedupStore`), with schema, migration, and conformance coverage. Reserved as a
  future PROD-014B **or equivalent** — the identifier is intentionally not hard-committed here.
  CORE-005's own archived plan already listed these files; they were never built.
- **F-2** — A durable `ReadSideStore` (the event view the projection polls), if a durable
  read-side event view is ever wanted (D-7).
- **F-3** — Removing or relocating the abandoned `ProjectionStateStore` / `ReadSideProcessor`
  fragment (D-4).
- **F-4** — Closing R-1 structurally, if it is ever judged worth the layering cost the
  architect rejected in D-2.

## Rollback Plan

Additive and default-inert. `Profile::Dev` is the default and reproduces today's behavior
exactly, so reverting is: remove the two `is_durable()` default methods, the registration
surface and its dup-guard, the read-side branch inside `validate_persistence_profile()`, the
test-only fake durable pair, and restore `ReadSideHandles::new()`'s construction. No schema, no
migration, no data, no persistence format, and no runtime storage behavior is touched — this
change writes validation and registration only. The spec deltas revert in one commit. Every
existing call site is untouched by both the change and the revert.

## Dependencies

- PROD-013 (`Profile`, `require_durably_configured`, `PersistenceCompositionError::NotConfigured`,
  `CompositionError::Validation`) — reused verbatim, not modified.
- CORE-026's `ProjectionSpec` / `TagSchedulerImpl::spawn` surface — consumed unchanged.
- `examples/reference-app`'s existing read-side wiring — rewired, not rebuilt.
- No new external dependency, crate, service, backend, or infrastructure.

## Success Criteria

- [ ] **SC-1** — `Profile::Production` with no read-side registered builds successfully.
- [ ] **SC-2** — `Profile::Production` with a registered pair whose `OffsetStore` and
      `DedupStore` are both durable builds successfully.
- [ ] **SC-3** — `Profile::Production` with either store of a registered pair volatile is
      refused at `AppBuilder::build()`, and the refusal is observable there — not at
      `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, or the first batch.
- [ ] **SC-4** — That refusal names both the missing/non-durable capability and the exact call
      that fixes it, in the shape of `PersistenceCompositionError::NotConfigured { capability, fix }`.
- [ ] **SC-5** — `Profile::Dev` with volatile stores is unchanged; `cargo test --workspace`
      shows zero new failures and no existing call site required modification.
- [ ] **SC-6** — Registering the same `projection_id` twice fails closed at `build()`, and the
      first registration is what would have resolved.
- [ ] **SC-7** — Registering only one store of the pair is not representable through the public
      surface — a partial configuration cannot pass validation as if both were covered.
- [ ] **SC-8** — Durability is determined solely by `is_durable()` fed into
      `require_durably_configured`. No `TypeId`, downcast, type-name match, or other heuristic
      appears anywhere in the change, and `require_durably_configured`'s signature is unmodified.
- [ ] **SC-9** — `ProjectionSpec`, `TagSchedulerImpl`, `ReadSideSession`, and `ReadSideRunner`
      contain no reference to `Profile`, and their polling/dedup/offset/ordering semantics are
      unchanged.
- [ ] **SC-10** — `ReadSideHandles::new()` no longer constructs `InMemoryOffsetStore` /
      `InMemoryDedupStore` on the Production composition path, and the reference host's
      Production read-side pair originates at the composition root.
- [ ] **SC-11** — The `read-side` delta states both axes explicitly: framework construction of
      stores remains out of scope and unaffected; composition-root acceptance and validation of
      a host-constructed pair is new. It does not read as reversing CORE-026.
- [ ] **SC-12** — `ProjectionStateStore` and `ReadSideProcessor` appear nowhere in the delivered
      change, and no real durable read-side backend is built.
- [ ] **SC-13** — `Profile::Production`'s doc comment no longer claims read-side has no
      composition-root slot, and names the correct successor scope.
