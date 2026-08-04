# Exploration: PROD-012 — End-to-End Idempotent Command Processing

**Phase**: `sdd-explore`
**Change**: `prod-012-idempotent-command-processing`
**Baseline**: `develop` @ `3d7648e`
**Status**: complete — §10's open questions are all answered in `decisions.md` (D1–D7); ready for `sdd-propose`

## Intent

Durable command idempotency so that client retries, request timeouts, or process
restarts can never re-execute a business transaction more than once.

## Scope Note: PROD-012 Is a New Item

`ROADMAP.md` §7 stops at `PROD-011 — Performance and Capacity`. Nothing beyond
that exists in the repository, OpenSpec, or Engram. This is a brand-new roadmap
item, not the continuation of prior planning.

This change was originally raised as "PROD-015". Since no work exists to justify
reserving PROD-012, PROD-013, and PROD-014, it was renumbered to **PROD-012**,
the next free identifier. Decisions D1–D7 (see `decisions.md`) are unaffected by
the renumbering.

`ROADMAP.md` is also stale relative to `develop` (PROD-003 and PROD-005 shipped
after its only commit). Every claim below is grounded against source, not the
roadmap.

## 1. The Command Ingress Path

Traced end to end on `develop` using `POST /register` as the concrete example.

| Hop | Location | What happens |
| --- | --- | --- |
| HTTP route | `examples/reference-app/src/ports/http/router.rs::build_router` | Mounts `register_handler` on `axum::Router<AppState>` |
| Handler | `examples/reference-app/src/ports/http/handlers.rs::register_handler` | Extracts `AuthenticatedContext` (`crates/transport/src/security.rs`), `TraceContextExtractor` (`crates/transport/src/propagation.rs`), `Json<RegisterInput>`; builds `ServiceContext` with `.with_security(..).with_tenant_id(..).with_trace_context(..)` |
| Service resolution | `state.runtime.resolve::<RegisterUserTag>()` | CORE-025 typed proxy → `proxy.register(ctx, input)` |
| Application service | `examples/reference-app/src/application.rs::RegisterUserImpl::register` | Guarded by `#[authorize]` / `#[tenant_scoped]`; calls two independent `EntityRuntime`s sequentially (org-then-user — the CORE-018 AD-5 non-atomic dual write) |
| Entity dispatch | `crates/persistent-entity/src/entity_ref_tokio.rs::TokioEntityRef::send_command` | Wraps command + context in `ActorEnvelope` with a oneshot reply; sends on a `BoundedMailbox` |
| Activation | `EntityRegistry::lookup_or_insert` (`entity_ref_tokio.rs:137`) | Single-flight critical section — exactly one actor per `(tenant, entity_type, entity_id)` |
| Command execution | `crates/persistent-entity/src/actor.rs::EntityActor::execute_command` (lines 201–402) | `handle_command` → `persist_events` → `apply_events` → fire-and-forget `publisher.publish` → optional CORE-019 `EffectAcceptor` |
| Persistence | `crates/persistent-entity/src/persistence.rs::PersistenceFacade::persist_events` → `EventStore::append` | Real backend `crates/persistence/src/postgres/event_store.rs::PostgreSQLEventStore::append` (lines 64–134): one `sqlx` transaction wrapping `SELECT COALESCE(MAX(version),0)` OCC check + inserts + commit |

**No idempotency-key extractor exists anywhere in `crates/transport`.**

### Finding: `CommandContext.expected_version` Is Dead Code

`CommandContext` (`crates/persistent-entity/src/command_context.rs:25`) declares
`expected_version: Option<u64>`, but every construction site in production code
sets it to `None` (`command_context.rs:47`, `command_envelope.rs:59`,
`command_envelope.rs:78`, `persistent_entity.rs:149`). `execute_command`
(`actor.rs:227–234`) passes `self.version` — the actor's own in-memory tracked
version — to `persist_events`, never `context.expected_version`.

Optimistic concurrency is therefore entirely actor-internal and is never driven
by client input. `causation_id` and `metadata` on the same struct are likewise
declared but never populated in production.

