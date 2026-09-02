# Proposal: CORE-PERSIST-A — Unified Persistence API Surface (Domain-Owned Ports)

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers).

## Objective

Give every domain-owned persistence port one owning crate. Today the persistence port
vocabulary is scattered across `ego_domain::persistence::*`, `ego_domain::read_side::*`, and
`ego_domain::operation::*` with no crate boundary distinguishing a port from the rest of the
domain model. CORE-PERSIST-A relocates that vocabulary into a new `ego-persistence-api` crate,
with `ego-domain` re-exporting every item at its exact current path so no consumer outside the
two crates changes a single line.

**Amended after `sdd-design`**: "that vocabulary" is wider than this proposal originally priced.
`design.md` found the moved ports do not compile in isolation — `EventTag`, `ProjectionState`,
`EventStreamElement`, and `DomainEvent` are pulled in by the ports' own signatures, and the
`id_type!` macro that generates `TenantId` is shared with four unrelated domain identity types.
See the resolved **OD-1 / OQ-1 / OQ-2** section below and design.md's AD-2/AD-3 for the full
closure and the reasoning for each addition.

## Intent

**The problem is ownership, not behavior.** A port has no home. `EventStore` sits beside
`AggregateRoot`; `OffsetStore` sits beside `EventTag`; a reader cannot tell from the module
tree which of them a persistence adapter is obliged to implement. Explore §10 finding 5 shows
where that ends: `crates/runtime/src/effects/store.rs` defines three ports, their whole
contract-type vocabulary, and a working in-memory implementation in one 1320-line file, in a
crate that is not `ego-domain` at all.

**This change is purely structural.** No SQL, no signature, no async/sync, no object-safety, no
runtime behavior changes. Every relocated item moves verbatim — doc comments and colocated unit
tests included — and is re-exported at its old path. The only thing a consumer can observe is
that the same items now also resolve under `ego_persistence_api::*`.

**It makes exactly one architecture decision** (D-2): `ego-domain` may depend on
`ego-persistence-api`. That decision is not a convenience — it is forced, and the evidence is in
§D-2. Naming it here is the point of doing this slice separately.

