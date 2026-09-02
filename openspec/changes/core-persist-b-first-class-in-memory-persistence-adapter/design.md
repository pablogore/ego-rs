# Design: CORE-PERSIST-B — First-Class In-Memory Persistence Adapter

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers:
> EC-1..EC-7, AD-1..AD-10, S1..S3, OQ-1..OQ-3).
>
> **Inputs**: `proposal.md` (D-1..D-12, NG-1..NG-12, IS-1..IS-6, R1..R18, R-1..R-7, KD-1/4/5/6,
> F-1/F-4/F-5/F-6) and `explore.md` (MOVE MATRIX, COMPATIBILITY REEXPORT MATRIX, DEPENDENCY
> GRAPH, EFFECT STORE BLOCKER ANALYSIS, TARGET CRATE / MODULE TREE). This document decides
> **how**: the crate's dependency closure, its module tree, the exact import rewrite, the
> re-export granularity in each vacating crate, where the compatibility proof lives, and the
> slice boundaries. Observable requirements belong to `spec.md` and are not restated here.
>
> **Baseline read**: `develop` @ `e74c9fc`. Every `file:line` below was read on this baseline,
> not recalled from the inputs. Where the baseline contradicts an input, it is recorded as an
> **Evidence Correction**, not silently applied.

## Technical Approach

One new leaf-ward crate, two one-directional edges, three vacating crates, and a compatibility
layer whose granularity is chosen *per vacating crate* by what that crate's public path actually
is — not by one uniform rule.

`ego-persistence-memory` depends on `ego-persistence-api` for every port it implements and on
`ego-domain` for exactly one item (`Clock`, `crates/domain/src/time/clock.rs:24`). Both targets
are `domain`-layer crates and the new crate is `foundation`, so both edges are already permitted
by `xtask/src/layers.rs:77` and **no gate matrix edit is required** (AD-1). The three vacating
crates each gain a normal dependency on the new crate and keep every path a consumer resolves
today.

**No sequence diagram is included, and that is a deliberate applicability call.**
`openspec/config.yaml`'s design rule asks for one on complex async flows. This change adds,
removes, and reorders zero call paths: every `#[async_trait]` method body moves byte-identical
(D-4, R5), so a diagram drawn here would depict a flow the change does not touch. The
load-bearing structure is the **dependency graph**, given below — same call CORE-PERSIST-A made
for the same reason (archived `design.md:27-31`).

---

## Evidence Corrections

Seven. Each was found by reading the baseline rather than the inputs, and each changes what the
implementation must do.

### EC-1 — `ego_persistence_api::operation` does not flatten what `ego_domain::operation` flattens

D-4 says the rewrite maps `use ego_domain::persistence::…` to `use ego_persistence_api::persistence::…`.
That one-to-one shape holds for `persistence::` and `read_side::`, and **fails for `operation::`**:

| Module | `ego-domain` | `ego-persistence-api` |
|---|---|---|
| `persistence` | re-exported wholesale | `mod.rs:39-44` exports `PersistenceError`, `EventStore`, `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `StoredEvent`, `resolve_tenant` — identical set |
| `read_side` | re-exported wholesale | `mod.rs:7-13` exports the submodules; both crates are addressed submodule-first (`read_side::offset::Offset`) — identical shape |
| `operation` | `crates/domain/src/operation/mod.rs:26` flattens the **whole reservation vocabulary** to `ego_domain::operation::{Lease, OwnerId, ReserveRequest, …}` | `crates/persistence-api/src/operation/mod.rs:18-19` flattens **only** `OperationKey` and `OperationReceipt` |

Consequence: `crates/testkit/src/reservation.rs:16-19`'s eleven-name import cannot be rewritten
by swapping the crate prefix. Its correct target is
`ego_persistence_api::operation::reservation::{…}` (all eleven items are declared there —
`reservation.rs:48,66,178,205,231,273,290,309,321,346,388`). `OperationReceipt`
(`event_store.rs:6`) *does* swap one-for-one. **AD-4 fixes the exact blocks.**

### EC-2 — `reservation.rs:70` is an **inline** fully-qualified path, not a `use` line

D-4 permits "rewriting `use` lines" and nothing else. The struct that moves carries a path
expression in its body:

```rust
// crates/testkit/src/reservation.rs:68-72
struct Record {
    fingerprint: ego_domain::operation::OperationFingerprint,
    state: RecordState,
}
```

`OperationFingerprint` is generated in `ego-persistence-api` (`operation/key.rs`) and reaches
`ego_domain::operation` through `crates/domain/src/operation/mod.rs:21,24`. Left as-is, it still
compiles (the new crate depends on `ego-domain` anyway) and is byte-identical; rewritten to
`ego_persistence_api::operation::key::OperationFingerprint` it names **the same item** and keeps
the crate's `ego_domain::` surface at exactly one line. **AD-5 decides; OQ-1 asks propose to
widen D-4's phrasing from "`use` lines" to "path expressions naming a relocated port item".**

### EC-3 — The reservation-store re-export must live in `reservation.rs`, not `lib.rs`

D-5 places it at `crates/testkit/src/lib.rs`. That location is insufficient. `reservation.rs`
carries two colocated `#[cfg(test)]` modules that resolve the store through the **module**, not
the crate root:

- `crates/testkit/src/reservation.rs:378` — `use super::{InMemoryOperationReservationStore, TestClock};`
- `crates/testkit/src/reservation.rs:525` — same line, in `mod oldest_completed_contract`

A `lib.rs`-only re-export leaves both `use super::` lines unresolvable, forcing an edit to
test modules that D-8 says stay untouched. A `pub use` **inside `reservation.rs`** makes
`super::InMemoryOperationReservationStore` resolve, and leaves `crates/testkit/src/lib.rs:50`
(`pub use reservation::{InMemoryOperationReservationStore, TestClock};`) byte-identical.
**AD-5.**

### EC-4 — Only one of the four `ego-infrastructure` files carries a test module

`#[cfg(test)]` appears exactly once under `crates/infrastructure/src/persistence/in_memory/`:
`read_side_store.rs:142`, whose module uses `chrono::Utc` (`:144`), `serde_json::json!`
(`:158`) and `#[tokio::test]` (`:165`). `event_store.rs`, `repository.rs`, and `snapshot.rs`
have **none** — their coverage is in `crates/infrastructure/tests/` (`in_memory_event_store_conformance.rs:17-18`,
`commit_publishes_atomically.rs:25`), which stays behind and keeps compiling through the
re-export. This fixes the new crate's `[dev-dependencies]` at `tokio` alone. **AD-2.**

### EC-5 — The reference-app move touches **two** files, not one

D-6 / IS-4 name `examples/reference-app/src/read_side/store.rs`. Two more facts on the baseline:

1. `examples/reference-app/src/read_side/mod.rs:36-39` publicly re-exports both moving types
   (`pub use store::{FakeDurableDedupStore, FakeDurableOffsetStore, InMemoryDedupStore, InMemoryOffsetStore, ReadSideSink, SharedReadSideStore};`)
   and `:106-107` constructs them in `ReadSideHandles::in_memory()`.
