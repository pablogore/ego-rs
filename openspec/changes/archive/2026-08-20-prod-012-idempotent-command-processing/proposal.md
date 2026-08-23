# Proposal: PROD-012 — End-to-End Idempotent Command Processing

## Metadata

| Field | Value |
|-------|-------|
| Change ID | PROD-012 |
| Title | End-to-End Idempotent Command Processing |
| Type | Production hardening (write-side command identity and recovery) |
| Date | 2026-08-02 |
| Baseline | `develop` @ `3d7648e` |
| Inputs | `explore.md` (exploration), `decisions.md` (binding D1–D7) |
| Related | CORE-019 (effect dedup precedent), CORE-008A (tenant authority), CORE-017 (lifecycle), PROD-003 (tracing) |
| Sequenced before | PROD-009 (multi-node), CORE-030 (outbox), PROD-008 (crash suite) |
| Status | CLOSED |

> **D1–D7 in `decisions.md` are settled.** This proposal records them as the
> chosen approach and reasons forward.
>
> **Status, as of 2026-08-19.** The change is delivered: all 115 active tasks are
> complete and the two withdrawn ones (B4.7a, E1.2) stay withdrawn. `CLOSED` above
> marks delivery, in the same sense CORE-021's archive report uses it; the change has
> not been physically archived yet.
>
> The three items §12 left open for design were all resolved there — §12.1 by AD-1,
> §12.2 by AD-2, §12.3 by AD-3 — and the three questions `design.md` in turn left open
> are answered in its own Open Questions section. Nothing in this change is still open
> for decision. What remains open is recorded as a *gap*, not as a question: the
> duplicate/coalesced-key arrival described under B1.13, and the conformance harnesses
> not being driven against PostgreSQL.
>
> Everything below this line is the proposal as reasoned at the time. Where the
> implementation later contradicted a factual statement, the statement is corrected in
> place and says so; decisions and their rationale are preserved unchanged, including
> the ones that were superseded.

---

## 1. The Problem, Concretely

`examples/reference-app/src/domain/user.rs::UserEntity::handle_command`
(lines 110–128) validates that the email is non-empty and then unconditionally
emits `UserRegistered`. A client that retries `POST /register` after a lost
response — the ordinary case, not an exotic one — gets a **second**
`UserRegistered` event and a **second** welcome-email effect. This is a live,
reachable duplicate-execution bug on `develop`, in the app the framework holds
up as its reference.

The framework offers no defense to fall back on:

| Existing mechanism | What it stops | Why it does not stop this |
|---|---|---|
| Single-writer per entity (CORE-006A, `entity_ref_tokio.rs:137`) | Two concurrent writers | A retry is just the next command in the same serialized queue |
| Optimistic concurrency (`actor.rs:227–234`) | A version race | The retry arrives *after* `self.version` advanced, so its implicit expectation matches |
| Hand-written state checks (`tenant_org.rs::TenantOrganizationEntity`) | Duplicates in **that one handler** | It is discipline, per handler. `UserEntity` is the proof that discipline fails |

The stakes are not framed as "a duplicate row". They are: **`ego-rs` cannot
today make an end-to-end statement about what happens when a client retries.**
Every application built on it inherits that gap and must re-solve it by hand,
per aggregate, forever — and the reference app already got it wrong.

Three distinct failure geometries produce it (`explore.md` §3): client retry,
timeout with unknown outcome, and process restart mid-transaction. They are not
one problem in three costumes, and only the third needs leases.

---

## 2. The Primary Architectural Decision

Stated once, plainly:

> **A mandatory, client-supplied `OperationKey` identifies one complete business
> operation. Before the first aggregate is touched, the operation is durably
> reserved under a lease with an owner and a fencing token. Each aggregate the
> operation reaches records a permanent receipt, confirmed in the same
> transaction as its event append. Recovery is re-execution: receipts make the
> second pass a no-op wherever the first pass already landed.**

Everything below follows from that sentence. Its four load-bearing parts and the
decision that fixed each:

| Part | Decision | Consequence |
|---|---|---|
| Mandatory key at every mutable ingress | D1 | The guarantee is verifiable, not best-effort. No server-side key generation — a server-minted key is a function of the request as received, so on a retry it deduplicates nothing |
| One key per **operation**, not per aggregate command | D2 | Covers `RegisterUserImpl`'s org-then-user dual write as one unit; forces a **pre-dispatch** store, since a command may emit zero events or be rejected by a guard before persisting |
| Lease with owner, expiry, and fencing | D3, D6 | Survives process death without manual intervention. A stale owner gets `StaleOwner` and cannot close the operation |
| Per-aggregate receipts, atomic with the append | D4 | "Last key applied" loses evidence when other operations interleave. The snapshot is never the source of truth |