## Active Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | **New crate `ego-persistence-api` at `crates/persistence-api/`, mapped to the `domain` layer in `layers.toml`** | Matches the `ego-*` package convention and stays distinct from the existing `ego-persistence` (Postgres adapters). Ports are domain artifacts in hexagonal architecture, so `domain` is the honest layer. `persistence-postgres` / `persistence-memory` renames are CORE-PERSIST-B/C's job, not this change's |
| D-2 | **The dependency edge `ego-domain → ego-persistence-api` is forced, not chosen.** Domain code that stays behind already consumes the moved ports: `crates/domain/src/read_side/scheduler.rs:5-10` imports `DedupStore`, `OffsetStore`, and `ReadSideStore`, and `session.rs`/`runner.rs` do the same | This is a real compile-time dependency independent of the re-export requirement. The reverse edge is therefore impossible — Cargo forbids circular crate dependencies outright, and FR-003 forbids them again. Direction is settled by the code, not by preference |
| D-3 | **D-2 has an unresolved consequence, and this proposal names it rather than assuming it away: `ego-persistence-api` MUST NOT depend on `ego-domain`, yet three moved items reference domain types that stay behind** — `DomainEvent` (`persistence/event_store.rs:3,47,186`) and `TenantId` (`operation/receipt.rs:11`, `operation/reservation.rs:32`) | Explore §5 treated this as conditional ("only if any moved port needs a domain type"). It is not conditional; it is confirmed. **`design.md` MUST resolve it before any code moves.** Candidate routes are listed under **Open Decision OD-1**. Choosing among them is a design decision with real tradeoffs, not a mechanical one |
| D-4 | **The `foundation-integrity` gate must be relaxed for a same-layer `domain → domain` edge.** `xtask/src/layers.rs:76` reads `"domain" => Some(&[])` — a `domain` crate may depend on **nothing**, including another `domain` crate. Any new `ego-domain` edge hard-fails FR-002 today | The relaxation is the narrowest one available: `Some(&[])` → `Some(&["domain"])`, matching the self-edge the matrix already grants `foundation` (`["domain","foundation"]`) and `infrastructure`. Mapping the new crate to `foundation` instead would be a far wider hole — it would legalize `ego-domain → ego-runtime`. FR-003 and Cargo still block the cycle, so the relaxation cannot be abused into an inversion |
| D-5 | **Every relocated item is re-exported at its exact old path.** Zero `use` statements change outside the two crates; zero `Cargo.toml` files gain an edge outside `ego-domain` and `ego-persistence-api` | Explore §6 counted 92 files resolving these items by exact module path. Re-exporting is what makes this change revertible mid-flight (see Rollback Plan) and what keeps it reviewable |
| D-6 | **Relocation is verbatim.** Doc comments, `#[cfg(test)]` modules, and blanket impls move with their item and are not rewritten, reorganized, or "improved" | The `Arc<T>` blanket-forwarding impls for `OffsetStore`/`DedupStore` (`offset.rs:92`, `dedup.rs:60`) are load-bearing: losing a forward silently reclassifies every registered durable pair as volatile (explore §7). Verbatim movement is the only way to guarantee no semantic drift in a diff this wide |
| D-7 | **Explore §9's matrix is a floor, not a ceiling.** Every item in a relocated module moves, including ones §9 omits (`OperationId`, `OwnerId`, `StoredServiceResponse`, `OperationKeyError`, `OperationKeyHash`, `AggregateOutcomeError`, `ProjectionStateStoreError`, listed in explore §3 but absent from §9) | A partially-moved module does not compile. §9 also disagrees with §11 on the count: §11 says "rows 1–26", direct count of the domain-owned rows is **27**. The matrix is authoritative over the summary; the exact set is fixed during `sdd-design` |
| D-8 | **`ProjectionStateStore` relocates as-is**, marked as known debt, not deleted | Explore §10 finding 3: zero implementations, zero consumers. Deleting it would make this change a behavior change wearing a reorg's name. Debt is named (KD-1), not silently cleaned up |
| D-9 | **The three `ego-runtime`-owned effect-store ports are entirely deferred.** `ego-runtime` and `ego-effect-store` are not touched | Explore §11: relocating them either leaves `InMemoryEffectStore` depending on a port defined elsewhere (a *new* dependency-direction decision) or requires moving an implementation (prohibited here). That is a second architecture decision and belongs to its own change (F-1) |

## Open Decisions — resolved by `design.md`, confirmed by the change owner

**OD-1 — resolved: relax the layer gate, do not add a crate, do not re-scope.**
`ego-persistence-api` reaches `DomainEvent`/`TenantId` by staying a leaf and letting
`ego-domain` depend on it (D-2), with `allowed_layers("domain")` relaxed from `Some(&[])` to
`Some(&["domain"])` (D-4). The other two routes from the original four — a third shared-type
leaf crate, and re-scoping A1 to leave `EventStore`/`Repository` behind — were rejected as,
respectively, a second new crate the slice's budget cannot absorb, and a split that defeats the
objective. Design.md AD-1 has the full record.

**OQ-1 — resolved: accept the widened closure.** Design found the compile closure is five
types, not the two D-3 named: `EventTag`, `ProjectionState`, `EventStreamElement` (already
read-side port vocabulary — `OffsetStore`/`DedupStore`/`ReadSideStore`/`ProjectionStateStore`
are keyed or yield on them), plus `DomainEvent` itself (the one item whose home this change
makes worse, moved only because `EventStore<E: DomainEvent>` cannot compile without it). All
four relocate verbatim, re-exported at their old paths, same as every other item. No OOS is
violated — no signature, no new type, no implementation moves. Design.md AD-2 has the full
record.

**OQ-2 — resolved: relocate the `id_type!` macro, one generator.** `TenantId`'s generator
(`context.rs:7-54`) is shared with four unrelated domain identity types (`AggregateId`,
`EntityId`, `CorrelationId`, `CausationId`, `RequestId`). The macro moves to
`ego-persistence-api` as `#[macro_export]`; `ego-domain` invokes the re-exported macro (via the
OD-1 edge) to keep generating the other five types locally. Duplicating the macro instead was
rejected — it is the only non-verbatim route available and violates D-6. Design.md AD-3 has the
full record.

## Atomicity Gate

**Run, and it already cut scope once.** CORE-PERSIST-A originally bundled the `ego-runtime`
effect-store ports; explore §11 found that this bundled two independent architecture decisions
and the split was taken (D-9 → F-1).