2. `store.rs:251` and `:282` — `FakeDurableOffsetStore(InMemoryOffsetStore)` and
   `FakeDurableDedupStore(InMemoryDedupStore)` **wrap the moving types by value** and delegate
   to them (`:265,275,296,305`). They stay (NG-8, R3), so `store.rs` must keep naming both
   types after the declarations leave.

**AD-7** handles both without changing the example's own public surface.

### EC-6 — `crates/persistent-entity/src/builder.rs` is confirmed unaffected

`explore.md`'s COMPATIBILITY REEXPORT MATRIX left this open ("need to confirm at propose time").
Confirmed on the baseline: `builder.rs:10` reads
`use crate::persistence::{InMemoryEventStore, InMemorySnapshotStore, PersistenceFacade};` — the
crate's **own** duplicates (`persistence.rs:571,733`), which D-9 does not move. `builder.rs:356,360`
construct those. No re-export is required for `persistent-entity`, and its two
`try_build_rejects_explicit_in_memory_*` tests (`builder.rs:768,793`) exercise types this change
never touches, which is why R6 holds trivially.

### EC-7 — `chrono` is dev-only until the reservation store arrives

None of the four `ego-infrastructure` implementations names `chrono` outside
`read_side_store.rs`'s test module (EC-4). The reservation store does, in its body
(`reservation.rs:59,64,195,301` — `DateTime<Utc>`). So the new crate's dependency set is not
constant across the slices: S1 needs `chrono` as a dev-dependency only, and S2 promotes it to a
normal one. Stated so the slice boundary is derivable rather than guessed. **AD-2, AD-9.**

### EC-8 — `records` is private, and one colocated test reads it directly: D-8/AD-8's "byte-identical" and "fields don't change" collide once the struct crosses crates

Discovered at apply time (S2), not at design time — recorded here rather than silently patched.
`InMemoryOperationReservationStore.records` (`reservation.rs:79`) was never `pub`; it only
compiled from `testkit/src/reservation.rs`'s `#[cfg(test)] mod tests` because struct and test
shared a crate. `a_lock_wait_that_spans_expiry_rejects_the_lapsed_holder` (originally
`:378-460`ish) locks `store.records` directly to hold the mutex across an awaited task — a
white-box test of the store's own lock-ordering, not of the `OperationReservationStore` port. D-8
and AD-8 both promised this module stays byte-identical in `ego-testkit`; AD-8 separately promised
the struct's fields don't change. Once the struct moves crate, both cannot hold simultaneously
without either widening `records`'s visibility (changing AD-8's "fields don't change", and to
`pub`, not `pub(crate)`, since visibility restrictions don't reach across a crate boundary) or
relocating the test.

**Resolution (asked of the user, a genuine architectural fork — not resolved unilaterally, same
posture as OQ-2): move only this one test.** It now lives colocated with the struct, in
`crates/persistence-memory/src/operation/reservation.rs`'s own `#[cfg(test)] mod tests`, where
`records` is legally private-but-visible (same module tree). `records` itself is untouched —
AD-8's "fields don't change" holds exactly as written. `TestClock` and
`the_in_memory_reservation_store_conforms` (the actual `OperationReservationStore` conformance
test, which never touches `records`) stay in `ego-testkit` per the original D-8/AD-5 — only the
one white-box test moves. The new crate needs its own minimal clock double (`FixedClock`,
~15 lines) since `ego-persistence-memory` (`foundation`) cannot depend on `ego-testkit`
(`tooling`, a sink) to reuse `TestClock` — the layer direction runs the other way. This adds
`tokio`'s `rt-multi-thread` dev-feature (the test's `flavor = "multi_thread"` requirement) and a
second, `#[cfg(test)]`-gated `use ego_domain::Clock;` line, amending AD-2 criterion 4 to
"exactly one **non-test** line" (see AD-2 above). **AD-8's D-8 amended accordingly: "both
`#[cfg(test)]` modules stay in `ego-testkit`, byte-identical" becomes "the conformance module
stays in `ego-testkit`; the lock-ordering test relocates with the struct it inspects, body
unchanged."**

---

## Dependency Graph

**Before** — the seven implementations sit in three crates across three layers:

```
        ego-persistence-api  [domain]  ← leaf, owns all eight ports
                  ▲
              ego-domain     [domain]  ← re-exports them at their old paths
                  ▲
     ┌────────────┼──────────────┬───────────────────┐
ego-infrastructure    ego-testkit        reference-app
 [infrastructure]      [tooling, SINK]    [not layer-checked]
  4 implementations     1 implementation   2 implementations
```

**After** — one new crate below the three consumers, two new outbound edges, three new inbound:

```
        ego-persistence-api  [domain]  ← still a leaf, still untouched (NG-5, R15)
                  ▲                ▲
                  │                │  ports
              ego-domain  ────┐    │
               [domain]       │    │
                  ▲       Clock│    │
                  │            ▼    │
     ┌────────────┤     ego-persistence-memory  [foundation]
     │            │        (7 implementations)
     │            │              ▲   ▲   ▲
     │            │              │   │   │  re-export / import
ego-infrastructure   ego-testkit    reference-app
 [infrastructure]     [tooling]      [not layer-checked]
```

**No cycle is introduced, and this is a fact about files rather than a review promise.**
`ego-persistence-api` names no workspace `path` dependency (`crates/persistence-api/Cargo.toml:6-20`),
`ego-domain` depends only on it, and `ego-persistence-memory` depends only on those two. The
reverse edges cannot exist: Cargo refuses the cycle before `xtask verify-layers` runs, and
FR-003's cycle check refuses it again. This satisfies `openspec/config.yaml`'s "No circular
dependencies between crates" rule by construction.

---

## Architecture Decisions

### AD-1 — Layer is `foundation`; the change is **one line in `layers.toml`** and zero lines in `xtask/`

**Decision** — add to `layers.toml`'s `[layers]` table (currently `:15-35`):

```toml
"ego-persistence-memory" = "foundation"
```

and to the workspace `members` list (`Cargo.toml:2-23`): `"crates/persistence-memory",`.
**`xtask/src/layers.rs` is not opened.**

**Criteria**:

1. **Crate-to-layer mapping lives in `layers.toml`, not in `layers.rs`.** Verified directly:
   `layers.rs` contains only `KNOWN_LAYERS` (`:14-23`), the `allowed_layers` matrix (`:74-92`),
   the two checks (`:97-145`), and `load_layers_toml` (`:148-158`), which parses the `[layers]`
   table. There is no crate name anywhere in `layers.rs` outside its `#[cfg(test)]` fixtures.
   D-2's "wherever crate-to-layer mapping actually lives" resolves to `layers.toml`.