**Why this and not the cheaper options.** The exploration ranked candidates by
new infrastructure (`explore.md` §8), and the ranking by cost is the *inverse*
of the ranking by coverage. Same-transaction dedupe (a) cannot see commands that
emit zero events or die at a guard. Reusing `EffectDedupStore` (c) means writing
its first durable implementation anyway — it is (b) wearing a familiar interface,
and it sits architecturally downstream (post-commit) from where the check must
happen. Only the pre-dispatch store covers the real failure set.

### 2.1 Two guarantees, named separately (D5)

They must never be blurred into one promise:

| Guarantee | Bounded by | Ends when |
|---|---|---|
| **Replay window** — the exact prior response is returned | Reservation TTL, counted from `completed_at` | The reservation is purged |
| **Domain duplication protection** — no aggregate re-mutates | Life of the stream (receipts are permanent) | Only on explicit, definitive deletion of the aggregate or tenant |

After the TTL there is no response replay and no boundary-level detection of a
reused key. Receipts still prevent re-mutation of any aggregate the operation
already reached. For operations **rejected before touching any aggregate**, or
**successful without reaching one**, protection ends with the TTL. That sentence
goes into the public spec verbatim.

### 2.2 Where the reservation happens

Pinned by D2: **after** `#[authorize]` and `#[tenant_scoped]`, **before** the
first `EntityRuntime` call. The key is namespaced by the `CanonicalTenant`
produced by `TenantResolver::resolve`, never by the raw client tenant hint —
exactly what CORE-008A forbids.

The *position* is settled. The *mechanism* is a design detail: the candidate
seams are the interceptor chain (`crates/service-sdk/src/interceptor/`) and an
attribute macro alongside the existing guards
(`crates/service-sdk-macros`). Named here so design does not rediscover them;
not ranked here, because D2 already constrains the outcome.

### 2.3 What "the same command" means

Client key **and** content fingerprint, matching the proven `EffectDedupStore`
shape (`explore.md` §9, `crates/runtime/src/effects/store.rs`):

- same key + same fingerprint → already applied, no-op or replay
- same key + different fingerprint → **permanent conflict**, never a silent
  dedupe and never a silently reopened business transaction

Receipts store the fingerprint too (D5). That is what lets stored responses age
out under TTL without weakening the permanent guarantee.

---

## 3. Capability vs Registry

The pattern has both, and conflating them is how the boundary gets lost. CORE-019
already made this split in shipped code: `ExternalEffectExecutor`
(`crates/runtime/src/effects/executor.rs:40`) is the **capability**, and
`ExecutorRegistry` (`crates/runtime/src/effects/registry.rs:28`) is the
**resolution mechanism** — keyed by `effect_type`, fail-closed on duplicate
registration.

PROD-012 needs the same discipline and reaches a *different* answer, which is
the point of asking the question:

| Concern | Capability (port) | Discovery / registration / resolution |
|---|---|---|
| Operation reservation + lease | `OperationReservationStore` — reserve, renew, complete, abandon, take over | **Single registration** on `RuntimeBuilder`. No key, no registry |
| Per-aggregate receipt | **Not an independently registered port.** D4 requires the receipt to be confirmed in the same transaction as the append, so it is satisfied by the same persistence backend as `EventStore` | Supplied by the persistence backend; a separately-registered receipt store could not join that transaction |
| Time | `Clock` — generalized from `crates/domain/src/auth/clock.rs:20` | Single registration, system-clock default, injected into both the new store and `EffectDedupStore` |
| Enforcement policy | `IdempotencyEnforcementMode` — a fixed-invariant enum, **not** `dyn`-dispatched | Runtime configuration, mirroring `TenantEnforcementMode` (`crates/service-sdk/src/runtime/tenant.rs:143`) |

**Why no keyed registry.** `effect_type` varies per effect, so CORE-019 needed
one owner *per type*. Operation reservation has no equivalent axis: there is
exactly one reservation authority per runtime. A keyed registry here would be an
SPI with one sensible entry — it does not earn its keep. This is a deliberate
"no", derived from the same criterion CORE-019 used to keep `EffectQueue`
internal, not an omission.

**Fail-closed registration.** If `IdempotencyEnforcementMode` resolves to the
enforcing variant and no `OperationReservationStore` is registered, **startup
fails**. Consistent with CORE-019 §10 and with `resolve_tenant`'s fail-closed
posture (`crates/persistence/src/postgres/mod.rs`).

