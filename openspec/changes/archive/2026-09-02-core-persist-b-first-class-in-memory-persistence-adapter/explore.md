# CORE-PERSIST-B — Explore: First-Class In-Memory Persistence Adapter

**Phase**: `sdd-explore`
**Change**: `core-persist-b-first-class-in-memory-persistence-adapter`
**Status**: complete — ready for `sdd-propose`

## CURRENT MEMORY IMPLEMENTATION INVENTORY

Every `InMemory*`-named persistence-relevant struct found workspace-wide, plus the two non-`InMemory*`-named unit-of-work types that are functionally in-memory implementations:

| # | Name | File:Line | Port implemented |
|---|---|---|---|
| 1 | `InMemoryEventStore` | `crates/infrastructure/src/persistence/in_memory/event_store.rs:89` | `EventStore<E>` |
| 2 | `InMemoryEventStoreUnitOfWork` (private) | `crates/infrastructure/src/persistence/in_memory/event_store.rs:214` | `EventStoreUnitOfWork<E>` |
| 3 | `InMemoryRepository` | `crates/infrastructure/src/persistence/in_memory/repository.rs:11` | `Repository<A>` |
| 4 | `InMemorySnapshotStore` | `crates/infrastructure/src/persistence/in_memory/snapshot.rs:12` | `Snapshot` |
| 5 | `InMemoryReadSideStore` (+ `paginate` fn) | `crates/infrastructure/src/persistence/in_memory/read_side_store.rs:24,105` | `ReadSideStore<Value>` |
| 6 | `InMemoryEventStore` | `crates/persistent-entity/src/persistence.rs:571` | `EventStore<E>` (DUPLICATE) |
| 7 | `StagingUnitOfWork` (private) | `crates/persistent-entity/src/persistence.rs:787` | `EventStoreUnitOfWork<E>` (DUPLICATE) |
| 8 | `InMemorySnapshotStore` | `crates/persistent-entity/src/persistence.rs:733` | `Snapshot` (DUPLICATE — divergent) |
| 9 | `InMemoryOperationReservationStore` | `crates/testkit/src/reservation.rs:79` | `OperationReservationStore` |
| 10 | `InMemoryOffsetStore` | `examples/reference-app/src/read_side/store.rs:153` | `OffsetStore` |
| 11 | `InMemoryDedupStore` | `examples/reference-app/src/read_side/store.rs:199` | `DedupStore` |
| 12 | `SharedReadSideStore` | `examples/reference-app/src/read_side/store.rs:33` | `ReadSideStore<Value>` (wrapper) |
| 13 | `FakeDurableOffsetStore` | `examples/reference-app/src/read_side/store.rs:251` | `OffsetStore` (lies about durability) |
| 14 | `FakeDurableDedupStore` | `examples/reference-app/src/read_side/store.rs:282` | `DedupStore` (lies about durability) |
| 15 | `InMemoryEffectStore` | `crates/runtime/src/effects/store.rs:531` | `EffectStateStore`, `EffectDedupStore` (D-9-deferred) |
| — | `ProjectionStateStore` | `crates/persistence-api/src/read_side/projection_state.rs:27` | **Zero implementations anywhere** (confirmed via `openspec/changes/archive/2026-09-02-core-persist-a-unified-persistence-api-surface/verify-report.md:64`: `rg "impl.*ProjectionStateStore for"` → zero) |

Also ~150+ test-local specialized fakes (`FailingSnapshotStore`, `PanicOnLoadEventStore`, `StubOffsetStore`, `RecordingStore`, etc.) scattered across `#[cfg(test)]` modules and `tests/*.rs` in `crates/persistent-entity/`, `crates/service-sdk/`, `crates/runtime/`, `examples/reference-app/tests/` — see SPECIALIZED TEST FAKE INVENTORY.

## PORT → IMPLEMENTATION MATRIX

| Port | File:Line | In-memory impl(s) | Count |
|---|---|---|---|
| `EventStore<E>` | `crates/persistence-api/src/persistence/event_store.rs:47` | infra `event_store.rs:89`; persistent-entity `persistence.rs:571` | 2 (duplicate) |
| `EventStoreUnitOfWork<E>` | `event_store.rs:186` | infra `event_store.rs:214`; persistent-entity `persistence.rs:787` | 2 (duplicate) |
| `Repository<A>` | `crates/persistence-api/src/persistence/repository.rs:12` | infra `repository.rs:11` | 1 |
| `Snapshot` | `crates/persistence-api/src/persistence/snapshot.rs:14` | infra `snapshot.rs:12`; persistent-entity `persistence.rs:733` | 2 (duplicate, **divergent semantics**) |
| `OffsetStore` | `crates/persistence-api/src/read_side/offset.rs:55` | reference-app `store.rs:153` | 1 (only impl workspace-wide, self-documented at `store.rs:150-151`) |
| `DedupStore` | `crates/persistence-api/src/read_side/dedup.rs:25` | reference-app `store.rs:199` | 1 (only impl workspace-wide, self-documented at `store.rs:196-197`) |
| `ReadSideStore<E>` | `crates/persistence-api/src/read_side/store.rs:26` | infra `read_side_store.rs:24`; reference-app `store.rs:33` (delegating wrapper) | 1 canonical + 1 example wrapper |
| `ProjectionStateStore` | `crates/persistence-api/src/read_side/projection_state.rs:27` | none | 0 (MISSING, by design — KD-1) |
| `OperationReservationStore` | `crates/persistence-api/src/operation/reservation.rs:66` | testkit `reservation.rs:79` | 1 (only impl workspace-wide; testkit doc comment `reservation.rs:74-78`: "real, full implementation... not a parallel model") |
| `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance` | `crates/runtime/src/effects/store.rs:238,418,474` | `store.rs:531` (`InMemoryEffectStore`) | 1 each — **D-9 boundary, not owned by ego-persistence-api** |

