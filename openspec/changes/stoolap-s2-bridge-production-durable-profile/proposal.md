# Proposal: STOOLAP-S2 — Stoolap-Backed Durable Production Profile

> Canonical / English. Spanish companion: `proposal.es.md` (1:1 headings).

## Intent

Production has one unconditional gate: `EntityRuntimeBuilder::validate_persistence`
(`crates/persistent-entity/src/builder.rs:290-305`) requires `is_durable()` from the entity's
`EventStore<E>` **and** `Snapshot`, via `require_durably_configured` (`profile.rs:51-63`).
`PersistenceFacade<E>` (`persistence.rs:211-245`) is built from exactly those two traits — no
alternate constructor, no `Repository<A>` route into entity-runtime construction. So
`Profile::Production` means PostgreSQL, and S1's `StoolapRepository<A>` sits off that path entirely.
A future host (Verimand Bridge, unbuilt) needs Production on an embedded file. This proposal scopes
what ego-rs must offer, not Bridge's design.

## Scope

### In Scope

- Stoolap-backed `EventStore<E>` and `Snapshot`.
- Real durability: write, drop the runtime, reopen the same file, state recovers.
- `Profile::Production` passing `try_build()` for real, zero PostgreSQL.
- Single-process/single-node file owner (S1's R12); single tenant per process
  (`examples/reference-app/src/lib.rs:722-724`).

### Out of Scope

- `OperationReservationStore`, `OffsetStore`, `DedupStore`, `ReadSideClaimStore`.
- Any change to `Repository<A>`/`StoolapRepository`, or to the Production gates.
- Effect-store reimplementation — `StoolapEffectStore` already satisfies its ports.
- Multi-process or multi-node Stoolap.
- Memory wrappers reporting `is_durable() == true`.

Deferred, conditional on Bridge adopting enforced idempotency or durable read-side projections:
`OperationReservationStore`, durable read-side persistence.

## Capabilities

### New Capabilities

- `persistence-stoolap-event-sourcing`: Stoolap-backed `EventStore<E>` and `Snapshot` exist, survive
  process restart, and let a `Profile::Production` runtime build and recover without PostgreSQL.

New, not modified: `openspec/specs/persistence-stoolap-adapter/spec.md` states in its Purpose that it
"does not cover any other Stoolap-backed store (`EventStore`, `Snapshot`…)", and R1–R12 are entirely
`Repository<A>` save/load/delete plus optimistic concurrency.

### Modified Capabilities

- **None expected.** The spec phase confirms; a required change is a blocking question.

## Approach

Follow `StoolapEffectStore` (`crates/effect-store/src/stoolap/mod.rs`), not S1: it already wraps
Stoolap's synchronous `Database` behind async traits via `spawn_blocking`, with a proven error
classifier — the shape `EventStore`'s async, append-only semantics need. Reuse from S1 only what
fits: tenant/aggregate/version/payload columns, the `file://{path}?sync=full` DSN, `SYSTEMWIDE_SCOPE`
+ `encode_tenant`. Do not carry S1's synchronous optimistic-concurrency `save` into `EventStore`.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistence-stoolap/` | Modified | Both stores plus schema (design may pick a sibling crate) |
| `crates/persistent-entity/`, `crates/effect-store/` | Untouched | Gates and effect store are reference only |
| `openspec/specs/persistence-stoolap-event-sourcing/` | New | Capability spec |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Durability claimed, not real (Stoolap's non-fsync default) | Med | Reopen test plus a sync-mode assertion; S1's KD-2 records this regressing silently in-tree |
| S1's sync `Repository` code copied into async `EventStore` | Med | `StoolapEffectStore` is the template; S1 reuse limited to schema/DSN/tenant encoding |
| Review budget over 400 lines | High | Slice: (1) `Snapshot`, (2) `EventStore`, (3) Production build + restart recovery |

## Rollback Plan

One revert commit. Purely additive: no existing crate gains a non-dev dependency, no framework path
wires the new stores, gates and `StoolapEffectStore` are untouched, and only the adapter's own tables
in its own file are created. No migration either direction; mid-flight rollback is equally safe.

## Dependencies

- `persistence-api-surface` (shipped) — `EventStore<E>`, `Snapshot`, consumed unchanged.
- `persistence-stoolap-adapter` (S1) — patterns only, no code coupling.
- `stoolap`, already pinned in `Cargo.lock`. No new external dependency.

## Success Criteria

- [ ] A `Profile::Production` runtime builds on Stoolap with no PostgreSQL in its dependency graph.
- [ ] Events written, runtime dropped, same file reopened, entity state recovers identically.
- [ ] `validate_persistence` and `require_durably_configured` are unmodified in the diff.
- [ ] Both stores report `is_durable() == true` because they fsync, not because a wrapper says so.
- [ ] No out-of-scope store implementation appears in the diff.