**The receipt/EventStore coupling is a real constraint, not tidiness.** Two ports
that must share one transaction cannot be independently swapped. CORE-019 hit
the same wall and shipped `InMemoryEffectStore` as a composite. Design must state
the coupling explicitly rather than pretend the receipt store is pluggable.

---

## 4. Naming

### 4.1 `OperationKey` is a new, distinct type (D7)

```
OperationKey            → external business intent
                          ingress → operation → internal commands → receipts

IdempotencyKey          → post-commit effect deduplication
                          f(uow_id, effect_index)
```

| Rule | Detail |
|---|---|
| Home | `crates/domain/src/`, beside the existing `idempotency.rs` — not under HTTP, not under runtime, because it crosses every layer |
| `IdempotencyKey` | Untouched, including its current documentation |
| Sharing | The validation function may be shared internally. The public newtype may not |
| Conversion | **No** `From<OperationKey>`. Any future bridge must be deliberately named, e.g. `EffectIdempotencyKey::from_operation_effect(&operation_key, effect_index)` — out of scope here |

Both currently validate a non-empty string. That is a coincidence of validation,
not a shared identity. The compiler must prevent a key derived for an email or
webhook from ever identifying an external operation.

### 4.2 Specific vs general, stated in the right direction

`IdempotencyKey` is **a particular case** of the general operation-identity
pattern: it identifies one post-commit effect within one unit of work.
`OperationKey` names the general case: the identity of an external business
intent as it crosses ingress, reservation, dispatch, receipt, and recovery.

The general pattern is arriving *second*, after the narrow case shipped. That
does not make the narrow case the parent. Nothing in this change makes
`IdempotencyKey` a subtype, a wrapper, or a conversion target of `OperationKey`
— D7 forbids exactly that erasure. They are siblings under a shared *concept*,
distinct in code.

### 4.3 Other names to settle in spec/design

| Concept | Proposed | Rejected / rationale |
|---|---|---|
| Reservation outcome enum | `ReservationOutcome` | Reuses the proven six-way shape of `DedupOutcome` (`crates/runtime/src/effects/store.rs`) without reusing the type across a layer boundary |
| Stale-fence rejection | `StaleOwner` (D6) | Named in the decisions; keep verbatim |
| Durable record of the operation | `OperationReservation` | `IdempotencyRecord` — inherits the effect-layer vocabulary this change works to separate |
| Per-aggregate applied marker | `OperationReceipt` | `AppliedKey` — loses the fingerprint half of the record (D5) |

---

## 5. Runtime Lifecycle

Flagged at proposal depth; decided in design.

| Stage | Question this change must answer |
|---|---|
| **Startup** | Store construction and registration on `RuntimeBuilder`; fail-closed when enforcement is on and no store is registered; migration ordering relative to the D6 constraint fixes |
| **Readiness** | Does an uninitialized reservation store gate readiness, or only startup? |
| **Reservation** | `Fresh` → execute; `OwnedInProgress` / `OtherInProgress` → lease policy; `Succeeded` → replay; `Conflict` → permanent, terminal |
| **Renewal** | Who renews a long-running operation, on what cadence, and against which `Clock`? Every renewal is a conditional update on `operation_id + owner_id + fencing_token` (D6) |
| **Takeover** | A later retry claims an expired lease atomically and re-executes. The prior owner is fenced out and receives `StaleOwner` |
| **Shutdown** | D3 forbids release-on-failure. A graceful shutdown that simply drops in-flight reservations makes every clean restart wait out a lease. Whether graceful shutdown may *abandon with fencing* is open |
| **Purge worker** | Batched, observable, safe under concurrent workers (D5). Runtime-owned background task under CORE-017 ordering, or operator-scheduled? Open. `InProgress` reservations are **never** TTL-purged — they must first be recovered via lease expiry |

No `Utc::now()` anywhere in lease logic. Expiry, renewal, and takeover must be
deterministically testable under Strict TDD (D3). Note that
`crates/runtime/src/effects/store.rs:58` calls `Utc::now()` directly today — a
defect this change fixes, not a precedent to copy.

---

## 6. Observability

Flagged at proposal depth; decided in design. Aligned with CORE-012A
infrastructure and PROD-003 (`distributed-tracing`).

| Domain | Signals |
|---|---|
| Ingress | key missing (rejected), key malformed, key accepted |
| Reservation | reserved fresh, replayed from stored response, fingerprint conflict, in-progress contention |
| Lease | acquired, renewed, expired, taken over, `StaleOwner` rejection |
| Receipt | confirmed, already-applied no-op, receipt conflict |
| Retention | batch size, rows purged, purge lag, age of the oldest surviving `completed_at` |

