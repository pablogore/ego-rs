# Design: CORE-PERSIST-A — Unified Persistence API Surface (Domain-Owned Ports)

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers).
>
> **Inputs**: `proposal.md` (D-1 … D-9, OD-1, IS-1 … IS-7, OOS-1 … OOS-14, KD-1 … KD-4,
> R-1 … R-6, F-1 … F-4, SC-1 … SC-12) and `explore.md` (§1, §3, §5, §8, §9, §11). This
> document decides **how**: dependency direction, crate contents, re-export granularity,
> gate relaxation, and slice boundaries. Observable requirements are `spec.md`'s and are
> not restated here.
>
> **Baseline read**: `develop` @ `885d1da`. Every file:line below was read on this
> baseline, not recalled from the inputs.

## Technical Approach

One new leaf crate, one one-directional edge, one gate relaxation, and a re-export layer at
**module** granularity so that no `use` statement changes anywhere — including inside
`ego-domain` itself.

`ego-persistence-api` depends on **no workspace crate at all**. `ego-domain` depends on it
and republishes every relocated module at its exact former path. That direction is forced
by code that stays behind (`read_side/scheduler.rs:5-10`, `session.rs:5-13`,
`runner.rs:3-10` all consume `DedupStore` / `OffsetStore` / `ReadSideStore`), and the
reverse edge is impossible once it holds — Cargo rejects the cycle before
`foundation-integrity` ever runs.

**No sequence diagram is included, and that is a deliberate applicability call.**
`openspec/config.yaml`'s design rule asks for one on complex async flows. This change adds,
removes, and reorders zero call paths: every `#[async_trait]` item moves with a
byte-identical signature (OOS-4), so any diagram drawn here would depict a flow the change
does not touch. The load-bearing structure is the **dependency graph**, given below.

---

## Evidence Corrections

Four, all found by reading the baseline rather than the inputs. Each changes what the
implementation must do.

### EC-1 — The shared-type closure is **five** types, not the two D-3 names

D-3 states that `ego-persistence-api` must reach `DomainEvent` and `TenantId`. A grep of
`^use crate::|^use super::` across the ten files IS-2 relocates finds three more, all in
`read_side/` but outside IS-2's file list:

| Type it needs | Needed by | Defined at |
|---|---|---|
| `DomainEvent` | `persistence/event_store.rs:3` | `event.rs:47` |
| `TenantId` | `operation/receipt.rs:11`, `operation/reservation.rs:32` | `context.rs:56` |
| **`EventTag`** | `read_side/offset.rs:6`, `dedup.rs:6`, `store.rs:7`, `projection_state_store.rs:8` | `read_side/event_tag.rs:12` |
| **`ProjectionState`** | `read_side/projection_state_store.rs:9` | `read_side/state.rs:16` |
| **`EventStreamElement`** | `read_side/store.rs:6` | `read_side/event_stream.rs:13` |

Under AD-1's one-way edge, every one of these must be inside `ego-persistence-api`. OD-1
was priced against two types; it must be re-priced against five. **AD-2 resolves this.**

### EC-2 — `TenantId` is macro-generated, so "relocate `TenantId`" is not a file move

`context.rs:7` defines `macro_rules! id_type!`; line 56 generates `TenantId` /
`TenantIdError` from it, and line 55 and its siblings generate `EntityId`, `AggregateId`,
`CorrelationId`, `CausationId`, and `RequestId` from the same generator. There is no
`TenantId` file to move. D-6 requires verbatim relocation, and hand-expanding a macro at the
destination is not verbatim. **AD-3 resolves this.**

### EC-3 — R-4's chained-PR order is backwards; `persistence/` depends on `operation/`

R-4 recommends slicing `persistence/` → `read_side/` → `operation/`. The source says the
first arrow points the other way:

- `persistence/event_store.rs:4` → `crate::operation::OperationReceipt`
- `persistence/stored_event.rs:1` → `crate::operation::OperationKey`
- `operation/key.rs` has **zero** `crate::` / `super::` imports — it is the closure's floor.
- `read_side/`'s four relocated files reference nothing in `persistence/` or `operation/`.

Correct order is `read_side/` → `operation/` → `persistence/`. **AD-6 lays out the slices.**

### EC-4 — The authoritative item count is **35**, not 27

27 is the count of explore §9's domain-owned rows. D-7 names 7 items §9 omits (→ 34). Direct
enumeration of `^pub (trait|struct|enum|fn|type|const)` across the ten relocated files yields
**35**: `operation/key.rs:19` declares `pub const MAX_LEN: usize = 255`, public at
`ego_domain::operation::key::MAX_LEN` and named by neither §9 nor D-7. Per D-7 the rule —
"every public item in a relocated module" — governs, and 35 is the number IS-6's compile-time
proof must cover.