2. **Every edge this change creates is already permitted.** No matrix arm changes:

   | Edge | Layers | Permitted by |
   |---|---|---|
   | `ego-persistence-memory → ego-persistence-api` | foundation → domain | `layers.rs:77` |
   | `ego-persistence-memory → ego-domain` | foundation → domain | `layers.rs:77` |
   | `ego-infrastructure → ego-persistence-memory` | infrastructure → foundation | `layers.rs:80-86` |
   | `ego-testkit → ego-persistence-memory` | tooling → * | `layers.rs:89` (`None` = sink) |
   | `reference-app → ego-persistence-memory` | — | out of scope entirely (see 4) |

3. **The `layers.toml` entry is mandatory, not optional.** `xtask/src/metadata.rs:82-85,100-105`
   restricts every check to workspace members whose manifest lives under `<root>/crates/`.
   `crates/persistence-memory/` qualifies, so FR-001's completeness check
   (`layers.rs:125-131`) emits `UnmappedCrate` without the entry. This satisfies FR-001; it does
   not modify it (proposal Capabilities → `foundation-integrity`: "no modification expected" —
   **confirmed**).
4. **The reference-app edge is not checked at all.** `metadata.rs:100-105` and its test
   (`:209-220`) exclude `examples/reference-app` and `xtask` from all three checks, so IS-4's
   new dependency creates no gate obligation in either direction.
5. **`domain` was considered and rejected**, per D-2(b): mapping the new crate to `domain` would
   ride CORE-PERSIST-A's `"domain" => Some(&["domain"])` self-edge (`layers.rs:76`) and thereby
   widen it from "a domain crate may reach a *port* crate" to "…may reach an *adapter*",
   legalizing `ego-domain → ego-persistence-memory`. `foundation` closes that door: `domain`'s
   allowed set does not contain `foundation`, and `layers.rs:259-275` already asserts it.

**Known-stale, not fixed here**: `layers.toml:6`'s header comment still reads
`domain → nothing`, which `layers.rs:76` has contradicted since CORE-PERSIST-A. The executable
matrix is authoritative. Untouched per NG-7 and proposal risk R-6.

### AD-2 — The dependency set is derived from the seven files, and `ego-domain` earns exactly one line

**Decision** — `crates/persistence-memory/Cargo.toml`:

```toml
[package]
name = "ego-persistence-memory"
version = "0.1.0"
edition = "2021"

[dependencies]
ego-persistence-api = { path = "../persistence-api" }
# `Clock` only. CORE-PERSIST-A did not relocate it — it is still declared at
# crates/domain/src/time/clock.rs:24 — and the reservation store holds an
# `Arc<dyn Clock>` (D-3, EC-1). This is the crate's ONLY ego-domain item.
ego-domain = { path = "../domain" }
async-trait = { workspace = true }
chrono = "0.4"
serde_json = "1"

[dev-dependencies]
# read_side/store.rs's relocated `#[cfg(test)]` module: `#[tokio::test]` (EC-4).
tokio = { version = "1", features = ["macros", "rt"] }
```

**Criteria**:

1. **Every entry is traceable to an import on the baseline**, and nothing else is present:

   | Dependency | Required by | Evidence |
   |---|---|---|
   | `ego-persistence-api` | all seven | every port and every value type they name |
   | `ego-domain` | `operation/reservation.rs` only | `reservation.rs:20` (`use ego_domain::Clock`), `:80` (`clock: Arc<dyn Clock>`) |
   | `async-trait` | event_store, read-side store, offset, dedup, reservation | `event_store.rs:4`, `store.rs:11`, `reservation.rs:14` |
   | `chrono` | reservation only | `reservation.rs:59,64,195,301` (`DateTime<Utc>`) — EC-7 |
   | `serde_json` | read-side store, snapshot | `read_side_store.rs:13`, `snapshot.rs:5` |
   | `tokio` (dev) | relocated read-side test module | `read_side_store.rs:165` — EC-4 |

2. **`serde` is absent on purpose.** No moved body derives `Serialize`/`Deserialize`;
   `EventStreamElement`'s derives live in `ego-persistence-api`
   (`read_side/event_stream.rs:12`), which carries its own `serde`
   (`persistence-api/Cargo.toml:7`).
3. **R11's list is met exactly.** No `ego-application`, `ego-runtime`, `ego-infrastructure`,
   `ego-persistence`, `ego-testkit`, transport, or example dependency — and no `sqlx`, no
   OpenTelemetry, no `dashmap`, satisfying R7's backend-neutrality clause. `ego-infrastructure`
   carries all of those today (`infrastructure/Cargo.toml:8-20`) while its `in_memory` submodule
   imports none of them; that gap is the concrete thing this crate closes.
4. **The `ego-domain` edge is checkable, not asserted.** `rg '^use ego_domain::|ego_domain::'
   crates/persistence-memory/src` must return exactly one **non-test** line —
   `operation/reservation.rs`'s `use ego_domain::Clock;`. AD-5's EC-2 rewrite is what makes that
   count one rather than two, and Testing pins it as a diff property. **Amended by EC-8**: a
   second, `#[cfg(test)]`-gated `use ego_domain::Clock;` exists inside the colocated white-box
   test module for its local `FixedClock` double — a production-surface property, not a
   test-surface one, so it does not weaken this criterion.
5. **No third edge exists.** The prompt asks this be confirmed rather than assumed: the other
   six implementations' import lists (`event_store.rs:1-8`, `repository.rs:1-4`,
   `snapshot.rs:1-5`, `read_side_store.rs:6-11`, `store.rs:7-19`) name only `std`, `async_trait`,
   `serde_json` and items re-exported by `ego-domain` from `ego-persistence-api`. Every one of
   those items resolves directly out of `ego-persistence-api` (verified against
   `persistence-api/src/{lib.rs:18-34, persistence/mod.rs:39-44, read_side/mod.rs:7-13}`), so
   none of the six needs `ego-domain` at all. **R-3's "third edge" risk is closed here, at
   design time, not at apply time.**

### AD-3 — The module tree mirrors `ego-persistence-api`'s; no crate-root flattening; no new lint gate

**Decision** — refining `explore.md`'s proposed tree in two places:

```
crates/persistence-memory/            (package: ego-persistence-memory)
├── Cargo.toml                        # AD-2
└── src/
    ├── lib.rs                        # module declarations + crate doc, no re-exports
    ├── persistence/
    │   ├── mod.rs
    │   ├── event_store.rs            # InMemoryEventStore + InMemoryEventStoreUnitOfWork  ← infra event_store.rs
    │   ├── repository.rs             # InMemoryRepository                                  ← infra repository.rs
    │   └── snapshot.rs               # InMemorySnapshotStore (tenant-correct)              ← infra snapshot.rs
    ├── read_side/
    │   ├── mod.rs
    │   ├── store.rs                  # InMemoryReadSideStore + paginate + its test module  ← infra read_side_store.rs
    │   ├── offset.rs                 # InMemoryOffsetStore                                 ← reference-app store.rs:153
    │   └── dedup.rs                  # InMemoryDedupStore                                  ← reference-app store.rs:199
    └── operation/
        ├── mod.rs
        └── reservation.rs            # InMemoryOperationReservationStore + Record/RecordState ← testkit reservation.rs
```

