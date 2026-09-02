# Proposal: CORE-PERSIST-B — First-Class In-Memory Persistence Adapter

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers:
> D-1..D-12, NG-1..NG-12, R1..R18, KD-1..KD-4, F-1..F-4).
>
> Ground truth: `explore.md` in this folder. Its inventory, MOVE MATRIX, COMPATIBILITY REEXPORT
> MATRIX, and READINESS block are consumed as given, not re-derived.

## Objective

Give the workspace **one** in-memory persistence adapter crate. Seven canonical-candidate
implementations of `ego-persistence-api` ports are relocated verbatim into a new
`ego-persistence-memory` crate, with a compatibility re-export at every path a consumer resolves
today. Zero new behavior, zero contract change, zero Postgres work.

## Intent

**The problem is ownership, not behavior.** After CORE-PERSIST-A every domain-owned persistence
port has exactly one owning crate — but their in-memory implementations do not. They are scattered
across three crates in three different layers, and the layer each one sits in is an accident of who
needed it first, not a decision anyone made:

- `ego-infrastructure` (layer `infrastructure`) owns four of them, in a crate whose `Cargo.toml`
  also drags in `sqlx`, `ego-application`, and the OpenTelemetry stack — none of which the
  `in_memory` submodule imports (`explore.md` DEPENDENCY GRAPH).
- `ego-testkit` (layer `tooling`, a **sink**) owns `InMemoryOperationReservationStore` — the only
  implementation of `OperationReservationStore` anywhere in the workspace, and by its own doc
  comment (`crates/testkit/src/reservation.rs:74-78`) "a real, full implementation of the real
  production port, not a parallel model of it". Because `tooling` is a sink, **no production crate
  can reach it.**
- `examples/reference-app` owns `InMemoryOffsetStore` and `InMemoryDedupStore` — the only
  implementations of `OffsetStore` and `DedupStore` anywhere, a fact the example itself admits in
  a doc comment (`store.rs:150-151`, `store.rs:196-197`). **An example is acting as the
  workspace's infrastructure owner.**

The observable cost: a consumer who needs an in-memory `OffsetStore` today must either depend on an
example crate or write a fifteenth fake. A framework that ships eight ports and hides three of
their only implementations in a test crate and an example is not offering a persistence surface —
it is offering a scavenger hunt.

**This change is purely structural.** Every moved implementation keeps its body byte-for-byte;
only its module path and its `use` lines change (D-4). Nothing is observable at runtime.

## Active Decisions