Any design that assumes `expected_version` is already wired from the caller
builds on a false premise.

## 2. What Is Replay-Safe Today, and Why

**Single-writer-per-entity (CORE-006A) — shipped and real.**
`EntityRegistry::lookup_or_insert` guarantees exactly one actor per entity,
serializing all commands through one `execute_command` at a time. This stops
*concurrent* conflicting writers. It does **not** detect that two
sequentially-arriving commands are the same logical request — a retry is simply
the next command in the same serialized queue.

**Optimistic concurrency.** Collapses only the narrow case of two truly
concurrent writers racing the same stream version, which single-writer
serialization already makes rare. It does not stop a retry arriving after the
first attempt advanced `self.version`: the retry's implicit expected version
matches and is accepted as a fresh mutation.

**Domain-level accidental idempotency — per handler, not framework-wide.**
`examples/reference-app/src/domain/tenant_org.rs::TenantOrganizationEntity::handle_command`
returns `Ok(vec![])` when state is `TenantOrgState::Present`. That is a
deliberate hand-written "Ensure" check, not a guarantee the runtime provides.

**Live counter-example.**
`examples/reference-app/src/domain/user.rs::UserEntity::handle_command`
(lines 110–128) has no such check: `UserCommand::Register` validates only that
the email is non-empty and then unconditionally emits a new `UserRegistered`
event. A client retry of `POST /register` for the same `user_id` after a lost
response emits a **second** `UserRegistered` event and a second welcome-email
effect. This is a present-day duplicate-execution bug in the reference app, not
a hypothetical.

## 3. The Three Duplicate Sources

| Source | Where it leaks today |
| --- | --- |
| **Client retry** | At the domain handler, for any entity not hand-written to check current state. No transport, service-sdk, or persistent-entity layer carries a request-identity concept. |
| **Timeout, unknown outcome** | Same leak surface — a resubmission after timeout is indistinguishable from the original at the actor and event-store level. `CommandResult::EffectsAcceptanceFailed` (`actor.rs:38–45`) protects a caller from retrying an ambiguous *effects* outcome; there is no equivalent at the *command* level. |
| **Process restart mid-transaction** | Storage is safe: the Postgres `append` commits version-check and inserts in one SQL transaction, so there is no partial-append hazard. The hazard is upstream — if the actor crashes after commit but before replying, the oneshot reply is lost (`TeardownGuard::drop`, `entity_ref_tokio.rs:51–74`, answers queued callers `Err(EntityNotActive)`). The caller sees failure for a command that actually succeeded, and any retry re-enters the non-idempotent path above. |

These are three different failure geometries, not one problem in three costumes.

## 4. Where an Idempotency Key Would Have to Travel

| Layer | Current state | Change required |
| --- | --- | --- |
| `crates/transport` | No idempotency extractor | New `FromRequestParts` extractor, following the `security.rs` / `propagation.rs` pattern |
| `ego_service_sdk::context::ServiceContext` (`crates/service-sdk/src/context/mod.rs`) | No idempotency field | New private field + builder/accessor, mirroring the CORE-008A `tenant_hint` / `canonical_tenant` split |
| `persistent_entity::CommandContext` | `causation_id` / `metadata` declared but never populated | Natural carrier into the actor; the service layer currently forwards nothing from `ServiceContext` into it |
| `CommandEnvelope<C>` / `ActorEnvelope<C>` | Context already rides to the actor | No new envelope type needed if the key lives in `CommandContext` |

`ego_domain::idempotency::IdempotencyKey` (`crates/domain/src/idempotency.rs`)
**already exists** as a validated non-empty-string newtype — but its own doc
comment scopes it to post-commit external-effect dispatch (`f(uow_id,
effect_index)`), not command ingress. Reusing the type may be reasonable;
reusing its derivation semantics is not.

## 5. Durable Storage Actually Available on `develop`