**Refinement 1** — `read_side/read_side_store.rs` → `read_side/store.rs`. **Criteria**: every
other file in the tree already mirrors its port module one-for-one
(`persistence::event_store` ⇄ `ego_persistence_api::persistence::event_store`;
`read_side::offset` ⇄ `read_side::offset`). Keeping `read_side_store.rs` makes the read-side
store the single row a reader has to look up. The stutter the original name avoided
(`in_memory::store`) does not arise here — the crate name is the disambiguator
(`ego_persistence_memory::read_side::store::InMemoryReadSideStore` implements
`ego_persistence_api::read_side::store::ReadSideStore`). `explore.md` marked its tree "working
name, not forced"; the canonical paths in its COMPATIBILITY REEXPORT MATRIX shift accordingly
and are restated in full under **Integration Points**.

**Refinement 2** — `lib.rs` declares modules and re-exports nothing at the crate root. **Criteria**:
`ego-persistence-api` sets this precedent (`lib.rs:18-34` — five `pub mod`, zero `pub use`), and a
root-level flattening would be new public API surface, which NG-11 forbids. Consumers reach items
at their module path; the vacating crates' compatibility layer absorbs the verbosity for everyone
who already had a shorter path.

**Refinement 3 (a trap, named)** — **the crate does not carry `#![deny(missing_docs)]`**, even
though `ego-testkit` (`lib.rs:1`), `ego-security-sdk`, `security-apikey`, and `security-jwt` do.
Several moved items have no doc comment (e.g. `InMemoryRepository::new`, `repository.rs:16`;
`InMemorySnapshotStore::new`, `snapshot.rs:17`). Adding the lint would force doc comments onto
moved bodies — a body edit, forbidden by D-4 and detectable as a violation of R5. The
originating crates (`ego-infrastructure`, `reference-app`) carry no crate-level lint attribute
either, so this preserves the exact lint posture each body compiles under today.

### AD-4 — The import rewrite is enumerated per file, not described by a rule

**Decision** — the complete set of edits permitted inside a moved body. Every line below was read
on the baseline; each `after` names the same item as its `before` by construction (CORE-PERSIST-A
shipped that identity, `persistence-api-surface` spec: "Old Path Resolves To The Same Item").

| # | File (new path) | Before | After |
|---|---|---|---|
| 1 | `persistence/event_store.rs` | `use ego_domain::event::DomainEvent;` (`:5`) | `use ego_persistence_api::event::DomainEvent;` |
| | | `use ego_domain::operation::OperationReceipt;` (`:6`) | `use ego_persistence_api::operation::OperationReceipt;` |
| | | `use ego_domain::persistence::resolve_tenant;` (`:7`) | `use ego_persistence_api::persistence::resolve_tenant;` |
| | | `use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};` (`:8`) | `use ego_persistence_api::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};` |
| 2 | `persistence/repository.rs` | `:3-4` (`resolve_tenant`; `PersistenceError, Repository`) | same names under `ego_persistence_api::persistence` |
| 3 | `persistence/snapshot.rs` | `:3-4` (`resolve_tenant`; `PersistenceError, Snapshot`) | same names under `ego_persistence_api::persistence`; `:5` `use serde_json::Value;` **unchanged** |
| 4 | `read_side/store.rs` | `:8-11` (`event_stream::EventStreamElement`, `event_tag::EventTag`, `offset::Offset`, `store::{ReadSideStore, ReadSideStoreError}`) | same four submodule paths under `ego_persistence_api::read_side` |
| 5 | `read_side/offset.rs` | from reference-app `store.rs:16` — `use ego_domain::read_side::offset::{Offset, OffsetStore, OffsetStoreError};` | `use ego_persistence_api::read_side::offset::{Offset, OffsetStore, OffsetStoreError};` plus `event_tag::EventTag` (`store.rs:15`) |
| 6 | `read_side/dedup.rs` | from reference-app `store.rs:13` — `use ego_domain::read_side::dedup::{DedupStore, DedupStoreError};` | `use ego_persistence_api::read_side::dedup::{DedupStore, DedupStoreError};` plus `event_tag::EventTag` |
| 7 | `operation/reservation.rs` | `use ego_domain::operation::{FencingToken, Lease, OldestCompleted, OperationId, OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse};` (`:16-19`) | **`use ego_persistence_api::operation::reservation::{…same eleven names…}`** — the submodule, not `operation::` (EC-1) |
| | | `fingerprint: ego_domain::operation::OperationFingerprint` (`:70`, inline) | `ego_persistence_api::operation::key::OperationFingerprint` (EC-2, AD-5) |
| | | `use ego_domain::Clock;` (`:20`) | **unchanged** — the one surviving `ego_domain::` line (AD-2 criterion 4) |

Splitting one source file into two destination files (items 5 and 6 both come from
`examples/reference-app/src/read_side/store.rs`) means each destination carries only the imports
its own body names. That is a mechanical consequence of the split, not a body edit: the struct,
its `#[async_trait] impl`, its key tuple aliases (`DedupKey`/`OffsetKey`, `store.rs:145,148`) and
its doc comments move byte-identical, and `std::sync::{Arc, Mutex}` plus
`std::collections::{HashMap, HashSet}` follow the type that uses each.

**Nothing else in any moved body changes.** R5's list — tenant resolution, locking strategy,
version-conflict arithmetic, `paginate`'s fail-closed empty-tenant guard
(`read_side_store.rs:111-115`) — is untouched, and no moved type declares `is_durable()`, which
is what makes R6 hold without a single new test.

### AD-5 — `reservation.rs` is **split**, not moved; its compatibility re-export lives in the module

**Decision** — `crates/testkit/src/reservation.rs` after the change contains, in order:

1. its module doc (`:1-9`), unchanged;
2. a pruned import block: `use std::sync::Mutex;` and `use chrono::{DateTime, Duration, Utc};`
   and `use ego_domain::Clock;` — everything `TestClock` still needs, and nothing more;
3. **`pub use ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore;`**;
4. `TestClock` and its `impl Clock` (`:22-50`), byte-identical (D-8, R16);
5. both `#[cfg(test)]` modules (`:370-512`, `:514-…`), **byte-identical**, including their
   `use super::{InMemoryOperationReservationStore, TestClock};` lines at `:378` and `:525`.

`RecordState` (`:52-66`), `Record` (`:68-72`), the store (`:79-97`) and its
`impl OperationReservationStore` (`:99-…`) relocate; `Record`/`RecordState` are private and
travel with the only type that names them.

**Criteria**:

1. **The `pub use` sits in the module because that is where the two test modules look** (EC-3).
   It simultaneously keeps `crates/testkit/src/lib.rs:50` byte-identical, so the change costs
   `lib.rs` zero edits. This is the same insight CORE-PERSIST-A's AD-4 had — put the re-export
   where the existing paths already resolve — applied to a different shape.