## DUPLICATE IMPLEMENTATION ANALYSIS

**Pair 1 — `EventStore<E>` (additive-capability duplicate, not a semantic conflict)**
- `crates/infrastructure/src/persistence/in_memory/event_store.rs:89` — `std::sync::Mutex`, poison-recovering `lock()` helper (`event_store.rs:291-295`), `stream_version_offset` uses the trait default (0).
- `crates/persistent-entity/src/persistence.rs:571` — `parking_lot::Mutex` (non-poisoning), plus an extra public builder `with_version_offset()` (`persistence.rs:600-611`) and a real `stream_version_offset` override (`persistence.rs:719-727`) that lets a test simulate events already covered by a pre-seeded snapshot.
- Consumed exclusively by `crates/persistent-entity/tests/in_memory_version_offset_parity.rs:15,22-23` (`InMemoryEventStore::<TestEvent>::new().with_version_offset(...)`).
- Both resolve tenant identically and share the same conflict/version-check arithmetic (`offset + committed + staged`, `persistence.rs:816` vs `committed + staged`, `event_store.rs:241`, the latter having no offset term because it has no version-offset feature).
- **Verdict**: genuine fork with additive capability, not a contradiction. Consolidating would either (a) drop the `with_version_offset` capability persistent-entity depends on, or (b) add that capability to the canonical crate — both are new behavior relative to whichever side didn't have it, which violates the zero-new-behavior constraint. **DEFER consolidation; keep both, cite this pair explicitly as named debt in propose.**

**Pair 2 — `EventStoreUnitOfWork<E>` implementations (tied to Pair 1)**
- `InMemoryEventStoreUnitOfWork` (private, `event_store.rs:214`) vs `StagingUnitOfWork` (private, `persistence.rs:787`). Neither is `pub`; both are only reachable via `Box<dyn EventStoreUnitOfWork<E>>` returned from `begin()`. Same fate as Pair 1 — coupled to their respective `EventStore`, not independently movable.

**Pair 3 — `Snapshot` (CONFIRMED SEMANTIC CONFLICT — real bug)**
- `crates/infrastructure/src/persistence/in_memory/snapshot.rs:12` — keys by `(aggregate_id, resolve_tenant(tenant_id)?)` (`snapshot.rs:38-39,49-50`). Correctly tenant-isolates.
- `crates/persistent-entity/src/persistence.rs:733` — keys by `stream_id` alone; `save_snapshot`/`load_snapshot` (`persistence.rs:746-765`) take `_tenant_id: Option<&str>` and **never read it, never call `resolve_tenant`**.
- **Consequence**: two different tenants saving a snapshot under the same `aggregate_id` against persistent-entity's `InMemorySnapshotStore` silently overwrite each other's data.
- **Verdict**: this is a real, DIFFERENT-semantics duplicate — not resolvable by "just picking one and moving it," because moving the infra version to be the sole canonical implementation everywhere persistent-entity uses it today would be a silent *behavior change* for any persistent-entity caller currently relying on (or unaware of) the tenant-less key. **Must be raised explicitly at propose time as a named decision, not silently resolved here.** Recorded again in OUT-OF-SCOPE CORRECTNESS FINDINGS below.

## SPECIALIZED TEST FAKE INVENTORY

Real, full-contract in-memory implementations that must **stay** where they are (per hard constraint — never promoted into the general adapter):

| Name | File:Line | Reason it's a fake, not a candidate |
|---|---|---|
| `FakeDurableOffsetStore` | `examples/reference-app/src/read_side/store.rs:251` | Wraps `InMemoryOffsetStore`, overrides `is_durable() -> true` (a lie); doc comment `store.rs:240-249` explicitly: "Never wire this into a deployment" |
| `FakeDurableDedupStore` | `examples/reference-app/src/read_side/store.rs:282` | Same pattern, `store.rs:279-280` |

Broad class (not individually enumerated per the MOVE MATRIX, since by definition they never move): ~150+ test-local structs matching `(Failing|Blocking|Spy|Stub|Recording|PanicOn|Flaky|Scripted|Unusable|ProbeCounting)\w*(Store|Repository)` inside `#[cfg(test)]` modules or `tests/*.rs`, across `crates/persistent-entity/tests/`, `crates/service-sdk/tests/`, `crates/service-sdk/src/runtime/builder.rs`, `crates/runtime/src/effects/runner.rs`, `crates/runtime/src/read_side/scheduler.rs`, `examples/reference-app/tests/`. These stay in place by construction — the objective's hard constraint already excludes them, and their volume makes per-item MOVE MATRIX rows non-actionable noise.