| ID | Decision | Rationale / evidence |
|----|----------|----------------------|
| D-1 | **New crate `ego-persistence-memory` at `crates/persistence-memory/`.** Name confirmed, not merely inherited | Matches the `ego-*` package / bare-directory convention `ego-persistence-api` at `crates/persistence-api/` already set, and stays unambiguously distinct from the existing `ego-persistence` (Postgres adapters). CORE-PERSIST-A's own D-1 forward-declared this exact name: "`persistence-postgres` / `persistence-memory` renames are CORE-PERSIST-B/C's job" (archived `proposal.md:43`) |
| D-2 | **Layer classification is `foundation`, not `domain`.** `layers.toml` gains `"ego-persistence-memory" = "foundation"` and **`xtask/src/layers.rs` is not modified at all** | Three reasons. (a) **Honesty**: an in-memory adapter is an adapter; in hexagonal terms ports are domain artifacts and implementations are not. (b) **Containment**: `foundation → domain` is already permitted (`layers.rs:77`), so no gate relaxation is needed; mapping to `domain` instead would ride CORE-PERSIST-A's D-4 self-edge and thereby widen it from "a domain crate may reach a *port* crate" to "a domain crate may reach an *adapter*", legalizing `ego-domain → ego-persistence-memory`. (c) **Reachability is sufficient**: every real consumer's layer already permits a `foundation` dependency — `infrastructure` (`layers.rs:80-86`), `sdk` (`:87`), `foundation` (`:77`), `tooling` (sink, `:89`). `domain` and `application` cannot reach it, which is the correct outcome for an adapter |
| D-3 | **Dependency set is `ego-persistence-api` **plus `ego-domain`**, and the second edge is named rather than assumed away.** `InMemoryOperationReservationStore` holds an `Arc<dyn Clock>` (`crates/testkit/src/reservation.rs:80`), and `Clock` was **not** relocated by CORE-PERSIST-A — it lives at `crates/domain/src/time/clock.rs:24` | `explore.md`'s DEPENDENCY GRAPH lists `ego_domain::Clock` among `reservation.rs`'s imports but its dependency-rule summary reads as `ego-persistence-api`-only; this proposal closes that gap explicitly. `Clock` is a domain value abstraction, so the edge is the "unavoidable domain-value crate" case, not scope creep. It is legal (`foundation → domain`, `layers.rs:77`) and acyclic (`memory → domain → persistence-api`). **Relocating `Clock` into `ego-persistence-api` is rejected** — that would widen a shipped, archived contract surface to serve a move, which is exactly what this change is forbidden to do. `design.md` owns the final import list |
| D-4 | **Relocation is verbatim in body, rewritten only in `use` lines.** Struct bodies, trait impls, doc comments, locking strategy, and tenant-resolution logic move unedited; the only permitted edit is rewriting `use ego_domain::persistence::…` to `use ego_persistence_api::persistence::…` where the item now resolves directly | Mirrors CORE-PERSIST-A's D-6, and is forced by D-3: the new crate resolves ports from `ego-persistence-api` directly rather than through `ego-domain`'s re-export layer. The rewrite touches import lines only and changes no resolved item — the two paths name the same types by construction (`persistence-api-surface` spec, "Old Path Resolves To The Same Item") |
| D-5 | **Every old path keeps resolving, via a compatibility re-export in the vacating crate.** `crates/infrastructure/src/persistence/in_memory/mod.rs:12-15` becomes `pub use ego_persistence_memory::…`; `crates/testkit/src/lib.rs` likewise for the reservation store | Mirrors CORE-PERSIST-A's D-5, and is what keeps this change revertible mid-flight (Rollback Plan). Confirmed consumers that must compile unedited: `crates/infrastructure/tests/in_memory_event_store_conformance.rs:17-18`, `crates/infrastructure/tests/commit_publishes_atomically.rs`, `examples/reference-app/src/lib.rs:432-439`, `crates/transport/tests/operation_key_extractor.rs`, `crates/service-sdk/tests/{retention_worker_lifecycle,cross_tenant_reservation_isolation}.rs` (`explore.md` COMPATIBILITY REEXPORT MATRIX) |
| D-6 | **`examples/reference-app` stops being an infrastructure owner.** `InMemoryOffsetStore` and `InMemoryDedupStore` move out; the example's own `use` statements are updated to the new crate. **No re-export is created in the example** | The example is a leaf with no external consumer, so a compatibility shim there would be dead weight — this is the one place where updating imports is cheaper and clearer than re-exporting. Everything genuinely example-specific stays untouched: `SharedReadSideStore` (carries reference-app tenant/tag logic, `store.rs:66-78`), `ReadSideSink`, and both `Fake*Durable*` stores |
| D-7 | **Moving `InMemoryOperationReservationStore` out of `ego-testkit` makes it production-reachable for the first time. This proposal treats that as a decision requiring explicit sign-off, not a side effect** | `ego-testkit` is layer `tooling` — a sink no production crate may depend on (`layers.toml:13`). Today the store's only cross-crate consumers are dev-dependency test files. After the move it sits in `foundation` and any `foundation`/`infrastructure`/`sdk` crate may depend on it. **No code behavior changes** — same struct, same trait impl, same `is_durable()` default. What changes is who is allowed to wire it. The change is justified because it is the *only* implementation of `OperationReservationStore` in the workspace and its own doc comment already claims production fidelity (`reservation.rs:74-78`), but the widened reach is stated here so a reviewer approves it deliberately |
| D-8 | **`TestClock` and the colocated reservation tests stay in `ego-testkit`.** Only the store moves | `TestClock` (`crates/testkit/src/reservation.rs:27`) is a deterministic test double and belongs in the tooling crate by construction (NG-8, R16). Its colocated tests (`reservation.rs:528+`) drive the store *through* `TestClock`, so they exercise the re-exported store from testkit unchanged. `design.md` owns the exact split mechanism; the boundary — fakes stay, real implementations move — is fixed here |
| D-9 | **The two `persistent-entity` duplicates stay forked.** `InMemoryEventStore` (`crates/persistent-entity/src/persistence.rs:571`) and `InMemorySnapshotStore` (`:733`) are not moved, not merged, and not fixed | Merging either requires a behavior decision, which this change forbids. The event store carries an additive `with_version_offset()` capability (`persistence.rs:600-611,719-727`) that `crates/persistent-entity/tests/in_memory_version_offset_parity.rs:15,22-23` depends on: consolidating would either drop it or add it to the canonical crate — both are new behavior. The snapshot store is worse: it is a **confirmed tenant-isolation bug** (KD-5). Named debt, not silent resolution (NG-1, NG-2, R17) |
| D-10 | **The effect-store boundary stays closed.** `InMemoryEffectStore` (`crates/runtime/src/effects/store.rs:531`) is not moved and `ego-runtime` is not touched | Its ports (`EffectStateStore` `:238`, `EffectDedupStore` `:418`, `RetentionMaintenance` `:474`) are owned by `ego-runtime`, not `ego-persistence-api` — CORE-PERSIST-A's D-9 deferred exactly this. Moving the implementation without its ports leaves it implementing nothing; moving it with them in place forces `ego-persistence-memory → ego-runtime`, inverting `foundation → foundation`-and-below into a dependency on a crate that itself depends on `persistent-entity` (`crates/runtime/Cargo.toml:7,11`). The port relocation must land first, as its own change: **CORE-PERSIST-E** (F-1, R18) |
| D-11 | **Durability semantics are untouched, therefore production rejection is preserved.** No moved type gains, loses, or overrides `is_durable()` | `EventStore::is_durable()` and `Snapshot::is_durable()` default to `false` (`crates/persistence-api/src/persistence/event_store.rs:54-56`, `snapshot.rs:19-21`) and no moved implementation overrides them. `Profile::Production`'s `require_durably_configured` (`crates/persistent-entity/src/profile.rs:51-63`) rejects on `is_durable()`, not on presence — pinned by `presence_alone_is_not_durability` (`profile.rs:99-117`) and by two builder tests (`crates/persistent-entity/src/builder.rs:764-783,788-805`). A pure move cannot change a default it does not touch, so those rejections fire identically after this change (R6) |
| D-12 | **Missing stays missing; fakes stay fakes.** `ProjectionStateStore` gains no implementation (KD-1), and no `#[cfg(test)]`-local double is promoted | `ProjectionStateStore` has zero implementations workspace-wide, confirmed twice (`crates/persistence-api/src/read_side/projection_state.rs:27`; CORE-PERSIST-A `verify-report.md:64`). Implementing it here would make this change a feature wearing a move's name. Likewise the ~150 test-local fakes and the two `Fake*Durable*` stores that lie about `is_durable()` (`store.rs:251,282`) are excluded by construction (R3, R4, R16) |