Correlation fields: canonical tenant, aggregate type + id, operation outcome,
trace context. **Redaction:** `operation_key` is client-supplied and may carry
business identifiers — hash or redact by default, following CORE-019 §12's
treatment of idempotency keys. Stored responses are never logged.

Open: whether reservation opens its own span or annotates the request span, and
whether these are `Observability` counters or `Tracer` events.

---

## 7. Security

| Rule | Why it binds here |
|---|---|
| Every reservation and receipt query binds `tenant_id` as `$N`; no interpolation, ever | `skills/security` Rule 1 + Rule 2. New tables mean new queries |
| The uniqueness key is namespaced by `CanonicalTenant`, never `tenant_hint` (D2) | CORE-008A. `TenantResolver::resolve` is the only authoritative source |
| A key presented by tenant A must never replay a response stored for tenant B | Stored-response replay is an **information-disclosure vector**, not merely a correctness bug. Tenant is part of the identity by construction |
| `operation_key` is untrusted boundary input: validated newtype, length-bounded, never concatenated into SQL, redacted in logs | It arrives from a client header |

---

## 8. Scope

### In Scope

**Foundation fixes (D6 — unavoidable, in scope):**

1. Unique constraint on `events (tenant_id, aggregate_type, aggregate_id, version)`, adjusted to the real tenant model.
2. Real unique constraint on the receipts table.
3. A new `EventStore` contract that confirms append + receipt in one transaction.
4. A transactional path for successful commands that produce zero events.

**Core capability:**

- `OperationKey` newtype in `crates/domain`; `IdempotencyEnforcementMode`.
- Mandatory `Idempotency-Key` extraction at HTTP ingress; carriage through `ServiceContext` → `CommandContext` → the actor.
- `OperationReservationStore` port + in-memory implementation + Postgres implementation.
- Leases with `owner_id` / `lease_until` / `fencing_token`; atomic takeover; conditional close.
- Per-aggregate receipts keyed by `(tenant_id, aggregate_type, aggregate_id, operation_key)`, storing the fingerprint, confirmed atomically with the append; permanent retention.
- Stored operation responses with replay; split retention and a batched, concurrent-safe purge job.
- `Clock` generalized out of `auth` and injected into both the new store and `EffectDedupStore`.
- Real event-metadata channel — there is none today (constraint 3), so `operation_key` needs the first one built.
- `crates/integration-tests/` created (testcontainers only), with crash and duplicate-delivery coverage.
- `UserEntity`'s duplicate-registration bug closed end to end.

**Bounded migration path.** D1 permits a temporary compatibility mode. It is an
enum variant with a fail-closed default, following `TenantEnforcementMode` — not
free per-endpoint configuration and not a permanent opt-out.

### Out of Scope

- **Atomicity between `RegisterUserImpl`'s two aggregates.** See §9 — this is a stated non-promise, not an oversight.
- gRPC and Kafka key enforcement. D1's policy binds them, but neither transport exists in the workspace today: `crates/transport` is axum-only (`security.rs`, `propagation.rs`, `server.rs`), and the root `Cargo.toml` has no `tonic` and no Kafka dependency. The contract is built transport-agnostic so those adapters inherit it; enforcing it in adapters that do not exist is not deliverable work.
- Multi-node activation authority, membership, distributed contention tests (PROD-009).
- Transactional outbox and atomic effect publication (CORE-030). It consumes `operation_key` / `causation_id`; it does not define command deduplication.
- Saga orchestration and step checkpointing (CORE-029) — rejected in D4 precisely because it collides with unstarted work.
- Waking `CommandContext.expected_version`, which is dead code today (`explore.md` §1). PROD-012 neither uses nor revives it.
- Read-side projection dedup (`crates/domain/src/read_side/dedup.rs`) — a different concern with its own shipped table.
- Any `From<OperationKey>` bridge to `IdempotencyKey` (D7).

---

## 9. The Explicit Non-Promise

PROD-012 does **not** promise atomicity between the two aggregates of
`RegisterUserImpl`. It promises **safe recovery by re-execution**.

Worked example — `RegisterUser(K)`, lease expires after the first command:

| Aggregate | Receipt for K before recovery | Recovery behaviour |
|---|---|---|
| `TenantOrganization` / `org-1` | applied | no-op |
| `User` / `user-7` | absent | executes |

The operation completes. No `UserRegistered` is duplicated. There is still a
window in which only the organization exists — closing *that* is CORE-018 AD-5
territory, not this change. Saying so plainly is the point.

---

## 10. Capabilities

> Contract with `sdd-spec`.

### New Capabilities