2. **Pruning the vacating file's imports is unavoidable and in-scope.** `async_trait`, the eleven
   `operation` names, `HashMap`, and `Arc` all leave with the store; leaving them behind is an
   unused-import warning, and `make clippy` runs `-D warnings`. This edit is inside the vacating
   crate, which D-5 already opens.
3. **A `pub use` satisfies `#![deny(missing_docs)]`** (`testkit/src/lib.rs:1`): a re-export
   inherits the target's documentation, and the store's doc comment (`reservation.rs:74-78`)
   moves with it.
4. **The two conformance suites are unaffected.** `crates/testkit/src/reservation_conformance.rs`
   is generic over `OperationReservationStore` and is exported separately
   (`lib.rs:51-54`); it neither names the concrete store nor changes.

**EC-2's inline path is rewritten** (item 7's second row in AD-4): `Record` is private, the two
paths name one item, and rewriting keeps the crate's `ego_domain::` surface at exactly one
grep-able line. **OQ-1** asks propose to widen D-4's phrasing to match; nothing about the
decision changes if it declines — the alternative is to leave `:70` byte-identical, which also
compiles, and only costs AD-2 criterion 4 a carve-out.

### AD-6 — `ego-infrastructure`'s re-export is at **item** granularity — the opposite of CORE-PERSIST-A's AD-4, for a reason found in the source

**Decision** — `crates/infrastructure/src/persistence/in_memory/mod.rs` becomes, keeping its
module doc (`:1-5`) unchanged:

```rust
pub use ego_persistence_memory::persistence::event_store::InMemoryEventStore;
pub use ego_persistence_memory::persistence::repository::InMemoryRepository;
pub use ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore;
pub use ego_persistence_memory::read_side::store::{paginate, InMemoryReadSideStore};
```

(the reservation store is **not** part of `ego-infrastructure`'s surface and therefore does not
appear here). The four `mod` declarations (`:7-10`) and the four source files are deleted.

**Criteria**:

1. **The vacated modules are private.** `mod event_store;` (`:7`), `mod read_side_store;` (`:8`),
   `mod repository;` (`:9`), `mod snapshot;` (`:10`) — none is `pub mod`. The only public path a
   consumer can resolve is the item path (`ego_infrastructure::persistence::in_memory::InMemoryEventStore`),
   confirmed at all four call sites: `examples/reference-app/src/lib.rs:432-439`,
   `examples/reference-app/src/read_side/store.rs:18`,
   `crates/infrastructure/tests/in_memory_event_store_conformance.rs:17-18`, and
   `crates/infrastructure/tests/commit_publishes_atomically.rs:25`.
2. **Module-granularity re-export would therefore *widen* the public surface**, newly exposing
   `ego_infrastructure::persistence::in_memory::event_store::…`. CORE-PERSIST-A chose module
   granularity because `ego-domain`'s vacated modules were `pub mod` and dozens of internal
   `super::`/`crate::` paths ran through them (archived `design.md:206-231`). Here neither
   condition holds: no path runs through the module, and item granularity is both the narrower
   and the shorter diff — four `pub use` lines retargeted, `:12-15` in place.
3. **`paginate` keeps a public path.** It is a free function, imported directly by
   `examples/reference-app/src/read_side/store.rs:18`, and stays in the same `pub use` line it
   shares with `InMemoryReadSideStore` today (`:13`) — R9's requirement that *every* row in the
   matrix resolve, including the non-struct one.
4. **`ego-infrastructure` gains one normal dependency** (`ego-persistence-memory`) and loses
   none. That edge is `infrastructure → foundation` (AD-1), and it does not make the new crate
   inherit anything: dependency direction, not feature unification, is what matters here — the
   new crate names none of `ego-infrastructure`'s deps (AD-2 criterion 3).

### AD-7 — `examples/reference-app` keeps its public surface and gains no shim

**Decision** — two files, both minimal:

```rust
// examples/reference-app/src/read_side/store.rs — replaces the two declarations (:150-238).
// Private: the file needs these names only so FakeDurable{Offset,Dedup}Store can keep
// wrapping them (:251, :282). Not re-published from here — mod.rs owns the crate's surface.
use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};
```

```rust
// examples/reference-app/src/read_side/mod.rs — :36-39 loses two names, gains one line
pub use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};
pub use store::{
    FakeDurableDedupStore, FakeDurableOffsetStore, ReadSideSink, SharedReadSideStore,
};
```

`mod.rs:106-107`, `:115-116`, and every other body in the example stay byte-identical.
`store.rs` keeps `DedupKey`/`OffsetKey` only if the fakes still name them — they do not
(`:251,282` wrap the store types, not the key aliases), so those two aliases (`:143-148`) move
with the structs that use them, and `HashMap`/`HashSet` leave `store.rs`'s import block with
them. `SharedReadSideStore` (`:33`), `ReadSideSink` (`:101`), both `FakeDurable*` types, and the
file's `#[cfg(test)]` module (`:309+`) all stay (NG-9, R3).

**Criteria**:

1. **D-6 is upheld in the sense that matters.** "No re-export is created in the example" means
   no *compatibility shim* — no path preserved for a consumer that would otherwise break.
   `mod.rs:36-39` is not that: it is the example's own pre-existing public surface, and keeping
   it identical is what stops this change from leaking an unrelated visibility narrowing into
   the diff (NG-7). Nothing outside the example resolves either name — grep over
   `examples/reference-app/tests/` returns zero hits, and the crate is a leaf
   (`reference-app/Cargo.toml:5` — `publish = false`, no dependent).
2. **The alternative was to drop both names from `mod.rs` entirely.** Rejected: it changes the
   example's public API in a change whose whole claim is that it changes nothing observable, and
   it buys nothing — `ReadSideHandles::in_memory()` still constructs both types either way.
3. **The example gains one normal dependency**, `ego-persistence-memory`
   (`reference-app/Cargo.toml`), joining the fourteen it already carries (`:31-64`). It is
   outside `verify-layers`' scope (AD-1 criterion 4).
4. **The orphan rule stays satisfied.** `FakeDurableOffsetStore` is a local type, so
   `impl OffsetStore for FakeDurableOffsetStore` remains legal with both the trait and the
   wrapped type foreign — exactly the situation `SharedReadSideStore` (`store.rs:21-33`) has
   lived in since PROD-014A.

### AD-8 — D-7's reachability change: what actually becomes possible, stated once and plainly

`InMemoryOperationReservationStore` moves from `ego-testkit` (`layers.toml:34`, layer `tooling`,
which `allowed_layers` maps to `None` — a sink: it may depend on anything, and **nothing may
depend on it** in the build graph, which is why `crates/infrastructure/Cargo.toml:22-26`
documents its own testkit edge as dev-only) into `ego-persistence-memory` (layer `foundation`).

