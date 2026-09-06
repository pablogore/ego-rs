# Proposal: STOOLAP-RS-01 — Durable Read-Side Stores on Stoolap

> Canonical / English. Spanish companion: `proposal.es.md` (1:1 headings).

## Intent

Three read-side ports — `OffsetStore`, `DedupStore`, `ReadSideClaimStore` — each have exactly one
durable implementation, and all three are PostgreSQL. `Profile::Production` gates all three
(`validate_read_side_progress_profile` / `validate_read_side_claim_profile`,
`crates/service-sdk/src/runtime/builder.rs:905-945`), so a Stoolap-only production composition
cannot run a projection at all. S1 (repository), S2 (event sourcing) and S3 (operation reservation)
closed every other durable port; the read-side trio is the last hole in the Stoolap durable profile.

This is a general framework capability. No product-specific naming, contract, or projection shape
enters this change.

## Scope

### In Scope

- Stoolap-backed `OffsetStore`, `DedupStore` and `ReadSideClaimStore`, behind a new `read-side`
  feature on the existing `crates/persistence-stoolap` crate.
- Real file-backed durability: value written, database closed, same file reopened, value intact.
  `is_durable()` returns `true` only once that is proven per store.
- Isolation exactly as each port keys it — offsets per `(projection_id, tag, tenant)`; dedup per
  `(projection_id, tag, event_id)`, with **no** tenant parameter (intentional on that port, not a
  gap); claims per `ClaimId { projection_id, tag, tenant }`.
- Claim correctness under real intra-process concurrency: exclusion, takeover after lease expiry,
  stale-owner rejection, fencing-token advance. `lease_until` stays caller-computed via the injected
  `Clock`; `try_claim` returning `Ok(None)` is a refusal, not an error.
- A **real** `Profile::Production` composition test over an on-disk Stoolap database, plus a negative
  control proving a volatile store is still rejected. Today that gate has stub-only coverage
  (`builder.rs:3950-4010`), unlike Postgres
  (`integration-tests/tests/infrastructure/read_side_progress_postgres.rs`).
- Conformance tests for all three stores.

### Out of Scope

- Every other port: `EventStore`, `Snapshot`, `Repository`, `EffectStateStore`, `EffectDedupStore`,
  `OperationReservationStore` — all shipped. The last is reused as a concurrency-pattern template only.
- Any PostgreSQL change, and any change to the three trait contracts. No evidence was found that any
  of them is defective.
- Multi-process Stoolap, multi-node/distributed coordination, Kubernetes leader election,
  `LISTEN`/`NOTIFY`, brokers, event buses. One ego-rs process owns the file. Concurrency **inside**
  that process (multiple tasks, workers, read-side sessions) is in scope; cross-process coordination
  is not, and nothing shipped here may claim it.
- Dedup pruning, TTL, or retention — an explicit Non-Goal of the existing read-side spec.
- Monotonicity or compare-and-swap on `write_offset`. That port is last-write-wins by contract.

## Capabilities

### New Capabilities

- `persistence-stoolap-read-side`: Stoolap-backed offset, dedup and read-side claim stores that
  survive close/reopen, isolate per the real port keys, remain correct under intra-process
  concurrency, and satisfy the existing Production gates.

### Modified Capabilities

- None. The `Profile::Production` read-side gates already exist and are not weakened, relaxed, or
  re-specified. This change adds a backend that satisfies them.

## Approach

Follow `StoolapOperationReservationStore` (`crates/persistence-stoolap/src/operation/reservation.rs`)
as the template — **not** the Postgres read-side stores. `try_claim` ports its two-statement CAS:
`INSERT ... ON CONFLICT DO NOTHING`, then a conditional `UPDATE` re-verifying the live row. Postgres
uses a single-statement `INSERT ... ON CONFLICT DO UPDATE ... WHERE ... RETURNING`; nothing in this
repo has ever proven that shape works on Stoolap 0.4, whereas the two-statement shape is already
stress-tested for intra-process concurrency (`crates/persistence-stoolap/tests/reservation_conformance.rs:192-240`).
Offsets and dedup are plain upserts.

Async bridging reuses the crate's per-store `run_blocking()` → `tokio::task::spawn_blocking`, never
`block_in_place`, consistent with every existing Stoolap store. The `read-side` feature composes from
`tokio`, `async-trait`, `chrono` and `ego-domain`, already optional on that manifest — zero new
transitive dependencies, and no new crate.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistence-stoolap/Cargo.toml` | Modified | New `read-side` feature; no new dependency |
| `crates/persistence-stoolap/src/read_side/` | New | The three stores plus schema |
| `crates/persistence-stoolap/tests/` | New | Conformance + reopen-durability tests |
| Production-profile composition test (crate placement per design) | New | Real Stoolap composition + volatile negative control |
| `crates/persistence-api/src/read_side/` | Unchanged | Contracts consumed as-is |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Stoolap 0.4 rejects or mis-executes the claim CAS SQL | Med | Port the already-proven two-statement pattern, not the Postgres single-statement one; conformance test is the gate |
| Durability claimed but not real (Stoolap's non-fsync default) | Med | Reopen test plus the crate's existing `sync=full` fail-closed `open()` check before `is_durable()` returns `true` |
| The Production gate stays stub-verified and a real composition still breaks | Med | Real composition test is in scope, not optional polish; negative control ships with it |
| Scope creep into multi-process/multi-node claims | Med | Non-goal stated in the spec text and in each store's module doc |
| Review budget over 400 lines | High | Slice by store: offset+dedup, claim, composition gate test |

## Open Questions

1. **Shared conformance harness or Stoolap-local tests?** `ego-testkit` has harnesses for
   reservation, carrier, event store and repository, but none for `OffsetStore`/`DedupStore`/
   `ReadSideClaimStore`. Adding three shared harnesses benefits future backends but widens the diff.
   Design phase decides.
2. **Where does the real composition test live?** `integration-tests/` mirrors the Postgres
   precedent, but Stoolap needs no container — a `tempfile` dir in `persistence-stoolap/tests/` may
   be enough. Design phase decides.
3. **Does `real-infrastructure-verification` need a delta?** That capability currently names no
   Stoolap backend. If a real-composition requirement belongs there rather than in the new
   capability, the spec phase must add a delta.

## Rollback Plan

Revert per slice, newest first. The whole change is additive and feature-gated: with the `read-side`
feature off, `cargo build` and `cargo test --workspace` behave exactly as before. No existing store,
gate, or trait is modified, so a full revert of the change removes files and one feature entry and
touches nothing already in production use.

## Dependencies

- `persistence-stoolap-adapter` (S1), `persistence-stoolap-event-sourcing` (S2),
  `persistence-stoolap-operation-reservation` (S3) — all shipped.
- `stoolap` 0.4 already pinned in `Cargo.lock`. No new external dependency.

## Success Criteria

- [ ] All three stores exist behind `read-side` on `persistence-stoolap`; `cargo check --workspace` with the feature off is unchanged.
- [ ] Each store: value written, database closed, same file reopened, value intact — and only then does `is_durable()` return `true`.
- [ ] Isolation proven per the real port keys, including dedup's deliberate absence of a tenant parameter.
- [ ] Concurrent `try_claim` from multiple tasks in one process yields exactly one holder; takeover, stale-owner rejection and fencing-token advance all verified.
- [ ] A real `Profile::Production` composition over an on-disk Stoolap database builds successfully, and a volatile store in the same composition is rejected.
- [ ] No shipped artifact claims multi-process, multi-node, or distributed coordination.