What remains is one indivisible move. The relocated items (35 per design.md EC-4's direct
enumeration, superseding this count) share one destination crate, one
dependency-direction decision (D-2), one gate relaxation (D-4), and one re-export layer (D-5).
Relocating a subset would leave the port vocabulary split across two crates, which is the exact
condition this change exists to end.

**ATOMICITY: PASS** — with the caveat that OD-1 is an open *design* question inside an atomic
scope, not a hidden second change.

## Scope

**Boundary at a glance**

| | |
|---|---|
| **CORE-PERSIST-A includes** | New `ego-persistence-api` crate · 35 domain-owned ports and contract types relocated verbatim (design.md EC-4) plus the OQ-1/OQ-2 closure additions above · re-exports at every old path · `layers.toml` entry · `foundation-integrity` direction relaxation |
| **CORE-PERSIST-A excludes** | Every implementation · `ego-runtime` / `ego-effect-store` · any SQL, migration, signature, or behavior change · every known-debt item |

### In Scope

- **IS-1** — A new `ego-persistence-api` crate at `crates/persistence-api/`, package name
  `ego-persistence-api`, laid out per explore §8 minus the `effects/` subtree (D-1, D-9).
- **IS-2** — Verbatim relocation of every domain-owned port and contract type from
  `crates/domain/src/{persistence,read_side/{offset,dedup,store,projection_state_store},operation/{reservation,key,receipt}}`
  into that crate (D-6, D-7). **Amended by OQ-1**: also includes
  `read_side/{event_tag,state,event_stream}.rs` and `event.rs` (`DomainEvent`), forced into the
  crate by the moved ports' own signatures — see the resolved Open Decisions section above.
- **IS-2b** (new, OQ-2) — The `id_type!` macro (`context.rs:7-54`) relocates verbatim and gains
  `#[macro_export]`; `TenantId`/`TenantIdError` are generated in `ego-persistence-api`.
  `ego-domain` re-invokes the macro for its four other identity types and re-exports
  `TenantId`/`TenantIdError` at their existing `ego_domain::context::*` / `ego_domain::*` paths.
- **IS-3** — A `pub use` re-export in `ego-domain` at every relocated item's exact current path
  (D-5), including the crate-root re-exports `ego-domain` already publishes.
- **IS-4** — `ego-domain`'s own internal consumers (`read_side/scheduler.rs`, `session.rs`,
  `runner.rs`) rewired to the new crate, plus the one new `Cargo.toml` edge (D-2).
- **IS-5** — A `layers.toml` entry mapping `ego-persistence-api` to `domain` (FR-001), and the
  `allowed_layers("domain")` relaxation from `Some(&[])` to `Some(&["domain"])` in
  `xtask/src/layers.rs:76` with its covering unit test (D-4).
- **IS-6** — A compile-time proof that every old path still resolves to the same item — not to a
  re-declared same-named copy.
- **IS-7** — Spec deltas per the Capabilities section.

### Out of Scope

Every item below is a **non-goal**, not an oversight. Several are named debt with an owner.

- **OOS-1 — No implementation moves.** `InMemoryEventStore`, `InMemoryRepository`,
  `InMemorySnapshotStore`, `InMemoryReadSideStore`, `InMemoryOperationReservationStore`, and every
  `PostgreSQL*`/`Postgres*` adapter stay exactly where they are. Only port traits and contract
  types move.
- **OOS-2 — `ego-runtime` and `ego-effect-store` are not touched at all.** `EffectStateStore`,
  `EffectDedupStore`, and `RetentionMaintenance` stay in `crates/runtime/src/effects/store.rs`
  (D-9 → F-1).
- **OOS-3 — No SQL, migration, index, transaction, retry, error-classification, connection-pool,
  or durability change of any kind.**
- **OOS-4 — No method-signature, async/sync, `Send`/`Sync`, or object-safety change.** A trait's
  shape after this change must be byte-identical to its shape before, modulo module path.
- **OOS-5 — No tenant-semantics change.** `resolve_tenant`'s three-way rule
  (`None` / `Some("")` / `Some(t)`) moves verbatim and is not revisited.
- **OOS-6 — No production runtime behavior change.** Nothing in this change is observable at
  runtime.
- **OOS-7 — No crate merges.** No existing crate is folded into another.
- **OOS-8 — No new capability.** Not one trait, method, or type that does not exist today is
  added.
