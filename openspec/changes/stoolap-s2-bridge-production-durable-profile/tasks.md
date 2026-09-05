# Tasks: STOOLAP-S2 — Stoolap-Backed Durable Production Profile

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~900-1100 total (Slice1 ~350-450, Slice2 ~350-450, Slice3 ~150-200) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 (Snapshot+helpers) -> PR2 (EventStore, may split to 2a/2b) -> PR3 (restart recovery, test-only) |
| Delivery strategy | ask-on-risk |
| Chain strategy | stacked-to-main |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | `stoolap_common.rs` + `snapshots` table + DSN `sync=full` guard, no new dep | PR 1 | `cargo test -p ego-persistence-stoolap --lib` | N/A — unit tests only, no full runtime yet | Revert `stoolap_common.rs`, `snapshot.rs`, repository.rs import diff; no downstream consumer |
| 2 | `EventStore<E>` behind `event-sourcing` feature | PR 2 | `cargo test -p ego-persistence-stoolap --features event-sourcing --lib` | N/A — colocated `#[tokio::test]` (current-thread), no full builder yet | Revert `event_sourcing/` module + Cargo.toml feature/deps; Unit 1 intact |
| 3 | Restart recovery via real Production composition | PR 3 | `cargo test -p ego-persistence-stoolap --features event-sourcing --test production_restart_recovery` | Real `EntityRuntimeBuilder::profile(Profile::Production).try_build()`, write->drop->reopen | Delete the single test file; nothing else references it |

**Risk**: Slice 2 (design.md) may need a further split at the `begin`/unit-of-work boundary if append/load/list + UoW/receipts together exceed 400 lines. Do not force one PR; confirm actual diff size before starting 2.4.

## Phase 1: Snapshot + Shared Helpers (PR1)

- [x] 1.1 Create `crates/persistence-stoolap/src/persistence/stoolap_common.rs`: promote `dsn_for`, `SYSTEMWIDE_SCOPE`, `encode_tenant`, `internal_err`, `is_write_conflict` to `pub(crate)`.
- [x] 1.2 Edit `crates/persistence-stoolap/src/persistence/repository.rs`: remove duplicated private fns, `use` the promoted items; behavior unchanged. Verify `tests/repository_conformance.rs` (read-only) still green.
- [x] 1.3 Create `crates/persistence-stoolap/src/persistence/snapshot.rs`: sync `Snapshot` impl + `snapshots` table (`tenant_id/aggregate_id/version/payload`, `UNIQUE(...)`), `open()` DSN guard (AD-3: read back `db.dsn()`, fail closed if not `sync=full`).
- [x] 1.4 Unit tests in `snapshot.rs`: DSN carries `sync=full`; non-`sync=full` engine refused; round-trip; tenant vs systemwide isolation (spec: Tenant Scoping Is Honored Correctly).
- [x] 1.5 Unit test in `snapshot.rs`: `stoolap::test_failpoints` (`WAL_WRITE_FAIL`) — failed WAL sync surfaces as error, never silent success (gates AD-3).

## Phase 2: EventStore<E> (`event-sourcing` feature) (PR2, may split)

- [ ] 2.1 Edit `crates/persistence-stoolap/Cargo.toml`: optional `tokio`/`async-trait`, `event-sourcing = ["dep:tokio", "dep:async-trait"]` feature, dev-dep `ego-persistent-entity`.
- [ ] 2.2 Create `crates/persistence-stoolap/src/event_sourcing/event_store.rs`: `StoolapEventStore::open()` reusing `stoolap_common::dsn_for` + DSN guard; `run_blocking` shaped on `StoolapEffectStore::run_blocking` (`crates/effect-store/src/stoolap/mod.rs`) (read-only pattern reference).
- [ ] 2.3 Implement `append`/`load`/`list`, each ending in exactly one `tx.commit()` (AD-3.4); classify conflicts via `stoolap_common::is_write_conflict`.
- [ ] 2.4 Implement unit-of-work + receipts per the `EventStore<E>` trait contract (`persistence-api/src/persistence/event_store.rs:47`, read-only). Confirm slice size before/after; split to 2a/2b if over budget.
- [ ] 2.5 Unit tests (`#[tokio::test]`, current-thread): append/load/list; optimistic-concurrency conflict; receipt conflict; dropped UoW leaves nothing committed.
- [ ] 2.6 Unit test: tenant isolation — same aggregate id under two `tenant_id`s, no cross-tenant visibility (spec: Tenant Scoping Is Honored Correctly).

## Phase 3: Production Composition + Restart Recovery (PR3, test-only)

- [ ] 3.1 Create `crates/persistence-stoolap/tests/production_restart_recovery.rs`: phase-1 scope builds real `EntityRuntimeBuilder::profile(Profile::Production)` with `StoolapEventStore`/snapshot store, `try_build()`, commands across snapshot threshold, drop.
- [ ] 3.2 Same file, phase 2: reopen identical path, recover entity, assert state+version match phase 1 (spec: Committed State Survives Runtime Destruction and File Reopen).
- [ ] 3.3 Same file: negative control — `Profile::Production` + in-memory stores still refused.
- [ ] 3.4 Same file: assert `try_build()` succeeds with zero PostgreSQL dependency present (spec: Production Builds Without PostgreSQL).
