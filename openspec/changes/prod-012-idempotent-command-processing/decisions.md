# Decisions: PROD-012 — End-to-End Idempotent Command Processing

**Phase**: interactive proposal question round (between `sdd-explore` and `sdd-propose`)
**Date**: 2026-08-02
**Status**: binding — these are the user's answers and are the primary input to `sdd-propose`

## Identifier

Originally raised as PROD-015. Renumbered to **PROD-012**, the next free
identifier after `ROADMAP.md` §7.11. No existing work justified reserving
PROD-012 through PROD-014, and keeping PROD-015 would leave three ambiguous
gaps. Decisions D1–D7 are unaffected by the renumbering.

## D1 — The key is mandatory on every external mutable command

HTTP, gRPC, and Kafka must all supply an `Idempotency-Key`. A missing key
rejects the command. The guarantee is clear and verifiable.

**No server-side key generation.** A key generated on the server is a function
of the request as received, not of the client's intent. On a retry the server
sees a new request, generates a new key, and deduplicates nothing. The key must
originate where the intent originates.

**Temporary migration path allowed** if compatibility requires it, but explicit
and bounded. Precedent already in the codebase:
`TenantEnforcementMode` (`crates/service-sdk/src/runtime/tenant.rs:143`) — a
fixed-invariant enum with a fail-closed default whose own documentation states it
is "not `dyn`-dispatched — the resolution policy is a fixed invariant, not a
per-deployment plugin." The idempotency enforcement mode follows that shape, not
free per-endpoint configuration.

Rejected: per-operation opt-in (leaves unguaranteed zones, depends on correct
configuration) and client opt-in (PROD-012 could not then promise end-to-end
idempotent processing at all).

## D2 — The key identifies the complete business operation

One key per external request, not per aggregate command. It covers
`RegisterUserImpl`'s org-then-user dual write as a single unit.

Consequences:

- Forces the pre-dispatch store (exploration candidate b) and rules out the
  same-transaction-only dedupe (candidate a), which cannot cover commands that
  emit zero events or are rejected by a guard before persisting.
- The operation result must be stored so a retry can be answered with the same
  response.

**Derived constraint (not asked — forced by D1 + §6 of the exploration):** the
reservation happens *after* `#[authorize]` and `#[tenant_scoped]`, and before the
first `EntityRuntime` call. The key must be namespaced by the `CanonicalTenant`
produced by `TenantResolver::resolve`, never by the raw client-supplied tenant
hint, which is exactly what CORE-008A forbids.

## D3 — In-progress reservations use a lease with owner and expiry

A reservation in progress has an owner and an expiry. If the process dies, the
lease expires and a later retry takes it over and re-executes. This is the only
option that covers the process-restart case without manual intervention.

Design vocabulary precedent:
`DedupOutcome::{OwnedInProgress, OtherInProgress}` in
`crates/runtime/src/effects/store.rs`. The model exists; only a durable
implementation is missing.

**Clock requirement.** Generalize the auth `Clock`
(`crates/domain/src/auth/clock.rs:20`) into a common abstraction and inject it
into both the new store and `EffectDedupStore`. No direct `Utc::now()` anywhere
in lease logic: expiry, renewal, and takeover must be deterministically testable
under Strict TDD. Note that `crates/runtime/src/effects/store.rs:58` currently
calls `Utc::now()` directly and therefore has no testable clock — that is a
defect to fix here, not a precedent to inherit.

Rejected: releasing on failure (a crash leaves nobody to mark the failure, so the
reservation hangs forever) and rejecting while in progress (an orphaned
reservation blocks that key indefinitely).

## D4 — Re-execution safety via durable per-aggregate receipts

The actor records receipts keyed by
`(tenant_id, aggregate_type, aggregate_id, operation_key)`.

**Not "the last key applied".** The aggregate can process other operations
between the original attempt and the recovery, so a single slot is overwritten
and the evidence is lost.

Worked example — `RegisterUser(K)`:

| Aggregate | Key | Receipt |
| --- | --- | --- |
| `TenantOrganization` / `org-1` | K | applied |
| `User` / `user-7` | K | applied |

If the lease expires after the first command, the new owner re-executes:
`TenantOrganization + K` is already applied and no-ops; `User + K` is not yet
applied and executes. The operation completes without duplicating
`UserRegistered`.

**The snapshot is not the source of truth.** The receipt is confirmed atomically
with the append. Event metadata carries `operation_key`, and a durable index or
record makes the lookup efficient. A snapshot may accelerate the check but must
never be the only evidence.

**Zero-event successes still write a receipt** inside the same transaction.

**Responsibility split.** `operation_key` + `fingerprint` make delivery to the
aggregate idempotent. `owner_id` + `fencing_token` belong to the lease and
control who may renew or close the operation. Events do not carry the lease
owner — mixing it into the durable identity of the operation would complicate the
domain for nothing.

