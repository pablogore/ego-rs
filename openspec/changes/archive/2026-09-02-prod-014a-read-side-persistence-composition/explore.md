# Exploration: PROD-014A — Read-Side Persistence Composition

## Current State

**PROD-013's mechanism (the thing to reuse verbatim).**
`crates/persistent-entity/src/profile.rs` defines `Profile { Dev (#[default]), Production }` and the single shared predicate `require_durably_configured(profile, durably_configured: bool, capability, fix) -> Result<(), PersistenceCompositionError>`. Only `Profile::Production` with `durably_configured == false` refuses, via `PersistenceCompositionError::NotConfigured { capability, fix }` (`crates/persistent-entity/src/error.rs`). `durably_configured` is deliberately not `.is_some()`. Two established durability-declaration idioms: `is_durable(&self) -> bool { false }` default method (on `EventStore`, `Snapshot`, added for PROD-013 AD-12) and `EffectStoreCapabilities{durable,...}` (on `EffectStateStore`, richer because that capability has orthogonal concurrency/multi-node/lease axes). Registration precedent: a dedicated named field + presence-guard bool on `RuntimeBuilder`/`AppBuilder`, fail-closed duplicate latched into `pending_error`.

**Original finding (still valid): `ProjectionStateStore` is orphaned.** `crates/domain/src/read_side/projection_state_store.rs` has zero implementations and zero callers workspace-wide, confirmed by both source grep and an OpenSpec-wide grep for `ProjectionStateStore` (only self-reference). `ReadSideProcessor` (its natural consumer) also has zero implementations. The real, currently-running engine (`ReadSideSession`/`ReadSideRunner` in `crates/domain/src/read_side/{session,runner}.rs`, `TagSchedulerImpl`/`ProjectionSpec` in `crates/runtime/src/read_side/scheduler.rs`) is generic over `OffsetStore` + `DedupStore`, not `ProjectionStateStore`.

**PROD-013 Phase 8.1/8.2 risk — verified non-blocking.** Confirmed via `verify-report.md`: both `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` exit 0; unchecked boxes are administrative only. PROD-013 remains unarchived on `develop` — minor process debt, not a blocker.

## REVISION (superseding the original recommendation below)

The architect reviewed the original finding and **rejected governing `ProjectionStateStore`** as the next step: it would be "prolija arquitectura" — hardening a dead port that solves no real production problem. This section re-investigates the REAL read-side persistence gap and supersedes the original "Approaches"/"Recommendation" sections further down, which are kept for historical record only.

### 1–4. Where OffsetStore/DedupStore are actually constructed, what implementations exist, fallbacks, and reference-app usage

All four questions converge on one file: `examples/reference-app/src/read_side/mod.rs`, `ReadSideHandles::new()` (lines 103-113):

```rust
pub fn new(store: SharedReadSideStore) -> Self {
    let query = UsersByTenantStore::default();
    Self {
        handler: UsersByTenantHandler::new(query.clone()),
        query, store,
        offset_store: InMemoryOffsetStore::default(),
        dedup_store: InMemoryDedupStore::default(),
        logger: None,
    }
}
```

This hardcodes `InMemoryOffsetStore`/`InMemoryDedupStore` (`examples/reference-app/src/read_side/store.rs:150-238`) unconditionally — there is no parameter, no injection point, no way to plug in anything else today. These two types are **app-level, not framework-level**: unlike `InMemoryReadSideStore` (which lives in the reusable `crates/infrastructure/src/persistence/in_memory/read_side_store.rs`), the workspace ships no reusable in-memory `OffsetStore`/`DedupStore` at all — reference-app had to hand-roll its own (confirmed by its own doc comment: "this workspace has no other in-memory reference implementation of it").

`crates/service-sdk/src/app/mod.rs` (`AppBuilder`) has **zero** references to `read_side`, `OffsetStore`, `DedupStore`, or `ProjectionStateStore` (confirmed by grep — its only `DedupStore`-adjacent hits are `EffectDedupStore`, an unrelated effect-system type). `crates/runtime/src/read_side/scheduler.rs` has **zero** references to `Profile` (confirmed by grep). Read-side construction and wiring bypasses `AppBuilder`/`RuntimeBuilder`/`Profile` entirely — `main.rs` calls `read_side_handles.spawn()` independently of the `App::builder()...build()` chain.