- `idempotent-command-processing`: `OperationKey` contract, mandatory-key
  enforcement mode and its bounded migration variant, pre-dispatch reservation
  lifecycle (reserve / renew / complete / abandon / take over), lease with owner
  + expiry + fencing token, `StaleOwner` semantics, per-aggregate receipts with
  fingerprint, replay-window vs domain-duplication-protection guarantees,
  split retention and purge.
- `event-store`: **no canonical spec exists today** — `EventStore` has zero
  occurrences across `openspec/specs/` (the `2026-06-22-persistence-spi` change
  is archived but was never merged into `specs/`). PROD-012 changes this
  contract, so it must first be written down. Alternative name for `sdd-spec` to
  consider: `persistence-spi`.

### Modified Capabilities

- `persistent-entity`: the actor consults receipts before dispatch and no-ops on
  already-applied; the zero-event branch (`actor.rs:219`) opens a transaction to
  write a receipt; `CommandContext` carries `operation_key`; `EntityTriple`
  stops concatenating type into id.
- `service-sdk`: `ServiceContext` gains an operation-key accessor; `RuntimeBuilder`
  registers the reservation store, the `Clock`, the enforcement mode, and the
  retention policy; purge-worker lifecycle under CORE-017 ordering.
- `http-transport`: mandatory `Idempotency-Key` extractor, rejection contract for
  a missing or invalid key, replay-response semantics.
- `reference-service`: dual-aggregate recovery for `RegisterUserImpl`;
  `UserEntity` duplicate registration closed.

### Implementation obligations, no spec delta expected

- `foundation-integrity`: `crates/integration-tests/` needs a layer-map entry
  (FR-001 requires every workspace crate be mapped). The requirement does not
  change; the data does.
- `external-effects`: **no change.** An earlier draft expected
  `EffectDedupStore` to gain an injected `Clock`; inspecting the code showed its
  methods neither take nor read time, so there is nothing to constrain. The
  effects subsystem's real wall-clock reads live in `EffectRunner` and are
  tracked as a separate follow-up, outside this change.
- `testkit`: a reservation-store double is likely needed. Confirm during spec.

---

## 11. Blast Radius

Honest, including the parts that hurt.

| Area | Impact | What changes |
|---|---|---|
| `crates/domain/src/` (new module) | New | `OperationKey`; `Clock` re-homed out of `auth/clock.rs:20` |
| `crates/domain/src/persistence/event_store.rs` | **Breaking** | New contract admitting append + receipt in one transaction (constraint 2) |
| `crates/domain/src/persistence/stored_event.rs:6` | Modified | First real metadata channel — today only `correlation_id` is declared, and it is never persisted (constraint 3) |
| `crates/persistence/src/postgres/event_store.rs:76–129` | **Breaking** | Transaction is opened *and* committed inside `append`, inside `block_on`, on a synchronous trait. No caller can join it |
| `crates/persistence` migrations | New + altering | `aggregate_type` as a real column; unique constraint on `events`; receipts table; reservations table. `001_create_events.sql` has no unique constraint today, which makes the existing `23505` handling in `append` unreachable (constraint 4) |
| `crates/persistent-entity/src/actor.rs:219, 230` | Modified | Zero-event branch never opens a transaction today (constraint 1); receipt gating before dispatch |
| `crates/persistent-entity/src/scheduler.rs:30` | **Breaking + data** | `EntityTriple::aggregate_id()` returns `format!("{}-{}", entity_type, entity_id)`. Hyphen-joining is ambiguous — type `user-account` + id `7` collides with type `user` + id `account-7` (constraint 5) |
| `crates/persistent-entity/src/command_context.rs` | Modified | Carries `operation_key` |
| `crates/service-sdk/src/context/mod.rs`, `runtime/builder.rs` | Modified | Context field; store/clock/policy registration; purge lifecycle |
| `crates/transport` | New | First idempotency extractor, following `security.rs` / `propagation.rs` |
| `crates/runtime/src/effects/store.rs:58` | Modified | Replace the direct `Utc::now()` with the injected `Clock` |
| `examples/reference-app` | Modified | Handlers supply the key; `UserEntity` bug closed; dual-aggregate recovery test |
| `crates/integration-tests/` | **New crate** | Does not exist. Required by `skills/testing` for anything touching a real Postgres; activates the skill's `PENDING [INFRA-CRATE]` block |

Two impacts deserve naming rather than burying in a table:

1. **`InMemoryEventStore` does not scope streams by tenant**, by its own
   admission (`crates/persistent-entity/src/persistence.rs`). Whatever the
   Postgres store does for tenant-scoped uniqueness, the in-memory store must
   not silently diverge — or the tests that run against it prove nothing.
2. **The `aggregate_id` split is the only genuinely irreversible step.** Every
   other slice reverts as code. That one rewrites persisted identifiers.

