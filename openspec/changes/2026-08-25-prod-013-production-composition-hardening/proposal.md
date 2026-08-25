# Proposal: PROD-013 — Production Composition Hardening

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers).

## Objective

A composition declared as production must never start on volatile storage because a
durable persistent capability was not explicitly wired. Introduce `Profile::Production`
as an explicit opt-in gate that rejects the bootstrap — with an actionable error — when
any of the three composition-root-observable persistent capabilities (event store,
snapshot store, effect store) lacks an explicitly configured durable implementation.

## Intent

`EntityRuntimeBuilder::build()` (`crates/persistent-entity/src/builder.rs:279-286`)
silently substitutes `InMemoryEventStore` and `InMemorySnapshotStore` when neither store
was configured. There is no warning, no log line, and no error path — only a one-line
doc comment ("Defaults to in-memory."). A deployment that believes itself production-ready
therefore loses every event and every snapshot on restart, and learns about it only from
missing data.

The effect store fails the same way, not differently: `RuntimeBuilder::with_effect_store()`
(`crates/service-sdk/src/runtime/builder.rs:501`) also has a silent in-memory fallback —
`builder.rs:811` constructs `InMemoryEffectStore` whenever at least one effect executor is
registered and no store was explicitly configured. A production deployment that forgot to
wire it does not fail deferred at first use; it silently runs on volatile storage from the
start, exactly like the event and snapshot stores.

PROD-012 (Durable Idempotency, archived) already established the fail-closed rule for one
persistent capability, the operation reservation/receipt store: unconfigured means refused,
and the refusal names both the registration call and the opt-out. PROD-013 generalizes that
established rule to the remaining persistent capabilities a production composition depends
on, and does so *now* because PROD-012's own audit is what surfaced the sibling gap.

In-memory storage is not the problem and is not being removed. The problem is in-memory
storage arriving as production infrastructure *by default*, chosen by omission rather than
by declaration.

## Active Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | The mechanism is an explicit opt-in **`Profile`** enum with two variants: `Profile::Dev` (default) and `Profile::Production`. `Profile::Dev` preserves today's behavior byte-for-byte. `Profile::Production` requires explicit durable configuration for the capabilities in scope | Zero blast radius across all 67 existing `EntityRuntimeBuilder::new()` call sites; explicit and discoverable at the composition site; keeps in-memory legitimately valid for dev/test, which is what it is for |
| D-2 | **Revised** — the "partial configuration" check is not a separate mechanism and does not run in every profile. Under `Profile::Production`, the missing-capability rule (D-1) already refuses any composition missing `event_store` OR `snapshot_store` — this subsumes the partial case with no additional rule needed. Under `Profile::Dev`, nothing changes | The original premise ("zero call sites configure this today") was reverted after verifying against real code: 15 call sites across 8 files configure exactly one store, including reference-app's own production composition root (`lib.rs:502`) — an unconditional check would have broken it and contradicted IS-8/SC-7. Confirmed by the architect; see `design.md` AD-7 |
| D-3 | The gated capability set is **exactly three**: event store and snapshot store (`EntityRuntimeBuilder::build()`, `crates/persistent-entity/src/builder.rs:279-286`) and effect store (`RuntimeBuilder::with_effect_store()`, `crates/service-sdk/src/runtime/builder.rs:501`; `AppBuilder::effect_store()`, `crates/service-sdk/src/app/mod.rs:562`). One validation mechanism covers all three — not three separate mechanisms | These are the only persistent capabilities with a real, generic, composition-root-observable registration slot today. The effect store fails the same way as the other two, not differently (see EC-2 correction above) — one gate covering all three is simpler than three separate mechanisms, and now more accurate than the original framing, not less |
| D-4 | **Read-side/projection persistence is out of scope entirely — no gate, real or pseudo.** No generic read-side registration exists at the composition root today: `AppBuilder::projection()` (`crates/service-sdk/src/app/mod.rs:362-391`) is DI for arbitrary already-built projection instances, not a persistence backend slot, and the real read-side wiring (`SharedReadSideStore` / `ReadSideSink` / `.with_read_side_sink()`) is entirely bespoke to `examples/reference-app` (explore §1.5) | PROD-013 hardens registration surfaces that exist; it will not invent the very surface it is supposed to be validating just to have something to gate. Carried forward as a binding constraint on PROD-014 (see below) |
| D-5 | The successor spec is named **PROD-014 — Read-Side Persistence Composition & Durable Store**, not merely "Durable Read-Side Projection Store" | Its scope is inseparably two things: the durable contract (checkpoint semantics, consistency model, schema) AND read-side's first-ever generic composition-root registration point. A name covering only the store would license shipping half of it |
| D-6 | In-memory implementations are **not** removed, deprecated, or hidden. They remain valid, explicit, and first-class for `Profile::Dev` and tests | The defect is silent default selection, not existence. Removing them would break the dev/test path this proposal explicitly protects |
| D-7 | Approach C (flip the default to fail-closed with a named opt-out, mirroring `IdempotencyEnforcementMode`) is **evaluated and deferred, not rejected on merit** | It is the strongest end-state contract, but it breaks ~14 files / ~32 call sites immediately and requires rolling out a `compat()`-style test helper across `persistent-entity`, `service-sdk`, and reference-app tests in the same change. That migration must be sized explicitly in `design.md`, never folded silently into "add a gate" |
| D-8 | PROD-013 includes a **second enforcement layer**, not just the flag and the error: `examples/reference-app`'s production composition (concretely, via `EntityEventStores` — see IS-11) must explicitly declare `Profile::Production`, and a check (an `xtask` lint or a test, decided in `design.md`) must fail if it ever stops declaring it | The flag alone is a discoverable convention, not a guarantee — a production host can still forget it. Confirmed with the architect: this slice is worth the modest extra surface (one composition call plus one regression check) rather than deferring the only real verification that the mechanism is actually used |