- **OOS-9 — `ProjectionStateStore` is not removed** (D-8 → KD-1).
- **OOS-10 — No generic SQL abstraction, dialect engine, query builder, or ORM-shaped construct.**
- **OOS-11 — No Oracle, MySQL, or any other backend work.**
- **OOS-12 — The confirmed `crates/persistence/src/postgres/repository.rs` defect is not fixed
  here** (KD-2).
- **OOS-13 — `crates/persistent-entity/src/types.rs` is not deleted or wired in** (KD-3).
- **OOS-14 — No conformance harness is added** for the capabilities that lack one
  (`Repository`, `Snapshot`, `OffsetStore`, `DedupStore`) — explore §10 finding 4, owned by
  CORE-PERSIST-D/E.

## Capabilities

### New Capabilities

- `persistence-api-surface`: the observable contract that the domain-owned persistence port
  vocabulary has exactly one owning crate, and that every path a consumer resolves today keeps
  resolving to the same item.

### Modified Capabilities

- `foundation-integrity`: FR-002's direction rule currently permits a `domain` crate zero
  dependencies. It must admit a same-layer `domain → domain` edge, matching the self-edge the
  matrix already grants `foundation` and `infrastructure` (D-4).

If the spec phase finds an existing requirement already implies one of these, it folds rather
than manufacturing a delta.

## Approach

Create the crate, move each module's file verbatim, replace the vacated `ego-domain` module with
a `pub use ego_persistence_api::…::*;` re-export at the identical path, and add the single
`Cargo.toml` edge. Rewire only `ego-domain`'s own internal port consumers. Nothing outside the
two crates is edited — not a `use` statement, not a `Cargo.toml`, not a test.

Order matters for reviewability: resolve OD-1 first (it may change what "verbatim" can mean for
`EventStore` and `OperationReceipt`), then move, then re-export, then relax the gate. The gate
relaxation lands with the edge that needs it, never before.

## Known Debt (carried, not fixed)

Each item is recorded so it has a named owner rather than an implicit one.

- **KD-1 — `ProjectionStateStore` is dead.** Zero implementations, zero consumers anywhere in the
  workspace (explore §10 finding 3). Relocated as-is (D-8). Removal belongs to whichever change
  decides the read-side port set.
- **KD-2 — `crates/persistence/src/postgres/repository.rs` carries a confirmed two-part defect.**
  Lines 82, 135, 161 use `tenant_id = $2` where every sibling adapter in the same crate correctly
  uses `IS NOT DISTINCT FROM $2`, so the systemwide (`NULL`) tenant partition is mishandled.
  Worse, line 109's `INSERT … ON CONFLICT (aggregate_id, tenant_id)` targets a constraint that
  **does not exist** — migration `002_create_aggregates.sql` declares `aggregate_id VARCHAR(255)
  PRIMARY KEY` alone — which Postgres rejects with `42P10`. This is a live runtime-failure and
  tenant-isolation risk, not cosmetic. **Not fixed here** (OOS-12), and it should not wait on the
  CORE-PERSIST series: F-2.
- **KD-3 — `crates/persistent-entity/src/types.rs` is dead code with an internal duplication.**
  Never referenced by a `mod` declaration, and self-duplicates `EntityTriple`, `EntityId`, and
  `ExecutionKey` (lines 18/122, 52/143, 85/168) — a hard `E0428` if it were ever wired in. It
  also aliases `TenantId = String`, colliding by name with `ego-domain`'s validated newtype.
  Not deleted (OOS-13): F-3.
- **KD-4 — Conformance coverage is asymmetric.** `Repository`, `Snapshot`, `OffsetStore`, and
  `DedupStore` have no conformance harness anywhere, despite the `is_durable()` default being a
  documented landmine. Owned by CORE-PERSIST-D/E (OOS-14).

## Required Semantics