Rejected: a documented state-checking-handler contract (this is precisely the
discipline that already failed once — `UserEntity` is the live bug) and
step checkpointing in the reservation (that is a saga log and collides with
CORE-029, which is unstarted).

## D5 — Split retention

| Record | Retention |
| --- | --- |
| Operation reservation + stored response | Configurable TTL, counted from `completed_at`, **never** from `created_at` |
| `InProgress` reservations | Never purged by TTL — must first be recovered via lease expiry/takeover |
| Per-aggregate receipts | Permanent for the life of the stream |

Receipt deletion happens **only** alongside an explicit, definitive deletion of
the aggregate or tenant, never through the ordinary retention job.

Purge must be batched, observable, and safe for multiple concurrent workers.

**The TTL consequence must be documented precisely.** Once the reservation is
purged there is no longer an exact response replay and no boundary-level
detection of a reused key. The permanent receipts still prevent that operation
from re-mutating any aggregate it already reached. For operations rejected before
touching any aggregate, or successful without reaching one, protection ends with
the TTL.

This yields two distinct guarantees, which must be named separately rather than
blurred into one promise:

- **Replay window** = reservation TTL
- **Domain duplication protection** = life of the stream

**The receipts table stores the fingerprint too**, not only `operation_key`:

- same key + same fingerprint → already applied
- same key + different fingerprint → permanent conflict

This bounds the growth of potentially large stored responses without ever
silently reopening a business transaction that was already applied.

## D6 — Proceed now with a multi-node-ready schema

PROD-012 waits for neither PROD-009 nor CORE-030.

| Change | Responsibility |
| --- | --- |
| **PROD-012 (now)** | Durable idempotency, real unique constraints, leases with `owner_id` / `lease_until` / `fencing_token`, atomic takeover, per-aggregate receipts, injectable `Clock` |
| **PROD-009 (later)** | Activates multiple workers/nodes over that contract and adds real distributed-contention tests |
| **CORE-030 (later)** | Guarantees idempotent/atomic effect publication via outbox; consumes `operation_key` / `causation_id` but does not define command deduplication |

**Storing a `fencing_token` is not enough.** Every operation that renews,
completes, or abandons a reservation must verify
`operation_id + owner_id + fencing_token` through a conditional update. An
expired owner receives `StaleOwner` and cannot close the operation.

**Explicit non-promise.** PROD-012 does not promise atomicity between the two
aggregates of `RegisterUserImpl`. It promises safe recovery by re-execution:
organization receipt already confirmed → no-op; user receipt absent → execute;
no duplicated events.

**Unavoidable foundation fixes, in scope:**

1. Unique constraint on `events (tenant_id, aggregate_type, aggregate_id, version)`,
   adjusted to the real tenant model.
2. Real unique constraint on the receipts table.
3. A new `EventStore` contract that confirms append + receipt in one transaction.
4. A transactional path for successful commands that produce zero events.

## D7 — A new, distinct `OperationKey` type

```
OperationKey
└─ external business intent
   └─ ingress → operation → internal commands → per-aggregate receipts

IdempotencyKey
└─ post-commit effect deduplication
   └─ f(uow_id, effect_index)
```

Both initially validate a non-empty string, but they are **not
interchangeable**. The compiler must prevent a key derived for an email, webhook,
or other effect from identifying an external operation.

- `OperationKey` is defined in the **common domain crate**
  (`crates/domain/src/`, alongside the existing `idempotency.rs`) — not under
  HTTP and not under runtime — because it crosses every layer: external
  boundary, durable operation reservation, propagation to the actors,
  per-aggregate receipt, and recovery by re-execution.
- `IdempotencyKey` stays untouched, including its current documentation.
- The validation function may be shared internally. The public newtype may not.
- **No implicit conversions.** If a future integration needs to relate them, the
  derivation must be deliberate and named, e.g.
  `EffectIdempotencyKey::from_operation_effect(&operation_key, effect_index)` —
  never a generic `From<OperationKey>`, which would erase the boundary again.

## Verified Constraints Discovered During the Question Round

Each of these was confirmed against `develop` while validating the decisions
above. They are facts, not assumptions, and the design must account for them.

1. **The zero-event path never opens a transaction.** `actor.rs:219` —
   `Ok(events) if events.is_empty()` returns `CommandResult::NoEvents` and never
   calls `persist_events`. D4's zero-event receipt requires changing that branch,
   not extending it.
2. **The transaction boundary lives inside `EventStore::append`.** It is opened
   with `pool.begin()` and committed in the same function
   (`crates/persistence/src/postgres/event_store.rs:76–129`), inside a
   `block_on`, and the trait is synchronous. No caller can join that transaction,
   so D4's atomicity requires changing the `EventStore` trait and both
   implementors.