## Read-Side Follow-Up Constraint (binding on PROD-014)

This constraint is inherited verbatim from exploration §1.5 and is a requirement on the
successor spec, not on PROD-013's implementation:

> **Read-side follow-up constraint**: PROD-014 must introduce a generic
> read-side/projection persistence registration at the composition root. From its
> introduction, Production must apply the same fail-closed policy PROD-013 established:
> capability not configured → valid; capability configured with a non-durable/in-memory
> backend → startup rejected; capability configured with a durable backend → valid.

The point of stating it here, rather than as a backlog note, is that PROD-014 must be born
already honoring the contract PROD-013 establishes — not retrofitted into it afterwards.

## Architecture Principle: Persistence Completeness Rule

PROD-013 documents a principle, not only this instance of it:

> **Persistence completeness rule** — a database is not considered supported by `ego-rs`
> until it implements EVERY persistent capability that a production composition declares it
> uses. Missing capabilities may not be completed by falling back to in-memory storage.
> Backend support is all-or-nothing across the durable capabilities a composition enables.

This is **forward-looking**. PostgreSQL is the only backend that exists today
(`crates/persistence/src/postgres/`), and it is not in violation. The rule exists so the
first partially-implemented second backend is refused as a backend, rather than shipped as
a production composition quietly completed by in-memory parts.

## Boundaries With Adjacent Specs

| Adjacent | Its concern | PROD-013's concern | Overlap |
|----------|-------------|--------------------|---------|
| **PROD-005** (Health, Readiness and Startup) | Signalling the health of an application that has **already started**, with degraded mode permitted for optional dependencies | **Rejecting the bootstrap itself**, before anything starts | None. Stated explicitly so the two are never conflated: PROD-013 decides whether the app may start at all; PROD-005 describes an app that already did |
| **PROD-012** (Durable Idempotency, archived) | The same fail-closed rule for one capability: the operation reservation/receipt store | The remaining persistent capabilities, using PROD-012's validator/error shape as the template | Complementary, not overlapping. PROD-013 does not re-open or modify PROD-012's rule |
| **CORE-027** (`xtask verify-layers` / `verify-isolation` / `verify-hygiene`) | Inter-crate dependency direction (architectural layering) | Persistence backend completeness at composition time | None. Confirmed this gap is covered by nothing existing |

## Atomicity Gate

**Already run, and it changed the scope — not a pending step.** The gate identified read-side
persistence as a capability that would have broken atomicity if absorbed: it is not a
fallback bug to harden but a missing capability plus a missing registration surface, and
including it would have forced this spec to *invent* the composition-root slot it claims to
validate. Absorbing it was considered and explicitly rejected; it was separated into
PROD-014 with the binding constraint above. The remaining three capabilities share one
mechanism, one error shape, and one acceptance criterion, so the spec is atomic as scoped.