---

## Dependency Graph

**Before** — `ego-domain` is a leaf with no internal dependencies (`crates/domain/Cargo.toml:6-17`
names only external crates):

```
                    ego-domain  [domain]   ← leaf
                        ▲
   ┌────────┬───────────┼───────────┬──────────┬────────────┐
ego-application  ego-persistence  ego-runtime  persistent-entity  …
```

**After** — one new leaf below it, one new edge:

```
              ego-persistence-api  [domain]   ← leaf: no workspace dependency
                        ▲
                        │  the only new edge, one direction only
                    ego-domain     [domain]
                        ▲
   ┌────────┬───────────┼───────────┬──────────┬────────────┐
ego-application  ego-persistence  ego-runtime  persistent-entity  …
                     (all unchanged — protected by the re-export layer, D-5)
```

**No cycle is introduced, and this is checkable rather than asserted.**
`crates/persistence-api/Cargo.toml` names no `path =` dependency, so the reverse edge does
not exist as a fact about the file, not as a review promise. Were anyone to add it, Cargo
refuses to resolve the workspace before `xtask verify-layers` runs, and FR-003's cycle check
refuses it again. AD-1's gate relaxation widens the *layer matrix*, never the *crate graph*.

---

## Architecture Decisions

### AD-1 — Direction: `ego-domain → ego-persistence-api`; `allowed_layers("domain")` relaxes to `Some(&["domain"])`

**Decision** — `xtask/src/layers.rs:76`:

```rust
"domain" => Some(&["domain"]),   // was: Some(&[])
```

plus `layers.toml`: `"ego-persistence-api" = "domain"`, and one line in
`crates/domain/Cargo.toml`.

**Criteria**:

1. **The edge is forced, not chosen.** `read_side/scheduler.rs:5-10` imports `DedupStore`,
   `OffsetStore`, and `ReadSideStore`; `session.rs:5-13` and `runner.rs:3-10` do the same.
   Those three files stay in `ego-domain` and consume ports that leave it. The edge exists
   independently of D-5's re-export convenience — deleting the re-export requirement would
   not remove it.
2. **The relaxation is the narrowest one available.** `Some(&["domain"])` is a same-layer
   self-edge, exactly the shape the matrix already grants `foundation`
   (`Some(&["domain","foundation"])`, `layers.rs:77`) and `infrastructure`
   (self-referencing, `layers.rs:80-86`). It admits `domain → domain` and nothing else:
   `domain → foundation`, `domain → infrastructure`, `domain → sdk` all still fail
   `check_direction` (`layers.rs:107-116`), which SC-7 asserts.
3. **It is not the "no circular dependencies between crates" rule being bent.** That rule is
   about the crate graph, which stays a DAG (see Dependency Graph). The layer matrix is a
   *direction* rule over layers, and a same-layer edge is not a direction violation — it is
   the case the matrix previously had no way to express, because `ego-domain` was the only
   `domain` crate in the workspace. Mapping the new crate to `foundation` instead would be a
   far wider hole: it would legalize `ego-domain → ego-runtime`.

**Considered and rejected** (both are OD-1 routes, named here for the record and designed
nowhere in this document):

- *A third leaf crate holding the shared value types, depended on by both* — correct-shaped,
  but adds a second new crate to a slice already over its review budget (R-4).
- *Re-scoping A1 to leave `EventStore` / `Repository` behind* — splits the port vocabulary
  across two crates, which is the exact condition this change exists to end.
- *Relaxing `EventStore<E>`'s `DomainEvent` bound* — a signature change, prohibited by OOS-4.

### AD-2 — The five EC-1 types relocate with the ports; `ego-persistence-api` stays closed under compilation

**Decision**: `read_side/event_tag.rs`, `read_side/state.rs`, `read_side/event_stream.rs`,
and `event.rs` relocate verbatim alongside the ports, re-exported at their old paths like
everything else. `TenantId` is AD-3.

**Criteria**:

1. **Three of the five are read-side port vocabulary already.** `OffsetStore`, `DedupStore`,
   `ReadSideStore`, and `ProjectionStateStore` are *keyed* on `EventTag`; `ReadSideStore`
   *yields* `EventStreamElement`. Their home was already wrong — this design does not widen
   scope to reach them so much as finish the sentence IS-2 started.