## Atomicity Gate

**Run.** One indivisible move: seven implementations sharing one destination crate, one layer
decision (D-2), one dependency-direction decision (D-3), and one re-export strategy per vacating
crate (D-5, D-6). Relocating a subset leaves the in-memory adapter surface split across two
crates, which is the exact condition this change exists to end.

Explicitly **OUT**, each because it is an independent decision rather than a missing piece of this
one: a new store implementation · any contract or trait signature change · effect-port relocation ·
PostgreSQL consolidation · a conformance-test framework · any bug fix · any new runtime behavior.

**ATOMICITY: PASS** — matching `explore.md`'s ATOMICITY VERDICT and its `RECOMMENDATION: PROCEED`.
No CORE-PERSIST-A contract requires modification (verified against
`openspec/specs/persistence-api-surface/spec.md`); see Risks R-5 for the one documentation-scoping
caveat, which is a spec-phrasing matter, not a contract change.

## Scope

**Boundary at a glance**

| | |
|---|---|
| **CORE-PERSIST-B includes** | New `ego-persistence-memory` crate · 7 implementations relocated verbatim · compatibility re-exports at every old path · `layers.toml` entry · reference-app imports updated |
| **CORE-PERSIST-B excludes** | Every duplicate in `persistent-entity` · every effect store · all PostgreSQL work · every conformance harness · every bug fix · every test fake |