**No durable (Postgres) implementation of `ReadSideStore`, `OffsetStore`, or `DedupStore` exists anywhere in the workspace.** `crates/persistence/src/postgres/` contains `event_store.rs`, `snapshot.rs`, `repository.rs`, `reservation.rs` — real, PROD-015-verified-against-real-Postgres durable backends for the entity-runtime side — but nothing for read-side. This is despite CORE-005's own original implementation plan (`openspec/changes/archive/2026-06-22-read-side-projections/plan.md:118-130`) explicitly planning `crates/infrastructure/src/persistence/postgres/{read_side_store,offset_store,dedup_store}.rs` as part of its Storage section ("Four separate storage SPIs in domain, each with in-memory + Postgres backends"). Those Postgres read-side backends were never built. This is a genuine, separate implementation gap (a PROD-014B-shaped concern), distinct from the composition-governance gap.

### 5. Restart behavior today

`InMemoryOffsetStore`/`InMemoryDedupStore` store state in a plain `Arc<Mutex<HashMap>>`/`HashSet` in process memory. On process restart, both are fully reconstructed empty (`Default`) — every projection resumes from `read_offset() -> Ok(None)` for every key, causing a full replay of all events currently in `SharedReadSideStore` (itself also in-memory and lost on restart in reference-app's current wiring, so in practice the whole pipeline's state — events, offsets, and dedup — resets together in this specific reference app; a real deployment with a durable `ReadSideStore`/`EventStore` upstream but volatile `OffsetStore`/`DedupStore` would replay all historical events with no dedup memory, which handler-side idempotency may or may not absorb). This is unconditional — it happens regardless of any `Profile` setting, because nothing observes `Profile` here at all.

### 6. Can `Profile::Production` bootstrap succeed today with volatile read-side stores?

**Yes, trivially — and not merely "unguarded": read-side persistence has zero composition-time visibility at all.** `RuntimeBuilder::validate_persistence_profile()` (`crates/service-sdk/src/runtime/builder.rs`) only checks event store, snapshot store, and effect store. `AppBuilder`/`RuntimeBuilder`/`App` hold no reference to `ReadSideHandles`, `OffsetStore`, or `DedupStore` at all. `ReadSideHandles::spawn()` takes no `Profile` parameter. A composition can declare `Profile::Production`, pass PROD-013's event/snapshot/effect-store gates, and still spawn a fully volatile read-side pipeline with no refusal, no warning, and no code path that could even observe the mismatch today.

### 7. CORE-026's exact stated reasoning

CORE-026's live spec (`openspec/specs/read-side/spec.md:160-171`, Non-Goals):