3. **There is no event metadata channel.** `StoredEvent`
   (`crates/domain/src/persistence/stored_event.rs:6`) carries only
   `correlation_id`, and the Postgres store never persists it — there is no
   column in `001_create_events.sql` and no bind in the INSERT; `correlation_id`
   has zero occurrences in `crates/persistence/src`. Carrying `operation_key` in
   event metadata means building the first such channel.
4. **`events` has no unique constraint** on `(aggregate_id, tenant_id, version)`
   — only the non-unique index `idx_events_aggregate`. The `23505` handling in
   `append` is therefore unreachable code.
5. **`aggregate_type` is not a column, and the type is concatenated into
   `aggregate_id`.** `EntityTriple::aggregate_id()`
   (`crates/persistent-entity/src/scheduler.rs:30`) returns
   `format!("{}-{}", entity_type, entity_id)`, and that string is what
   `actor.rs:230` persists. Hyphen-joining is ambiguous: entity type
   `user-account` with id `7` produces the same string as type `user` with id
   `account-7`. Splitting these into real columns is a correctness requirement
   for D4/D6, not tidiness.
6. **`tenant_id` is nullable and NULL is a blessed mode.**
   `resolve_tenant(None) → Ok(None)` is the spec-blessed tenant-less/systemwide
   mode from CORE-008A D1, with its own test
   (`crates/persistence/src/postgres/mod.rs`). In Postgres a plain `UNIQUE`
   treats NULLs as distinct, so D6's constraint would enforce nothing for
   systemwide aggregates.
7. **No minimum Postgres version is declared anywhere in the repository** — no
   compose file, no Dockerfile with a Postgres image. `NULLS NOT DISTINCT`
   (PG15+) cannot be assumed.

## Open for the Design Phase

- How to make the unique constraints hold for the NULL-tenant systemwide mode:
  `NULLS NOT DISTINCT` versus a sentinel value versus a partial index — and, as
  part of that, declaring an explicit minimum Postgres version (see verified
  constraints 6 and 7).
- The exact shape of the new `EventStore` contract that admits append + receipt
  in one transaction, given the trait is synchronous and currently owns its own
  transaction lifecycle end to end.
- Where the per-aggregate receipt index physically lives relative to the `events`
  table, and its migration ordering against the constraint fixes in D6.

## D8 — Full scope, one campaign (post-proposal)

Decided after reviewing `proposal.md`'s delivery forecast (3,800–5,050 authored
lines, 11–14 PRs across 11 slices).

**PROD-012 is not split.** Proposal slices 1–4 stay inside PROD-012 as internal
prerequisites, not as a separate change and not under a separate identifier:

1. Integration-test infrastructure.
2. `aggregate_type` as a real column, with a safe migration.
3. Effective event uniqueness including `tenant_id = NULL`, plus a declared
   minimum PostgreSQL version.
4. A common, testable `Clock`.

Then the idempotency implementation through to closing the live `UserEntity` bug.

**One spec, one design, one identifier.** No debt is moved outside the main
guarantee. PROD-013 is not created now — that identifier is reserved for the next
topic, once PROD-012 is finished.

**Requirement on the design phase:** the design must make two blocks clearly
visible *within the same campaign*, with explicit dependencies between them:

- **Block A — persistence foundations** (prerequisites 1–4 above)
- **Block B — end-to-end idempotency** (D1–D7 implemented, closing the live bug)

Separating the blocks is a structural requirement of the design document. It is
not permission to create a second spec, a second change folder, or a second
roadmap ID.

## D9 — Transport-agnostic by construction (supersedes the D1 handling note)

Raised by the user after the proposal: binding idempotency to HTTP would bake in
a coupling that ego-rs does not otherwise have. Correct — and the architecture
already supports the alternative.

**This supersedes finding 1 below, which framed enforcement as HTTP-scoped.**
Enforcement is not transport-scoped at all. Only extraction is.

### Verified support already in the codebase

- `ServiceContext` (`crates/service-sdk/src/context/mod.rs:51`) is already
  transport-neutral: tenant hint, correlation id, trace context, security
  context. It knows nothing about HTTP.
- `TenantResolver` (`crates/service-sdk/src/runtime/tenant.rs:151-155`) states the
  precedent explicitly in its own doc: "Transport-neutral inputs only: an
  already-produced `SecurityContext` and an optional caller-supplied tenant hint."
- `crates/transport` is axum-coupled in every module, by design. Its `lib.rs`
  declares itself "a minimal, generic axum HTTP layer — AD-2: mechanism only, no
  gRPC transport". Transport specificity is already quarantined there.
- `GrpcServerConfig` already exists (`crates/transport/src/config.rs`, CORE-016,
  default port 50051), so gRPC was anticipated at the configuration layer even
  though no gRPC server exists.

### The decision