**Shipped.** `crates/persistence/src/postgres/` is real: `PostgreSQLEventStore`,
`PostgreSQLRepository`, `PostgreSQLSnapshotStore`, real `sqlx`-backed
migrations. `append` wraps version-check and inserts in one `tx.begin()` /
`commit()` — the one place a same-transaction dedupe write could physically
attach today.

**Not present.** No key-value store and no Stoolap anywhere in the workspace —
zero hits repo-wide. This candidate does not exist in any form.

**Closest precedent, in-memory only.** `crates/runtime/src/effects/store.rs`
defines `EffectStateStore` / `EffectDedupStore` with `DedupScope { tenant,
effect_type, key: IdempotencyKey }`, a `reserve()` returning
`DedupOutcome::{Fresh, OwnedInProgress, OwnedSucceeded, OtherInProgress,
OtherSucceeded, Conflict}`, and SHA-256 fingerprint conflict detection. The only
implementor is `InMemoryEffectStore` — confirming PROD-002 never shipped a
durable version. The *pattern* is proven; the *durability* is not built.

**Different concern.** The read side has its own `DedupStore`
(`crates/domain/src/read_side/dedup.rs`, scoped `(projection_id, tag,
event_id)`) plus a real `processed_events` Postgres table (migration
`005_create_processed_events.sql`). That is idempotent projection *consumption*,
not command execution.

## 6. Tenant Scoping

CORE-008A tenant enforcement is **shipped in code** — only the OpenSpec change
record is archived. `ego_service_sdk::runtime::tenant::{CanonicalTenant,
TenantResolver, TenantEnforcementMode}` plus
`ServiceContext::canonical_tenant()` / `tenant_hint()` are active.
`TenantResolver::resolve` is the sole seam producing an authoritative tenant and
never trusts a raw client string.
`crates/persistence/src/postgres/mod.rs::resolve_tenant` fails closed on
`Some("")` rather than silently writing NULL.

Implication: any idempotency key must be namespaced by the same
`CanonicalTenant`, mirroring `DedupScope`'s `(tenant, effect_type, key)` triple
— never the raw client-supplied hint. Note that `InMemoryEventStore`
(`persistence.rs`) does not scope streams by tenant, by its own admission; do
not copy that shortcut.

## 7. Multi-Node and Crash-Recovery Interactions

| Item | Roadmap claim | Actual state on `develop` |
| --- | --- | --- |
| PROD-008 Crash/Recovery Suite | §7.8 | **Not started.** No `crates/integration-tests/` directory exists. Actor-level crash-adjacent unit tests exist (`panic_mid_processing_answers_all_already_enqueued_callers`, `TeardownGuard`) but no cross-cutting suite. |
| PROD-009 Multi-Node Runtime Contract | §7.9 | **Not started.** No node identity, membership, distributed activation authority, or lease/fencing code anywhere. CORE-006A's single-writer guarantee is single-process only (`Arc<EntityRegistry>`). |
| CORE-030 Transactional Outbox | §5.1 | **Not started.** No outbox model, repository, or publisher. `EntityActor`'s event publish is fire-and-forget with no atomic-with-commit guarantee. |

The "reuse the outbox" option has nothing to reuse yet, and any design leaning
on single-process `EntityRegistry` uniqueness as an implicit safety net will not
hold once multi-node lands.

## 8. Candidate Approaches, Ranked by New Infrastructure Demanded

### (a) Dedupe record in the same transaction as the event append — least new infrastructure

The Postgres `append` transaction already exists to attach to.

Breaks because: `EventStore::append`'s trait signature has no idempotency-key
parameter, so every implementor changes; a bare dedupe row does not capture a
prior result to replay; and commands producing **zero events** (such as `Ensure`
on an already-`Present` org) never reach `append` at all, so no-op paths cannot
be deduped this way.

### (c) Reuse or extend `EffectDedupStore` — medium

Real design precedent: `DedupOutcome`'s five-way shape is the right model.

Breaks because: there are zero durable implementations today, so "reusing" it
means building the first durable implementation — which is really (b) wearing a
familiar interface. It also lives architecturally downstream (post-commit);
moving it upstream is a genuine boundary question, not a refactor.

