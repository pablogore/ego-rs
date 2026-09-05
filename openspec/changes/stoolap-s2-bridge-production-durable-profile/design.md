# Design: STOOLAP-S2 — Stoolap-Backed Durable Production Profile

> Canonical / English. Spanish companion: `design.es.md` (1:1 headings).

## Technical Approach

Two new stores in the existing `ego-persistence-stoolap` crate. `Snapshot`
(`persistence-api/src/persistence/snapshot.rs:14`) is **synchronous**, so it needs nothing S1 does not
already have. `EventStore<E>` (`event_store.rs:47`) is `#[async_trait]`, so only it needs
`spawn_blocking` — copied in shape from `StoolapEffectStore::run_blocking`
(`effect-store/src/stoolap/mod.rs:227`). No gate, no facade, no `Repository<A>` code is touched.

## Architecture Decisions

### AD-1: Crate placement — same crate, `event-sourcing` feature

| Option | Tradeoff | Decision |
|---|---|---|
| Sibling crate `ego-persistence-stoolap-event-sourcing` | Keeps S1 tokio-free, but `dsn_for`/`encode_tenant`/`is_write_conflict`/`internal_err` are private fns in `repository.rs` — they would be copied or newly exported | Rejected: duplicates exactly what AD-2 wants shared |
| Same crate, unconditional `tokio` + `async-trait` deps | Smallest diff, but every `Repository<A>` consumer gains an async runtime, contradicting the proposal's "no existing crate gains a non-dev dependency" rollback claim | Rejected |
| **Same crate, `event_sourcing` module behind `event-sourcing = ["dep:tokio", "dep:async-trait"]`** | Two Cargo lines; helpers shared in-crate; sync-only consumers unchanged | **Chosen** — mirrors `ego-effect-store`'s own optional-dep pattern (`effect-store/Cargo.toml:46-50`), which is one crate with feature-gated backends, not a sync/async split |

`Snapshot` lands **outside** the feature: it adds zero dependencies.

### AD-2: Reuse boundary

| Source | Reused | Not reused |
|---|---|---|
| S1 `repository.rs` | `dsn_for` (`file://{p}?sync=full`), `SYSTEMWIDE_SCOPE`, `encode_tenant`, `internal_err`, `is_write_conflict` — promoted from private to `pub(crate)` in a new `stoolap_common` module; `aggregates` table shape (`tenant_id/aggregate_id/version/payload` + `UNIQUE(...)`) reused verbatim as `snapshots` | `save()`'s synchronous read-check-write body |
| `StoolapEffectStore` | `run_blocking` (clone `Database`, `spawn_blocking` — **not** `block_in_place`, which panics on current-thread runtimes); dialect rules: `UNIQUE` not composite `PRIMARY KEY`, TEXT payloads (no BYTEA), no `DELETE ... IN (SELECT ...)` | `backend_err` — S2 returns `PersistenceError`, so S1's `is_write_conflict` classifies instead |

### AD-3: Durability is checked, not asserted

`StoolapEffectStore::open` builds `file://{path}` with **no** `sync=full` yet reports
`durable: true` — the exact defect this design must not repeat. Rule:

1. Both stores open only through the shared `dsn_for()`.
2. `open()` opens through the shared `dsn_for()` (`sync=full`), then reads back `db.dsn()` and
   returns `PersistenceError::Internal` if it lacks `sync=full` — a defensive second check.
   Verified during implementation (`snapshot.rs`'s
   `open_refuses_a_path_already_locked_by_a_non_durable_engine`): Stoolap's process-global registry
   shares one live engine only for an *identical* DSN string (`effect-store/src/stoolap/mod.rs:170-173`)
   — a different DSN for the same path, such as an already-open weaker-sync engine, is never handed
   back. Instead `Database::open()` itself fails with `stoolap::Error::DatabaseLocked` (the on-disk
   file lock is already held), caught by the same `map_err(internal_err)` as any other open failure,
   before the `sync=full` check ever runs. Either failure mode fails closed the same way.
