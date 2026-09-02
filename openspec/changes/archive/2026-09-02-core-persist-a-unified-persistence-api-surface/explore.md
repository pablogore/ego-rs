# CORE-PERSIST-A — Explore: Unified Persistence API Surface

**Phase**: `sdd-explore`
**Change**: `core-persist-a-unified-persistence-api-surface`
**Status**: complete — ready for `sdd-propose`

## 1. Current Inventory

**Domain-owned ports** (`crates/domain/src/`):

| File:Line | Trait/Type | Capability |
|---|---|---|
| `persistence/repository.rs:12` | `Repository<A>` (sync) | Repository |
| `persistence/event_store.rs:47` | `EventStore<E: DomainEvent>` (async) | EventStore |
| `persistence/event_store.rs:186` | `EventStoreUnitOfWork<E>: Send` (async) | EventStore (unit-of-work) |
| `persistence/snapshot.rs:14` | `Snapshot` (sync) | Snapshot |
| `persistence/stored_event.rs` | `StoredEvent<E>` | EventStore (contract type) |
| `persistence/tenant.rs:29` | `resolve_tenant(...)` | Tenant |
| `persistence/error.rs:8` | `PersistenceError` | Tenant/EventStore/Repository/Snapshot (shared error) |
| `read_side/offset.rs:55` | `OffsetStore` (async) | Offset |
| `read_side/dedup.rs:25` | `DedupStore` (async) | Dedup |
| `read_side/store.rs` (~line 20) | `ReadSideStore<E>` (async) | ReadSide |
| `read_side/projection_state_store.rs` | `ProjectionStateStore` (async) | ProjectionState — **dead/unused** |
| `operation/reservation.rs:66` | `OperationReservationStore: Send + Sync` (async) | OperationReservation |

**Runtime-owned ports** (`crates/runtime/src/effects/store.rs` — NOT domain-owned):

| File:Line | Trait/Type | Capability |
|---|---|---|
| `store.rs:238` | `EffectStateStore: Send + Sync` (async) | EffectState |
| `store.rs:418` | `EffectDedupStore: Send + Sync` (async) | EffectDedup |
| `store.rs:474` | `RetentionMaintenance: Send + Sync` (async) | RetentionMaintenance |
| `store.rs:531` | `InMemoryEffectStore` (implements both ports — **implementation co-located with ports in the same file**) | EffectState/EffectDedup (impl) |

## 2. Current Ownership Map

| Capability | Port owner | In-memory impl | Postgres impl | Stoolap impl | Conformance tests |
|---|---|---|---|---|---|
| EventStore | `ego-domain` | `ego-infrastructure::persistence::in_memory::InMemoryEventStore` (event_store.rs:89) | `ego-persistence::postgres::PostgreSQLEventStore` (event_store.rs:53) | — | `ego-testkit::assert_event_store_conformance` (testkit/src/event_store.rs) |
| Repository | `ego-domain` | `ego-infrastructure::…::InMemoryRepository` (repository.rs:11) | `ego-persistence::postgres::PostgreSQLRepository` (repository.rs:27) | — | none found |
| Snapshot | `ego-domain` | `ego-infrastructure::…::InMemorySnapshotStore` (snapshot.rs:12) | `ego-persistence::postgres::PostgreSQLSnapshotStore` (snapshot.rs:27) | — | none found |
| OperationReservationStore | `ego-domain` | `ego-testkit::InMemoryOperationReservationStore` (reservation.rs:79 — **a test double, not a production in-memory adapter**) | `ego-persistence::postgres::PostgresOperationReservationStore` (reservation.rs:70) | — | `ego-testkit::reservation_conformance` |
| OffsetStore | `ego-domain` | **examples/reference-app** `InMemoryOffsetStore`/`FakeDurableOffsetStore` (read_side/store.rs:153,251) — anomaly, see row below | `ego-persistence::postgres::PostgreSQLOffsetStore` (read_side_offset.rs:38) | — | **none anywhere** |
| DedupStore | `ego-domain` | **examples/reference-app** `InMemoryDedupStore`/`FakeDurableDedupStore` (read_side/store.rs:199,282) — anomaly | `ego-persistence::postgres::PostgreSQLDedupStore` (read_side_dedup.rs:54) | — | **none anywhere** |
| ReadSideStore | `ego-domain` | `ego-infrastructure::…::InMemoryReadSideStore` (read_side_store.rs:24) | — (none) | — | none found |
| ProjectionStateStore | `ego-domain` | none | none | — | none — **wholly dead trait** |
| EffectStateStore | `ego-runtime` (not domain) | `ego-runtime::effects::store::InMemoryEffectStore` (store.rs:531, in the same file as the port) | `ego-effect-store::PostgresEffectStore` (postgres/mod.rs:170, impl at 378) | `ego-effect-store::StoolapEffectStore` (stoolap/mod.rs:163, impl at 451) | `ego-effect-store::conformance` (own crate, three-tier, PROD-002 AD-13) |
| EffectDedupStore | `ego-runtime` | same `InMemoryEffectStore` (impl at store.rs:697) | `PostgresEffectStore` (impl at 689) | `StoolapEffectStore` (impl at 686) | same `ego-effect-store::conformance` |
| RetentionMaintenance | `ego-runtime` | none (InMemoryEffectStore does not implement it) | `PostgresEffectStore` (impl at 368) | `StoolapEffectStore` (impl at 441) | same |