---

## 12. Open for the Design Phase

Exactly three items, per `decisions.md`. Named alternatives with tradeoffs, no
pick — that is design's job.

### 12.1 Uniqueness under the NULL-tenant systemwide mode

`resolve_tenant(None) → Ok(None)` is the **spec-blessed** tenant-less mode from
CORE-008A D1, with its own test (`crates/persistence/src/postgres/mod.rs`)
(constraint 6). In Postgres a plain `UNIQUE` treats NULLs as distinct, so D6's
constraint would enforce **nothing** for systemwide aggregates. Compounding it,
**no minimum Postgres version is declared anywhere in the repository** — no
compose file, no Dockerfile with a Postgres image (constraint 7).

| Option | Upside | Downside |
|---|---|---|
| `NULLS NOT DISTINCT` | Smallest schema change; expresses the intent directly | PG15+ only, and nothing in the repo currently declares a floor. Requires making that floor explicit and enforcing it |
| Sentinel tenant value | Works on any Postgres version; one uniform index | Introduces a magic value into a domain that deliberately models absence as `None`. Every read path must translate it, and a missed translation is a silent cross-tenant bug |
| Two partial unique indexes (`WHERE tenant_id IS NOT NULL` / `IS NULL`) | No version floor, no sentinel, NULL stays NULL | Two indexes to keep in sync; more schema surface; the intent is less obvious to a future reader |

Declaring the minimum Postgres version is part of this decision either way.

### 12.2 The shape of the new `EventStore` contract

The trait is synchronous and owns its transaction lifecycle end to end
(constraint 2), so admitting a co-transactional receipt is a contract change, not
a parameter addition.

| Option | Upside | Downside |
|---|---|---|
| Caller-owned transaction handle (`append_in_tx(&mut Tx, …)`) | Maximum composability; any future co-transactional concern joins for free | Leaks `sqlx` into a domain port — a hexagonal-boundary violation the workspace does not otherwise tolerate |
| Unit-of-work closure (`append_with(|uow| …)`) | Transaction stays behind the port; backend-agnostic; one seam for future concerns | New abstraction to design and document; the closure's error/lifetime shape against a synchronous trait inside `block_on` is the hard part |
| Widen `append` with a receipt parameter | Smallest diff; both implementors change once | Hardcodes exactly one co-transactional concern forever. CORE-030's outbox would need the same surgery again |

Whether the trait also becomes async is entangled with this and should be decided
here, not separately.

### 12.3 Physical home of the receipt index, and migration ordering

| Option | Upside | Downside |
|---|---|---|
| Dedicated `operation_receipts` table | Clean lifecycle (D5 permanence, deletion only with the aggregate/tenant); indexable exactly for the lookup | A second write per append; the unique constraint must hold under the same NULL-tenant question as 12.1 |
| Columns on `events` + a partial unique index | One write, atomic by construction | Only works for event-producing commands. D4 requires a receipt for zero-event successes too, so this cannot be the whole answer |
| Metadata on `events` + a derived index table | Events stay the single source of truth; the index is rebuildable | Two structures to keep consistent; rebuild cost grows with the stream |

Ordering matters: the receipts constraint and the `events` constraint fixes touch
overlapping identity columns, and the `aggregate_type` split (constraint 5) must
land before any constraint that names that column.

---

## 13. Delivery Shape

**This does not fit in one PR, and pretending otherwise would be dishonest.**
Review budget is 400 authored changed lines; strategy is `ask-on-risk`.

Feature-branch chain: PR #1 targets `feat/prod-012-idempotent-command-processing`;
each later PR targets the immediately previous slice branch.

| # | Slice | Est. lines | Why here |
|---|---|---|---|
| 1 | `crates/integration-tests/` crate + testcontainers + characterization test of today's `append` + layer-map entry | 350–500 | Strict TDD needs a real-Postgres home **before** the schema work. Nothing else can be tested honestly first |
| 2 | `aggregate_type` as a real column + `EntityTriple` split (constraint 5) | 350–450 | Every later constraint names this column |
| 3 | Unique constraint on `events` + the 12.1 NULL-tenant resolution + declared minimum Postgres version | 300–400 | Makes the unreachable `23505` path real (constraint 4) |
| 4 | `Clock` generalized out of `auth`; injected into `EffectDedupStore`, replacing `store.rs:58`'s `Utc::now()` | 150–250 | Small, independent, and a prerequisite for testable leases |
| 5 | `OperationKey` + `IdempotencyEnforcementMode` + HTTP extractor + `ServiceContext`/`CommandContext` carriage (key travels and validates; no storage yet) | 350–450 | Contract visible end to end before any durability |
| 6 | `OperationReservationStore` port + `ReservationOutcome` + lease/fencing semantics + in-memory implementation | 400–550 | Likely needs splitting |
| 7 | Postgres reservation store: atomic takeover, `StaleOwner`, conditional close | 400–500 | The first slice that genuinely needs slice 1 |
| 8 | Event metadata channel + new `EventStore` contract (12.2) + both implementors + zero-event transactional path (constraint 1) | 450–600 | **Highest risk.** Plan on two sub-slices |
| 9 | Per-aggregate receipts: table, atomic confirm-with-append, fingerprint conflict, actor no-op on already-applied | 400–500 | Depends on 8 |
| 10 | Pre-dispatch wiring at the D2 position + stored-response replay + `RegisterUserImpl` recovery; **closes the `UserEntity` bug** | 350–450 | The slice that pays the debt in §1 |
| 11 | Split retention, batched concurrent-safe purge, observability signals | 300–400 | Safe to land last; nothing depends on it |