3. `is_durable() -> true` is then backed by a construction invariant, not by presence — the property
   `require_durably_configured` (`profile.rs:44-50`) demands.
4. `append` and unit-of-work `commit` each end in exactly one `tx.commit()`. No deferred or batched
   commit path may exist, or `sync=full` stops meaning "durable when the call returns".

### AD-4: Restart recovery through the real composition path

Template: S1's `a_committed_save_survives_close_and_reopen` (`repository.rs:344-367`) — inner scope
writes and drops, outer scope reopens the same `TempDir` path. Adapted for event sourcing:

```
{ // phase 1
  EntityRuntimeBuilder::new().profile(Profile::Production)
    .with_event_store(Arc::new(StoolapEventStore::open(path).await?))
    .with_snapshot_store(Arc::new(Mutex::new(StoolapSnapshotStore::open(path)?)))
    .with_snapshot_strategy(/* fires below the event count */)
    .try_build()?                       // the real gate, no shortcut
  // send commands -> events across the snapshot threshold
}                                       // drop releases the engine
// phase 2: identical builder chain, same path -> state and version match
```

Plus a negative control in the same file: `Profile::Production` + in-memory stores still refuses.

## Data Flow

    EntityRuntimeBuilder(Production) ──validate_persistence──> is_durable()==true (AD-3 invariant)
              │
        PersistenceFacade ──> StoolapEventStore ──spawn_blocking──> Database (sync=full)
              │                                                          │
              └──────────> StoolapSnapshotStore ──(sync, direct)─────────┘  one file

## File Changes

| File | Action | Description |
|---|---|---|
| `crates/persistence-stoolap/src/persistence/stoolap_common.rs` | Create | DSN, tenant encoding, error classification promoted to `pub(crate)` |
| `crates/persistence-stoolap/src/persistence/repository.rs` | Modify | Import the promoted helpers; behavior unchanged |
| `crates/persistence-stoolap/src/persistence/snapshot.rs` | Create | Slice 1 — sync `Snapshot` + `snapshots` table + DSN guard |
| `crates/persistence-stoolap/src/event_sourcing/event_store.rs` | Create | Slice 2 — `EventStore<E>`, unit of work, receipts |
| `crates/persistence-stoolap/Cargo.toml` | Modify | Optional `tokio`/`async-trait`, `event-sourcing` feature, dev-dep `ego-persistent-entity` |
| `crates/persistence-stoolap/tests/production_restart_recovery.rs` | Create | Slice 3 — restart recovery + negative control |

No cycle: `persistent-entity` does not depend on `persistence-stoolap`.

## Testing Strategy

| Slice | Layer | What | Approach |
|---|---|---|---|
| 1 | Unit | DSN carries `sync=full`; non-`sync=full` engine refused; snapshot round-trip; tenant vs systemwide isolation | Colocated, `TempDir`, S1's `db_test_guard()` |
| 1 | Unit | A failed WAL sync surfaces as an error, never silent success | `stoolap::test_failpoints` (`WAL_WRITE_FAIL`), as S1 does |
| 2 | Unit | append/load/list, optimistic-concurrency conflict, receipt conflict, dropped UoW leaves nothing | Colocated, `#[tokio::test]` (current-thread) |
| 3 | Integration | Production build, write, drop, reopen, state+version identical; volatile stores still refused | `tests/production_restart_recovery.rs` |

Each slice leaves the workspace green alone: 1 adds no dependency, 2 adds the feature, 3 is test-only.

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file classification, or
process-integration boundary.

## Migration / Rollout

No migration. Purely additive; `CREATE TABLE IF NOT EXISTS` in the adapter's own file.

## Open Questions

- [ ] Whether `sync=full` fsyncs per commit or on an interval is a Stoolap-internal fact this design
      asserts from S1's DSN choice, not from reading Stoolap. Slice 1's failpoint test is the gate:
      if a suppressed WAL sync does not surface as an error, AD-3 is unproven and slice 2 must stop.
- [ ] Reopen-after-drop proves clean-close recovery, not kill -9 crash durability. Out of scope here;
      name it in the spec's durability requirement rather than implying more.