### In Scope

- **IS-1** — The new crate (D-1) mapped to `foundation` in `layers.toml` (D-2), depending only on
  `ego-persistence-api` and `ego-domain` plus external crates (D-3).
- **IS-2** — Verbatim relocation of the seven canonical candidates (D-4), all rows marked
  `Move allowed: YES` in `explore.md`'s MOVE MATRIX:
  1. `InMemoryEventStore` + `InMemoryEventStoreUnitOfWork` — `crates/infrastructure/src/persistence/in_memory/event_store.rs:89,214`
  2. `InMemoryRepository` — `.../in_memory/repository.rs:11`
  3. `InMemorySnapshotStore` (the tenant-correct one) — `.../in_memory/snapshot.rs:12`
  4. `InMemoryReadSideStore` + `paginate` — `.../in_memory/read_side_store.rs:24,105`
  5. `InMemoryOffsetStore` — `examples/reference-app/src/read_side/store.rs:153`
  6. `InMemoryDedupStore` — `examples/reference-app/src/read_side/store.rs:199`
  7. `InMemoryOperationReservationStore` — `crates/testkit/src/reservation.rs:79`
- **IS-3** — Compatibility re-exports in `ego-infrastructure` and `ego-testkit` at every path in
  `explore.md`'s COMPATIBILITY REEXPORT MATRIX (D-5).
- **IS-4** — `examples/reference-app`'s own `use` statements updated to the new crate (D-6).
- **IS-5** — A compile-time proof that every old path resolves to the *same* item, not a
  same-named copy.
- **IS-6** — Spec deltas per the Capabilities section.

### Out of Scope — Non-Goals

Every item is a **non-goal with a stated reason**, not an omission.

- **NG-1 — No bug is fixed, including the confirmed one.** `crates/persistent-entity/src/persistence.rs:733`'s
  `InMemorySnapshotStore` takes `_tenant_id: Option<&str>` and never reads it (`:746-765`), keying
  snapshots by `stream_id` alone (`:734`) — two tenants writing the same `aggregate_id` silently
  overwrite each other. **Reason**: fixing it here would be a correctness change smuggled inside a
  move, and it would land without the dedicated tests and blast-radius review a tenant-isolation
  fix deserves. Carried as **KD-5 → F-5** (R17).
- **NG-2 — The `persistent-entity` `InMemoryEventStore`/`StagingUnitOfWork` duplicate is not
  consolidated.** **Reason**: its `with_version_offset()` capability is additive relative to the
  canonical implementation; merging adds behavior on one side or removes it on the other (D-9).
  Carried as **KD-6 → F-6** (R17).
- **NG-3 — No PostgreSQL consolidation.** No SQL, migration, index, transaction, retry, or
  connection-pool change. **Reason**: a different backend, a different risk profile, and a
  different reviewer audience — deferred to **CORE-PERSIST-C** (R13).
- **NG-4 — No conformance-test framework is built or extended.** `Repository`, `Snapshot`,
  `OffsetStore`, and `DedupStore` still have no harness (KD-4). **Reason**: designing a conformance
  surface is a capability, not a relocation — deferred to **CORE-PERSIST-D** (R14).
- **NG-5 — No port or contract signature is redesigned in `ego-persistence-api`.** No method,
  bound, supertrait, default body, async/sync shape, or object-safety property changes.
  **Reason**: CORE-PERSIST-A shipped that surface; reopening it inside a move would make the diff
  unreviewable (R15).