**Forecast**: roughly 3,800–5,050 authored lines. Expect **11–14 PRs** after
slices 6 and 8 split. Slices 2 and 3 carry migration risk; slice 8 carries
contract-breaking risk across two implementors.

Each slice has a clear start, a clear finish, autonomous scope, its own
verification (`cargo test --workspace`), and its own rollback.

---

## 14. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The `aggregate_id` split rewrites persisted identifiers and cannot be cleanly reverted | High | Isolate in slice 2 with a forward-only migration and a documented data-migration step before any dependent slice lands |
| Slice 8's `EventStore` contract change breaks both implementors and every caller | High | 12.2 decides the shape *before* apply; characterization tests from slice 1 pin current behaviour first |
| Mandatory keys break every existing client at once (D1) | High | `IdempotencyEnforcementMode`'s bounded compatibility variant, with a fail-closed default and an explicit removal condition |
| Stored-response replay leaks data across tenants | Med | Tenant is part of the identity by construction; a cross-tenant replay test is a success criterion, not an afterthought |
| "Idempotent" read as an atomicity promise for the dual write | Med | §9 is normative and repeats verbatim in the spec; the two guarantees in §2.1 are named separately everywhere |
| Design leans on single-process `EntityRegistry` uniqueness as an implicit safety net | Med | D6 mandates a multi-node-ready schema now: real unique constraints, fencing, atomic takeover. PROD-009 activates workers over that contract, it does not redesign it |
| TTL semantics misunderstood as "protection expires" | Med | §2.1's table and the D5 consequence text go into the public spec verbatim |
| No durable `EffectDedupStore` exists, so "reuse existing infrastructure" is unavailable | Confirmed | Budgeted as new work in slices 6 and 7, not assumed as a shortcut |
| Slice count induces reviewer fatigue and rubber-stamping | Med | Slice 4 and slice 11 are deliberately small and independent; slices 6 and 8 pre-split rather than arriving oversized |

---

## 15. Rollback Plan

Per slice, because the chain is long and the reversibility is not uniform.

| Slice group | Rollback |
|---|---|
| 1, 4 (test crate, `Clock`) | Clean commit-range revert. No persisted state |
| 5, 6, 7, 10, 11 (key, ports, wiring, purge) | Set `IdempotencyEnforcementMode` to the compatibility variant — **the runtime kill switch**, no code revert needed. Full revert drops new tables; no existing data depends on them |
| 3, 8, 9 (constraints, `EventStore`, receipts) | Constraints drop cleanly. The `EventStore` contract reverts as code. Receipts written before the revert become orphan rows that prior code ignores — harmless, but they persist |
| 2 (`aggregate_id` split) | **Not cleanly reversible.** Requires a reverse data migration. This slice gates the chain deliberately: nothing after it lands until it is verified in a real environment |

---

## 16. Dependencies

- CORE-008A tenant contracts (shipped) — `CanonicalTenant` is reused as the namespace, not redefined.
- CORE-017 lifecycle (shipped) — purge worker startup/shutdown ordering.
- CORE-012A observability and PROD-003 tracing (shipped) — signal plumbing.
- `testcontainers` / `testcontainers-modules` — new workspace dev-dependencies; starter config at `assets/integration-crate-template.toml`.
- An explicit minimum Postgres version. Undeclared when this was written (§12.1); **declared as PostgreSQL 14 by A1.4**, in `README.md`, and it is what makes AD-1's two-partial-index strategy necessary rather than optional.
- **No dependency on PROD-009, CORE-030, or CORE-029.** D6 is explicit: PROD-012 waits for none of them.

---

## 17. Success Criteria