**What does not change**: the struct, its fields, its `impl OperationReservationStore`
(`reservation.rs:99+`), its lease/fencing/takeover arithmetic, its `Mutex` strategy, and the
`is_durable()` question — `OperationReservationStore` declares no such method, so no durability
posture exists to preserve or break here. Zero behavior, zero contract, zero test-assertion
change.

**What changes**: today its only cross-crate consumers are three dev-dependency test files
(`crates/transport/tests/operation_key_extractor.rs:46,260`,
`crates/service-sdk/tests/retention_worker_lifecycle.rs:22`,
`crates/service-sdk/tests/cross_tenant_reservation_isolation.rs:101`), and the layer graph makes
a production edge to it **impossible to write**. After the move, any `foundation`,
`infrastructure`, `sdk`, `cross-cutting`, `application`, or `transport` crate may take a normal
dependency on it and wire it into a composition root with a `SystemClock`
(`crates/domain/src/time/clock.rs:33`). The gate stops being the answer; the reviewer becomes
the answer.

**Why the design accepts it**: it is the *only* implementation of `OperationReservationStore`
anywhere in the workspace, and its own doc comment already claims production fidelity — "a real,
full implementation of the real production port, not a parallel model of it"
(`reservation.rs:74-78`), with the constructor doc adding that "production code drives an
equivalent store with `SystemClock`" (`:84-90`). Leaving the workspace's sole implementation of
a shipped port inside a sink crate is the exact ownership defect this change exists to end.

**What this design does *not* do**: it adds no guard, no `#[cfg]`, no feature flag, and no
`Profile::Production` refusal for this store. Unlike `EventStore`/`Snapshot`, whose
`is_durable()` default (`persistence-api/src/persistence/event_store.rs:54-56`, `snapshot.rs:19-21`)
gives `require_durably_configured` (`persistent-entity/src/profile.rs:51-63`) something to reject,
`OperationReservationStore` has no durability predicate — so a refusal would have to be invented,
and inventing one is new behavior (NG-11). **The reachability is therefore genuinely open after
this change, and the mitigation is the reviewer's sign-off, not a mechanism.** Carried as
**OQ-2**, which restates the proposal's own unanswered question round item 1. If the answer is
"no", the correct outcome is dropping item 7 from IS-2 — slice S2 is independently revertible by
construction (AD-9) — not weakening this decision.

### AD-9 — Three slices, ordered by dependency growth; every intermediate state compiles workspace-wide

`sdd-tasks` owns task decomposition. This design owns only the boundaries and their order.

| Slice | Contents | Crate deps after this slice | RED test |
|---|---|---|---|
| **S1 — infrastructure four** | new crate skeleton (`Cargo.toml`, `lib.rs`, three `mod.rs`), workspace member, `layers.toml` entry (AD-1); `persistence/{event_store,repository,snapshot}.rs`, `read_side/store.rs` (+ its relocated test module); `in_memory/mod.rs` retargeted to four `pub use`s, four source files deleted (AD-6); `ego-infrastructure` gains the dep | `ego-persistence-api`, `async-trait`, `serde_json`; **dev**: `tokio`, `chrono` (EC-4/EC-7) | `crates/infrastructure/tests/in_memory_reexport_identity.rs` — names `ego_persistence_memory::…`, which does not exist yet |
| **S2 — reservation store** | `operation/reservation.rs`; `testkit/src/reservation.rs` split + module `pub use` (AD-5); `ego-testkit` gains the dep; **`ego-domain` + `chrono` promoted to normal deps** | adds `ego-domain`, `chrono` | `crates/testkit/tests/reservation_reexport_identity.rs` |
| **S3 — reference-app two** | `read_side/{offset,dedup}.rs`; `store.rs` + `mod.rs` retargeted (AD-7); `reference-app` gains the dep | unchanged | `cargo build -p reference-app` — the two declarations are gone and the new paths must resolve |

**Criteria**:

1. **The order is forced by the dependency closure, not by size.** S1's four files need only
   `ego-persistence-api` (AD-2 criterion 5), so the crate can exist and compile with a
   single-edge `Cargo.toml`. S2 is what makes `ego-domain` necessary (D-3), and landing that edge
   *with the file that needs it* keeps the reason for it visible in one diff instead of two.
   S3 needs nothing new at all, which is why it is last and smallest.
2. **This matches the proposal's own Approach ordering** ("infrastructure → testkit →
   reference-app") and answers R-4's slicing requirement, with the closure argument added.
3. **Every intermediate state is a compiling workspace.** After S1 the four infra
   implementations live in a second crate and every consumer resolves them unedited through
   `in_memory/mod.rs`. After S2 the same holds for testkit. Only S3 edits consumers, and its
   consumer is a leaf example with no dependent.
4. **Rollback stays per-slice.** The proposal's mid-flight rollback property holds at each
   boundary: revert S3 and the example's two files come back; revert S2 and `reservation.rs`
   reassembles (and, per AD-8, so does the layer-enforced unreachability); revert S1 and the
   crate disappears. No gate state was ever changed (AD-1), so there is nothing to unwind.