> **Constructing a dedup store, an offset store, or a tag-discovery mechanism is out of scope.** `spawn_projection` takes these as required arguments; it does not provide a default or internally construct them. An application obtains them exactly as it does today (e.g. reference-app's own `ReadSideHandles::new`, itself unchanged by this capability) and passes them to `spawn_projection` to spawn the poller. A framework-level convenience that also constructs these internally (e.g. defaulting to in-memory implementations) was considered and rejected — see design.md AD-1, alternative (b) — because the handler and tag-discovery closure are irreducibly application-specific, and bundling the dedup/offset stores' construction with them would only cover half the boilerplate while suggesting the other half was solved too.

CORE-026's own change folder (with the literal `design.md` AD-1 text) could not be located under a `*026*`/`CORE-026` filename pattern in `openspec/changes/archive/` — the committed spec at `openspec/specs/read-side/spec.md` is the authoritative surviving record of this reasoning and is treated as such.

### 8. Was CORE-026's non-goal a scoped API decision, or a durable stance that should still hold?

**Scoped API/ergonomics decision — and it does not conflict with adding a durability check.** The rejected alternative (b) was specifically about the framework *auto-constructing default stores* to reduce caller boilerplate ("defaulting to in-memory implementations"), rejected because that would be a half-measure (handler/tag-discovery still need app code) that misleadingly implies more is solved than is. Nothing in this reasoning addresses whether the framework may *inspect* a durability property of whatever store the application *does* construct and pass in — that is a check on caller-supplied input, not construction of a default. These are orthogonal axes: "who constructs the store" (CORE-026's actual subject, still valid, unaffected) vs. "does the composition refuse to start in Production if what was constructed is volatile" (a new, additive requirement).

This non-goal predates `Profile::Production` (PROD-013, 2026-08-25) — CORE-026 could not have reasoned about a governance mechanism that did not exist yet. But the non-goal's own text is about default-construction ergonomics, not about production-safety gating, so it is not stale/superseded by PROD-013 — it simply never addressed this axis. A durability-check requirement is additive to CORE-026's spec, not a reversal of it. It does, however, still require a spec.md delta to `openspec/specs/read-side/spec.md` (new requirement, and a clarifying note that "out of scope: default construction" is unaffected), because the current Non-Goals text also states this capability wraps the engine's existing contract without renegotiating any part of it — adding a possible refusal at spawn time is a real (if narrow, additive) change to that contract's observable behavior and needs to be specified, not silently added.

### 9. Minimal atomic change, and where the gate should actually live

Two structurally different insertion points exist, and the codebase's actual current decoupling of read-side from `AppBuilder` makes this a real fork, not just an implementation detail:

- **A1 (recommended, minimal).** Add `is_durable(&self) -> bool { false }` to `OffsetStore` and `DedupStore` (mirrors `EventStore`/`Snapshot`). Add a `Profile` parameter to `ProjectionSpec::new()`/`TagSchedulerImpl::spawn()` (`crates/runtime/src/read_side/scheduler.rs`), which already sits in a crate that depends on `persistent-entity` (confirmed via `crates/runtime/Cargo.toml`), so it can call the SAME `require_durably_configured` predicate directly — refusing at `spawn()` time (always called once, before the poll loop's first tick, at application startup) if `Profile::Production` and either store's `is_durable()` is `false`. This satisfies "rejection at bootstrap/composition time, not on first event processed" functionally, without touching `AppBuilder`/`RuntimeBuilder` or reference-app's current App/ReadSideHandles decoupling. Smallest diff; does not cross into "new projection-engine semantics" (explicitly a PROD-014A non-goal) beyond one new refusal path.
- **A2 (fuller precedent match, larger).** Add a real `AppBuilder`/`RuntimeBuilder` registration slot for `OffsetStore`/`DedupStore` (mirroring `.effect_store()` exactly) and gate at `build()`/`try_build()`. This is architecturally closer to PROD-013's pattern, but requires re-plumbing reference-app so `ReadSideHandles` is constructed FROM values registered on `AppBuilder` instead of hand-constructing `InMemoryOffsetStore`/`InMemoryDedupStore` itself — a materially bigger change that touches composition-root wiring, not just a validation predicate.

**Recommend A1** as the atomicity-gate-compatible minimal change; flag A2 explicitly to `sdd-propose` as the "matches PROD-013's shape more closely, but bigger" alternative for the architect to weigh.

### Decision among A / B / C

**A — govern `OffsetStore`/`DedupStore` directly**, because they are the real (and only) read-side persistence mechanism running in production today. This is NOT blocked on a prerequisite CORE-026 amendment (rejecting **B** as a hard prerequisite): CORE-026's non-goal is about default construction, which a durability check does not touch. A spec delta to `openspec/specs/read-side/spec.md` is still required as PART OF the same change (additive new requirement + a clarifying note), which is normal SDD practice, not a separate governance change that must land first.

**`ProjectionStateStore` classification (answering C's premise): a disconnected/abandoned fragment, not a legitimate future abstraction.** Evidence: CORE-005's original tasks.md (`openspec/changes/archive/2026-06-22-read-side-projections/tasks.md`) created `ProjectionState` (T011) and `ReadSideProcessor` (T029) as part of a fuller per-processor state-machine design, but its own FR-007 and data-model.md define state persistence purely in terms of `OffsetStore`+`DedupStore` atomic commit — a dedicated `ProjectionStateStore` trait for `ProjectionState` was never part of CORE-005's original task list, spec, data-model, or contracts/README.md. It has no traceable origin in any OpenSpec document at all (a workspace-wide OpenSpec grep for the literal string returns zero hits outside this very exploration). Its only plausible consumer, `ReadSideProcessor`, also has zero implementations. CORE-026 later shipped a leaner, concrete successor (`TagSchedulerImpl`/`ProjectionSpec`) that solves polling/dedup/offset tracking without any persisted 5-state state machine and without `ReadSideProcessor`. Conclusion: `ProjectionStateStore` was most likely scaffolded during or after CORE-005's implementation as a natural companion to `ReadSideProcessor`'s state machine, then abandoned once CORE-026's simpler design shipped instead. It should stay out of the critical path; PROD-013's `Profile` doc comment naming "PROD-014" as its intended extension point should be corrected once the real successor change lands, since it names the wrong trait.

## Affected Areas (revised)

- `crates/domain/src/read_side/offset.rs`, `dedup.rs` — add `is_durable(&self) -> bool { false }` default methods.
- `crates/runtime/src/read_side/scheduler.rs` (`ProjectionSpec::new`/`TagSchedulerImpl::spawn`) — thread a `Profile` parameter, call `require_durably_configured` (A1) — needs a new dependency edge to `persistent-entity`'s `Profile`/predicate, already satisfiable since `crates/runtime` depends on `persistent-entity`.
- `openspec/specs/read-side/spec.md` — new requirement + Given/When/Then scenarios (Dev+volatile allowed; Production+volatile rejected at spawn; Production+durable fake allowed; clarifying note that default-construction remains out of scope, unaffected).
- `examples/reference-app/src/read_side/store.rs` — add a test-only fake durable `OffsetStore`/`DedupStore` (`is_durable() -> true`) to demonstrate the Production-path acceptance without inventing a real Postgres backend (that remains PROD-014B/future work).
- `crates/persistent-entity/src/profile.rs` doc comment — correct once the real successor lands (currently names PROD-014/`ProjectionStateStore` inaccurately per this revision's findings).
- NOT touched: `ProjectionStateStore`, `ReadSideProcessor`, `DependencyTable`, `.projection()` DI, `AppBuilder`/`RuntimeBuilder` (under A1) — all correctly out of scope.
- Explicitly flagged, not resolved here: whether to instead do A2 (real `AppBuilder` registration slot, bigger, closer to PROD-013's shape) is left for `sdd-propose`/the architect to decide.

## Risks (revised)

1. No durable (Postgres) `OffsetStore`/`DedupStore`/`ReadSideStore` implementation exists anywhere in the workspace today — even after a composition-governance change ships, Production users have no real durable option to configure yet (a separate, larger implementation gap, likely PROD-014B-shaped).
2. A1 vs A2 is a genuine unresolved fork: A1 is minimal and atomicity-gate-compatible but gates at `spawn()` rather than literal `AppBuilder::build()`; A2 matches PROD-013's precedent more closely but requires re-plumbing reference-app's App/ReadSideHandles decoupling — `sdd-propose` must pick one explicitly, not default silently.
3. `openspec/specs/read-side/spec.md` will need a delta even though this is framed as "not blocked on CORE-026" — the delta is small and additive, but must be drafted carefully to avoid contradicting the existing Non-Goals text about "not renegotiating" the engine's contract.
4. `ProjectionStateStore`'s "abandoned fragment" classification rests on documentary/code evidence only — no git history access in this session to confirm via commit authorship.
5. PROD-013's `Profile` doc comment explicitly promises "PROD-014 introduces [read-side governance]" without naming a trait; it should be corrected as part of the next change to avoid a stale pointer.

## Ready for Proposal

Yes, with one explicit open question for the architect to resolve before/at `sdd-propose`: A1 (gate at `ProjectionSpec::new`/`TagSchedulerImpl::spawn`, minimal) vs A2 (real `AppBuilder` registration slot, matches PROD-013 precedent exactly, larger). Atomicity of the resulting change is not evaluated here per explicit instruction — that judgment belongs to `sdd-propose`.

---

## ORIGINAL EXPLORATION (superseded recommendation, kept for record)

### Original Approaches Table

| Approach | Pros | Cons | Effort |
|---|---|---|---|
| A (original). Govern `ProjectionStateStore` only | Matches literal original brief; zero conflict with CORE-026 | Governs a dead port — closes no real gap | Small |
| B (original). Extend governance to `OffsetStore`/`DedupStore` | Closes the real gap | Was believed to conflict with CORE-026's non-goal, requiring a prerequisite amendment | Was believed large/blocked |
| C (original). Do both | Comprehensive | Violates Atomicity Gate | Not recommended |

### Original Recommendation (REJECTED by architect, see REVISION above)

The original recommendation to govern `ProjectionStateStore` (Approach A) was rejected by the architect as solving a non-problem. The REVISION section above supersedes this with Approach A targeting `OffsetStore`/`DedupStore` directly, having re-verified that CORE-026's non-goal does not actually block a durability check (only default construction), correcting the original exploration's unexamined assumption that "governing OffsetStore/DedupStore requires first amending CORE-026."