- **NG-6 — The effect-store ownership boundary is not resolved.** `InMemoryEffectStore`,
  `EffectStateStore`, `EffectDedupStore`, and `RetentionMaintenance` stay in `ego-runtime`.
  **Reason**: blocked by CORE-PERSIST-A's D-9 — the ports must relocate first, which is a separate
  architecture decision. Named **CORE-PERSIST-E** (D-10, F-1, R18).
- **NG-7 — No cosmetic rename, reorganization, or "improvement" unrelated to the move.**
  **Reason**: every non-move edit inside this diff is a place a semantic drift can hide (D-4).
- **NG-8 — No specialized test fake is promoted to adapter status.** `FakeDurableOffsetStore`
  (`store.rs:251`), `FakeDurableDedupStore` (`store.rs:282`), `TestClock`, and every
  `#[cfg(test)]`-local double stay exactly where they are. **Reason**: `Fake*Durable*` stores
  override `is_durable()` to return `true` when they are not — their own doc comment says "Never
  wire this into a deployment" (`store.rs:240-249`). Promoting one would put a lie about durability
  into a shipped adapter crate (R3, R16).
- **NG-9 — The reference app is not treated as a canonical infrastructure owner.**
  `SharedReadSideStore`, `ReadSideSink`, and both fakes stay in the example, untouched.
  **Reason**: they carry reference-app-specific logic (`store.rs:66-78`), not a generic contract
  implementation (D-6, R8).
- **NG-10 — `ProjectionStateStore` is not implemented.** It stays at zero implementations.
  **Reason**: transparency beats a convenient stub — a fake implementation would hide a real gap
  behind a green build. Carried as **KD-1** (D-12, R4).
- **NG-11 — No new capability, trait, method, or type that does not exist today is added.**
  **Reason**: this change moves code; it does not write any.
- **NG-12 — No `Cargo.toml` outside the new crate, the two vacating crates, the reference app, and
  the workspace member list gains or loses a dependency.** **Reason**: the re-export layer (D-5)
  exists precisely so nobody else has to change.

## Capabilities

### New Capabilities

- `persistence-memory-adapter`: the observable contract that the workspace's in-memory
  implementations of the domain-owned persistence ports have exactly one owning crate; that every
  path resolving one today keeps resolving to the same item; that no port gains, loses, or changes
  an implementation; and that durability classification is unchanged.

### Modified Capabilities

- `persistence-api-surface`: two statements in the shipped spec are phrased as standing absolutes
  but describe CORE-PERSIST-A's own boundary, and this change's legitimate edits would read as
  violations of them. They must be re-scoped to CORE-PERSIST-A: the requirement "No Consumer
  Outside The Two Crates Is Edited" (`spec.md:96-104`), and the Non-Goals bullet "No implementation
  move — every `InMemory*` and `PostgreSQL*`/`Postgres*` adapter stays in its current crate"
  (`spec.md:131-132`). **No requirement about port shape, path resolution, or trait identity
  changes.**
- `foundation-integrity`: **no modification expected.** D-2 needs no matrix change —
  `foundation → domain` and `infrastructure → foundation` are already permitted
  (`xtask/src/layers.rs:77,80-86`). The `layers.toml` entry satisfies the existing completeness
  requirement (FR-001) rather than modifying it. Listed here only so the spec phase confirms rather
  than assumes.

If the spec phase finds an existing requirement already implies one of these, it folds rather than
manufacturing a delta.

## Approach

Create the crate; move each implementation file with its body unedited and its `use` lines
retargeted; replace each vacated declaration with a `pub use ego_persistence_memory::…` at the
identical path; update the reference app's imports; add the `layers.toml` entry and the workspace
member. Nothing else is edited.

Order matters for reviewability: settle the import closure (D-3) before any file moves, then move
the four `ego-infrastructure` implementations (largest, best-covered, protected by one re-export
site), then the reservation store with its sign-off (D-7), then the two read-side stores out of the
example. Each step leaves the workspace compiling with the re-export layer intact.

## Acceptance Requirements

Each is independently checkable and doubles as this change's success criteria.

- [ ] **R1 — Canonical ownership.** Each of the seven implementations resolves from exactly one
      declaring crate, `ego-persistence-memory`, and is declared nowhere else.