**Reference-app anomaly row.** `examples/reference-app/src/read_side/store.rs` holds the workspace's *only* in-memory `OffsetStore`/`DedupStore` reference implementations — its own doc comments say so verbatim ("this workspace has no other in-memory reference implementation of it"). PostgreSQL adapters for both already exist (shipped by the already-archived PROD-014B change), so the reference-app pair is the only *in-memory* one, not the only implementation of any kind — and it is an application example, not a reusable crate. Real ownership gap for CORE-PERSIST-B/C to close, not this change.

## 3. Contract Type Map

| Type | Defined | Consumed by |
|---|---|---|
| `PersistenceError` | `domain/persistence/error.rs:8` | Repository, EventStore, Snapshot, resolve_tenant, all in-memory + Postgres adapters |
| `OffsetStoreError` | `domain/read_side/offset.rs:42` | OffsetStore, its Postgres/in-memory adapters |
| `DedupStoreError` | `domain/read_side/dedup.rs:10` | DedupStore, its adapters |
| `ReadSideStoreError` | `domain/read_side/store.rs` | ReadSideStore |
| `ProjectionStateStoreError` | `domain/read_side/projection_state_store.rs` | dead trait only |
| `ReservationError` | `domain/operation/reservation.rs:388` | OperationReservationStore + adapters |
| `OperationId`, `OwnerId` (205), `FencingToken` (231), `Lease` (290), `OwnerFence` (309), `ReserveRequest` (321), `ReservationOutcome` (346), `OldestCompleted` (48), `StoredServiceResponse` (273) | `domain/operation/reservation.rs` | OperationReservationStore boundary, testkit, Postgres adapter |
| `OperationKey` (key.rs:29), `OperationKeyError` (33), `OperationFingerprint` (99), `OperationKeyHash` (203) | `domain/operation/key.rs` | EventStore append/receipts, reservation |
| `OperationReceipt` (receipt.rs:113), `AggregateOutcome` (40), `AggregateOutcomeError` (63) | `domain/operation/receipt.rs` | EventStore::find_receipt, EventStoreUnitOfWork::confirm_receipt |
| `TenantId`/`TenantIdError` | `domain/context.rs:56`, re-exported at `ego_domain::TenantId` (lib.rs:105) | reservation, effect-store, Postgres event_store.rs |
| `EntityId`/`EntityIdError` | `domain/context.rs`, re-exported at `ego_domain::EntityId` (lib.rs:105) | domain-level entity identity — **name-collides (but does not type-collide) with `persistent_entity::types::EntityId`, which is dead code (see §10)** |
| `EffectId` (23), `Timestamp` (62), `EffectState` (83), `TerminalReason` (98), `EffectStoreError` (116), `AcceptedEffect` (153), `StoredEffect` (190), `EffectStoreCapabilities` (215), `EffectFingerprint` (316), `DedupOutcome` (372), `DedupScope` (407) | `runtime/src/effects/store.rs` | EffectStateStore/EffectDedupStore/RetentionMaintenance, `ego-effect-store` adapters, `ego-effect-store::conformance`, `ego-testkit::effects` |
| `persistent_entity::types::TenantId = String` | `persistent-entity/src/types.rs:12` | **nothing — the file is dead code** (see §10) |