## Scope

### In Scope

- **IS-1** — Introduce the `Profile` enum (`Profile::Dev` default, `Profile::Production`) and
  the builder method that sets it, at the composition root (D-1).
- **IS-2** — Under `Profile::Production`, reject the bootstrap when the **event store** has no
  explicitly configured durable implementation, with an error naming the capability and
  `EntityRuntimeBuilder::with_event_store()`.
- **IS-3** — Under `Profile::Production`, reject the bootstrap when the **snapshot store** has no
  explicitly configured durable implementation, with an error naming the capability and
  `EntityRuntimeBuilder::with_snapshot_store()`.
- **IS-4** — Under `Profile::Production`, reject the bootstrap when the **effect store** has no
  explicitly configured durable implementation, with an error naming the capability and the
  configuration call for the composition surface in use (`RuntimeBuilder::with_effect_store()`
  or `AppBuilder::effect_store()`).
- **IS-5 (revised)** — Removed as a standalone rule. The "exactly one configured" case is
  covered by IS-2/IS-3 under `Profile::Production` — the gate already refuses any missing
  store, partial or total. See AD-7 in `design.md`.
- **IS-6** — One validator as the single source of truth for the rule across all three
  capabilities, following PROD-012's `validate_idempotency()` shape
  (`crates/service-sdk/src/runtime/builder.rs:735-771`).
- **IS-7** — Errors are actionable: each names the missing capability AND the exact
  configuration call that fixes it, in the style asserted by PROD-012's
  `the_refusal_names_the_registration_and_the_opt_out`.
- **IS-8** — Preserve today's behavior unchanged for every composition that does not set
  `Profile::Production` — all 67 existing `EntityRuntimeBuilder::new()` call sites keep compiling
  and passing without modification.
- **IS-9** — Document the **persistence completeness rule** as an architecture principle, as
  forward-looking guidance rather than a report of a current violation.
- **IS-10** — Document the PROD-005 boundary explicitly (bootstrap rejection vs. post-start
  health), so the two specs are never conflated.
- **IS-11 (revised)** — The profile travels on the `EntityEventStores` type, not as a
  parameter to `build_runtime_with` (which is shared between dev and production).
  `EntityEventStores::open(pool)` produces `Profile::Production`;
  `EntityEventStores::in_memory()` produces `Profile::Dev`. The profile field is private —
  these two constructors are the only way to set it, so a durable store and a production
  declaration cannot drift apart. This closes R-1 structurally, stronger than the original
  "one call plus one regression check" guarantee. See `design.md` AD-8.
- **IS-12** — A check (an `xtask` lint or a test — the exact mechanism is a `design.md`
  decision) fails the build if reference-app's production composition (via
  `EntityEventStores`, consumed by `build_runtime_with`) ever stops declaring
  `Profile::Production`, so this reference stays a live regression guard, not a one-time
  example (D-8).