- [ ] **R2 — No duplicate canonical implementation is introduced.** The move creates zero new
      declarations; the count of `impl <Port> for` blocks per moved port is unchanged workspace-wide.
- [ ] **R3 — Named test fakes are not promoted.** `FakeDurableOffsetStore` and
      `FakeDurableDedupStore` remain declared in `examples/reference-app`, byte-identical, and
      appear nowhere in the new crate.
- [ ] **R4 — Missing stays visibly missing.** `ProjectionStateStore` has zero implementations after
      this change, and no stub, placeholder, or `todo!()` implementation is added.
- [ ] **R5 — Behavior preservation.** Every moved type's body — including tenant resolution, locking
      strategy, version-conflict arithmetic, and fail-closed empty-tenant handling
      (`read_side_store.rs:113-115`) — is textually identical to its pre-change form, modulo module
      path and `use` lines.
- [ ] **R6 — Durability and production preservation.** No moved type declares `is_durable()`;
      `presence_alone_is_not_durability` and both `try_build_rejects_explicit_in_memory_*` tests
      pass unmodified, still rejecting in-memory stores under `Profile::Production`.
- [ ] **R7 — Backend neutrality.** `ego-persistence-memory` contains no reference to any backend —
      no `sqlx`, Postgres, Stoolap, HTTP, or Kafka type, dependency, or feature flag — and offers no
      backend-selection surface.
- [ ] **R8 — Read-side consolidation.** `InMemoryOffsetStore` and `InMemoryDedupStore` are declared
      in `ego-persistence-memory` and no longer in `examples/reference-app`; the example consumes
      them as an ordinary dependency.
- [ ] **R9 — Compatibility re-exports at every old path.** Every path in `explore.md`'s
      COMPATIBILITY REEXPORT MATRIX still resolves, unedited, to the same item — proven at compile
      time over the full list, not by sampling. All five confirmed downstream consumer files compile
      with byte-identical source.