### (b) Separate pre-dispatch idempotency store — most new infrastructure

The only approach that can dedupe **before** `handle_command` runs, covering
both the zero-event gap in (a) and commands rejected by a guard before ever
reaching persistence.

No winner is selected here — that is the proposal phase's job. But note that (a)
and (c) both fail to cover cases (b) covers, so the ranking by cost is the
inverse of the ranking by coverage.

## 9. The Hard Question: What Does "The Same Command" Mean?

| Definition | Strength | Weakness |
| --- | --- | --- |
| Client-supplied key only | Simplest; mirrors `EffectDedupStore::reserve` | Needs a policy for same-key-different-payload (the effect layer already had to solve this via `DedupOutcome::Conflict` / `TerminalReason::InvalidEffect`) |
| Content hash only | No client cooperation required | Cannot distinguish two genuinely separate identical-payload requests from one retry — directly conflicts with a legitimate re-registration having byte-identical fields |
| Both | Matches the proven `EffectDedupStore` pattern; least new design risk | Still needs new storage; and is the client key mandatory (fail-closed) or optional (fallback to today's behavior)? |

## 10. Open Questions for the Human, Before a Proposal

> **All resolved.** These questions were answered by the user in the interactive
> proposal question round on 2026-08-02. The binding answers, including several
> refinements that go beyond what was asked, are recorded in `decisions.md`
> (D1–D7). That file — not this section — is the input to `sdd-propose`. The
> list below is kept as the record of what was open at exploration time.

1. **Mandatory or opt-in idempotency-key header?** Mandatory changes the public
   HTTP contract for every existing route; opt-in leaves `UserEntity`'s bug
   unresolved for non-adopters.
2. **Retention and expiry policy** for stored idempotency records. No comparable
   TTL pattern exists anywhere in the codebase today.
3. **Where does the reservation happen** relative to `#[tenant_scoped]` /
   `#[authorize]`, and relative to `RegisterUserImpl`'s two-entity dual write?
   One key spanning two `EntityRuntime`s is a substantially harder problem than
   one key per single-aggregate command.
4. **Sequencing against CORE-030 and PROD-009**, both fully unstarted, either of
   which could change the safety-net assumptions this design rests on.
5. **Relationship to the existing `ego_domain::idempotency::IdempotencyKey`** —
   new type, or generalization of the existing one?
6. **The ROADMAP numbering gap** (PROD-012/013/014 missing) — confirm or
   renumber; do not assume.

## Affected Areas If Implemented

- `crates/transport/src/security.rs`, `propagation.rs` — extractor pattern to follow
- `crates/service-sdk/src/context/mod.rs` — new `ServiceContext` field and accessor
- `crates/persistent-entity/src/command_context.rs`, `command_envelope.rs` — dead `causation_id` / `metadata` fields as the natural carrier
- `crates/persistent-entity/src/actor.rs::execute_command` (lines 201–402, especially 213–216) — the seam for a pre-dispatch dedupe check
- `crates/domain/src/persistence/event_store.rs` (`EventStore::append`) and both implementors (`persistence.rs::InMemoryEventStore`, `crates/persistence/src/postgres/event_store.rs`) — trait-signature impact if approach (a) is chosen
- `crates/runtime/src/effects/store.rs` — closest design precedent, no durable implementation to reuse
- `examples/reference-app/src/domain/user.rs::UserEntity::handle_command` — live, reachable non-idempotent bug
- `crates/integration-tests/` — does not exist yet; required by the project testing skill for crash and duplicate-delivery tests

## Risks Carried Into Proposal

1. `CommandContext.expected_version` looks load-bearing but is dead code.
2. `UserEntity::handle_command` is a live, reachable duplicate-execution bug in the reference app itself.
3. No durable implementation of the closest analog (`EffectDedupStore`) exists — "reuse existing infrastructure" is not actually available.
4. PROD-009 (multi-node) is unstarted; a design leaning on single-process `EntityRegistry` uniqueness will not survive it.
5. `crates/integration-tests/` does not exist and must be created before crash and duplicate-delivery tests can be written.