`InMemoryOperationReservationStore` (`crates/testkit/src/reservation.rs:79`) is explicitly **not** a specialized fake — its own doc comment (`reservation.rs:74-78`) invokes the "same-contract principle": "a real, full implementation of the real production port, not a parallel model of it." Its constructor doc (`reservation.rs:84-90`) states production code drives an equivalent store with `SystemClock`; only the clock differs between test and production use. It is classified as a CANONICAL CANDIDATE currently misplaced in a tooling-layer crate — see MOVE MATRIX.

## EXAMPLE-LOCAL IMPLEMENTATION ANALYSIS

`examples/reference-app/src/read_side/store.rs` (458 lines) holds five structs:

- `InMemoryOffsetStore` (`store.rs:153`) — **generic, no reference-app-specific logic**. Doc comment (`store.rs:150-151`): "this workspace has no other in-memory reference implementation of it." **MOVE CANDIDATE.**
- `InMemoryDedupStore` (`store.rs:199`) — same pattern, same doc-comment admission (`store.rs:196-197`). **MOVE CANDIDATE.**
- `SharedReadSideStore` (`store.rs:33`) — an orphan-rule wrapper (`Arc<Mutex<InMemoryReadSideStore>>`) whose `fetch` (`store.rs:57-94`) contains example-specific tenant/tag cross-check logic (`super::tenant_from_tag`, `store.rs:74-78`) that is genuinely reference-app business logic, not a generic contract implementation. **STAYS — example-local, not a move candidate.**
- `ReadSideSink` (`store.rs:101`) — writes `RegisterUser` events; explicitly example-specific. **STAYS.**
- `FakeDurableOffsetStore`/`FakeDurableDedupStore` (`store.rs:251,282`) — specialized fakes. **STAY** (see above).

`examples/reference-app/src/lib.rs:430-442` (`EntityEventStores::in_memory()`) directly instantiates `ego_infrastructure::persistence::in_memory::InMemoryEventStore`/`InMemorySnapshotStore` — confirms the infra in-memory adapters are load-bearing for the example, and any move of those types must preserve this exact call path (via re-export or updated import, per the compatibility strategy chosen at propose time).

## DEPENDENCY GRAPH

Import-level evidence for every canonical-candidate implementation file (confirms no accidental dependency on Postgres, application, or heavier infra deps):

| File | Imports (non-std) |
|---|---|
| `crates/infrastructure/src/persistence/in_memory/event_store.rs:1-8` | `ego_domain::event::DomainEvent`, `ego_domain::operation::OperationReceipt`, `ego_domain::persistence::{resolve_tenant, EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent}` |
| `crates/infrastructure/src/persistence/in_memory/repository.rs:1-4` | `ego_domain::persistence::{resolve_tenant, PersistenceError, Repository}` |
| `crates/infrastructure/src/persistence/in_memory/snapshot.rs:1-5` | `ego_domain::persistence::{resolve_tenant, PersistenceError, Snapshot}`, `serde_json::Value` |
| `crates/infrastructure/src/persistence/in_memory/read_side_store.rs:6-11` | `ego_domain::read_side::{event_stream::EventStreamElement, event_tag::EventTag, offset::Offset, store::{ReadSideStore, ReadSideStoreError}}` |
| `crates/testkit/src/reservation.rs:11-20` | `ego_domain::operation::{...}`, `ego_domain::Clock` — **no** `ego_runtime`/`ego_service_sdk`/`ego_security_sdk`/`persistent_entity` import, despite `ego-testkit`'s `Cargo.toml` carrying all four as crate-level deps for other modules |
| `examples/reference-app/src/read_side/store.rs:7-19` | `ego_domain::read_side::{dedup, event_stream, event_tag, offset, store}`, `ego_infrastructure::persistence::in_memory::{paginate, InMemoryReadSideStore}` |

Crate-level `Cargo.toml` deps (confirms layer footprint each source crate currently carries — not necessarily inherited by every module):
- `crates/infrastructure/Cargo.toml` — `ego-domain`, `ego-application`, `ego-persistence` (Postgres), `sqlx`, opentelemetry stack, `dashmap`. **The in_memory submodule imports none of these beyond `ego_domain`.**
- `crates/persistent-entity/Cargo.toml` — only `ego-domain` as a workspace path dep (dev-dep on `ego-testkit`, excluded from the layer graph).
- `crates/testkit/Cargo.toml` — `ego-domain`, `ego-runtime`, `ego-security-sdk`, `ego-service-sdk`, `persistent-entity`. **`reservation.rs` uses none of the latter four.**
- `crates/runtime/Cargo.toml:7,11` — `ego-domain`, `persistent-entity`.
- `examples/reference-app/Cargo.toml` — `ego-domain`, `ego-infrastructure`, `ego-runtime`, `ego-effect-store`, `persistent-entity`, `ego-persistence` (Postgres), plus axum/sqlx/utoipa.