5. **Strict TDD is satisfiable at every slice.** Each RED is a compile failure for the right
   reason — a path that does not exist yet — which is exactly the RED shape
   `ego-rs-testing-tdd` accepts ("a test that fails to compile because the type does not exist
   yet is a valid RED").

### AD-10 — The compile-time identity proof lives in the **vacating** crates, not the new one

**Decision** — `crates/infrastructure/tests/in_memory_reexport_identity.rs` and
`crates/testkit/tests/reservation_reexport_identity.rs`. Each carries one identity witness per
row of the compatibility matrix, in CORE-PERSIST-A's shape: an identity coercion for
object-safe traits and a `where`-clause witness carrying both paths' bounds on one type
parameter for generic ones. A bare `use` is insufficient — it compiles just as happily against a
re-declared copy that merely shares the name (IS-5, R9).

**Criteria**:

1. **Placing it in `crates/persistence-memory/tests/` would require the new crate to
   dev-depend on `ego-infrastructure`.** Dev edges are excluded from the layer graph
   (`metadata.rs:122-128`, asserted at `:188-206`; the same carve-out
   `crates/persistence-api/Cargo.toml:27-32` relies on), so it would *pass* — but it would drag
   `sqlx` and the whole OpenTelemetry stack into the new crate's test build, and it would make
   the workspace's cleanest crate name its heaviest consumer in `cargo metadata`. Not worth it.
2. **The promise belongs to whoever makes it.** `ego-infrastructure` is the crate telling the
   world "`ego_infrastructure::persistence::in_memory::InMemoryEventStore` still resolves"; the
   test that proves it belongs next to that claim, where a future editor of `in_memory/mod.rs`
   trips over it.
3. **`reference-app` needs no such file.** It publishes no compatibility promise (AD-7), so its
   proof is `cargo build -p reference-app` — which is already in
   `openspec/config.yaml`'s `verify.build_command` (`cargo build --workspace`).

---

## Integration Points

| Boundary | Direction | Mechanism | Verified at |
|---|---|---|---|
| `ego-persistence-memory` → `ego-persistence-api` | new, one-way | `path` dependency; ports resolved directly, never through `ego-domain` | AD-2, AD-4 |
| `ego-persistence-memory` → `ego-domain` | new, one-way, **one item** | `path` dependency for `Clock` only | `reservation.rs:20,80`; AD-2 |
| `ego-persistence-memory` → any other workspace crate | **none** | no `path` dependency exists | AD-2 criterion 3; R11 |
| `ego-infrastructure` → `ego-persistence-memory` | new, one-way | `path` dependency + four item re-exports | AD-6 |
| `ego-testkit` → `ego-persistence-memory` | new, one-way | `path` dependency + one module-level `pub use` | AD-5 |
| `reference-app` → `ego-persistence-memory` | new, one-way | `path` dependency + two ordinary imports | AD-7 |
| every existing consumer → moved items | **unchanged** | resolved through the vacating crates' re-exports | table below; R9 |
| `layers.toml` → `verify-layers` | in | one new entry, existing loader | `layers.rs:148-158`; AD-1 |
| `allowed_layers` → `check_direction` | **none** | no match arm changes | `layers.rs:74-92`; AD-1 |
| runtime behavior | **none** | nothing executes differently | D-4, R5 |

**The compatibility matrix, restated with AD-3's paths and AD-5/AD-6/AD-7's mechanisms:**

| Old path (must resolve, unedited) | New canonical path | Re-export site |
|---|---|---|
| `ego_infrastructure::persistence::in_memory::InMemoryEventStore` | `ego_persistence_memory::persistence::event_store::InMemoryEventStore` | `in_memory/mod.rs` item `pub use` |
| `ego_infrastructure::persistence::in_memory::InMemoryRepository` | `ego_persistence_memory::persistence::repository::InMemoryRepository` | same |
| `ego_infrastructure::persistence::in_memory::InMemorySnapshotStore` | `ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore` | same |
| `ego_infrastructure::persistence::in_memory::{InMemoryReadSideStore, paginate}` | `ego_persistence_memory::read_side::store::{InMemoryReadSideStore, paginate}` | same |
| `ego_testkit::InMemoryOperationReservationStore` | `ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore` | `testkit/src/reservation.rs` `pub use`; `lib.rs:50` unchanged |
| `crate::reservation::…` inside testkit's two test modules (`:378`, `:525`) | same | same `pub use` — EC-3 |
| `InMemoryEventStoreUnitOfWork` | `ego_persistence_memory::persistence::event_store::InMemoryEventStoreUnitOfWork` | **none required** — private, reachable only as `Box<dyn EventStoreUnitOfWork<E>>` from `begin()` |
| reference-app's `InMemoryOffsetStore` / `InMemoryDedupStore` | `ego_persistence_memory::read_side::{offset,dedup}::…` | **none** — imports updated in place (AD-7) |
| `persistent_entity::persistence::{InMemoryEventStore, InMemorySnapshotStore}` | — | **unchanged**: `persistent-entity`'s own duplicates (`persistence.rs:571,733`), not moved (D-9, EC-6) |

Confirmed downstream consumers that must compile with byte-identical source:
`crates/infrastructure/tests/in_memory_event_store_conformance.rs:17-18`,
`crates/infrastructure/tests/commit_publishes_atomically.rs:25`,
`examples/reference-app/src/lib.rs:432-439`,
`crates/transport/tests/operation_key_extractor.rs:46,260`,
`crates/service-sdk/tests/retention_worker_lifecycle.rs:22`,
`crates/service-sdk/tests/cross_tenant_reservation_isolation.rs:101`.
`crates/persistent-entity/src/builder.rs` is **not** on this list (EC-6).

## Testing Strategy

Strict TDD (`openspec/config.yaml` → `apply.tdd: true`). Every slice's RED is a compile failure
naming a path that does not exist yet (AD-9), which `ego-rs-testing-tdd` accepts as a valid RED.
This change writes no new behavior, so it earns no new behavioral test — the assertions that
matter already exist and must keep passing **unmodified**.

| Level | Location | What it proves |
|---|---|---|
| Compile-time (primary) | `crates/infrastructure/tests/in_memory_reexport_identity.rs`, `crates/testkit/tests/reservation_reexport_identity.rs` | **IS-5 / R9** over the full matrix, never a sample. Identity witnesses, not bare `use`s (AD-10) |
| Relocated unit | `read_side/store.rs`'s moved `#[cfg(test)]` module | **D-4 / R5**: it moves verbatim with its file. Assertion count and text identical before and after — a changed assertion is a drift signal, not a cleanup (EC-4) |
| Retained unit | `testkit/src/reservation.rs:370-512`, `:514-…` | **D-8 / R16**: `TestClock` and both suites stay and drive the re-exported store through `super::` — byte-identical (EC-3, AD-5) |
| Retained unit | `examples/reference-app/src/read_side/store.rs:309+` | the example's own suite still exercises `SharedReadSideStore`, `ReadSideSink`, and both `FakeDurable*` types (NG-8, R3) |
| Integration (untouched) | `crates/infrastructure/tests/{in_memory_event_store_conformance,commit_publishes_atomically}.rs` | **R9 / R14**: conformance harness keeps its current shape and home; compiles through the re-export with byte-identical source |
| Integration (untouched) | `crates/persistent-entity/src/builder.rs:768,793`, `profile.rs:99-117` | **R6**: `presence_alone_is_not_durability` and both `try_build_rejects_explicit_in_memory_*` tests pass unmodified. They exercise `persistent-entity`'s own types (EC-6), so they hold trivially — and would hold anyway, since no moved type declares `is_durable()` (AD-4) |
| Gate | `cargo run -p xtask -- verify-layers` | **R11**: mapped (FR-001), every edge permitted (FR-002), no cycle (FR-003), isolated compile (FR-005) — with **no matrix edit** (AD-1) |
| Workspace | `cargo build --workspace`, `cargo test --workspace` | zero new failures, zero changed assertions; covers every in-tree consumer including the ones no test names (R-7) |

Six properties are **diff properties** — checked by reading the change, not by a test:

- **R5** — every moved body textually identical modulo module path and the AD-4 import table.
- **R7 / R11** — the new `Cargo.toml` names exactly the AD-2 set; no `sqlx`, Postgres, Stoolap,
  HTTP, or Kafka token appears anywhere under `crates/persistence-memory/`.
- **AD-2 criterion 4** — `rg 'ego_domain::' crates/persistence-memory/src` returns exactly one
  line.
- **R12 / R13** — `crates/runtime/`, `crates/effect-store/`, `crates/persistence/`, and every
  `.sql`/migration file are absent from the file list.
- **R15** — `crates/persistence-api/src/**` is absent from the file list.
- **R2 / R10** — the workspace-wide count of `impl <Port> for` blocks per moved port is
  unchanged; the only surviving non-canonical declarations are `persistent-entity`'s two
  duplicates and the declared test fakes.

## Threat Matrix

N/A — no routing, shell command, subprocess, VCS/PR automation, executable-file classification,
or process-integration boundary. This change moves Rust source files between four crates and
adds one line to a TOML table.

`ego-rs-security` is applicable only to confirm it is untouched: zero SQL text, zero query
construction, zero auth path, zero JWT verification, and zero `CrossTenantPermit` check appears
in the diff. Two tenant-isolation behaviors relocate and must relocate **verbatim**, which R5
already pins: `paginate`'s fail-closed empty-tenant guard (`read_side_store.rs:111-115`) and
`resolve_tenant`-keyed storage in `event_store.rs`, `repository.rs`, and `snapshot.rs`
(`snapshot.rs:38-39,49-50`). `resolve_tenant` itself is not moved — it stays in
`ego-persistence-api` (`persistence/tenant.rs`), where CORE-PERSIST-A put it.

One access-boundary change is real and is not hidden here: **AD-8**. It is a reachability
change, not a code-behavior change, and it is the one item in this design that needs a human
answer (OQ-2).

## Migration / Rollout

**No migration required.** No data, schema, migration file, or persisted state exists in either
direction — this change writes nothing at runtime. No feature flag and no phased rollout: the
workspace either compiles with the new layout or it does not.

Rollback is the proposal's, unchanged, and available at each of AD-9's three boundaries: drop
`crates/persistence-memory/`, restore the four `ego-infrastructure` files and `in_memory/mod.rs`,
reassemble `crates/testkit/src/reservation.rs`, restore the reference app's two declarations and
its two import sites, remove the `layers.toml` entry and the workspace member, and drop the three
`Cargo.toml` edges. `xtask/src/layers.rs` was never opened, so there is no gate state to unwind.

## Traceability

| Proposal / explore item | Resolved by | Note |
|---|---|---|
| D-1, IS-1 | AD-1, AD-2, AD-3 | crate at `crates/persistence-memory/`, package `ego-persistence-memory` |
| **D-2** | **AD-1** | mapping lives in `layers.toml`, not `layers.rs`; one line; every edge already permitted; `layers.rs` untouched — **confirmed against source, not assumed** |
| **D-3, R-3** | **AD-2 (esp. criteria 1, 4, 5)** | exact dependency list derived per file; the `ego-domain` edge is `Clock` and only `Clock`; no third edge exists, confirmed for all seven implementations |
| D-4, R5 | **AD-4**, EC-1, EC-2 | the rewrite enumerated per file; `operation::` does not flatten like `ego_domain::operation` does; one inline path exists that D-4's phrasing does not cover (→ OQ-1) |
| D-5, IS-3, R9 | **AD-6**, **AD-5**, EC-3 | item granularity for infrastructure (private modules), module-level `pub use` for testkit (two test modules resolve through `super::`) |
| D-6, IS-4, R8 | **AD-7**, EC-5 | two files, not one; example's public surface preserved; no compatibility shim |
| **D-7, R-1** | **AD-8**, **OQ-2** | the reachability change stated in full, with what does and does not change, and why no mechanism replaces the reviewer |
| D-8, R16 | AD-5 | `TestClock` and both colocated suites stay, byte-identical |
| D-9, KD-5, KD-6, NG-1, NG-2, R17, F-5, F-6 | — | `crates/persistent-entity/` appears in no file list here; EC-6 confirms `builder.rs` needs nothing |
| D-10, NG-6, R12, R18, F-1 | — | `crates/runtime/` and `crates/effect-store/` appear in no file list here; `explore.md`'s EFFECT STORE BLOCKER ANALYSIS consumed as given |
| D-11, R6 | AD-4, Testing | no moved type declares `is_durable()`; the rejecting tests exercise `persistent-entity`'s own types (EC-6) and are untouched either way |
| D-12, KD-1, NG-10, R4 | AD-3 | `ProjectionStateStore` gets no module, no file, no `todo!()` — the tree above has no row for it |
| IS-2 | AD-3, AD-4, AD-9 | all seven, each mapped to a destination file and a slice |
| IS-5, R9 | **AD-10** | identity witnesses in the vacating crates, full matrix, no sampling |
| NG-8, R3 | AD-7 | `FakeDurable*` stay in the example, byte-identical, wrapping the now-external types |
| NG-11, R7 | AD-2, AD-3 | no root re-export, no new item, no backend token, no `#![deny(missing_docs)]`-forced doc additions |
| NG-12 | AD-6, AD-7, AD-9 | exactly four `Cargo.toml` files change: the new crate, `ego-infrastructure`, `ego-testkit`, `reference-app`, plus the root member list |
| R-2 | AD-4, Testing diff properties | verbatim is a text comparison, and the permitted edits are an enumerated table rather than a judgement call |
| **R-4** | **AD-9** | three slices by source crate, each independently compiling and revertible; closure argument added to the proposal's ordering |
| R-5 | — | `persistence-api-surface` re-scoping is `sdd-spec`'s; this design edits no spec and touches no `crates/persistence-api/` file |
| R-6 | AD-1 (closing note) | `layers.toml:6`'s stale comment named, not fixed (NG-7) |
| R-7 | AD-10, Testing | compile-time proof over the full matrix plus `cargo build --workspace` |
| KD-4, NG-4, R14, F-4 | Testing | no harness added, extended, or generalized |
| `config.yaml` "sequence diagrams" | Technical Approach | explicit N/A — zero call paths added, removed, or reordered |
| `config.yaml` "no circular dependencies" | Dependency Graph | two one-way edges into a crate that names no workspace dependency of its own |
| `config.yaml` "decisions with rationale" | AD-1..AD-10 | each carries criteria and, where one existed, the rejected alternative |

## Open Questions

- [ ] **OQ-1 — D-4's phrasing does not cover `reservation.rs:70`.** It is an inline
      fully-qualified path, not a `use` line (EC-2). AD-5 rewrites it, which keeps the crate's
      `ego_domain::` surface at exactly one line and names the same item either way. Confirm the
      one-clause amendment ("`use` lines" → "path expressions naming a relocated port item")
      before `sdd-tasks` plans S2. **Non-blocking**: leaving `:70` byte-identical also compiles;
      only AD-2 criterion 4 loses its clean grep.
- [ ] **OQ-2 — D-7's reachability change still has no answer** (proposal question round, item 1).
      AD-8 states exactly what becomes possible and confirms no mechanism can replace the
      sign-off without inventing behavior (NG-11). **Blocking for slice S2 only** — S1 and S3
      are unaffected and can proceed. A "no" drops item 7 from IS-2; it does not weaken D-7.
- [ ] **OQ-3 — AD-3 refines `explore.md`'s tree** (`read_side/read_side_store.rs` →
      `read_side/store.rs`), which shifts one row of its COMPATIBILITY REEXPORT MATRIX. The
      restated matrix under Integration Points is authoritative for `sdd-tasks`. Flagged so the
      shift is a decision on the record rather than a discrepancy someone finds later.
      **Non-blocking.**