## 4. Implementation Map

| Struct | File:Line | Backend | Implements |
|---|---|---|---|
| `InMemoryEventStore<E>` | `infrastructure/…/event_store.rs:89` | in-memory | EventStore, EventStoreUnitOfWork (inner struct, line 214) |
| `InMemoryRepository<A>` | `infrastructure/…/repository.rs:11` | in-memory | Repository |
| `InMemorySnapshotStore` | `infrastructure/…/snapshot.rs:12` | in-memory | Snapshot |
| `InMemoryReadSideStore` | `infrastructure/…/read_side_store.rs:24` | in-memory | ReadSideStore |
| `PostgreSQLEventStore<E,F>` | `persistence/postgres/event_store.rs:53` | Postgres | EventStore, EventStoreUnitOfWork (`PostgresEventStoreUnitOfWork`, line 438) |
| `PostgreSQLRepository<A,F>` | `persistence/postgres/repository.rs:27` | Postgres | Repository — **carries the confirmed bug, §10** |
| `PostgreSQLSnapshotStore` | `persistence/postgres/snapshot.rs:27` | Postgres | Snapshot |
| `PostgreSQLOffsetStore` | `persistence/postgres/read_side_offset.rs:38` | Postgres | OffsetStore |
| `PostgreSQLDedupStore` | `persistence/postgres/read_side_dedup.rs:54` | Postgres | DedupStore |
| `PostgresOperationReservationStore` | `persistence/postgres/reservation.rs:70` | Postgres | OperationReservationStore |
| `InMemoryOperationReservationStore` | `testkit/src/reservation.rs:79` | in-memory (test double) | OperationReservationStore |
| `InMemoryOffsetStore` | `examples/reference-app/src/read_side/store.rs:153` | in-memory | OffsetStore |
| `FakeDurableOffsetStore` | `examples/reference-app/…/store.rs:251` | in-memory, declares `is_durable()=true` | OffsetStore |
| `InMemoryDedupStore` | `examples/reference-app/…/store.rs:199` | in-memory | DedupStore |
| `FakeDurableDedupStore` | `examples/reference-app/…/store.rs:282` | in-memory, declares `is_durable()=true` | DedupStore |
| `InMemoryEffectStore` | `runtime/src/effects/store.rs:531` | in-memory, self-documented "convenience only" | EffectStateStore (562), EffectDedupStore (697); **no RetentionMaintenance** |
| `PostgresEffectStore` | `effect-store/src/postgres/mod.rs:170` | Postgres | EffectStateStore (378), EffectDedupStore (689), RetentionMaintenance (368) |
| `StoolapEffectStore` | `effect-store/src/stoolap/mod.rs:163` | Stoolap (embedded) | EffectStateStore (451), EffectDedupStore (686), RetentionMaintenance (441) |

## 5. Dependency Graph

Current crate edges relevant to persistence (`path = "../…"` in each `Cargo.toml`):

```
ego-domain            (no internal deps — leaf)
ego-application     → ego-domain
ego-persistence     → ego-domain
ego-runtime          → ego-domain, persistent-entity
persistent-entity   → ego-domain            [+ dev-dep: ego-testkit]
ego-infrastructure  → ego-domain, ego-application, ego-persistence   [+ dev-dep: ego-testkit]
ego-effect-store    → ego-runtime, ego-domain   [+ dev-dep: ego-testkit]
ego-service-sdk     → ego-domain, ego-runtime, persistent-entity, ego-security-sdk
ego-testkit         → ego-domain, ego-runtime, persistent-entity, ego-security-sdk, ego-service-sdk
reference-app       → (application-level, depends on ego-persistence, ego-infrastructure, ego-runtime, ego-service-sdk, ego-domain)
```

Design constraint already in force (`openspec/config.yaml`): "no circular deps between crates."