- [ ] **R10 — Single implementation ownership per moved port.** For `EventStore`,
      `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `ReadSideStore`, `OffsetStore`, `DedupStore`,
      and `OperationReservationStore`, `ego-persistence-memory` is the sole general-purpose in-memory
      owner; the only other declarations that survive are the two named `persistent-entity`
      duplicates (D-9) and declared test fakes.
- [ ] **R11 — Dependency integrity.** `ego-persistence-memory`'s `Cargo.toml` names exactly
      `ego-persistence-api` and `ego-domain` as workspace path dependencies and nothing else; it
      names no `ego-application`, `ego-runtime`, `ego-infrastructure`, `ego-persistence`,
      `ego-testkit`, transport, or example dependency. `cargo run -p xtask -- verify-layers` passes
      with no new violation and no matrix edit.
- [ ] **R12 — Effects scope integrity.** `crates/runtime/` and `crates/effect-store/` are unmodified;
      `InMemoryEffectStore` and its three ports are byte-identical; CORE-PERSIST-A's D-9 boundary is
      intact.
- [ ] **R13 — No Postgres refactor.** Zero SQL, migration, schema, or `crates/persistence/` file
      appears in the diff.
- [ ] **R14 — No conformance framework expansion.** No conformance harness is added, extended, or
      generalized; `assert_event_store_conformance` and the reservation lease tests keep their
      current shape and home.
- [ ] **R15 — No contract or trait redesign.** `crates/persistence-api/src/**` is unmodified apart
      from nothing at all; no port's method set, bounds, supertraits, default bodies, or
      object-safety changes.
- [ ] **R16 — No test double of any kind is promoted.** `TestClock` stays in `ego-testkit`, and no
      `#[cfg(test)]`-local or `tests/`-local double is moved into the new crate.
- [ ] **R17 — The two `persistent-entity` duplicates are named debt, not silently handled.** Both are
      recorded as KD-5 and KD-6 with named follow-up owners (F-5, F-6), and neither is moved, merged,
      fixed, nor partially addressed.
- [ ] **R18 — The effect-store boundary is named debt, not silently handled.** The future change is
      named **CORE-PERSIST-E** with its prerequisite stated (port relocation first), and nothing in
      that boundary is touched.

## Known Debt (carried, not fixed)

- **KD-1** — `ProjectionStateStore` remains dead: zero implementations, zero consumers. Carried from
  CORE-PERSIST-A (NG-10).
- **KD-4** — Conformance coverage is asymmetric: `Repository`, `Snapshot`, `OffsetStore`, and
  `DedupStore` have no harness. This is why KD-5 below was found by manual reading rather than by a
  failing test (NG-4).
- **KD-5 — `crates/persistent-entity/src/persistence.rs:733`'s `InMemorySnapshotStore` ignores
  `tenant_id` entirely.** A confirmed tenant-isolation defect: two tenants collide on the same
  `aggregate_id` (`:734,746-765`). **Not fixed here** (NG-1) → F-5.
- **KD-6 — `crates/persistent-entity/src/persistence.rs:571`'s `InMemoryEventStore` is an
  unconsolidated fork** carrying an additive `with_version_offset()` capability (`:600-611,719-727`)
  that one test depends on. **Not merged here** (NG-2) → F-6.

## Named Follow-Ups

- **F-1 — CORE-PERSIST-E**: relocate `EffectStateStore`, `EffectDedupStore`, and
  `RetentionMaintenance` out of `ego-runtime`, then consolidate `InMemoryEffectStore` into
  `ego-persistence-memory` (D-10, NG-6).
- **F-5 — Fix `persistent-entity`'s tenant-ignoring `InMemorySnapshotStore`** as a standalone
  reviewed bugfix with its own tests and a stated blast radius (KD-5). This should not wait on the
  CORE-PERSIST series.
- **F-6 — Decide the fate of `persistent-entity`'s forked `InMemoryEventStore`**: merge the
  `with_version_offset` capability into the canonical implementation, or keep the fork with a stated
  reason (KD-6).
- **F-4 — CORE-PERSIST-D**: conformance harnesses for `Repository`, `Snapshot`, `OffsetStore`, and
  `DedupStore` (KD-4, NG-4). **CORE-PERSIST-C** owns the PostgreSQL consolidation (NG-3).

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistence-memory/` | New | The whole crate: `Cargo.toml` + seven relocated implementations (IS-1, IS-2) |
| `crates/infrastructure/src/persistence/in_memory/` | Modified | Four declarations replaced by re-exports at identical paths (IS-3) |
| `crates/testkit/src/reservation.rs`, `crates/testkit/src/lib.rs` | Modified | Store relocated; `TestClock` and colocated tests stay; re-export added (D-8, IS-3) |
| `examples/reference-app/src/read_side/store.rs` | Modified | Two declarations removed; imports retargeted (IS-4, D-6) |
| `layers.toml`, root `Cargo.toml` | Modified | One layer entry, one workspace member (IS-1) |
| `xtask/src/layers.rs` | Untouched | No matrix change required (D-2) |
| `crates/persistence-api/` | Untouched | No contract change (NG-5, R15) |
| `crates/persistent-entity/`, `crates/runtime/`, `crates/effect-store/`, `crates/persistence/` | Untouched | Deferred (NG-1, NG-2, NG-3, NG-6) |
| `openspec/specs/{persistence-memory-adapter,persistence-api-surface}/spec.md` | New / Modified | Deltas per IS-6 |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| R-1 | **D-7's reachability change is approved by omission** — the reservation store silently becomes wireable in production without anyone deciding it should be | Med | D-7 states it as a standalone decision with its own rationale, and R11 pins the resulting dependency set. If a reviewer rejects it, the correct outcome is dropping item 7 from IS-2, not weakening D-7 |
| R-2 | **A moved body drifts** — a lock, a tenant key, or a version term altered inside a diff too wide to read line by line | Med | D-4 makes verbatim a rule; R5 makes it checkable as a text comparison rather than a judgement call |
| R-3 | **The import closure is wider than D-3 priced**, and apply discovers a third needed edge mid-move | Med | `Clock` was already found this way and named (D-3). `design.md` fixes the exact import list against source before any file moves. A third edge is a design decision, not an apply-time improvisation |
| R-4 | **Review budget.** Seven implementations across three vacating crates will exceed the 400-line budget | High | Forecast, not hidden. `sdd-tasks` must slice by source crate (infrastructure → testkit → reference-app), each slice keeping the re-export layer intact so every intermediate state compiles workspace-wide |
| R-5 | **`persistence-api-surface`'s change-scoped phrasing is read as a standing prohibition**, making this change look like a spec violation | Med | Named in Capabilities → Modified. The fix is re-scoping two statements to CORE-PERSIST-A; no requirement about port shape or path resolution changes. If the spec phase disagrees, this becomes a blocking question, not a silent reinterpretation |
| R-6 | **`layers.toml`'s header comment is already stale** — line 6 reads `domain → nothing`, but `xtask/src/layers.rs:76` grants the domain self-edge since CORE-PERSIST-A's D-4. A reader may use the comment to judge D-2 | Low | Noted, not fixed here (NG-7). The executable matrix is authoritative. Worth a one-line follow-up outside this change |
| R-7 | **A re-export is missed for a path no test exercises**, breaking a consumer only at their build | Med | IS-5 requires a compile-time proof over the full COMPATIBILITY REEXPORT MATRIX, not spot checks; `cargo build --workspace` covers every in-tree consumer |

## Rollback Plan

**One revert commit, at any point, with zero external breakage.**

Because every vacated path is re-exported (D-5), no crate outside the new crate and its two vacating
crates depends on the new layout. Reverting is: drop `crates/persistence-memory/`, restore the
`ego-infrastructure` and `ego-testkit` declarations from the pre-change tree, restore the reference
app's two declarations and its imports, remove the `layers.toml` entry and the workspace member.
Nothing else is touched in either direction, and `xtask/src/layers.rs` never changed, so there is
no gate state to unwind.

This holds **mid-flight**, which is what makes R-4's slicing safe: a partially-landed CORE-PERSIST-B
is a workspace where some in-memory implementations happen to live in a second crate and every
consumer still compiles unchanged. The one exception is the reference app (D-6), which has no
re-export shim — its revert is a two-file import restoration, contained to a leaf example with no
downstream consumer.

No data, schema, migration, or persisted state is involved in either direction. This change writes
nothing at runtime.

## Dependencies

- `persistence-api-surface` (shipped, CORE-PERSIST-A) — the ports this crate implements, consumed
  unchanged except for the documentation re-scoping named in Capabilities.
- `foundation-integrity` (archived) — FR-001 (completeness), FR-002 (direction), FR-003 (no cycles),
  FR-005 (isolated compilation), consumed unchanged.
- `openspec/config.yaml` design rule "No circular dependencies between crates" — upheld by
  construction (D-2, D-3).
- `explore.md`'s MOVE MATRIX and COMPATIBILITY REEXPORT MATRIX, with D-3's correction applied.
- No new external crate, service, or infrastructure.

## Proposal question round

This proposal was produced without an interactive round. Four product questions would sharpen it;
until they are answered, the stated assumption applies.

1. **D-7 reachability** — is making `InMemoryOperationReservationStore` production-wireable an
   intended outcome, or should it stay unreachable from production code until an operator story
   exists for it? *Assumption: intended — it is the port's only implementation and claims production
   fidelity in its own doc comment.*
2. **Who is the customer of the new crate?** Framework adopters writing tests against real ports, or
   also small production deployments that genuinely want in-memory persistence? *Assumption: adopters
   and tests; production use stays blocked by `Profile::Production` (D-11).*
3. **KD-5 urgency** — the tenant-isolation defect is real and shipped. Should F-5 be scheduled ahead
   of the rest of the CORE-PERSIST series rather than after it? *Assumption: it is independent and
   should not wait, but this proposal does not schedule it.*
4. **D-6 example policy** — should `examples/reference-app` be prohibited from declaring generic
   adapter implementations going forward, as a standing rule, or is this a one-time cleanup?
   *Assumption: one-time cleanup; no standing rule is proposed here.*