`xtask/src/layers.rs:15-78` layer assignments relevant to this change (verified directly against source, not `layers.toml` as a literal file — the domain-self-edge relaxation from CORE-PERSIST-A's D-4 is confirmed live at line 76: `"domain" => Some(&["domain"])`):
```
"ego-domain" = "domain"
"ego-persistence-api" = "domain"
"ego-infrastructure" = "infrastructure"
"ego-runtime" = "foundation"
"persistent-entity" = "foundation"
"ego-testkit" = "tooling"   # sink; no production crate may depend on tooling
```

**Consequence**: `InMemoryOperationReservationStore` currently lives in a "tooling" (sink) layer crate. Its only cross-crate consumers today are dev-dependency test files: `crates/transport/tests/operation_key_extractor.rs`, `crates/service-sdk/tests/retention_worker_lifecycle.rs`, `crates/service-sdk/tests/cross_tenant_reservation_isolation.rs`. No production crate can depend on it as it sits today — moving it to a domain/foundation-layer crate would be the first time it becomes production-reachable. This is a reachability change, not a behavior change (same struct, same trait impl) — see MOVE MATRIX entry.

## TARGET CRATE / MODULE TREE

Per D-1 of the CORE-PERSIST-A proposal (`openspec/changes/archive/.../proposal.md:43`): "`persistence-postgres` / `persistence-memory` renames are CORE-PERSIST-B/C's job, not this change's" — confirming this change is the intended home for that consolidation.

Proposed tree (working name, not forced):

```
crates/persistence-memory/            (package: ego-persistence-memory)
├── Cargo.toml                        # deps: ego-persistence-api only (+ std, async-trait, serde_json, chrono if needed)
└── src/
    ├── lib.rs
    ├── persistence/
    │   ├── mod.rs
    │   ├── event_store.rs            # InMemoryEventStore + InMemoryEventStoreUnitOfWork (from infra)
    │   ├── repository.rs             # InMemoryRepository (from infra)
    │   └── snapshot.rs               # InMemorySnapshotStore (from infra — the tenant-correct one)
    ├── read_side/
    │   ├── mod.rs
    │   ├── read_side_store.rs        # InMemoryReadSideStore + paginate (from infra)
    │   ├── offset.rs                 # InMemoryOffsetStore (from reference-app)
    │   └── dedup.rs                  # InMemoryDedupStore (from reference-app)
    └── operation/
        ├── mod.rs
        └── reservation.rs            # InMemoryOperationReservationStore (from testkit)
```

Layer classification (open design question for `sdd-propose`): either `domain` (matching `ego-persistence-api`, exercising the domain-self-edge relaxation `xtask/src/layers.rs` already grants per CORE-PERSIST-A's D-4) or `foundation` (matching `persistent-entity`/`ego-runtime`'s precedent of depending on a single domain crate). Either satisfies the layer graph's rules; the choice affects which layer's consumers may depend on it without a new rule.

## MOVE MATRIX

```
Implementation: InMemoryEventStore
Current owner: crates/infrastructure/src/persistence/in_memory/event_store.rs:89
Implements: ego_persistence_api::persistence::event_store::EventStore<E>
Classification: CANONICAL CANDIDATE
Canonical target: ego_persistence_memory::persistence::event_store::InMemoryEventStore
Compatibility: reexport from ego_infrastructure::persistence::in_memory::InMemoryEventStore (mod.rs:12); also load-bearing for examples/reference-app/src/lib.rs:432-433 and crates/persistent-entity/src/builder.rs:356
Behavior change: NONE — file imports only ego_domain::{event,operation,persistence}::* (event_store.rs:5-8)
Move allowed: YES
```

```
Implementation: InMemoryEventStoreUnitOfWork
Current owner: crates/infrastructure/src/persistence/in_memory/event_store.rs:214
Implements: ego_persistence_api::persistence::event_store::EventStoreUnitOfWork<E>
Classification: CANONICAL CANDIDATE
Canonical target: ego_persistence_memory::persistence::event_store::InMemoryEventStoreUnitOfWork
Compatibility: not required — private (non-pub) struct, only reachable via Box<dyn EventStoreUnitOfWork<E>> returned from InMemoryEventStore::begin()
Behavior change: NONE — moves with its parent store
Move allowed: YES
```

```
Implementation: InMemoryRepository
Current owner: crates/infrastructure/src/persistence/in_memory/repository.rs:11
Implements: ego_persistence_api::persistence::repository::Repository<A>
Classification: CANONICAL CANDIDATE
Canonical target: ego_persistence_memory::persistence::repository::InMemoryRepository
Compatibility: reexport from ego_infrastructure::persistence::in_memory::InMemoryRepository (mod.rs:14)
Behavior change: NONE — imports only ego_domain::persistence::{resolve_tenant, PersistenceError, Repository} (repository.rs:3-4)
Move allowed: YES
```

```
Implementation: InMemorySnapshotStore (infrastructure)
Current owner: crates/infrastructure/src/persistence/in_memory/snapshot.rs:12
Implements: ego_persistence_api::persistence::snapshot::Snapshot
Classification: CANONICAL CANDIDATE
Canonical target: ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore
Compatibility: reexport from ego_infrastructure::persistence::in_memory::InMemorySnapshotStore (mod.rs:15); load-bearing for examples/reference-app/src/lib.rs:434-439 and crates/persistent-entity/src/builder.rs:360
Behavior change: NONE — imports only ego_domain::persistence::{resolve_tenant, PersistenceError, Snapshot} (snapshot.rs:3-4); correctly tenant-scopes (snapshot.rs:38,49)
Move allowed: YES
```

```
Implementation: InMemoryReadSideStore (+ paginate)
Current owner: crates/infrastructure/src/persistence/in_memory/read_side_store.rs:24,105
Implements: ego_persistence_api::read_side::store::ReadSideStore<serde_json::Value>
Classification: CANONICAL CANDIDATE
Canonical target: ego_persistence_memory::read_side::read_side_store::{InMemoryReadSideStore, paginate}
Compatibility: reexport from ego_infrastructure::persistence::in_memory::{InMemoryReadSideStore, paginate} (mod.rs:13); paginate is directly imported by examples/reference-app/src/read_side/store.rs:18 and must keep resolving
Behavior change: NONE — imports only ego_domain::read_side::{event_stream, event_tag, offset, store}::* (read_side_store.rs:8-11); fail-closed empty-tenant behavior preserved verbatim (read_side_store.rs:113-115)
Move allowed: YES
```

```
Implementation: InMemoryEventStore (persistent-entity)
Current owner: crates/persistent-entity/src/persistence.rs:571
Implements: ego_persistence_api::persistence::event_store::EventStore<E> (via ego_domain re-export)
Classification: DUPLICATE (additive capability — with_version_offset/stream_version_offset override, persistence.rs:600-611,719-727 — not present on the canonical candidate)
Canonical target: N/A — stays in persistent-entity this change
Compatibility: not required (no path change)
Behavior change: N/A — not moved. Consolidating onto the canonical type would either drop this capability (a behavior loss for crates/persistent-entity/tests/in_memory_version_offset_parity.rs) or add it to the canonical crate (new behavior) — both violate the zero-new-behavior constraint.
Move allowed: NO — DEFER to a follow-up change (named debt)
```

```
Implementation: StagingUnitOfWork
Current owner: crates/persistent-entity/src/persistence.rs:787
Implements: ego_persistence_api::persistence::event_store::EventStoreUnitOfWork<E>
Classification: DUPLICATE (tied to persistent-entity's InMemoryEventStore above)
Canonical target: N/A
Compatibility: not required — private struct
Behavior change: N/A — not moved
Move allowed: NO — DEFER, coupled to the entry above
```

```
Implementation: InMemorySnapshotStore (persistent-entity)
Current owner: crates/persistent-entity/src/persistence.rs:733
Implements: ego_persistence_api::persistence::snapshot::Snapshot (via ego_domain re-export)
Classification: DUPLICATE — CONFIRMED SEMANTIC CONFLICT (tenant_id parameter ignored entirely, persistence.rs:746-765, unlike the tenant-correct infra version)
Canonical target: N/A this change
Compatibility: not required (no path change)
Behavior change: N/A — not moved. Any consolidation onto the tenant-correct canonical type would silently change behavior for whatever persistent-entity caller currently depends on (or is unaware of) tenant-less keying — this is a correctness fix wearing a move's name, explicitly forbidden by the hard constraints.
Move allowed: NO — flag as blocking named debt requiring an explicit propose-time decision (which caller sites are affected, whether persistent-entity should be migrated onto the correct implementation as a *separate*, reviewed bug-fix change)
```

```
Implementation: InMemoryOperationReservationStore
Current owner: crates/testkit/src/reservation.rs:79
Implements: ego_persistence_api::operation::reservation::OperationReservationStore
Classification: CANONICAL CANDIDATE (only implementation of this port workspace-wide; testkit's own doc comment, reservation.rs:74-78, declares it a full production-faithful implementation, not a fake)
Canonical target: ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore
Compatibility: reexport from ego_testkit::InMemoryOperationReservationStore (crates/testkit/src/lib.rs re-export) — required because crates/transport/tests/operation_key_extractor.rs, crates/service-sdk/tests/{retention_worker_lifecycle,cross_tenant_reservation_isolation}.rs consume it via that path today
Behavior change: NONE at the code level — imports only ego_domain::operation::* and ego_domain::Clock (reservation.rs:16,20), no ego_runtime/ego_service_sdk/ego_security_sdk/persistent_entity dependency despite the crate carrying them. NOTE: moving out of ego-testkit's "tooling" (sink) layer into a domain/foundation-layer crate makes it production-reachable for the first time — a reachability change, not a code-behavior change, but material enough to flag explicitly for propose-phase sign-off.
Move allowed: YES, with the reachability note flagged above
```

```
Implementation: InMemoryOffsetStore
Current owner: examples/reference-app/src/read_side/store.rs:153
Implements: ego_persistence_api::read_side::offset::OffsetStore
Classification: EXAMPLE-LOCAL (generic — no reference-app-specific logic; only in-memory OffsetStore workspace-wide, store.rs:150-151)
Canonical target: ego_persistence_memory::read_side::offset::InMemoryOffsetStore
Compatibility: not required as a public API guarantee (reference-app is a leaf example, no external consumer), but reference-app's own call sites must be updated to import from the new crate
Behavior change: NONE — imports only ego_domain::read_side::{dedup,event_stream,event_tag,offset,store}::* (store.rs:13-17)
Move allowed: YES
```

```
Implementation: InMemoryDedupStore
Current owner: examples/reference-app/src/read_side/store.rs:199
Implements: ego_persistence_api::read_side::dedup::DedupStore
Classification: EXAMPLE-LOCAL (generic — same pattern as InMemoryOffsetStore, store.rs:196-197)
Canonical target: ego_persistence_memory::read_side::dedup::InMemoryDedupStore
Compatibility: not required as a public API guarantee; reference-app call sites updated
Behavior change: NONE
Move allowed: YES
```

```
Implementation: SharedReadSideStore
Current owner: examples/reference-app/src/read_side/store.rs:33
Implements: ego_persistence_api::read_side::store::ReadSideStore<serde_json::Value> (by delegation to InMemoryReadSideStore)
Classification: EXAMPLE-LOCAL (orphan-rule wrapper carrying example-specific tenant/tag cross-check logic, store.rs:66-78)
Canonical target: N/A
Compatibility: not required
Behavior change: N/A — not moved
Move allowed: NO — stays; not a generic reusable implementation, it is reference-app wiring
```

```
Implementation: FakeDurableOffsetStore
Current owner: examples/reference-app/src/read_side/store.rs:251
Implements: ego_persistence_api::read_side::offset::OffsetStore (is_durable() lies -> true)
Classification: SPECIALIZED TEST FAKE
Canonical target: N/A
Compatibility: not required
Behavior change: N/A — not moved
Move allowed: NO — explicitly excluded by hard constraint (never promote fault-injection/fake-durable stores)
```

```
Implementation: FakeDurableDedupStore
Current owner: examples/reference-app/src/read_side/store.rs:282
Implements: ego_persistence_api::read_side::dedup::DedupStore (is_durable() lies -> true)
Classification: SPECIALIZED TEST FAKE
Canonical target: N/A
Compatibility: not required
Behavior change: N/A — not moved
Move allowed: NO — same reason as above
```

```
Implementation: InMemoryEffectStore
Current owner: crates/runtime/src/effects/store.rs:531
Implements: EffectStateStore (store.rs:562), EffectDedupStore (store.rs:697) — ports owned by ego-runtime, not ego-persistence-api
Classification: MISSING FROM SCOPE (D-9 boundary — deferred, not classified as movable or non-movable within this change)
Canonical target: N/A this change
Compatibility: not required — untouched
Behavior change: N/A — not moved
Move allowed: NO — blocked by D-9 (see EFFECT STORE BLOCKER ANALYSIS)
```

## COMPATIBILITY REEXPORT MATRIX

| Old path | New canonical path | Reexport required at |
|---|---|---|
| `ego_infrastructure::persistence::in_memory::InMemoryEventStore` | `ego_persistence_memory::persistence::event_store::InMemoryEventStore` | `crates/infrastructure/src/persistence/in_memory/mod.rs:12` (`pub use`) |
| `ego_infrastructure::persistence::in_memory::InMemoryRepository` | `ego_persistence_memory::persistence::repository::InMemoryRepository` | `mod.rs:14` |
| `ego_infrastructure::persistence::in_memory::InMemorySnapshotStore` | `ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore` | `mod.rs:15` |
| `ego_infrastructure::persistence::in_memory::{InMemoryReadSideStore, paginate}` | `ego_persistence_memory::read_side::read_side_store::{InMemoryReadSideStore, paginate}` | `mod.rs:13` |
| `persistent_entity::persistence::{InMemoryEventStore, InMemorySnapshotStore}` (persistent-entity's own re-export, `testing.rs:23`) | **unchanged** — these are persistent-entity's own duplicate types, not moved | N/A |
| `ego_testkit::InMemoryOperationReservationStore` | `ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore` | `crates/testkit/src/lib.rs` (`pub use`) |
| `examples/reference-app` internal `InMemoryOffsetStore`/`InMemoryDedupStore` | `ego_persistence_memory::read_side::{offset::InMemoryOffsetStore, dedup::InMemoryDedupStore}` | No external reexport needed (leaf example); update `use` statements directly in `examples/reference-app/src/read_side/store.rs` |

Consumers requiring the reexport to compile unedited:
- `crates/infrastructure/tests/in_memory_event_store_conformance.rs:17-18` — `use ego_infrastructure::persistence::in_memory::InMemoryEventStore; use ego_testkit::assert_event_store_conformance;`
- `crates/infrastructure/tests/commit_publishes_atomically.rs` (exists, not fully read — same import family expected)
- `crates/persistent-entity/src/builder.rs:356,360` — default-constructs infra's types via whatever path `persistent-entity`'s own imports use today (need to confirm at propose time whether persistent-entity imports these from `ego_infrastructure` or re-declares its own — evidence gathered shows persistent-entity has its **own** duplicate `InMemoryEventStore`/`InMemorySnapshotStore` at `persistence.rs:571,733`, referenced via `crate::persistence::{InMemoryEventStore, InMemorySnapshotStore}` in `builder.rs`, so builder.rs is unaffected by the infra-side move)
- `examples/reference-app/src/lib.rs:432-439` — direct `ego_infrastructure::persistence::in_memory::{InMemoryEventStore, InMemorySnapshotStore}` paths, requires the reexport
- `crates/transport/tests/operation_key_extractor.rs`, `crates/service-sdk/tests/{retention_worker_lifecycle,cross_tenant_reservation_isolation}.rs` — require `ego_testkit::InMemoryOperationReservationStore` reexport

## BEHAVIOR EQUIVALENCE ANALYSIS

For every YES-move candidate: no signature change, no new method, no new trait, no changed tenant-resolution logic, no changed concurrency/locking semantics — verified by direct import-list inspection (each file imports only `ego_domain::*` types plus std/serde/chrono/async-trait) and full-body reads of `event_store.rs`, `repository.rs`, `snapshot.rs`, `read_side_store.rs`, `reservation.rs`, and `examples/reference-app/src/read_side/store.rs`. The two DEFERRED duplicates (persistent-entity's `InMemoryEventStore`/`InMemorySnapshotStore`) are excluded from movement precisely because moving them would require a behavior decision (see MOVE MATRIX). No YES-move candidate requires any change to its own source beyond the module path it lives at.

## DURABILITY / PRODUCTION ANALYSIS

`crates/persistent-entity/src/profile.rs` — `Profile::Production` requires `require_durably_configured` (`profile.rs:51-63`) to reject any capability whose `is_durable()` (not `.is_some()`) is `false`. Test `presence_alone_is_not_durability` (`profile.rs:99-117`) pins the exact rule the objective's constraint warns about: "`Some(InMemoryEventStore::new()).is_some()` is `true`... but that store's `is_durable()` is `false`" (comment at `profile.rs:101-102`), and the test asserts `Profile::Production` still refuses it.

`crates/persistent-entity/src/builder.rs:764-783` (`try_build_rejects_explicit_in_memory_event_store_under_production`) and `:788-805` (`try_build_rejects_explicit_in_memory_snapshot_store_under_production`) are executable proof: constructing `EntityRuntimeBuilder` with `Profile::Production` and an explicit `InMemoryEventStore`/`InMemorySnapshotStore` currently **fails** with an error naming the non-durable capability.

`EventStore::is_durable()` and `Snapshot::is_durable()` both default to `false` (`crates/persistence-api/src/persistence/event_store.rs:54-56`, `snapshot.rs:19-21`) and neither `InMemoryEventStore` implementation nor either `InMemorySnapshotStore` implementation overrides it — confirmed by absence of any `fn is_durable` override in `crates/infrastructure/src/persistence/in_memory/{event_store,snapshot}.rs` or `crates/persistent-entity/src/persistence.rs`'s equivalents.

**Verdict**: a pure move/reexport of any in-memory store preserves its `is_durable() == false` default (the method isn't touched), so `Profile::Production`'s rejection continues to fire identically after this change. No proposed move risks accidentally making an in-memory store pass production validation.

## EFFECT STORE BLOCKER ANALYSIS

1. **Which effect persistence ports are still owned by ego-runtime/ego-effect-store (per D-9)?** `EffectStateStore` (`crates/runtime/src/effects/store.rs:238`), `EffectDedupStore` (`store.rs:418`), `RetentionMaintenance` (`store.rs:474`) — all three remain in `ego-runtime`, confirmed untouched by CORE-PERSIST-A (spec Non-Goals, `openspec/specs/persistence-api-surface/spec.md:133-134`).
2. **Where does their in-memory implementation live today?** `InMemoryEffectStore` (`store.rs:531`), implementing both `EffectStateStore` (`store.rs:562`) and `EffectDedupStore` (`store.rs:697`) in the same file as the port definitions.
3. **Can it move to ego-persistence-memory without a new dependency edge toward ego-runtime/ego-effect-store?** No. `InMemoryEffectStore` only compiles against the `EffectStateStore`/`EffectDedupStore` trait definitions, which live in `ego-runtime`. Moving the struct without the traits leaves it implementing nothing; moving it and keeping the traits in place requires `ego-persistence-memory` to depend on `ego-runtime`.
4. **Would that require moving the traits first (reopening D-9)?** Yes. `ego-runtime` is a `foundation`-layer crate that itself depends on `persistent-entity` (`crates/runtime/Cargo.toml:7,11`) — a domain-adjacent `ego-persistence-memory` crate depending on `ego-runtime` would very likely violate `foundation-integrity`'s direction rules (domain/foundation crates may depend on domain/foundation, not on crates that create upward cycles) and, regardless of gate mechanics, is exactly the "second architecture decision" D-9 explicitly carved out (`proposal.md:51`: "relocating them either leaves `InMemoryEffectStore` depending on a port defined elsewhere... or requires moving an implementation... That is a second architecture decision and belongs to its own change (F-1)").
5. **Should effects stay entirely out of this change?** Yes — this is the same conclusion CORE-PERSIST-A already reached for the ports; the in-memory implementation inherits the same boundary since it cannot be split from its ports.
6. **Is a future dedicated change needed?** Yes — recommend naming it explicitly (e.g. **CORE-PERSIST-E**, following the user's own suggested numbering) to first relocate `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance` (mirroring CORE-PERSIST-A's move for the other eight ports) before `InMemoryEffectStore` can be consolidated into the shared adapter crate.

## OUT-OF-SCOPE CORRECTNESS FINDINGS

1. **`persistent-entity`'s `InMemorySnapshotStore` ignores `tenant_id` entirely.** `crates/persistent-entity/src/persistence.rs:746-765` — `save_snapshot`/`load_snapshot` both take `_tenant_id: Option<&str>` and never read it; the snapshot key is `stream_id` (aggregate_id) alone (`persistence.rs:734`, `HashMap<String, (i64, Value)>`). Two different tenants persisting a snapshot for the same `aggregate_id` collide. Contrast with the correct sibling at `crates/infrastructure/src/persistence/in_memory/snapshot.rs:38-39,49-50`, which resolves and folds the tenant into the key. **Not fixed here** — flagged as named debt, decision deferred to propose/design.
2. **KD-2 (carried forward from CORE-PERSIST-A, still unresolved)**: `PostgreSQLRepository`'s tenant-scoping/`ON CONFLICT` defect at `crates/persistence/src/postgres/repository.rs` (lines 82,135,161,109 per the CORE-PERSIST-A proposal) — out of scope for an in-memory-only change, restated here for completeness since propose may be asked to compare in-memory vs. Postgres behavior.
3. **KD-4 (carried forward)**: no conformance harness exists for `Repository`, `Snapshot`, `OffsetStore`, or `DedupStore` (only `EventStore` and `OperationReservationStore` have one, per `crates/testkit/src/event_store.rs` and the `oldest_completed_contract`/lease tests in `crates/testkit/src/reservation.rs`). This means the `InMemorySnapshotStore` divergence found in #1 above was only caught by manual side-by-side reading, not by an automated harness — a fact worth surfacing to the design of CORE-PERSIST-D (conformance-test-framework), out of scope to build here.

## ATOMICITY VERDICT

The objective — "a single canonical in-memory adapter via pure move/reexport, zero new behavior" — decomposes cleanly into ONE atomic move covering seven implementations (`InMemoryEventStore`+`InMemoryEventStoreUnitOfWork`, `InMemoryRepository`, `InMemorySnapshotStore`(infra), `InMemoryReadSideStore`+`paginate`, `InMemoryOffsetStore`, `InMemoryDedupStore`, `InMemoryOperationReservationStore`) sharing one destination crate, one dependency-direction decision (layer classification), and one reexport strategy per source crate — mirroring CORE-PERSIST-A's own atomicity shape.

Three items are correctly EXCLUDED from that atomic move without breaking its atomicity, because each is its own independent decision, not a missing piece of the same one:
- The two persistent-entity duplicates (`InMemoryEventStore`, `InMemorySnapshotStore`) — consolidating either requires a behavior decision (additive-capability merge, or a correctness fix), which is a different, later change.
- The effect-store in-memory implementation — blocked by D-9, requires its own port-relocation change first (F-1/CORE-PERSIST-E).

**ATOMICITY: PASS** for the scoped move (seven implementations, one destination crate). The explicit exclusions above are correctly out-of-scope, not a hidden second change smuggled into this one — same shape CORE-PERSIST-A used for its own D-9 exclusion.

---

## CORE-PERSIST-B READINESS

```
ATOMICITY: PASS
CANONICAL MEMORY ADAPTER POSSIBLE: PARTIAL
MEMORY IMPLEMENTATIONS FOUND: 12
DUPLICATES FOUND: 3 (EventStore pair, EventStoreUnitOfWork pair, Snapshot pair)
EXAMPLE-OWNED GENERIC STORES: 2 (InMemoryOffsetStore, InMemoryDedupStore)
SPECIALIZED TEST FAKES: 2 explicitly-analyzed (FakeDurableOffsetStore, FakeDurableDedupStore) + ~150 test-local, not individually enumerated
MISSING MEMORY IMPLEMENTATIONS: 1 (ProjectionStateStore — dead by design, KD-1, not to be implemented)
EFFECTS BLOCKED BY RUNTIME OWNERSHIP: YES
BEHAVIOR CHANGE REQUIRED: NONE for the 7-implementation scoped move (EventStore/UoW, Repository, Snapshot-infra, ReadSideStore+paginate, OffsetStore, DedupStore, OperationReservationStore). Two named-debt items require a future, separately-reviewed decision: (1) persistent-entity's InMemoryEventStore additive with_version_offset capability — merge or leave forked; (2) persistent-entity's InMemorySnapshotStore tenant-ignoring bug — fix as its own change or leave forked, but never silently reconciled inside a "pure move."
CONTRACT CHANGE REQUIRED: NONE — no trait signature touched.
POSTGRES CHANGE REQUIRED: NONE.
DEPENDENCY CYCLES: NONE for the scoped move (all seven candidates import only ego_domain/ego_persistence_api-visible items). Effects excluded specifically to avoid creating one (ego-persistence-memory → ego-runtime would very likely invert the domain/foundation direction).
RECOMMENDATION: PROCEED, scoped to the 7-implementation move; explicitly carry the two persistent-entity duplicates and the effect-store D-9 boundary as named, out-of-scope debt in the resulting proposal rather than attempting to silently resolve or defer them without naming them.
```