**What a new `ego-persistence-api` crate's edges would need to be**, given only ports+contract-types move (no implementation moves, per scope):

- `ego-persistence-api → ego-domain` — only if any moved port needs a domain type it doesn't already re-export itself (e.g. `TenantId`); otherwise no edge needed at all for the domain-owned ports, since they *are* the domain-owned ports being relocated.
- `ego-domain` would **lose** `persistence/*`, `read_side/{offset,dedup,store,projection_state_store}.rs`, `operation/{reservation,key,receipt,identity}.rs` as owned modules if fully migrated (see §11 for why a full migration is unlikely to fit CORE-PERSIST-A's budget) — or `ego-domain` keeps them as thin re-exports of `ego-persistence-api`, which requires `ego-domain → ego-persistence-api`, a **downward-pointing edge from the domain layer into an infrastructure-facing crate that does not exist today** and is architecturally backwards for a domain crate. This is the central dependency-direction risk of the whole CORE-PERSIST-A..E series and is flagged, not resolved, here.
- `ego-persistence` (Postgres adapters) would gain `ego-persistence-api` alongside/instead of `ego-domain`.
- `ego-infrastructure` (in-memory adapters) would gain `ego-persistence-api` alongside/instead of `ego-domain`.
- `ego-runtime` would need `ego-persistence-api` too, since `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance` currently live in `ego-runtime` itself, not `ego-domain` — moving them out means `ego-runtime`'s own `InMemoryEffectStore` (store.rs:531) now depends on a port defined in a crate it did not previously need, and `ego-effect-store`'s existing `ego-runtime` dependency (taken *specifically* to reach these three ports, per its Cargo.toml comment) could instead point at `ego-persistence-api` directly — a real edge simplification, but a semver-breaking one for anything importing `ego_runtime::effects::store::{EffectStateStore, ...}`.
- `ego-testkit`, `persistent-entity` would need `ego-persistence-api` wherever they currently import `ego_domain::persistence::*` / `ego_domain::read_side::*` / `ego_domain::operation::*` / `ego_runtime::effects::store::*`.

No cycle is introduced by any of these edges as long as `ego-persistence-api` depends on nothing in the workspace except possibly `ego-domain` for a handful of shared value types — the risk is entirely about whether `ego-domain` can be made to point *at* `ego-persistence-api` for re-export purposes without inverting the hexagonal layering documented in `openspec/config.yaml`.

## 6. Public Path Map

92 files across the workspace import one or more of `ego_domain::persistence::*`, `ego_domain::read_side::*`, `ego_domain::operation::*`, `ego_runtime::effects::*`, `ego_persistence::*`, confirmed via workspace-wide grep. Concentration by layer:

| Import root | Approx. consumer count | Representative consumers |
|---|---|---|
| `ego_domain::persistence::*` | ~15 files | `ego-infrastructure` in-memory adapters, `ego-persistence` Postgres adapters, `persistent-entity/src/{persistence,actor,command_context,persistent_entity}.rs`, `ego-testkit::event_store` |
| `ego_domain::read_side::*` | ~10 files | reference-app read_side, `ego-persistence` Postgres read-side adapters, `ego-infrastructure` in-memory read-side store, `ego-runtime::read_side::{scheduler,batch_executor}` |
| `ego_domain::operation::*` | ~25 files | `ego-service-sdk` (idempotency, retention, health, app composition), `persistent-entity` builder/actor, `ego-persistence::postgres::{reservation,event_store,snapshot}`, `ego-testkit::{reservation,reservation_conformance}`, integration-tests fencing/lease/purge suites |
| `ego_runtime::effects::*` | ~15 files | `ego-effect-store` (postgres, stoolap, conformance), `ego-testkit::effects`, `ego-service-sdk` (retention, effect_retention, effect_acceptor wiring), reference-app effects, integration-tests |
| `ego_persistence::*` | ~10 files | `ego-infrastructure::persistence::mod` (`pub use ego_persistence::postgres;`), reference-app read_side mod, integration-tests infrastructure suite |

Every one of these is a **compile-time `use` path**, not a runtime string — moving any trait/type without a re-export at its current path breaks every listed consumer's build.

## 7. Compatibility Risks

| Risk | Detail |
|---|---|
| Semver-path breakage | All 92 files above resolve types by exact module path. Moving a trait without re-exporting at the old path breaks every one, even though the trait's shape is unchanged. |
| `Cargo.toml` edge additions | `ego-persistence`, `ego-infrastructure`, `ego-runtime`, `persistent-entity`, `ego-testkit`, `ego-service-sdk`, and `examples/reference-app` all need a new `ego-persistence-api = { path = "..." }` line if they consume a moved item directly rather than through `ego-domain`'s re-export. |
| Domain-layer inversion | If `ego-domain` re-exports from `ego-persistence-api` to preserve `ego_domain::persistence::EventStore` at its current path, `ego-domain` gains a dependency on a crate one layer "below" it in the current hexagonal picture — a design-review-relevant decision, not a mechanical one. |
| `ego-runtime`'s effect-store ports | These are the one case where the port is *not* currently domain-owned. Relocating them changes `ego-runtime`'s own dependency shape (it would need to depend on `ego-persistence-api` to keep `InMemoryEffectStore` compiling) and changes `ego-effect-store`'s reason for depending on `ego-runtime` at all — worth flagging to CORE-PERSIST-B, since fixing this reduces `ego-effect-store → ego-runtime` to a dependency that exists only for `ExternalEffectDescription`-style domain plumbing, if any remains. |
| Test-only cycles | `ego-testkit → ego-service-sdk` and `ego-service-sdk`'s dev-dep back on `ego-testkit` are an already-accepted, dev-dependency-only cycle (explicitly commented as intentional in `service-sdk/Cargo.toml`). A `persistence-api` crate must not be pulled into that cycle, since dev-only cycles are tolerated but a real one is not. |
| `is_durable()` default landmine | `OffsetStore`/`DedupStore`'s `Arc<T>` blanket-forwarding impls (offset.rs:92, dedup.rs:60) are load-bearing and must move *with* their trait, not be reconstructed from memory — losing the forward silently reclassifies every registered durable pair as volatile. |
| Conformance test relocation | `ego-testkit`'s `event_store.rs`/`reservation_conformance.rs` reference the exact trait paths they assert against; moving the traits without updating these `use` statements breaks `ego-testkit`'s own compile, which cascades to every crate with a dev-dependency on it. |

## 8. Proposed Target Tree

```
crates/persistence-api/
├── Cargo.toml                      # deps: ego-domain only if a shared value type is reused (e.g. TenantId); otherwise none
├── src/
│   ├── lib.rs                      # capability submodule table + top-level re-exports
│   ├── error.rs                    # shared PersistenceError (if promoted above per-capability errors) — NOT a new type, just the existing one relocated
│   ├── event_store/
│   │   ├── mod.rs                  # EventStore<E>, EventStoreUnitOfWork<E>, StoredEvent<E>
│   ├── repository/
│   │   ├── mod.rs                  # Repository<A>
│   ├── snapshot/
│   │   ├── mod.rs                  # Snapshot
│   ├── tenant/
│   │   ├── mod.rs                  # resolve_tenant
│   ├── read_side/
│   │   ├── offset.rs               # OffsetStore, Offset, OffsetStoreError, Arc<T> forwarding impl
│   │   ├── dedup.rs                # DedupStore, DedupStoreError, Arc<T> forwarding impl
│   │   ├── store.rs                # ReadSideStore<E>, ReadSideStoreError
│   │   └── projection_state.rs     # ProjectionStateStore (relocated as-is, marked deprecated/unused — not deleted)
│   ├── operation/
│   │   ├── reservation.rs          # OperationReservationStore + Lease/OwnerFence/ReserveRequest/ReservationOutcome/ReservationError/OldestCompleted
│   │   ├── key.rs                  # OperationKey, OperationKeyError, OperationFingerprint, OperationKeyHash
│   │   └── receipt.rs              # OperationReceipt, AggregateOutcome, AggregateOutcomeError
│   └── effects/
│       └── store.rs                # EffectStateStore, EffectDedupStore, RetentionMaintenance, EffectStoreCapabilities + every effect contract type — ports ONLY, InMemoryEffectStore stays in ego-runtime (implementation, out of scope)
```

Naming: `ego-persistence-api` (crate name) / `persistence-api` (directory), consistent with the existing `ego-*` package-name convention (`ego-domain`, `ego-runtime`, `ego-persistence`, `ego-infrastructure`) while distinct from the existing `ego-persistence` crate (adapters), matching the target-architecture naming the task specifies (`persistence-api` for ports, `persistence-postgres`/`persistence-memory` for adapters — future rename of `ego-persistence`/`ego-infrastructure`'s in-memory module, explicitly CORE-PERSIST-B/C's job, not this change's).

## 9. Move/Reexport Matrix

Format: `<current path> -> <new path> | re-export at old path: YES/NO | breaking without re-export: YES/NO`

```
ego_domain::persistence::Repository -> ego_persistence_api::repository::Repository | re-export at old path: YES | breaking without re-export: YES
ego_domain::persistence::EventStore -> ego_persistence_api::event_store::EventStore | re-export at old path: YES | breaking without re-export: YES
ego_domain::persistence::EventStoreUnitOfWork -> ego_persistence_api::event_store::EventStoreUnitOfWork | re-export at old path: YES | breaking without re-export: YES
ego_domain::persistence::StoredEvent -> ego_persistence_api::event_store::StoredEvent | re-export at old path: YES | breaking without re-export: YES
ego_domain::persistence::Snapshot -> ego_persistence_api::snapshot::Snapshot | re-export at old path: YES | breaking without re-export: YES
ego_domain::persistence::PersistenceError -> ego_persistence_api::error::PersistenceError | re-export at old path: YES | breaking without re-export: YES
ego_domain::persistence::resolve_tenant -> ego_persistence_api::tenant::resolve_tenant | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::offset::OffsetStore -> ego_persistence_api::read_side::offset::OffsetStore | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::offset::Offset -> ego_persistence_api::read_side::offset::Offset | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::offset::OffsetStoreError -> ego_persistence_api::read_side::offset::OffsetStoreError | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::dedup::DedupStore -> ego_persistence_api::read_side::dedup::DedupStore | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::dedup::DedupStoreError -> ego_persistence_api::read_side::dedup::DedupStoreError | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::store::ReadSideStore -> ego_persistence_api::read_side::store::ReadSideStore | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::store::ReadSideStoreError -> ego_persistence_api::read_side::store::ReadSideStoreError | re-export at old path: YES | breaking without re-export: YES
ego_domain::read_side::projection_state_store::ProjectionStateStore -> ego_persistence_api::read_side::projection_state::ProjectionStateStore | re-export at old path: YES | breaking without re-export: NO (unused — see §10)
ego_domain::operation::OperationReservationStore -> ego_persistence_api::operation::reservation::OperationReservationStore | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::ReservationError -> ego_persistence_api::operation::reservation::ReservationError | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::ReserveRequest -> ego_persistence_api::operation::reservation::ReserveRequest | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::ReservationOutcome -> ego_persistence_api::operation::reservation::ReservationOutcome | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::Lease -> ego_persistence_api::operation::reservation::Lease | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::OwnerFence -> ego_persistence_api::operation::reservation::OwnerFence | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::FencingToken -> ego_persistence_api::operation::reservation::FencingToken | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::OldestCompleted -> ego_persistence_api::operation::reservation::OldestCompleted | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::OperationKey -> ego_persistence_api::operation::key::OperationKey | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::OperationFingerprint -> ego_persistence_api::operation::key::OperationFingerprint | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::OperationReceipt -> ego_persistence_api::operation::receipt::OperationReceipt | re-export at old path: YES | breaking without re-export: YES
ego_domain::operation::AggregateOutcome -> ego_persistence_api::operation::receipt::AggregateOutcome | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::EffectStateStore -> ego_persistence_api::effects::store::EffectStateStore | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::EffectDedupStore -> ego_persistence_api::effects::store::EffectDedupStore | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::RetentionMaintenance -> ego_persistence_api::effects::store::RetentionMaintenance | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::EffectStoreCapabilities -> ego_persistence_api::effects::store::EffectStoreCapabilities | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::EffectId -> ego_persistence_api::effects::store::EffectId | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::EffectStoreError -> ego_persistence_api::effects::store::EffectStoreError | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::AcceptedEffect -> ego_persistence_api::effects::store::AcceptedEffect | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::StoredEffect -> ego_persistence_api::effects::store::StoredEffect | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::EffectFingerprint -> ego_persistence_api::effects::store::EffectFingerprint | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::DedupOutcome -> ego_persistence_api::effects::store::DedupOutcome | re-export at old path: YES | breaking without re-export: YES
ego_runtime::effects::store::DedupScope -> ego_persistence_api::effects::store::DedupScope | re-export at old path: YES | breaking without re-export: YES
```

(`InMemoryEventStore`, `PostgreSQLRepository`, `PostgresEffectStore`, `StoolapEffectStore`, and every other concrete adapter struct are **excluded** from this matrix — they are implementations, not ports/contract-types, and stay exactly where they are per CORE-PERSIST-A's explicit scope.)

## 10. Out-of-Scope Findings

1. **`crates/persistence/src/postgres/repository.rs` tenant bug — CONFIRMED, more severe than described.** Lines 82, 135, 161 use `tenant_id = $2` instead of `IS NOT DISTINCT FROM $2` for the systemwide (`NULL`) tenant partition. Additionally, line 109's `INSERT … ON CONFLICT (aggregate_id, tenant_id) DO UPDATE` targets a constraint that **does not exist**: migration `002_create_aggregates.sql` declares `aggregate_id VARCHAR(255) PRIMARY KEY` alone, with no unique index on `(aggregate_id, tenant_id)`. Postgres requires an `ON CONFLICT` target to exactly match a live unique/exclusion constraint (error `42P10`), so this statement is one race away from a hard runtime failure, not only a silent tenant-isolation defect. Every sibling adapter in the same crate (`event_store.rs`, `snapshot.rs`, `reservation.rs`) already uses the correct `IS NOT DISTINCT FROM` pattern, and `snapshot.rs` additionally shows the correct two-partial-unique-index `ON CONFLICT` branching this file should mirror. **Not fixed here** — flagged for CORE-PERSIST-C or a standalone bugfix.
2. **`crates/persistent-entity/src/types.rs` is dead code with an internal duplication.** The file is never referenced by `mod types;`/`pub mod types;` anywhere in `crates/persistent-entity/src/lib.rs`, so it is excluded from compilation entirely. It also self-duplicates: `EntityTriple`, `EntityId`, `ExecutionKey`, and their impls each appear twice in the same file (lines 18/122, 52/143, 85/168), which would be a hard compile error (`E0428`) if the module were ever wired in. The live, compiled `EntityTriple` used throughout the crate is a structurally different type defined in `scheduler.rs:10` (`tenant_id: String` field vs. the dead file's `tenant: TenantId` alias field). Not deleted here per explicit instruction to note dead code as debt rather than remove it.
3. **`ProjectionStateStore` (`domain/read_side/projection_state_store.rs`) has zero implementations and zero consumers** anywhere in the workspace. Confirmed unused; noted as debt, not deleted.
4. **Conformance-testing ownership is asymmetric across capabilities.** `EventStore` and `OperationReservationStore` conformance harnesses live in `ego-testkit`; `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance` conformance lives inside `ego-effect-store` itself (three-tier PROD-002 AD-13 design); `Repository`, `Snapshot`, `OffsetStore`, and `DedupStore` have **no conformance harness anywhere**, despite `OffsetStore`/`DedupStore` both having a documented, load-bearing `is_durable()` default landmine. Relevant to the eventual `persistence-testkit` crate (CORE-PERSIST-D/E), not actionable here.
5. **Port/implementation co-location in `ego-runtime`.** `crates/runtime/src/effects/store.rs` defines three port traits and their full contract-type vocabulary *and* a working `InMemoryEffectStore` implementation in one 1320-line file, inside a crate (`ego-runtime`) that is not `ego-domain`. This is the clearest instance of the "no coherent ownership" problem CORE-PERSIST-A..E exists to fix, and it is the reason §5's dependency graph flags `ego-runtime` needing a new edge to `ego-persistence-api` if these ports move — moving the implementation out is explicitly a later change's job.
6. **`persistent_entity::types::TenantId = String`** is an unvalidated bare alias with the same name as `ego_domain::context::TenantId` (a validated newtype). Because `types.rs` is dead code (finding 2), this is not a live compile-time collision today, but it is a trap for anyone who later wires the file in without noticing the name clash with the domain's validated type.
7. **`InMemoryOperationReservationStore` lives in `ego-testkit`, not in `ego-infrastructure`** alongside the other in-memory adapters. It is documented as a test double satisfying the identical production port, which is a legitimate design choice, but it means `OperationReservationStore` has no *production-grade* in-memory reference implementation at all — only a Postgres adapter and a test double. Worth flagging for whichever future change decides the shape of `persistence-memory`.

## 11. Atomicity Verdict

**A purely structural, single-PR reorg is achievable for the domain-owned ports, but not within a ≤400-changed-line budget once the mandatory re-export layer and every consumer's `use` path are counted — and it is not achievable *at all* for the `ego-runtime`-owned effect-store ports without violating this change's own "no new dependency-direction decision" spirit.**

What forces it over budget, concretely:

- §9's move/reexport matrix alone lists 34 items. Each relocated item needs, at minimum: the definition moved (1 file touched at the destination), a `pub use` re-export at the old path (1 line at the source), and every non-`ego-domain` consumer's `Cargo.toml` updated if it previously reached the type only transitively through `ego-domain`. Given §6's 92-file public-path footprint, even a "just add the re-export, touch nothing else" version of this change plausibly stays under the compile-breaking threshold, but the destination crate's own source (§8's tree) is realistically 1,500–2,000 lines once every trait, error type, and their existing doc comments and tests are relocated verbatim (not rewritten) — comfortably past a 400-changed-line single-PR budget on the "lines moved" measure alone, even though no line of *logic* changes.
- The `ego-runtime` effects-store case (§5, §7, finding 5) is not a pure re-export problem: relocating those three ports either (a) leaves `InMemoryEffectStore` in `ego-runtime` depending on a port now defined in `ego-persistence-api`, which is a legitimate but *new* dependency-direction decision this change's scope statement says not to make casually, or (b) requires moving `InMemoryEffectStore` too, which is explicitly prohibited ("only PORT TRAITS + CONTRACT TYPES move"). This is a genuine, structural fork in the plan, not a line-count problem — it needs an explicit architecture decision before any code moves, which belongs in `design.md`, not in this exploration.
- The `ego-domain → ego-persistence-api` re-export question (§5, §7) is the same kind of fork: keeping every domain-owned port's old `ego_domain::persistence::*`/`ego_domain::read_side::*`/`ego_domain::operation::*` path alive requires the domain crate to depend on the new crate, which is a hexagonal-layering decision that needs to be made explicitly, not assumed.

**Recommendation:** split CORE-PERSIST-A itself along the fork lines already visible in this exploration, rather than expanding its scope:

- **CORE-PERSIST-A1**: relocate only the domain-owned ports/types (§1 rows 1–11, §9 rows 1–26) into `ego-persistence-api`, with `ego-domain` re-exporting at every old path. This resolves the domain-layering question by choosing "yes, `ego-domain` may depend on `ego-persistence-api` for re-export purposes" as the one explicit architecture decision this slice makes, and is otherwise mechanical.
- **CORE-PERSIST-A2** (or folded into CORE-PERSIST-B, since it touches `ego-runtime`'s dependency shape): relocate the three `ego-runtime`-owned effect-store ports, deciding explicitly whether `ego-runtime` keeps `InMemoryEffectStore` (and gains the new dependency) or the convenience implementation is deferred to a later change.

Both slices remain purely structural (module moves + re-exports only, no SQL/behavior/signature changes) and each is independently closer to a reviewable single-PR size than one combined change — though even CORE-PERSIST-A1 alone should be expected to run somewhat over 400 changed lines once doc comments and existing unit tests move with their traits verbatim, and the review-workload guard should be applied against the actual diff once written rather than assumed in advance.