2. **All three are leaves.** `event_tag.rs` and `state.rs` have zero `crate::`/`super::`
   imports; `event_stream.rs:6` imports only `EventTag`. Relocating them pulls nothing else.
3. **`DomainEvent` (`event.rs`, 62 lines, `chrono` + `serde_json` only) is the one item whose
   home this design makes worse, and it says so.** It is the domain's central event contract
   (`lib.rs:109`), not a persistence port. It relocates only because `EventStore<E: DomainEvent>`
   cannot compile without it and the alternatives are the three rejected in AD-1.

**Cost, stated rather than buried**: this exceeds IS-2's file list and contradicts the
Objective's "relocates that vocabulary — **and nothing else**". It violates no OOS
(no signature changes, no new types, no implementations move), but the proposal's scope
sentence is now wrong. **OQ-1 tracks the required amendment.**

### AD-3 — `id_type!` relocates and is `#[macro_export]`ed; `ego-domain` invokes it for its four remaining identity types

**Decision**: the `macro_rules! id_type` block (`context.rs:7-54`) moves to
`ego-persistence-api` and gains `#[macro_export]`. `TenantId` / `TenantIdError` are generated
there. `ego-domain`'s `context.rs` keeps generating `AggregateId`, `EntityId`,
`CorrelationId`, `CausationId`, and `RequestId` by invoking the re-exported macro, and
re-exports `TenantId` / `TenantIdError` at `ego_domain::context::TenantId` and
`ego_domain::TenantId` (`lib.rs:103-107`).

**Criteria**: (a) one definition of the generator, not two — the alternative (copy the macro
into the new crate) leaves a 47-line generator duplicated, which is the class of drift D-6
exists to prevent; (b) the macro moves verbatim, satisfying D-6, and the type it generates is
one type, satisfying SC-1's "not a re-declared copy that merely shares the name";
(c) hand-expanding the macro for `TenantId` alone was rejected outright — it is the only
route here that is *not* verbatim.

**This is the least comfortable decision in the document**: a domain identity generator ends
up in a persistence crate. It is recorded as **OQ-2**, not smoothed over.

### AD-4 — Re-exports are declared at **module** granularity, which reduces internal rewiring to zero

**Decision**: each vacated `ego-domain` module declaration becomes a module re-export, and
the existing item-level `pub use` lines are left byte-identical:

```rust
// crates/domain/src/persistence/mod.rs — after
pub use ego_persistence_api::persistence::{
    error, event_store, repository, snapshot, stored_event, tenant,
};
pub use error::PersistenceError;                       // unchanged, resolves through the above
pub use event_store::{EventStore, EventStoreUnitOfWork};
pub use repository::Repository;
pub use snapshot::Snapshot;
pub use stored_event::StoredEvent;
pub use tenant::resolve_tenant;
```

**Criteria**: (a) `super::event_tag::EventTag` (`handler.rs`, `processor.rs`, `progress.rs`,
`tagger.rs`, `session.rs`, `runner.rs`) and `crate::read_side::dedup::DedupStore`
(`scheduler.rs:5`) both resolve through a re-exported module without edit — **IS-4's "rewire
`ego-domain`'s internal consumers" collapses to nothing**, and the change's own crate becomes
as untouched as every consumer outside it; (b) item-granularity re-export would force every
one of those `use` lines to change, adding churn to a diff R-4 already flags as too large to
read line by line; (c) the six explicit module names beat `pub use …::*` because a glob makes
a missing module invisible until a downstream build fails (R-6).

### AD-5 — `ego-persistence-api`'s `Cargo.toml` is derived, not guessed

**Decision**: start from `ego-domain`'s `[dependencies]` block (`Cargo.toml:6-17`), then delete
every entry the relocated set does not name, proven by `cargo build -p ego-persistence-api`
in isolation (FR-005). `sha2` is known to move — `crates/domain/Cargo.toml:13-17` documents it
as existing solely for `OperationKeyHash` (`operation/key.rs:203`), which relocates — so
`ego-domain` is expected to lose it. `[dev-dependencies]` `mockall` and `tokio` follow the
`#[cfg(test)]` modules that move with their files (D-6).

**Criteria**: the dependency set is a compile fact, and listing it from memory here would be
the one place in a verbatim relocation where this document invents something.

### AD-6 — Three slices, ordered by the closure, each independently compiling workspace-wide

`sdd-tasks` owns task decomposition; this design owns only the boundaries EC-3 makes
mandatory.