Two layers, separated:

1. **Extraction — per transport.** Each adapter reads the key from its own
   carrier: HTTP from the `Idempotency-Key` header, gRPC from metadata, GraphQL
   from an extension, Kafka from a record header. Thin, and owned by the adapter.
2. **Enforcement — transport-agnostic by construction.** The key travels in
   `ServiceContext`. Everything D1–D7 defines — mandatory fail-closed behavior,
   reservation, lease, fencing, receipts — lives in service-sdk and below and
   reads only the neutral context. The core never sees a protocol.

**A shared extraction contract is required.** Validation of what constitutes a
valid key, and the policy for a missing key, must live in one place that every
adapter uses. Without it each adapter re-implements both and the guarantee
diverges per protocol.

**Explicit non-goal:** do NOT introduce a generic `Transport` trait or a
transport-abstraction layer for this. The seam already exists and is
`ServiceContext`. Adding an abstraction on top of it would be overengineering.

The service continues to own which transports it exposes, exactly as the
reference app owns its own router today. PROD-012's requirement is only that the
key arrives in `ServiceContext`, whatever carried it.

## D10 — Task-breakdown directives (post-design)

Given after `sdd-design` completed. The design is closed; these bind the
breakdown, and none of them reopen it.

- **One campaign.** PROD-012 stays a single change. Foundations and idempotency
  are internal blocks, not a second spec and not another identifier. No PROD-013.
- **The `aggregate_id` migration is reversible.** Rollback recomposes
  `aggregate_type || '-' || aggregate_id`, which is exact and lossless. The
  forward backfill is not derivable from data and must **abort on any
  ambiguity** rather than guess. Task text must state both directions.
- **The defensive `UserEntity` fix lands early**, not at the end of the chain. It
  must be presented as a defence-in-depth measure and explicitly **not** as a
  substitute for the runtime guarantee — the runtime enforcement is still
  required, and the task must say so.
- **The design's three open questions become explicit tasks**, not deferred
  prose:
  1. Lease renewal — cadence and owner.
  2. Readiness behaviour during migrations and while the reservation store is
     unavailable.
  3. Ownership and safe execution of the purge worker.
- **Small, reviewable slices**, with the dependency graph between them stated,
  and a line/PR forecast under `ask-on-risk`.

## D11 — Delivery: hybrid chain (post-tasks)

The `sdd-tasks` Review Workload Forecast reported ~3,800–5,050 authored lines,
High 400-line budget risk, chained PRs recommended, and decision needed before
apply. Under `ask-on-risk`, splitting was taken as settled — 15 units are already
cut — and the open question was how they chain.

**`chain_strategy: hybrid`** (a scoped combination of `stacked-to-main` for
Block A and `feature-branch-chain` for Block B):

1. **Foundations go to `develop` as a short incremental chain**: B0, then A1, A2,
   A3, A4. Each unit integrates into `develop` on its own. These are autonomous
   and independently valuable — today `events` has no uniqueness, the `23505`
   path is unreachable, and `EffectDedupStore` has no testable clock.
2. **Once that block is integrated, the idempotency tracker branch is created
   from the updated `develop`.**
3. **The remaining Block B units stack inside the tracker**: each PR targets the
   previous one, none merges to `develop` individually, and only the consolidated
   tracker merges to `develop`.

Rationale: useful foundations land early instead of being held hostage to the
idempotency work, while the long, tightly-coupled part stays contained and avoids
repeated exposure to the squash-merge-closes-stacked-child hazard.

**Restated because it is easy to lose:** B0 remains defence in depth, not the
closure of the guarantee. PROD-012's real payoff is the complete Block B.

Also applied: the residual stale label in the A2 unit row was corrected —
the focused test command reads `migration_007`, not `migration_002`.

## Verified Findings From the Proposal Review

Both confirmed against `develop` after `sdd-propose` returned.

1. **D1 names three transports; only one exists.** `crates/transport/src`
   contains only `config.rs`, `error.rs`, `lib.rs`, `propagation.rs`,
   `security.rs`, `server.rs`, `state.rs` — axum only. No Kafka dependency exists
   in any `Cargo.toml`. The workspace's only `tonic` is a transitive dependency of
   the OTLP exporter in `crates/infrastructure/Cargo.toml`, not a service
   transport. **Accepted handling:** enforcement is real for HTTP; for gRPC and
   Kafka, D1 stands as policy that binds those adapters when they are built. D1's
   intent — every external mutable command — is unchanged.
2. **`EventStore` has no canonical spec.** Zero occurrences of `EventStore`
   anywhere under `openspec/specs/`, and there is no persistence capability in
   that directory. The `2026-06-22-persistence-spi` change was archived but never
   merged into `specs/`. `sdd-spec` must therefore **author a canonical spec** for
   this capability, not write a delta against something that exists.
