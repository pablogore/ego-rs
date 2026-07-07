# Proposal: CORE-006A — Activation Authority & Linearizability

Remediates CORE-006 Known Architecture Debt gaps #1 and #5
(`openspec/changes/archive/2026-06-22-persistent-entity-runtime/spec.md:700-713`, both Critical).

## Intent

Entity activation in `persistent-entity` is not serialized, not linearizable, and has
no single source of truth. `EntityRuntime::entity_ref()`
(`crates/persistent-entity/src/runtime.rs:129-158`) unconditionally spawns a brand-new
`EntityActor` via `TokioEntityRef::new()` on every call — there is no lookup of an
existing mailbox for the same entity triple. N concurrent callers get N actors, each
independently recovering the same entity and racing `persist_events()` with divergent
versions. A multi-thread probe (20 concurrent `entity_ref()` + `send_command()` to one
entity ID) reproduced 7–10 optimistic-concurrency failures out of 20 across 5 runs.

This is a root-cause gap, not a regression: the coordination layer the architecture
documents was never wired in.

## Findings (verified against code)

| # | Question | Answer | Evidence |
|---|----------|--------|----------|
| 1 | Registry visible before `Active`? | Yes | `registry.mark_active()` runs in the sync constructor (`entity_ref_tokio.rs:119`), before `tokio::spawn` (`:130`) and before the actor's `recover_state()` transitions to `Active` (`actor.rs:132`). Self-documented "Known window" (`entity_ref_tokio.rs:114-118`). |
| 2 | Two independent sources of truth? | Yes | `EntityRegistry.active_entities: HashSet<String>` (`registry.rs:14`) vs. per-actor `LifecycleStateMachine` (`actor.rs:35`, `lifecycle.rs:25-28`). Never reconciled. Duplicate actors share one HashSet entry, so `active_count()` cannot even count actors; any one duplicate's `remove_active` deletes the entry for all. |
| 3 | `SharedActivation` dead code? | Yes | Defined in `activation.rs:12-31`; `lib.rs:28-52` declares no `mod activation` — never compiled, zero callers. Same for `supervisor.rs` (absent from `lib.rs`; would not compile: awaits sync `remove_active`, `supervisor.rs:38,54`). |
| 4 | Concurrent activations serialized? | No | No lookup, no guard, no single-flight — `runtime.rs:147` straight to `TokioEntityRef::new()`, which always spawns (`entity_ref_tokio.rs:130`). |
| 5 | Activation linearizable? | No | Overlapping actors per triple; registry state corresponds to no single actor's lifecycle; no total order of activate/passivate events exists. |
| 6 | Partial activation observable? | Yes | Registry reports active before the spawned future first polls and before recovery completes (finding 1). Mailbox-as-recovery-barrier holds *within* one actor (`actor.rs:61-68`) but nothing prevents a second actor. |
| 7 | Rollback centralized? | No | Split across `SpawnGuard::drop` (`entity_ref_tokio.rs:30-39`), recovery-failure drain (`actor.rs:315-321`), two passivation sub-paths (`actor.rs:328,340-341,360-361`), plus the orphaned `Supervisor`. Each mutates the registry unaware of duplicates. |

Corroborating gaps:

- **Docs describe an unimplemented design.** `ARCHITECTURE.md:118-119` and
  `contracts/runtime.md:64-65` (archive) describe an "active entity actor map" with
  `pending_activations → SharedActivation`; the real registry stores only ID strings.
  The term "registry" currently implies routing authority it does not have.
- **Tests don't exercise real concurrency.** `test_activation_mutex_serializes` and
  `test_no_double_spawn_concurrent`
  (`tests/activation_ordering_tests.rs:159,197`) use default `current_thread`
  `#[tokio::test]`; and their `active_count <= 2` assertions can't detect duplicate
  actors (finding 2).
- Prior drafts — `execution-authority/spec.md` (Draft),
  `final-consistency-lock/` (tasks 0/63), `reactivation-safety-spec.md` (Draft) —
  already converge on "the Actor is the sole Execution Authority; exactly one per
  triple". They are design input for the spec phase, not architecture to reinvent.

## Desired Outcome

The specification phase must define, as observable contracts (not mechanisms):

- A **single activation authority**: exactly one live actor per entity triple; all
  callers of `entity_ref()` for the same triple reach the same mailbox.
- A **single source of truth** for "is this entity active", ending the
  registry/lifecycle split. Concretely: `active_count()` (and any equivalent
  externally-visible query) counts only entities whose `EntityState == Active` —
  never `Recovering`, never a transient/duplicate registry entry. No intermediate
  state is exposed as "active."