| Slice | Contents | Closure it needs | Ready |
|---|---|---|---|
| **S1 — read side** | crate skeleton, `Cargo.toml`, `layers.toml` entry, **the AD-1 gate relaxation + its test**, `read_side/{offset,dedup,store,projection_state}` + `event_tag`, `state`, `event_stream` (AD-2), module re-exports | none — every file is a leaf or depends only on S1 | **now** |
| **S2 — operation** | `operation/{key,receipt,reservation}`, `id_type!` + `TenantId` (AD-3), module re-exports | `TenantId` (AD-3) | after OQ-2 |
| **S3 — persistence** | `persistence/{error,event_store,repository,snapshot,stored_event,tenant}`, `event.rs` (AD-2), module re-exports | S2's `OperationKey`/`OperationReceipt`, plus `DomainEvent` | after S2 + OQ-1 |

**Criteria**: (a) S1 carries the gate relaxation because it introduces the first edge — the
relaxation lands *with* the edge that needs it, never before; (b) S1 is unblocked by both open
questions, so work can start while they are decided; (c) every slice keeps the re-export layer
intact, so a partially-landed CORE-PERSIST-A is a workspace where some ports live in a second
crate and every consumer still compiles unchanged (the proposal's mid-flight rollback property).

### AD-7 — `ProjectionStateStore` relocates dead, and `PostgreSQLRepository`'s defect is not touched

**Decision**: `ProjectionStateStore` + `ProjectionStateStoreError` move verbatim with zero
implementations and zero consumers (KD-1, D-8) — deleting them would make a reorg into a
behavior change. `crates/persistence/src/postgres/repository.rs` is **not opened**: its
`tenant_id = $2` scoping (lines 82, 135, 161) and its line-109 `ON CONFLICT (aggregate_id,
tenant_id)` targeting a constraint `002_create_aggregates.sql` never declares — a live Postgres
`42P10` and a tenant-isolation defect — stay exactly as they are (KD-2, OOS-12, owned by F-2).
KD-3 and KD-4 likewise carry forward untouched.

**Criteria**: this change relocates ports; that file holds an implementation (OOS-1) and its
fix needs its own tests and its own schedule. F-2 is explicitly *not* gated on the
CORE-PERSIST series.

---

## Integration Points

| Boundary | Direction | Mechanism | Verified at |
|---|---|---|---|
| `ego-domain` → `ego-persistence-api` | new, one-way | `path` dependency + module re-exports | `crates/domain/Cargo.toml`; AD-1, AD-4 |
| `ego-persistence-api` → any workspace crate | **none** | no `path` dependency exists | `crates/persistence-api/Cargo.toml`; AD-5 |
| every existing consumer → moved items | unchanged | resolved through `ego-domain`'s re-exports | 92 files, explore §6; SC-2 |
| `ego-domain` internal consumers → moved ports | unchanged | `super::`/`crate::` paths resolve through re-exported modules | AD-4 |
| `layers.toml` → `verify-layers` | in | one new entry, existing loader | `layers.rs:150-158` |
| `allowed_layers` → `check_direction` | in | one match arm | `layers.rs:76`, `:107-116`; SC-7 |
| runtime behavior | **none** | nothing executes differently | OOS-6 |

Zero new plumbing: one crate, one edge, one match arm.

## Testing Strategy

Strict TDD. The RED test is the re-export identity file — it names `ego_persistence_api::`
paths that do not exist yet, so it fails to compile before any relocation.

| Level | Location | What it proves |
|---|---|---|
| Compile-time (primary) | `crates/persistence-api/tests/reexport_identity.rs` | **SC-1 / IS-6** over all **35** items (EC-4). A bare `use` is insufficient — it compiles against a re-declared copy. Each item gets an identity witness: an identity coercion for object-safe traits (`fn f(x: Box<dyn ego_domain::…::DedupStore>) -> Box<dyn ego_persistence_api::…::DedupStore> { x }`), and for generic traits a `where`-clause witness carrying both bounds on one parameter. Full list, never a sample |
| Unit | `xtask/src/layers.rs` `#[cfg(test)]` | **SC-7**: `domain → domain` yields no violation, and `domain → foundation` / `domain → infrastructure` / `domain → sdk` still yield `WrongDirection`. Follows the existing `graph_from` / `layers_from` shape (`layers.rs:164-208`) |
| Relocated | the moved `#[cfg(test)]` modules | **SC-3 / D-6**: they move verbatim with their files. Assertion count before and after must be identical — a changed assertion is a semantic drift signal, not a cleanup |
| Gate | `cargo run -p xtask -- verify-layers` | **SC-6**: mapped (FR-001), edge permitted (FR-002), no cycle (FR-003), isolation compile (FR-005) |
| Workspace | `cargo build --workspace`, `cargo test --workspace` | **SC-5**: zero new failures, zero changed assertions |

Three properties are **diff properties**, checked by reading the change rather than by a test:
**SC-2** (no `use` or `Cargo.toml` edit outside the two crates), **SC-8** (no `.sql`/migration
file in the diff), and **SC-9** (`crates/runtime/`, `crates/effect-store/`, and every OOS-1
implementation absent from the file list).

## Threat Matrix

N/A — no routing, shell command, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. This change moves Rust source files between
two crates and adds one `match` arm.

`ego-rs-security` is applicable only to confirm it is untouched: **zero** SQL text, query
construction, auth path, JWT verification, or `CrossTenantPermit` check appears in the diff
(OOS-3). `resolve_tenant`'s three-way rule (`persistence/tenant.rs:29`) relocates verbatim
under OOS-5, so tenant semantics are byte-identical before and after. Rules 1 through 4:
**PASS by absence**, not by argument.

## Migration / Rollout

**No migration required.** No data, schema, migration file, or persisted state exists in
either direction — this change writes nothing at runtime (OOS-6). No feature flag and no
phased rollout: the workspace either compiles with the new layout or it does not.

Rollback is the proposal's, unchanged and available mid-flight per AD-6: drop
`crates/persistence-api/`, restore the `ego-domain` modules from the pre-change tree, remove
the one `Cargo.toml` edge and the `layers.toml` entry, and restore `allowed_layers("domain")`
to `Some(&[])`.

## Traceability

| Proposal item | Resolved by | Note |
|---|---|---|
| **OD-1**, D-2, D-4, IS-5 | **AD-1** | direction forced by `scheduler.rs:5-10`; `Some(&[])` → `Some(&["domain"])`; both alternatives named and rejected |
| D-3 | **EC-1 + AD-2 + AD-3** | closure is five types, not two; `TenantId` is macro-generated |
| D-1, IS-1 | AD-5 | crate at `crates/persistence-api/`, mapped `domain`, dependency set derived by compiling |
| D-5, IS-3, R-6 | **AD-4** | module-granularity re-export, explicit names, no glob |
| D-6, R-2, SC-3, SC-4 | AD-2, AD-3, Testing | verbatim; `Arc<T>` forwarding impls (`offset.rs:92`, `dedup.rs:60`) move inside their own files |
| D-7, R-5, SC-1 | **EC-4** | 35 items, not 27 — `MAX_LEN` (`key.rs:19`) named by neither §9 nor D-7 |
| D-8, KD-1, OOS-9 | AD-7 | `ProjectionStateStore` relocates dead |
| D-9, OOS-2, SC-9, F-1 | — | `ego-runtime` / `ego-effect-store` appear in no file list here |
| IS-4 | **AD-4** | collapses to zero edits — module re-export keeps `super::`/`crate::` paths resolving |
| IS-6, SC-1 | Testing | identity witness per item, not a bare `use`, not a sample |
| R-3, SC-7 | AD-1 criterion 2 + Testing | matrix admits `domain → domain` and nothing wider, asserted |
| **R-4, SC-12** | **EC-3 + AD-6** | R-4's order is backwards; correct order is `read_side/` → `operation/` → `persistence/` |
| R-1 | **OQ-1, OQ-2** | OD-1's direction half is closed; its closure half is re-priced, not assumed away |
| KD-2, OOS-12, F-2 | AD-7 | `42P10` + tenant-scoping defect carried forward untouched |
| KD-3, KD-4, OOS-13, OOS-14, F-3, F-4 | AD-7 | carried, not fixed |
| `config.yaml` "no circular dependencies" | Dependency Graph | one-way edge; `crates/persistence-api/Cargo.toml` names no workspace crate |
| `config.yaml` "sequence diagrams" | Technical Approach | explicit N/A — no async flow changes |

## Open Questions

- [ ] **OQ-1 — AD-2 exceeds IS-2 and contradicts the Objective's "and nothing else".**
      Relocating `DomainEvent`, `EventTag`, `ProjectionState`, and `EventStreamElement` is
      forced by EC-1 under AD-1's one-way edge, and violates no OOS — but IS-2's file list and
      the Objective sentence are now inaccurate. **Confirm the proposal amendment before
      `sdd-tasks` plans S3.** S1 is unaffected and can start.
- [ ] **OQ-2 — AD-3 puts `id_type!`, a domain identity generator, in `ego-persistence-api`.**
      It is the only route that is both verbatim (D-6) and single-definition. The alternative
      is duplicating a 47-line macro. Confirm before `sdd-tasks` plans S2.
