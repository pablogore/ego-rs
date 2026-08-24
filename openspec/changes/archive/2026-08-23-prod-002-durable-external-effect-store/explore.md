# Exploration: PROD-002 Durable External Effect Store

## Current State

CORE-019 ("Reliable External Effects", shipped, archived at `openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/`, living spec `openspec/specs/external-effects/spec.md`) built the full at-least-once delivery pipeline (accept → queue → delivery runner → executor) around two async ports in `crates/runtime/src/effects/store.rs`:

- `EffectStateStore` — `pending → in-flight → succeeded | retryable-failed | terminal-failed`, with `accept`/`mark_in_flight`/`mark_succeeded`/`mark_retryable`/`mark_terminal`/`claim_due`/`recover_in_flight`.
- `EffectDedupStore` — `reserve`/`commit_success`/`release`, scoped `(tenant, effect_type, key)`, `DedupOutcome::{Fresh, OwnedInProgress, OwnedSucceeded, OtherInProgress, OtherSucceeded, Conflict}`.

Both are `#[async_trait] pub trait ...: Send + Sync`, built entirely from public types (`AcceptedEffect`, `EffectId`, `Timestamp`, `StoredEffect`, `TerminalReason`, `EffectStoreError`) — implementable from any crate depending on `ego-runtime`. This is exactly the seam PROD-002 fills.

The only shipped implementation, `InMemoryEffectStore` (same file), implements both ports as one composite and is explicitly labeled "convenience only" — the spec's own Non-Goals section states verbatim: *"Durable delivery store implementation (Postgres outbox) — the ports are shaped to enable one, but none ships in this capability."* It also documents today's gap: the in-memory store loses every `Pending`/`InFlight` effect on crash, degrading "at-least-once" to "at-most-once across a crash."

Runtime integration: `RuntimeEffectAcceptor` (`crates/runtime/src/effects/acceptor.rs`) implements `persistent_entity::effect_acceptor::EffectAcceptor` (`crates/persistent-entity/src/effect_acceptor.rs` — dependency direction is `runtime → persistent-entity`, never reversed, so the actor never depends on `ego-runtime`). `EntityRuntimeBuilder::with_effect_acceptor` wires it into every spawned actor; `ego-service-sdk`'s `RuntimeBuilder::register_effect_executor` is the host-facing registration entry point. `DeliveryRunner` (`crates/runtime/src/effects/runner.rs`) owns a periodic reclaim loop driven by `claim_due`/`recover_in_flight` — exactly the affordances a durable/crash-recoverable store needs, already contractually required by spec.md's "Delivery State Is Reconstructable After a Restart."

**`ROADMAP.md` §7.2 already scopes PROD-002 explicitly (Priority P0)**: *"CORE-019 provides the execution model. Production requires durable implementations of: `EffectStateStore`, `EffectDedupStore`, PostgreSQL persistence, Atomic state transitions, Claim ownership, Lease/fencing semantics, Retry persistence, Crash recovery, Stale claim recovery, Cleanup."* The proposal should track this checklist rather than re-derive scope.

Adjacent, textually similar planned capability: CORE-030 "Transactional Outbox" (`ROADMAP.md` §5.1, status PLANNED, no change folder yet) — a *different* capability (durable publication of domain/integration events, not `ExternalEffectDescription` delivery) but shares vocabulary worth mirroring for consistency: "claiming," "leases," "crash recovery," "retry," "cleanup," "poison message handling," "at-least-once ... exactly-once claims must not be made across arbitrary external systems."

## Affected Areas

- `crates/runtime/src/effects/store.rs` — the `EffectStateStore`/`EffectDedupStore` traits and error taxonomy PROD-002 implements (does not redesign).
- `crates/runtime/src/effects/observability.rs` — existing `log_*` signal functions to extend, not duplicate.
- `crates/runtime/src/effects/runner.rs` — the reclaim loop that will drive a durable store's `claim_due`/`recover_in_flight`.
- `crates/persistence/src/postgres/{event_store,repository,snapshot,migrations}.rs` and `crates/persistence/src/postgres/migrations/*.sql` — existing sqlx/Postgres conventions (numbered migration files, `PgPool`, DB-error-code → typed-error mapping) to reuse for a new durable effect store.
- `crates/persistence/Cargo.toml` — shows the `sqlx = "0.8"` feature set already vetted for this workspace.
- `crates/testkit/src/effects.rs` — existing `RecordingExecutor` test double convention ("real trait impl, not a mock") to mirror for a new durable-store TestKit double.
- `ARCHITECTURE.md` — the verified crate dependency graph that surfaces the placement gap below.
- `ROADMAP.md` §7.2 (PROD-002) and §5.1 (CORE-030) — scope and vocabulary source.

## Approaches

1. **New crate depending on both `ego-runtime` and `sqlx`** (e.g. `ego-persistence-effects` or similar) implementing `EffectStateStore`/`EffectDedupStore` against Postgres.
   - Pros: no new dependency edge on existing crates; keeps `ego-persistence` and `ego-runtime` exactly as documented in `ARCHITECTURE.md` today; clean single-responsibility crate.
   - Cons: one more crate in an already-16-crate workspace; some duplication of sqlx/migration boilerplate versus `ego-persistence`.
   - Effort: Medium.

2. **Extend `ego-persistence` with a new dependency on `ego-runtime`** and add the Postgres effect-store module there alongside the existing event/snapshot/repository Postgres modules.
   - Pros: reuses existing sqlx setup, migration-runner, and Cargo dependencies directly; one less crate to add.
   - Cons: adds a new edge `ego-persistence → ego-runtime` not present in the current verified dependency graph — `ARCHITECTURE.md` would need updating, and `ego-persistence` currently depends on nothing but `ego-domain`, which is presented there as a deliberate boundary.
   - Effort: Low-Medium.

## Recommendation

Lean toward Approach 1 (new crate) to avoid silently inverting today's documented dependency direction, but this is exactly the kind of AD that belongs in `design.md`, not decided here — flag it explicitly as an open architectural decision in the proposal.

## Risks

- Crate-placement/dependency-direction decision (above) is unresolved and blocks implementation until chosen.
- Claim ownership / lease / fencing semantics: today's `claim_due` has no ownership marking; a durable store used by multiple runner instances or across restarts needs real lease/fencing tokens to avoid double-dispatch — nothing in CORE-019 addresses multi-process contention.
- Stale claim recovery: `recover_in_flight` today assumes a single-process restart (sweep in-flight → pending); distributed lease expiry is a different, harder problem not covered by the existing contract.
- Cleanup/retention of terminal/succeeded rows has no precedent (in-memory store just grows unbounded in short-lived test processes).
- No TestKit fault-injection double exists yet for `EffectStateStore`/`EffectDedupStore` (crash simulation, transient failures, lease races) — must be built new, following `RecordingExecutor`'s "real trait impl" convention.
- Migration versioning must be reconciled if new tables land inside `ego-persistence`'s existing numbered migration sequence (currently 001–006).

## Ready for Proposal

Yes — scope is unusually well pre-defined by `ROADMAP.md` §7.2 and CORE-019's already-final trait/error/observability surface. The proposal's real work is picking the crate-placement AD and the lease/fencing model, not inventing new abstractions.