- **Deterministic activation visibility**: an entity is externally observable as
  active only under defined conditions; no partially-activated observation. This
  contract applies uniformly to **every transition into `Active`** — cold activation
  from no prior state, and reactivation from `Passivated` — not only the cold-start
  path. The two paths (`Passivated → Recovering → Active` and
  `∅ → Recovering → Active`) must satisfy the same visibility and linearizability
  guarantees; the spec does not define two separate contracts by origin.
- **Linearizable activation**: concurrent activation attempts resolve as if
  sequential, with one winner and all others converging on its result — regardless
  of whether the entity started cold or is being reactivated from `Passivated`.
- **Fail-closed semantics**: activation failure leaves no residual actor, mailbox, or
  registry entry.
- **Deterministic failure behavior**: one rollback contract instead of four
  independent cleanup paths.
- Tests that exercise multi-threaded concurrency and assert actor-level (not
  ID-set-level) invariants, covering both cold activation and reactivation-from-
  `Passivated`.

## Capabilities

### New Capabilities
- `entity-activation`: activation authority, source of truth, visibility, and
  failure semantics for the persistent-entity runtime.

### Modified Capabilities
- None (no existing spec under `openspec/specs/` covers this crate).

## Non-Goals

- Runtime bootstrap, dependency injection, transports, schedulers/scheduling policy,
  clustering (CORE-007), persistence protocol, authorization, tenancy, telemetry.
- Renaming existing concepts (e.g. "registry") — misleading terminology is documented,
  not churned.
- Prescribing synchronization primitives or algorithms — spec/design phases decide.

## Product Decisions (resolved 2026-07-07)

| # | Question | Decision |
|---|----------|----------|
| D1 | Can `active_count()`'s visible semantics change? | Yes. It counts only `EntityState == Active`; today's early-visible behavior is the bug, not a contract to preserve. |
| D2 | Disposition of orphaned `activation.rs`/`supervisor.rs`? | Not decided here. Proposal requires only "a single activation authority shall exist"; whether that reuses, rewrites, or removes these files is a design-phase decision. |
| D3 | Does the contract cover reactivation-from-`Passivated`, or cold activation only? | **Broadened from the initial assumption.** The contract covers every transition into `Active`, cold or reactivated — see Desired Outcome and Risks above. This does not require implementing new passivation functionality; it requires the same activation-authority guarantee to provably hold on the existing reactivation path too. |
| D4 | Does `ARCHITECTURE.md` update land in this same change? | Yes — an architectural-invariant change (single activation authority / single source of truth) updates its architecture doc in the same PR, not a follow-up. |

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistent-entity/src/runtime.rs` | Modified | `entity_ref()` activation path |
| `crates/persistent-entity/src/entity_ref_tokio.rs` | Modified | Spawn/visibility/rollback path |
| `crates/persistent-entity/src/registry.rs` | Modified | Source-of-truth contract |
| `crates/persistent-entity/src/activation.rs`, `supervisor.rs` | Modified/Removed | Orphaned files: wire in or delete |
| `crates/persistent-entity/src/actor.rs` | Modified | Failure/rollback contract |
| `crates/persistent-entity/tests/activation_ordering_tests.rs` | Modified | Multi-thread flavor, actor-level assertions |
| `ARCHITECTURE.md` | Modified | Align docs with implemented reality |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Handle caching changes `entity_ref()` observable behavior for existing callers | Med | Spec phase enumerates current callers and pins the visible contract first |
| Fixing visibility breaks tests that rely on eager `active_count()` | Med | Registry visibility semantics defined in spec before implementation |
| Reactivation-from-`Passivated` scenarios expand implementation surface beyond cold-start-only | Med | In scope by decision (not a risk to avoid): the contract covers every transition into `Active`. Reuse `reactivation-safety-spec.md` scenarios directly; this does not require implementing new passivation *functionality*, only proving the same activation-authority contract holds on the existing reactivation path. |

## Rollback Plan

Planning artifacts only until apply. Implementation lands behind normal PR revert;
no data or persisted-format migration is involved — event store and snapshot formats
are untouched.

## Dependencies

- Design input: `execution-authority/spec.md`, `final-consistency-lock/`
  (spec/tasks/data-model), `reactivation-safety-spec.md` (all archived drafts).

## Success Criteria

- [ ] Spec phase defines the six contracts under Desired Outcome without inheriting
      implementation details from this proposal.
- [ ] The 20-caller multi-thread probe scenario is representable as a spec acceptance
      scenario with expected outcome: 0 optimistic-concurrency conflicts, 1 actor.
- [ ] Every finding (1–7) maps to at least one requirement or explicit non-goal.
- [ ] Orphaned files (`activation.rs`, `supervisor.rs`) have a decided disposition.