- **IS-13 (new, per AD-9)** — `EntityEventStores::open(pool)` also wires two real
  `PostgreSQLSnapshotStore` instances (one per aggregate: organization and user), replacing
  the in-memory snapshot store that reference-app's production path silently uses today.
  This is wiring, not building a new backend — `PostgreSQLSnapshotStore` already exists
  (`crates/persistence/src/postgres/snapshot.rs`) — and it closes a real durability defect
  that PROD-013's own gate exposes in its reference host: without it, the gate would refuse
  reference-app's own production composition for a missing snapshot store. Known risk for
  tasks/apply: `PostgreSQLSnapshotStore::save_snapshot` uses `tokio::task::block_in_place`,
  which panics on a single-threaded Tokio runtime — any Postgres integration test able to
  trigger a real snapshot (over 100 events, `PeriodicSnapshotStrategy`'s threshold) must use
  `#[tokio::test(flavor = "multi_thread")]`.

### Out of Scope

- **OOS-1** — Building any new Postgres backend. Durable event store, snapshot store
  (`crates/persistence/src/postgres/{event_store,snapshot}.rs`) and effect store (via PROD-002)
  already exist. PROD-013 validates configuration, it does not implement storage. OOS-1 still
  excludes building a new backend — wiring `PostgreSQLSnapshotStore` (already existing) into
  reference-app's production path (IS-13) is wiring, not implementation, and is therefore
  consistent with OOS-1, not an exception to it.
- **OOS-2** — Read-side / projection / checkpoint persistence, and any read-side gate at all,
  real or pseudo (D-4). Deferred to PROD-014 with the binding constraint above.
- **OOS-3** — Observability, HA, migrations, and every other production-hardening theme. They
  are not mixed into this atomic spec.
- **OOS-4** — Support for a second database engine (Oracle, MySQL, SQLite, or otherwise). None
  exists today; the persistence completeness rule is forward-looking and has nothing to validate
  against yet.
- **OOS-5** — Deciding Approach C (default flip with a named opt-out) and its ~32-call-site
  migration. Recorded in `design.md` as an evaluated alternative, deferred here (D-7), not
  resolved in this proposal.
- **OOS-6** — Removing, deprecating, or hiding the in-memory implementations (D-6).
- **OOS-7** — Re-opening PROD-012's idempotency rule or its enforcement mode.
- **OOS-8** — Outbox/inbox patterns. Confirmed absent from the codebase entirely; not applicable.

## Capabilities

### New Capabilities

- `production-composition-hardening`: profile-gated fail-closed validation of durable
  persistent capabilities at the composition root, plus the persistence completeness rule.

### Modified Capabilities

- `persistent-entity`: `EntityRuntimeBuilder::build()` no longer silently substitutes
  in-memory event/snapshot stores under `Profile::Production` — the same missing-capability
  rule refuses a missing store whether it is the only one missing or both are (AD-7). No
  separate partial-configuration check exists, and `Profile::Dev` is unaffected.
- `application-composition`: the composition root gains an explicit profile declaration, and
  `AppBuilder`/`RuntimeBuilder` surface the gate's refusal through the existing
  composition-error path.

## Approach

Reuse PROD-012's proven shape rather than inventing a second mechanism: one enum with a
default that changes nothing, one private validator as the single source of truth for the
rule, and one error whose message names both the missing capability and the call that fixes
it. The validator checks all three capabilities in one place, so there is one rule to
understand, one error shape to test, and one thing to extend when PROD-014 adds read-side.

Two structural constraints shape where the pieces live. First, `persistent-entity` has **no
dependency on `service-sdk`** (explore §1.3), while `service-sdk` does depend on
`persistent-entity` — so a single shared `Profile` type must be declared in the lower crate
and re-exported upward, and the event/snapshot gate error must be defined locally in
`persistent-entity`, crossing the layer boundary exactly as
`RuntimeError::OperationReservationStoreNotRegistered` already does one layer up. Second,
`EntityRuntimeBuilder::build()` is infallible (`-> EntityRuntime<E>`) and has no `try_build()`
sibling, unlike `RuntimeBuilder` — so how the refusal surfaces there (a fallible sibling, or
PROD-012's validate-then-panic ordering) is a `design.md` decision, deliberately not
pre-committed here. `CompositionError` already carries `Validation(#[from] RuntimeError)`, so
a new `RuntimeError` variant surfaces through `AppBuilder` for free.

The partial-configuration case (D-2, revised) is not a separate check: under
`Profile::Production` it is already caught by the missing-capability rule (whichever store
is missing, missing is missing), and under `Profile::Dev` it remains valid, unchanged. A
profile-independent check was considered and reverted once real call sites showed it was not
free — see `design.md` AD-7.

## Acceptance Criterion

```
Given a composition configured with Profile::Production
When any of {event_store, snapshot_store, effect_store} has no explicitly
     configured durable implementation
Then the bootstrap MUST be rejected with an actionable error naming the missing
     capability and its corresponding configuration call
     (EntityRuntimeBuilder::with_event_store,
      EntityRuntimeBuilder::with_snapshot_store,
      RuntimeBuilder::with_effect_store / AppBuilder::effect_store)
And it MUST NEVER silently degrade to in-memory
     (InMemoryEventStore / InMemorySnapshotStore).

Given a composition WITHOUT Profile::Production (dev/test, the default)
When no capability is configured
Then today's behavior (in-memory) is preserved unchanged, and
     EntityRuntimeBuilder::build() still succeeds.

Given a composition configured with Profile::Production
When event_store OR snapshot_store is configured but not both
Then the bootstrap MUST be rejected (subsumed by the same rule as the fully-missing case, not
     a separate mechanism).

Given a composition WITHOUT Profile::Production (dev/test)
When event_store OR snapshot_store is configured but not both
Then today's behavior is preserved — partial configuration remains valid, unchanged.
```

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistent-entity/src/builder.rs:279-286` | Modified | The two `unwrap_or_else` in-memory fallbacks become profile-gated under `Profile::Production`; the same missing-capability rule subsumes partial configuration, no separate check (AD-7) |
| `crates/persistent-entity` (Profile type + local gate error) | New | `Profile` declared in the lower crate (no `service-sdk` dependency available) plus a local error type for the event/snapshot refusal |
| `crates/service-sdk/src/runtime/builder.rs` (`with_effect_store` at :501, validator pattern at :735-771) | Modified | Effect store gains the same gate; PROD-012's validator shape is the template |
| `crates/service-sdk/src/app/mod.rs` (`effect_store` at :562, `build()` at :791) | Modified | Profile declaration at the `AppBuilder` composition root; refusal surfaces through `CompositionError::Validation` |
| `crates/persistence/src/postgres/` | Unchanged | Durable implementations already exist; nothing is built here (OOS-1) |
| `crates/infrastructure/src/persistence/in_memory/` | Unchanged | In-memory implementations stay valid and explicit for `Profile::Dev` (D-6) |
| ~14 files / ~32 call sites relying on the silent default | Unchanged | They never set `Profile::Production`, so `Profile::Dev` preserves their behavior (IS-8) |
| `AppBuilder::projection()` (`app/mod.rs:362-391`), `SharedReadSideStore` / `ReadSideSink`, `examples/reference-app` read-side wiring | Untouched | No read-side gate at all (D-4, OOS-2) |
| `examples/reference-app/src/lib.rs` (`build_runtime_with` at :567, `build_runtime_in_memory` at :311, `build_runtime_observed_in_memory` at :522, `EntityEventStores`) | Modified | `EntityEventStores::open` carries `Profile::Production` and now also wires two `PostgreSQLSnapshotStore` instances (org + user); `EntityEventStores::in_memory()` stays `Profile::Dev` (D-8, IS-11, IS-13) |
| New regression check (`xtask` lint or test, mechanism decided in `design.md`) | New | Fails the build if reference-app's production composition (via `EntityEventStores`) stops declaring `Profile::Production` (D-8, IS-12) |
| Postgres integration tests able to trigger a real snapshot (e.g. `durable_entity_progress_postgres.rs` and siblings) | Risk | `PostgreSQLSnapshotStore::save_snapshot`'s `tokio::task::block_in_place` panics on a single-threaded runtime; any such test must use `#[tokio::test(flavor = "multi_thread")]` (IS-13) |
| Architecture documentation | Modified | Persistence completeness rule (IS-9) and the PROD-005 boundary (IS-10) |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| R-1 | `Profile::Production` must be *remembered*. A production host that forgets the flag gets exactly the failure class this spec exists to close, moved one level up (from "forgot the store" to "forgot the profile") | High | **Resolved (D-8).** `examples/reference-app`'s production composition declares `Profile::Production` explicitly, and a check fails the build if that declaration ever regresses — the reference stays a live guard, not a one-time example. This does not protect a *different* host that never looks at reference-app, which remains an accepted residual risk of any opt-in convention |
| R-2 | The blast-radius inventory (explore §2: 67 call sites / 25 files; ~32 sites / 14 files on the silent default) was derived by grepping for store-override calls **co-located in the same file**. A call site configuring its store through a shared fixture in another file would not appear | Med | Re-verify during design/apply before relying on the count. `Profile::Dev` being the default makes an incomplete inventory non-breaking rather than a regression |
| R-3 | `EntityRuntimeBuilder::build()` is infallible with no `try_build()` sibling, so the refusal has no existing `Result` path to travel | Med | Explicit `design.md` decision. PROD-012's validate-before-delegate ordering is the reference; the ordering is load-bearing (validation must run before delegating, or the panic unwinds before the `Result` path can return it) |
| R-4 | Layer boundary: `persistent-entity` cannot borrow `RuntimeError`/`CompositionError`, so a second error type is introduced in the lower crate | Low | Precedent exists — `RuntimeError::OperationReservationStoreNotRegistered` crosses the same boundary one layer up. `CompositionError::Validation(#[from] RuntimeError)` already forwards |
| R-5 | Scope creep: the effect store's inclusion invites "harden every capability", or the persistence completeness rule invites second-backend work | Med | D-3 fixes the set at exactly three; OOS-2/OOS-4 close read-side and multi-backend. The completeness rule ships as documented principle only, with nothing to validate against today |
| R-6 | The persistence completeness rule is read as a report of a current violation and triggers unnecessary Postgres work | Low | IS-9 states it as forward-looking. PostgreSQL is the only backend today and is not in violation |
| R-7 | If Approach C is adopted later, the required `compat()`-style test-helper rollout across ~14 files is itself nontrivial | Med | D-7/OOS-5: sized explicitly as its own unit of work in `design.md`, never folded silently into "add the gate" |

## Rollback Plan

The change is additive and default-inert. `Profile::Dev` is the default and reproduces today's
behavior exactly, so reverting is a matter of removing the `Profile` enum, its builder method,
the validator, and the new error variants; every existing call site is untouched by both the
change and the revert. There is no separate partial-configuration behavior to revert — the
partial case is subsumed by the missing-capability rule under `Profile::Production` (D-2,
revised; AD-7), and `Profile::Dev` never restricted it. No schema, no migration, no data, and no runtime
storage format is affected: PROD-013 writes no persistence code, only validation. Documentation
changes (IS-9, IS-10) revert in one commit.

## Dependencies

- PROD-012 (Durable Idempotency, archived) — supplies the validator/error template
  (`crates/service-sdk/src/runtime/builder.rs:735-771`). Reused, not modified.
- PROD-002 (archived) — the durable effect store PROD-013 gates against already exists.
- Existing durable Postgres event store and snapshot store
  (`crates/persistence/src/postgres/{event_store,snapshot}.rs`).
- No new external dependency, crate, service, or infrastructure.

## Success Criteria

- [ ] **SC-1** — A composition with `Profile::Production` and no configured event store is
      refused at bootstrap, and the error names both the capability and
      `EntityRuntimeBuilder::with_event_store()`.
- [ ] **SC-2** — The same holds for the snapshot store, naming
      `EntityRuntimeBuilder::with_snapshot_store()`.
- [ ] **SC-3 (corrected)** — The same holds for the effect store, naming the configuration
      call for the composition surface in use. The effect store DOES have a real silent
      fallback to `InMemoryEffectStore` (`crates/service-sdk/src/runtime/builder.rs:811`, a
      finding made after the initial exploration, which only searched for the
      `unwrap_or_else` pattern and missed this `match`) — the defect is the same class of
      silent volatility as the event/snapshot store, not a failure deferred to first use. The
      gate applies only when at least one effect executor is registered (with none registered,
      no store is constructed at all, so nothing is volatile).
- [ ] **SC-4** — Under `Profile::Production`, no code path can reach `InMemoryEventStore` or
      `InMemorySnapshotStore` by default; the silent `unwrap_or_else` fallback is unreachable.
- [ ] **SC-5** — A composition without `Profile::Production` and with nothing configured still
      builds successfully on in-memory storage, behavior identical to today.
- [ ] **SC-6 (revised)** — Configuring exactly one of `{event_store, snapshot_store}` under
      `Profile::Production` is refused (by IS-2/IS-3, not a separate rule). Under
      `Profile::Dev`, configuring partially remains valid, unchanged — this is today's
      behavior, and breaking it would violate IS-8/SC-7.
- [ ] **SC-7** — All 67 existing `EntityRuntimeBuilder::new()` call sites compile and pass
      unmodified; `cargo test --workspace` shows zero new failures.
- [ ] **SC-8** — One validator is the single source of truth for the rule across all three
      capabilities; there is no second, parallel check.
- [ ] **SC-9** — No read-side/projection gate exists anywhere in the change, and the
      read-side follow-up constraint is recorded verbatim against PROD-014.
- [ ] **SC-10** — The persistence completeness rule and the PROD-005 boundary are documented
      and readable by a human, not merely parseable by an agent.
- [ ] **SC-11 (revised)** — `examples/reference-app`'s `EntityEventStores::open(pool)` produces
      `Profile::Production` (consumed by `build_runtime_with`, which is shared between dev and
      production); `EntityEventStores::in_memory()` produces `Profile::Dev`. The profile field
      is private, so the check that the declaration is ever removed or weakened reduces to
      asserting `stores.profile()` at both constructors (AD-10).
