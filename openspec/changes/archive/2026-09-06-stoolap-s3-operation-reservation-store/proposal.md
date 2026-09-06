# Proposal: STOOLAP-S3 — Operation Reservation Store: Durability Signal, Stoolap Implementation, Production Gate

> Canonical / English. Spanish companion: `proposal.es.md` (1:1 headings).

## Intent

`OperationReservationStore` (`crates/persistence-api/src/operation/reservation.rs:66-167`) is the
only port in `persistence-api` with **no durability signal**. Every sibling — `Snapshot`,
`EventStore`, `OffsetStore`, `DedupStore`, `ReadSideClaimStore` — declares
`fn is_durable(&self) -> bool { false }` plus a load-bearing `Arc<T>` forwarding impl (e.g.
`read_side/dedup.rs:33-35`, `59-67`). Because the signal is absent,
`validate_persistence_profile` (`crates/service-sdk/src/runtime/builder.rs:867-872`) cannot gate it,
so `Profile::Production` accepts the deliberately volatile `InMemoryOperationReservationStore` and
gains nothing from the genuinely durable `PostgresOperationReservationStore`. That is a contract gap
in the shared framework, independent of any backend.

STOOLAP-S2 deferred this port explicitly. S3 closes it and adds the missing third implementation.

## Scope

### In Scope

- **(a) Port fix**: `is_durable()` on `OperationReservationStore`, default `false`, plus the `Arc<T>`
  forwarding impl. Same pattern as every sibling — not `effect-store`'s `capabilities()` struct.
- **(a) Implementor sweep**: in-memory declares non-durable; Postgres overrides to `true`; every
  mock/fake/test double reviewed (`crates/testkit/src/reservation.rs` re-exports the in-memory one).
- **(b)** `StoolapOperationReservationStore`, durable only once it proves atomic reservation,
  ownership, lease, fencing-token monotonicity, and tenant isolation across close/reopen.
- **(c)** Production gate wired like `validate_read_side_claim_profile`: fail closed when the
  composition requires reservations and the store is non-durable. No existing check weakened, no
  backend-specific exception.
- **(d)** Gate tests for all three backends (durable accepted, non-durable rejected) plus a Stoolap
  reopen-durability test.

### Out of Scope

- Multi-process / multi-node Stoolap safety. Evidence in-tree is same-process only;
  `StoolapEffectStore` declares `multi_node_safe: false` (`crates/effect-store/tests/conformance.rs:296-302`).
- Any change to reservation semantics, retention, or other ports' gates.
- A second way to express durability in this codebase.

## Capabilities

### New Capabilities

- `persistence-stoolap-operation-reservation`: a Stoolap-backed reservation store exists, preserves
  every reservation invariant, and survives close/reopen.

### Modified Capabilities

- `persistence-api-surface`: the reservation port gains the durability signal and its `Arc` forwarding.
- `idempotent-command-processing`: the PostgreSQL reservation store reports durable.
- `persistence-memory-adapter`: the in-memory reservation store reports non-durable.
- `production-composition-hardening`: a reservation-store gate under `Profile::Production`.

## Approach

Copy the sibling port pattern verbatim for (a). For (b), follow `StoolapEffectStore`
(`crates/effect-store/src/stoolap/mod.rs`): `spawn_blocking` over the synchronous `Database`,
fail-closed `open()` requiring `sync=full`, `INSERT ... ON CONFLICT DO NOTHING` + re-`SELECT` to
classify outcomes, and the conditional `UPDATE ... WHERE version = $N` compare-and-swap already used
in `repository.rs:23-24` / `snapshot.rs:40-41` for fencing. For (c), reuse
`require_durably_configured`.

## Affected Areas

| Area | Impact | Slice |
|------|--------|-------|
| `crates/persistence-api/src/operation/reservation.rs` | Modified | (a) |
| `crates/persistence-memory/src/operation/reservation.rs`, `crates/persistence/src/postgres/reservation.rs`, `crates/testkit/src/reservation.rs` | Modified | (a) |
| `crates/persistence-stoolap/` | New | (b) |
| `crates/service-sdk/src/runtime/builder.rs` | Modified | (c) |
| tests across the above crates | New | (d) |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Durability claimed, not real (Stoolap's non-fsync default) | Med | Reopen test plus a sync-mode assertion before the flag flips |
| An external implementor silently inherits `false` and a Production build breaks | Med | Default `false` is the honest answer; rejection message names the fix, per "Rejections Are Actionable" |
| Missing `Arc` forwarding regresses a durable store to the default | Med | Forwarding impl shipped with the trait method; gate test wraps in `Arc` |
| Review budget over 400 lines | High | Slice by (a) / (b) / (c)+(d) |

## Rollback Plan

Revert per slice, newest first. (c) is the only behavior-changing piece and reverts alone. (b) is
purely additive. (a) reverts as a signature removal; the `false` default means no external
implementor is broken while it is in place.

## Dependencies

- `persistence-api-surface`, `persistence-stoolap-adapter` (S1), `persistence-stoolap-event-sourcing`
  (S2) — shipped. `stoolap` already pinned in `Cargo.lock`. No new external dependency.

## Success Criteria

- [ ] `OperationReservationStore::is_durable()` matches the sibling ports exactly, `Arc` forwarding included.
- [ ] In-memory reports `false`, Postgres and Stoolap report `true`; no implementor left unreviewed.
- [ ] Stoolap: reservation written, database closed, same file reopened, ownership + fencing token + tenant scope intact.
- [ ] `Profile::Production` rejects a non-durable reservation store and accepts both durable ones.
- [ ] No pre-existing gate check is weakened, and no Stoolap claim exceeds same-process concurrency.