Ticked against evidence in the tree, not against intent. The three that are not
plain ticks say why, because a criterion quietly reworded to match what shipped is
worth less than one that records the difference.

- [x] A retried `POST /register` with the same `Idempotency-Key` produces exactly one `UserRegistered` event and one welcome-email effect — the §1 bug, closed by test. *(`integration-tests/tests/infrastructure/replay_from_postgres.rs`, counting the published event and the accepted effect independently.)*
- [x] A request without an `Idempotency-Key` is rejected when enforcement is on. *(Asserted at the router, not only at the extractor: 400, the service recorder still `None`, and zero reservations.)*
- [x] Same key + different fingerprint yields a permanent conflict, never a silent dedupe.
- [x] Killing the process after the org command but before the user command, then retrying, completes the operation with zero duplicated events (§9's table, asserted). *(A real child process killed by SIGABRT; the retry runs as a different owner past the lease.)*
- [x] An expired lease is taken over atomically; the original owner receives `StaleOwner` and cannot close the operation.
- [x] Lease expiry, renewal, and takeover are tested deterministically against an injected `Clock`, with no `Utc::now()` remaining in lease logic.
- [x] A tenant-A key never replays a tenant-B response, including in the NULL-tenant systemwide mode. *(This one reproduced as a real leak before it was closed — see B7.8.)*
- [x] The `events` unique constraint rejects a duplicate `(tenant_id, aggregate_type, aggregate_id, version)` — including for systemwide (`tenant_id IS NULL`) aggregates.
- [x] A successful zero-event command writes its receipt transactionally.
- [~] After the reservation TTL expires, the response is no longer replayable **and** the aggregate still refuses to re-apply the operation. **Guaranteed by construction, not by a dedicated test.** A purged reservation makes the next arrival `Fresh`, so it dispatches, and the permanent receipt then answers without re-mutating. Both halves are pinned separately — purge eligibility by `assert_purge_conformance`, the receipt gate by `receipt_gating.rs` — and the composition is stated in the spec's two scenarios. No test drives one operation across the TTL boundary end to end.
- [~] The purge job runs concurrently from two workers without double-purging or deadlocking, and never purges an `InProgress` reservation. **Reworded by measurement — see B7.2.** Double-purge is prevented by PostgreSQL itself regardless of our query, and no circular wait was reproducible, so neither discriminates this code. What was missing and is now fixed and guarded is **progress**: `FOR UPDATE SKIP LOCKED` in the selection subquery, without which a batch stalls on locked tuples while free eligible rows sit untouched. "Never purges an `InProgress` reservation" holds and is covered.
- [x] `OperationKey` and `IdempotencyKey` are not interchangeable — a compile-fail test proves it.
- [x] `cargo test --workspace` green; the infrastructure suite green under testcontainers. **Path corrected:** `crates/integration-tests/` was removed by #274; the suite now lives in `integration-tests/`, an independent Cargo workspace outside the root, started by `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.

---

## 18. Decision Summary

| # | Question | Answer | Source |
|---|---|---|---|
| 1 | Is the key mandatory? | Yes, on every external mutable command. No server-side generation. Bounded compatibility mode allowed | D1 |
| 2 | What does one key identify? | One complete business operation, spanning multiple aggregates | D2 |
| 3 | Where is it reserved? | After `#[authorize]` / `#[tenant_scoped]`, before the first `EntityRuntime` call, namespaced by `CanonicalTenant` | D2 |
| 4 | What happens to an in-progress reservation when a process dies? | The lease expires; a later retry takes it over atomically and re-executes | D3 |
| 5 | What makes re-execution safe? | Permanent per-aggregate receipts `(tenant_id, aggregate_type, aggregate_id, operation_key)` + fingerprint, confirmed atomically with the append | D4 |
| 6 | Is the snapshot the source of truth? | No. The receipt is. A snapshot may accelerate the check, never replace it | D4 |
| 7 | How long does protection last? | Replay window = reservation TTL. Domain duplication protection = life of the stream | D5 |
| 8 | Does this wait for multi-node or the outbox? | No. The schema is multi-node-ready now; PROD-009 and CORE-030 build on it | D6 |
| 9 | Is `RegisterUserImpl`'s dual write atomic? | **No** — explicit non-promise. Safe recovery by re-execution only | D6, §9 |
| 10 | New key type or reuse `IdempotencyKey`? | New `OperationKey` in `crates/domain`. No implicit conversions, ever | D7 |
| 11 | Is there a keyed registry for the store? | No — one reservation authority per runtime; single fail-closed registration | §3 |
| 12 | Is the receipt store independently pluggable? | No — it must share the append transaction, so the persistence backend supplies it | §3, D4 |