```
Given any crate that today compiles `use ego_domain::persistence::EventStore;`
When it is compiled after this change with that statement unedited
Then it MUST compile, and the resolved item MUST be the same trait — not a
     re-declared copy that merely shares the name.

Given the same holds for every path in explore.md §9's domain-owned rows
When the workspace is built
Then no crate outside ego-domain and ego-persistence-api has an edited `use`
     statement or an added Cargo.toml dependency.

Given a trait relocated by this change
When its post-change definition is compared to its pre-change definition
Then every method signature, bound, supertrait, and default body MUST be
     identical, differing only in module path.

Given the OffsetStore and DedupStore Arc<T> blanket-forwarding impls
When a store is registered behind an Arc after this change
Then is_durable() MUST still forward to the inner store, and a durable pair
     MUST NOT be reclassified as volatile.

Given the foundation-integrity gate
When it runs against the post-change workspace
Then it MUST pass: ego-persistence-api is mapped, the ego-domain edge is
     permitted, no cycle exists, and every crate compiles in isolation.

Given the workspace test suite
When `cargo test --workspace` runs after this change
Then it MUST show zero new failures and zero changed assertions.
```

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistence-api/` | New | The whole crate: `Cargo.toml` + relocated modules (IS-1, IS-2) |
| `crates/domain/src/{persistence,read_side,operation}/` | Modified | Definitions replaced by re-exports at identical paths (IS-3) |
| `crates/domain/src/read_side/{scheduler,session,runner}.rs` | Modified | Imports rewired to the new crate (IS-4) |
| `crates/domain/Cargo.toml` | Modified | One new edge (D-2) |
| `layers.toml` | Modified | One entry: `"ego-persistence-api" = "domain"` (IS-5) |
| `xtask/src/layers.rs:76` | Modified | `allowed_layers("domain")` relaxation + covering test (D-4, IS-5) |
| `Cargo.toml` (workspace members) | Modified | New member |
| `crates/{infrastructure,persistence,runtime,testkit,service-sdk,persistent-entity}`, `examples/reference-app`, `integration-tests` | Untouched | Protected by the re-exports (D-5) |
| `crates/runtime/src/effects/store.rs`, `crates/effect-store/` | Untouched | Deferred (OOS-2) |
| `openspec/specs/{persistence-api-surface,foundation-integrity}/spec.md` | New / Modified | Deltas per IS-7 |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| R-1 | **OD-1 has no answer that fits this slice's prohibitions**, and apply discovers it mid-move | High | OD-1 is a named blocking decision for `design.md`, explicitly ahead of any code movement (Approach). If design cannot close it, the correct outcome is re-scoping, not breaking OOS-4 or OOS-8 quietly |
| R-2 | A relocated item drifts — a bound dropped, a default body altered, a blanket impl lost — inside a diff too large to read line by line | Med | D-6 makes relocation verbatim a rule, not a preference. The `Arc<T>` forwarding impls have their own Required Semantics clause and SC-4, because losing one is silent (explore §7) |
| R-3 | The `foundation-integrity` relaxation (D-4) is later read as "domain crates may depend downward", and a genuine inversion slips through | Med | The relaxation is the narrowest available — same-layer only. FR-003 and Cargo both still forbid the cycle, so `ego-persistence-api → ego-domain` remains impossible by construction, not by review vigilance. SC-7 asserts the matrix admits `domain → domain` and nothing wider |
| R-4 | **Review budget.** Explore §11 estimates 1,500–2,000 relocated lines against a 400-line budget | High | Accepted and forecast, not hidden. `sdd-tasks` must slice this into chained PRs by capability group (`persistence/`, `read_side/`, `operation/`), each with the re-export layer intact so every intermediate slice compiles workspace-wide. The re-export design (D-5) is what makes chaining safe |
| R-5 | The exact move set is wrong — §9's matrix omits items §3 lists, and §11's count disagrees with §9's | Med | Named outright (D-7). The authoritative set is "every public item in a relocated module", fixed during `sdd-design` against the source, never against the summary |
| R-6 | A re-export is missed for a path no test exercises, breaking a downstream consumer only at their build | Med | IS-6 requires a compile-time proof over the full path list, not spot checks. `cargo build --workspace` plus FR-005 isolation compilation covers the in-tree consumers |

## Named Follow-Ups (deliberately not folded in)

- **F-1 — CORE-PERSIST-A2 (or folded into CORE-PERSIST-B) — relocate the `ego-runtime`-owned
  effect-store ports.** `EffectStateStore`, `EffectDedupStore`, `RetentionMaintenance` and their
  contract-type vocabulary. The change must decide explicitly whether `ego-runtime` keeps
  `InMemoryEffectStore` and gains a new dependency, or the convenience implementation moves too.
  It also gets to ask whether `ego-effect-store → ego-runtime` survives at all once its stated
  reason for existing is gone (D-9, OOS-2).
- **F-2 — Fix `PostgreSQLRepository`'s tenant scoping and `ON CONFLICT` target** (KD-2).
  **This should not wait on the CORE-PERSIST series.** The `42P10` exposure is a live
  runtime-failure path and the tenant-scoping half is an isolation defect; both warrant a
  standalone bugfix with its own tests, scheduled independently.
- **F-3 — Delete or repair `crates/persistent-entity/src/types.rs`** (KD-3).
- **F-4 — CORE-PERSIST-D/E — conformance harnesses** for `Repository`, `Snapshot`, `OffsetStore`,
  and `DedupStore`, and the eventual `persistence-testkit` home (KD-4).

## Rollback Plan

**One revert commit, at any point, with zero external breakage.**

Because every relocated item is re-exported at its exact old path (D-5), no crate outside
`ego-domain` and `ego-persistence-api` ever depends on the new layout. Reverting is therefore:
drop `crates/persistence-api/`, restore the `ego-domain` modules from the pre-change tree, remove
the one `Cargo.toml` edge, remove the `layers.toml` entry, and restore
`allowed_layers("domain")` to `Some(&[])`. Nothing else is touched in either direction.

This holds **mid-flight** too, which is what makes R-4's chained-PR slicing safe: each slice keeps
the re-export layer intact, so a partially-landed CORE-PERSIST-A is a workspace where some ports
happen to live in a second crate and every consumer still compiles unchanged. There is no
intermediate state that requires a coordinated multi-crate revert.

No data, schema, migration, or persisted state is involved in either direction — this change
writes nothing at runtime.

## Dependencies

- `foundation-integrity` (archived) — FR-001, FR-002, FR-003, FR-005 and the `xtask verify-layers`
  gate. FR-002 is modified by this change; the rest are consumed unchanged.
- `openspec/config.yaml` design rule "No circular dependencies between crates" — upheld by
  construction (D-2).
- The explore artifact `explore.md` §9 move/reexport matrix, with D-7's correction applied.
- No new external dependency, crate, service, or infrastructure.

## Success Criteria

- [ ] **SC-1** — Every domain-owned path in explore §9's matrix still resolves, unedited, to the
      same item. Proven at compile time over the full list (IS-6), not by sampling.
- [ ] **SC-2** — No crate outside `ego-domain` and `ego-persistence-api` has an edited `use`
      statement or an added `Cargo.toml` dependency. Verifiable from the diff alone.
- [ ] **SC-3** — Every relocated trait's method signatures, bounds, supertraits, and default
      bodies are identical to their pre-change text, differing only in module path (OOS-4).
- [ ] **SC-4** — The `OffsetStore` and `DedupStore` `Arc<T>` blanket-forwarding impls moved intact;
      `is_durable()` still forwards, and no durable pair is reclassified as volatile.
- [ ] **SC-5** — `cargo build --workspace` and `cargo test --workspace` pass with zero new failures
      and zero changed assertions.
- [ ] **SC-6** — `cargo run -p xtask -- verify-layers` passes: `ego-persistence-api` is mapped
      (FR-001), the `ego-domain` edge is permitted (FR-002), no cycle exists (FR-003), and the new
      crate compiles in isolation (FR-005).
- [ ] **SC-7** — The direction matrix admits `domain → domain` and nothing wider. A test asserts
      that `domain → foundation`, `domain → infrastructure`, and `domain → sdk` still fail.
- [ ] **SC-8** — Zero SQL, migration, or schema file appears in the diff (OOS-3).
- [ ] **SC-9** — `crates/runtime/`, `crates/effect-store/`, and every implementation struct named
      in OOS-1 are unmodified (OOS-1, OOS-2).
- [ ] **SC-10** — OD-1 is closed in `design.md` with a stated decision and rationale before any
      code moves, and the chosen route violates none of OOS-1 through OOS-14.
- [ ] **SC-11** — KD-1 through KD-4 appear as named debt with named follow-up owners, and none of
      them is fixed, deleted, or partially addressed in this change.
- [ ] **SC-12** — `sdd-tasks` produces a chained-PR plan whose every slice compiles workspace-wide
      on its own, with the re-export layer intact at each step (R-4).
