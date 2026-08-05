# Tasks: PROD-012 — End-to-End Idempotent Command Processing

> Strict TDD: every task's RED step lands before its GREEN step, one focused
> commit each, per `skills/work-unit-commits`. Verification default:
> `cargo test --workspace`; per-slice overrides noted where narrower.
>
> **101 tasks total** — 28 complete and 73 pending. Complete: B0.1–B0.3 (merged as
> `378a639`), A1.1–A1.4 (merged as `10b221d`), A4.1–A4.2 (merged as `cbc0187`),
> B1.1–B1.10, B2.1–B2.9.
>
> The total has moved twice, in this order:
>
> 1. **93 originally**, as first planned.
> 2. **92** once A4.3–A4.5 were removed for resting on a wrong premise — three tasks
>    out, two follow-ups in. See the note under Phase A4.
> 3. **93 again** when the context bridge B1 left unspecified was recorded as B6.4a.
>    See the note under Phase B1.
> 4. **96** when A2 gained an explicit preflight, post-verification and evidence
>    task (A2.7–A2.9). Three of the conditions attached to accepting the manual
>    migration risk had no task covering them, and one — an `aggregate_id`
>    matching no registered entity type at all — was a real hole in A2.3, which
>    only handled matching more than one.
> 5. **101** when a second transport was made part of the change (B1.11–B1.15), so
>    protocol-neutrality is demonstrated by two conforming adapters rather than
>    asserted by one. See the note under Phase B1b.
>
> The count is stated here so any prose that cites it can be checked against the file
> rather than drifting from it — which it has done more than once.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~3,800–5,050 authored (proposal §13, unchanged by design) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | 15 work units, PR 1 → PR 15 |
| Delivery strategy | ask-on-risk |
| Chain strategy | **hybrid** (D11, confirmed) — B0 + A1–A4 each merge to `develop` as a short incremental chain; the idempotency tracker branch is then created from the updated `develop`, the remaining Block B units stack inside it, and only the consolidated tracker merges to `develop` |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: hybrid
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| B0 | Defensive `UserEntity` state check (early, non-substitute) | PR 1 | `cargo test -p reference-app user_entity` | `examples/reference-app/tests/user_entity.rs` | Revert one handler diff; no schema, no data |
| A1 | `crates/integration-tests/` crate + testcontainers + `append` characterization | PR 2 | `cargo test -p integration-tests` (testcontainers) | Real Postgres via testcontainers | Delete new crate; drop workspace member entry |
| A2 | `aggregate_type` real column + `EntityTriple` split + backfill runbook | PR 3 | `cargo test -p integration-tests migration_007` | Testcontainers, backfill tool dry-run | Exact reverse migration (AD-9): rejoin `aggregate_type \|\| '-' \|\| aggregate_id` |
| A3 | Events uniqueness (AD-1 partial pair) + PG14 floor | PR 4 | `cargo test -p integration-tests uniqueness` | Testcontainers | `DROP INDEX` both partial indexes |
| A4 | Common `Clock` generalization | PR 3 of the chain (reordered ahead of A2/A3) | `cargo test -p ego-domain --lib time::clock` — hermetic unit tests only | N/A — no external service, no wall-clock dependency | Delete `crates/domain/src/time/`, drop the `pub mod time;` line, restore `auth/clock.rs` |
| B1 | `OperationKey` + `OperationKeyCarrier` + HTTP carrier + context carriage | PR 6 | `cargo test -p domain -p service-sdk -p transport idempotency_key` | N/A — unit/integration, no DB | Revert; no persisted state |
| B2 | `OperationReservationStore` port + in-memory + lease/fencing + renewal policy | PR 7 | `cargo test -p domain -p testkit reservation` | N/A — deterministic `TestClock` | Revert; no persisted state |
| B3 | Postgres reservation store + readiness health contributor | PR 8 | `cargo test -p integration-tests reservation_store` | Testcontainers | `DROP TABLE operation_reservations`; unregister health contributor |
| B4 | Async `EventStore` + UoW contract, both implementors | PR 9 | `cargo test -p integration-tests event_store_uow` | Testcontainers | Revert trait + both implementors as one unit |
| B5 | `operation_receipts` table + confirm-in-UoW + actor gating | PR 10 | `cargo test -p integration-tests receipts` and `cargo test -p persistent-entity` | Testcontainers | `DROP TABLE operation_receipts`; orphan rows harmless |
| B6 | `#[idempotent]` marker + slot-3 codegen + reference-app wiring — closes bug | PR 11 | `cargo test -p service-sdk-macros -p reference-app` | `examples/reference-app/tests/e2e_register.rs` | Set `IdempotencyEnforcementMode::Compatibility` — kill switch, no revert |
| B7a | Split retention + purge job + concurrency safety + worker ownership | PR 12 | `cargo test -p integration-tests purge` | Testcontainers, 2 concurrent workers | Disable purge worker registration; rows persist harmlessly |
| B7b | Observability (AD-10 spans/counters) | PR 13 | `cargo test -p service-sdk -p runtime idempotency_observability` | N/A — unit assertions on span/metric emission | Revert; no data impact |
| E1 | `RegisterUserImpl` dual-aggregate recovery E2E | PR 14 | `cargo test -p reference-app --test register_user_partial_failure` | Kill-after-org-command scenario, real actor restart | Revert test + wiring glue only |
| DOC | README PG14 floor, spec/design cross-links, ROADMAP update | PR 15 | N/A (docs) | N/A | Revert doc commit |

---

## Block A — Persistence Foundations

### Phase A1: Integration-Test Infrastructure

- [x] A1.1 RED: write `crates/integration-tests/tests/event_store_characterization.rs` asserting today's synchronous `EventStore::append` (`crates/persistence/src/postgres/event_store.rs:76-129`) behavior against a real Postgres testcontainer — pins current append/version/23505-unreachable behavior before any contract change.
- [x] A1.2 GREEN: create `crates/integration-tests/Cargo.toml` (testcontainers, testcontainers-modules, sqlx dev-deps per `assets/integration-crate-template.toml`), add it as a workspace member, make A1.1 pass.
- [x] A1.3 GREEN: add `crates/integration-tests` entry to the `layers.toml` layer map at the repository root — that file is the registry, and `xtask/src/layers.rs` is the reader that validates against it (activates `skills/testing`'s `PENDING [INFRA-CRATE]` block; satisfies `foundation-integrity` FR-001).
- [x] A1.4 Document declared minimum PostgreSQL version (14) in `README.md` and the integration-test container image tag.

### Phase A2: `aggregate_type` Real Column (AD-9 — reversible both directions)

- [ ] A2.1 RED: `crates/integration-tests/tests/aggregate_type_backfill.rs` — given ambiguous stored `aggregate_id` strings under a registered entity-type list (e.g. `user-account`/`7` vs `user`/`account-7`), the offline backfill tool MUST abort and name the ambiguous rows rather than guess (AD-9, verified constraint 5).
- [ ] A2.2 RED: `EntityTriple::aggregate_id()` (`crates/persistent-entity/src/scheduler.rs:30`) test asserting it exposes `aggregate_type`/`aggregate_id` as distinct fields, not a hyphen-joined string.
- [ ] A2.3 GREEN: add migration `crates/persistence/src/postgres/migrations/007_add_aggregate_type_to_events.sql` — nullable `aggregate_type` column; write the operator backfill tool that verifies longest-prefix-match non-ambiguity and aborts on any ambiguous row (**forward direction, not derivable from data alone**).
- [ ] A2.4 GREEN: after backfill succeeds, `SET NOT NULL` on `aggregate_type`; split `EntityTriple::aggregate_id()` into structural fields; update `actor.rs:230` write path.
- [ ] A2.5 GREEN: implement the **exact, lossless reverse migration** — `UPDATE events SET aggregate_id = aggregate_type || '-' || aggregate_id` rejoining precisely what was split, then drop the column. State explicitly in the migration file's down-comment: forward is abort-on-ambiguity (not derivable), reverse is exact and total.
- [ ] A2.7 RED/GREEN: **preflight, before a single row is written.** Three checks, each aborting the whole run and naming the offending rows:
      (a) an `aggregate_id` matching **no** registered entity type — distinct from ambiguity, which is matching more than one, and not covered by A2.1/A2.3;
      (b) an `aggregate_id` that is empty or whitespace-only, since the column is `NOT NULL` but not non-empty;
      (c) a post-split identity `(tenant_id, aggregate_type, aggregate_id, version)` that would collide with another row's — detected here rather than discovered when A3 tries to build its unique index.
      No write happens unless all three pass, alongside A2.1's ambiguity check.
- [ ] A2.8 RED/GREEN: **post-verification, after the backfill.** Row count unchanged; the post-split identity is unique across the table; and per identity the `version` sequence is gap-free and starts at 1. Referential integrity is deliberately **not** checked: no migration in this repository declares a foreign key, so there is nothing to verify and a check would only imply otherwise. Stream integrity is the meaningful analogue and is what A2.8 asserts.
- [ ] A2.9 GREEN: the backfill tool emits a machine-readable report — rows scanned, rows rewritten, and every abort with its reason and offending row identifiers — and the exact command sequence is recorded as a runbook so the external pipeline can execute it unchanged when it exists. Evidence is a deliverable of this slice, not a by-product of running it locally.
- [ ] A2.6 Gate: nothing in A3/B5 lands until A2.1–A2.5 and A2.7–A2.9 all pass under `crates/integration-tests` against real Postgres (per design.md AD-9 cost statement). A failing precondition leaves data untouched by construction, so a red gate here is a stop, never a partial migration.

### Phase A3: Effective Event Uniqueness (AD-1)

- [ ] A3.1 RED: `crates/integration-tests/tests/uniqueness.rs` — duplicate `(tenant_id, aggregate_type, aggregate_id, version)` rejected for a real tenant AND for `tenant_id IS NULL` systemwide mode (event-store spec scenarios).
- [ ] A3.2 GREEN: migration `008_events_stream_identity_unique.sql` — two partial unique indexes (`ux_events_identity_tenant` WHERE `tenant_id IS NOT NULL`, `ux_events_identity_systemwide` WHERE `tenant_id IS NULL`).
- [ ] A3.3 GREEN: map the now-reachable `23505` in `append` to `PersistenceError::Conflict` (verified constraint 4 — previously unreachable code).
- [ ] A3.4 RED: `crates/integration-tests/tests/schema_index_assertion.rs` — enumerates the expected six partial indexes across `events`, `operation_receipts`, `operation_reservations` and fails if any table has only one half of its pair.
- [ ] A3.5 GREEN: make A3.4 pass once B5/B3 land their own pairs (placeholder assertion checked in now, extended per-table later).

### Phase A4: Common `Clock` (independent — may run parallel with A1–A3)

- [x] A4.1 RED: `crates/domain/src/time/clock.rs` unit test — `Clock`/`SystemClock` behave identically to the current `crates/domain/src/auth/clock.rs:20` implementation (zero-behavior-change slice).
- [x] A4.2 GREEN: move `Clock`/`SystemClock` to `crates/domain/src/time/clock.rs`; `auth/clock.rs` becomes `pub use crate::time::clock::{Clock, SystemClock};` (compatibility re-export, no `#[deprecated]` per AD-8).

**A4.3–A4.5 removed — the premise was wrong.** They asked for a `Clock` to be
injected into `EffectDedupStore`, on the stated basis that `store.rs:58` calls
`Utc::now()` directly. Inspecting the real call sites showed otherwise:

- That `Utc::now()` lives inside `Timestamp::now()`, a free constructor on
  `Timestamp`, not inside any `EffectDedupStore` method.
- The trait's three methods — `reserve`, `commit_success`, `release` — neither
  take nor read time.
- `EffectStateStore`'s time-aware methods already receive it as a parameter:
  `claim_due(now, limit)`, `recover_in_flight(now)`, `mark_retryable(.., next_at)`.
  Time is already injected per call.
- Every `Timestamp::now()` inside `store.rs` sits below the `#[cfg(test)]`
  boundary at line 677.

Injecting a clock there would have produced a field nothing reads and a test
asserting that a getter returns what the constructor was handed — unfalsifiable
by construction. A4 is therefore complete at A4.1–A4.2, which is the deliverable
B2 actually consumes: one common `Clock` to inject into the reservation store.

## Follow-up — injectable clock for the effect retry subsystem

Not required by B2 and not part of Block A. Recorded so it is not lost.

- [ ] F1.1 RED: unit tests for `EffectRunner`'s due-claiming and retry-scheduling
      paths driven by a fake clock, asserting scheduling decisions without
      real elapsed time. Pure unit tests — no Docker, no containers, no real
      external resource.
- [ ] F1.2 GREEN: give `EffectRunner` an injected clock and route its two
      production wall-clock reads through it —
      `crates/runtime/src/effects/runner.rs:546` (`claim_due(Timestamp::now(), ..)`)
      and `:1017` (`mark_retryable_with_retry(.., Timestamp::now())`). These are
      the real untestable wall-clock reads in the effects subsystem; the store
      never had any.

Sized as its own unit because it touches the retry subsystem inside a file of
roughly three thousand lines, which does not belong inside an unrelated slice.

---

## Block B — End-to-End Idempotency

### Phase B0: Defensive `UserEntity` Fix (lands early, per D10 — NOT a substitute)

- [x] B0.1 RED: `examples/reference-app/tests/user_entity.rs` — given `UserState::Registered` already, `handle_command(UserCommand::Register)` returns `Ok(vec![])` (no-op) instead of a second `UserRegistered`.
- [x] B0.2 GREEN: in `examples/reference-app/src/domain/user.rs::UserEntity::handle_command` (currently ignores `_state` at line 106), match on `_state: &UserState` — if already `Registered`, return `Ok(vec![])`; otherwise proceed with existing validation.
- [x] B0.3 Document in the handler's doc-comment and in `reference-service` spec cross-reference: **this is defence-in-depth against in-process state drift, not a substitute for the runtime idempotency guarantee** — it does not stop two independent actors, two different aggregates, or a receipt-level replay; B6's `#[idempotent]` wiring is still required to close the operation-level duplicate-registration bug end to end.

### Phase B1: `OperationKey` + Carriage (may run parallel with Block A)

> **All ten tasks delivered, in slices.** B1.1–B1.2 — the two identity types —
> landed first, because the reservation contract cannot define its request type
> without them. B1.3–B1.5 (the no-conversion compile-fail assertion, the
> extraction contract and its policy table) followed, then B1.6–B1.7 (the HTTP
> carrier), then B1.8–B1.10 (both contexts able to hold the key, and its
> traversal from the command envelope to the actor).
>
> **This phase does not achieve end-to-end carriage, and no task in it ever
> specified that.** Ten tasks are individually satisfied, yet the value cannot
> flow from the transport edge to the actor, because nothing reads
> `ServiceContext::operation_key()` to construct the `CommandContext` a service
> passes down. That bridge is a genuine gap in this task list rather than in the
> code, and it is recorded as B6.4a below — the generated slot-3 code is where it
> belongs, since that is the point which already reads the resolved key.
>
> What B1 does deliver: the key has a type, one shared definition of validity and
> of the missing-key policy, one HTTP adapter conforming to that contract, and
> both contexts able to hold and hand on the value. Every piece the bridge will
> need, and not the bridge.

- [x] B1.1 RED: `crates/domain/src/operation/key.rs` unit test — `OperationKey::parse` rejects empty/whitespace-only strings, accepts a bounded-length valid string.
- [x] B1.2 GREEN: implement `OperationKey`, `OperationFingerprint` in `crates/domain/src/operation/key.rs` (sibling module to `idempotency.rs`, AD-7).
- [x] B1.3 RED: compile-fail test asserting no `From<OperationKey> for IdempotencyKey>` (or reverse) exists anywhere in the workspace (D7, spec scenario "No implicit conversion compiles").
- [x] B1.4 RED: `crates/service-sdk/src/idempotency/extraction.rs` unit test — `resolve_operation_key` policy table: missing key rejected under default mode, admitted under explicit compatibility mode (idempotent-command-processing spec scenarios).
- [x] B1.5 GREEN: implement `OperationKeyCarrier` trait, `resolve_operation_key`, `OperationKeyRejection`, and `IdempotencyEnforcementMode` (mirroring `crates/service-sdk/src/runtime/tenant.rs:143`) in `crates/service-sdk/src/runtime/idempotency.rs` and `crates/service-sdk/src/idempotency/extraction.rs`.
- [x] B1.6 RED: `crates/transport/tests/idempotency_carrier.rs` — `assert_carrier_conformance` (shipped in `crates/testkit`) run against the HTTP `HeaderCarrier`.
- [x] B1.7 GREEN: implement `HeaderCarrier(&HeaderMap)` in `crates/transport/src/idempotency.rs` (beside `security.rs`, `propagation.rs`); wire rejection to `crates/transport/src/error.rs`.
- [x] B1.8 RED: `crates/service-sdk/src/context/mod.rs` test — `ServiceContext::operation_key()` accessor returns the identical `OperationKey` set at ingress, no ambient lookup.
- [x] B1.9 GREEN: add `operation_key` field + accessor to `ServiceContext`.
- [x] B1.10 RED/GREEN: `crates/persistent-entity/src/command_context.rs` — `CommandContext` carries `operation_key` through to `EntityActor::execute_command`; test asserts identical value reaches the actor.

### Phase B1b: A second transport, so protocol-neutrality is demonstrated rather than asserted

> **Why this exists.** HTTP was implemented first because it is the only external
> transport that actually exists in this repository, not because idempotency is an
> HTTP concern. With one adapter, "protocol-agnostic" is a claim; with two it is a
> property somebody checked. gRPC is the minimum second adapter. Kafka is
> deliberately out of scope for this change.
>
> **Bounded deliberately.** This does not build a gRPC server. The repository has no
> gRPC transport — `tonic` appears only as an OTLP exporter dependency in
> `crates/infrastructure`, and `crates/transport` does not depend on it at all. What
> ships here is the reusable carrier surface over `tonic`'s metadata type plus its
> tests. Binding it to a real server belongs to whichever change introduces that
> transport.

- [ ] B1.11 RED: conformance test for a gRPC metadata carrier, driven by the **same**
      three-state harness HTTP already passes — no protocol-specific harness, no
      relaxed variant. Includes a real non-ASCII/non-UTF-8 metadata value so the
      unreadable state is exercised on this transport too.
- [ ] B1.12 GREEN: implement `GrpcMetadataCarrier` reading `idempotency-key` from
      `tonic::metadata::MetadataMap`, reporting `Absent`, `Present` or `Unreadable`.
      **Placement decision required before starting** — see the note below.
- [ ] B1.13 RED/GREEN: equivalence test across both adapters — for an absent key, a
      valid key, an invalid key and an unreadable value, HTTP and gRPC resolve to the
      **identical** outcome under both enforcement modes. Any divergence is a defect
      in whichever adapter differs, never a protocol-specific rule.
- [ ] B1.14 GREEN: correct `crates/transport/src/lib.rs`'s module doc, which claims
      the crate provides "no gRPC transport" while already exporting
      `GrpcServerConfig`. The charter is stale relative to its own contents and will
      be more so after B1.12.
- [ ] B1.15 RED/GREEN: assert no protocol type crosses the boundary — no `axum`,
      `HeaderMap`, `tonic` or `MetadataMap` symbol appears in `ego-domain`,
      `persistent-entity`, or the reservation and receipt surfaces. A grep-style
      structural test, so the neutrality is enforced rather than trusted.

**Placement, the one open decision.** `GrpcMetadataCarrier` needs `tonic`, which
`crates/transport` does not currently depend on. Two candidates: add it to
`crates/transport` behind a `grpc` feature, so HTTP-only builds do not compile tonic
and both adapters live where transport adapters already live — the crate already
hosts `GrpcServerConfig`; or a separate `crates/transport-grpc`, which keeps the
dependency fully isolated at the cost of a new workspace member and layer-map entry
for a small amount of code. Decide before B1.12, not during it.

### Phase B2: `OperationReservationStore` Port + In-Memory + Lease Mechanics (may run parallel with Block A)

> **Delivered in two slices.** The contract — the port, its supporting types and
> their type-level tests — lands first and compiles and tests on its own, since
> none of those tests needs an implementation. The in-memory implementation and
> the behavioural tests that exercise `reserve` follow in a second slice, whose
> volume is mostly behaviour coverage.

- [x] B2.1 RED: `crates/domain/src/operation/reservation.rs` unit tests against `TestClock` — `reserve` returns `Fresh` on first call, `OwnedInProgress` for the same owner mid-lease, `OtherInProgress` for a different owner mid-lease.
- [x] B2.2 GREEN: define `OperationReservationStore`, `ReserveRequest`, `OwnerFence`, `ReservationOutcome`, `ReservationError::StaleOwner`, `Lease`, `FencingToken` per the design.md Interfaces section.
- [x] B2.3 RED: lease-expiry-and-takeover test — advancing `TestClock` past `lease_until` makes a stale reservation eligible for takeover; takeover assigns a new `fencing_token` (F2 > F1) atomically.
- [x] B2.4 GREEN: implement takeover logic in `InMemoryOperationReservationStore` (`crates/testkit`).
- [x] B2.5 RED: `StaleOwner` conditional-update test — after takeover, the original owner's `complete`/`renew`/`abandon` call is rejected with `StaleOwner` and does not modify the reservation (verifies the triple `operation_id + owner_id + fencing_token`, not merely stored-token presence).
- [x] B2.6 GREEN: make every mutating call (`renew`, `complete`, `abandon`) perform the conditional triple-check.
- [x] B2.7 **Open-question task — lease renewal cadence and owner.** RED: test asserting a configured lease length with **no background renewal in this change** — a long-running operation either fits inside the configured lease or is taken over. GREEN: document this as the chosen default (design.md's stated assumption) in `IdempotencyEnforcementMode`'s doc-comment and `RetentionPolicy`/lease-length config; explicitly note renewal-on-demand (`OperationReservationStore::renew`) exists as a capability for a future caller-driven extension, but no runtime component invokes it automatically in this change.
- [x] B2.8 RED: fingerprint-conflict test at the reservation boundary — same key + different fingerprint → `Conflict`, never silent dedupe.
- [x] B2.9 GREEN: implement fingerprint comparison in `reserve`.

### Phase B3: Postgres Reservation Store + Readiness (needs A1, A4; cross-edge to B2)

- [ ] B3.1 RED: `crates/integration-tests/tests/reservation_store_postgres.rs` — `reserve`/`renew`/`complete`/`abandon`/`purge_completed_before` against real Postgres, mirroring B2's deterministic scenarios under `TestClock`-equivalent injected time.
- [ ] B3.2 GREEN: migration `010_create_operation_reservations.sql` + AD-1 partial-index pair on `(tenant_id, operation_key)`.
- [ ] B3.3 GREEN: implement `PostgresOperationReservationStore` — parameterized `$N` binds only, no interpolation (Security table).
- [ ] B3.4 RED: concurrent-takeover test — two processes racing to take over the same expired lease; exactly one succeeds atomically.
- [ ] B3.5 GREEN: implement the atomic conditional-update takeover query.
- [ ] B3.6 GREEN: `RuntimeBuilder::with_operation_reservation_store(...)`; `build()`/`try_build()` fails when enforcing mode resolves and no store is registered (service-sdk spec scenario).
- [ ] B3.7 **Open-question task — readiness during migrations and store unavailability.** RED: test asserting the readiness endpoint reports not-ready when the registered `OperationReservationStore`'s health contributor (following the existing `crates/service-sdk/src/health/mod.rs` `check() -> HealthCheck` contributor pattern) cannot reach Postgres, while a runtime with no store registered at all fails at **startup**, never reaching readiness. GREEN: implement `OperationReservationStoreHealthContributor` registered alongside the store in `RuntimeBuilder`; document explicitly that startup fail-closed covers "no store registered" and the readiness contributor covers "store registered but unreachable after start" — these are the two distinct failure modes, not one.

### Phase B4: Async `EventStore` + Unit-of-Work Contract (needs A1 characterization tests)

- [ ] B4.1 RED: `crates/domain/src/persistence/event_store.rs` — trait-level test (via a mock/double) asserting the new async `EventStore::begin() -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError>` shape compiles and is callable behind `Arc<dyn EventStore<E>>`.
- [ ] B4.2 GREEN: change `EventStore` trait to async (AD-2); add `EventStoreUnitOfWork` trait with `append`, `confirm_receipt`, `commit`.
- [ ] B4.3 RED: `crates/integration-tests/tests/event_store_uow.rs` — dropping a UoW without calling `commit()` rolls back (real Postgres transaction).
- [ ] B4.4 GREEN: implement `PostgresEventStoreUnitOfWork` in `crates/persistence/src/postgres/event_store.rs`, replacing the `block_on`-wrapped synchronous `append` (verified constraint 2) — `append(&mut self, ...)` becomes `&self`.
- [ ] B4.5 GREEN: implement the in-memory `EventStoreUnitOfWork` equivalent; ensure tenant-scoped uniqueness matches the durable store exactly (event-store spec: "In-Memory Store Does Not Silently Diverge").
- [ ] B4.6 RED: `crates/domain/src/persistence/stored_event.rs` test — `StoredEvent` metadata round-trips an `operation_key` through storage and back (event-store spec scenario; verified constraint 3 — no metadata channel exists today).
- [ ] B4.7 GREEN: add the metadata column/serialized field and bind it in the Postgres INSERT.
- [ ] B4.8 Update every existing `EventStore` caller (`EntityActor`, in-memory persistence adapter) for the new async signature; run full `cargo test --workspace` to catch ripple.

### Phase B5: Per-Aggregate `operation_receipts` (needs A2, A3, B4)

- [ ] B5.1 RED: `crates/integration-tests/tests/receipts.rs` — zero-event success still writes a receipt inside the same (empty) transaction (event-store + idempotent-command-processing spec scenarios).
- [ ] B5.2 GREEN: migration `009_create_operation_receipts.sql` + AD-1 partial-index pair on `(tenant_id, aggregate_type, aggregate_id, operation_key)`, storing the fingerprint.
- [ ] B5.3 GREEN: implement `confirm_receipt` on `EventStoreUnitOfWork` for both implementors, joining the same transaction as `append`.
- [ ] B5.4 RED: `crates/persistent-entity` test — actor consults the receipt before dispatch; matching fingerprint no-ops without invoking `handle_command`; mismatched fingerprint returns a permanent conflict without invoking `handle_command` (persistent-entity spec scenarios).
- [ ] B5.5 GREEN: add receipt-consultation gating in `crates/persistent-entity/src/actor.rs` before the `handle_command` call at line ~213.
- [ ] B5.6 RED: zero-event branch test — `actor.rs:219`'s `CommandResult::NoEvents` path now opens a transaction to confirm the receipt (today it never opens one — verified constraint 1).
- [ ] B5.7 GREEN: change the zero-event branch to call `event_store.begin()` → `confirm_receipt` → `commit()`.
- [ ] B5.8 Update A3.4/A3.5's schema-index assertion to include the `operation_receipts` pair now that it exists.

### Phase B6: `#[idempotent]` Marker + Slot-3 Wiring — Closes the Live Bug (needs B1, B2, B3, B5)

- [ ] B6.1 RED: `crates/service-sdk-macros/src/tests.rs` — `#[idempotent]` outside `#[service]` is a compile error; `#[idempotent]` without `#[operation]` is a compile error (mirrors the existing check at `lib.rs:528`).
- [ ] B6.2 GREEN: add the inert `#[idempotent]` marker attribute in `crates/service-sdk-macros/src/lib.rs`, read by the `#[service]` generator alongside `#[authorize]` (`lib.rs:808`) and `#[tenant_scoped]` (`lib.rs:824`).
- [ ] B6.3 RED: macro-expansion test asserting generated slot ordering — slot 1 `#[authorize]`, slot 2 `#[tenant_scoped]` (`enforce_tenant`), slot 3 the new reservation call — and that slot 3 never runs before a passing guard (design.md AD-5, spec scenario "Reservation happens after authorization and tenant scoping").
- [ ] B6.4a GREEN: bridge the two contexts — generated slot-3 code reads
      `ServiceContext::operation_key()` and threads that exact value into the
      `CommandContext` the service hands to the entity. Test asserts the key
      resolved at the transport edge is what `handle_command` observes, with no
      regeneration in between. **This closes the gap B1 left open**: B1 made both
      contexts able to hold the key and proved traversal from the command envelope
      onward, but nothing joined the two halves.
      **The bridge must not live in a transport adapter.** It belongs to the dispatch
      path every transport shares, so each adapter decides only how to *extract* the
      key while everything from `ServiceContext` inward is one identical path.
      Implementing it inside the axum layer would make the actor's idempotency
      accidentally HTTP-shaped, and the second adapter would then need its own copy.
- [ ] B6.4 GREEN: emit slot-3 codegen: `store.reserve(CanonicalTenant, OperationKey, fingerprint, owner, lease_until)`; branch on `Fresh`/`TakenOver` → continue, `Succeeded` → return stored response without invoking the handler, `Conflict` → permanent conflict, `*InProgress` → contention response.
- [ ] B6.5 RED: HTTP-level test (`crates/transport`) — missing/invalid `Idempotency-Key` rejected before the guarded operation runs; valid key surfaces identically on `ServiceContext` (http-transport spec scenarios).
- [ ] B6.6 GREEN: wire the HTTP carrier + `resolve_operation_key` at the axum layer ahead of the guarded operation.
- [ ] B6.7 RED: replay vs. conflict HTTP response test — same key/same fingerprint returns the original stored response unexecuted; same key/different fingerprint returns a distinguishable permanent-conflict response (http-transport spec scenarios).
- [ ] B6.8 GREEN: implement the slot-3 epilogue — `store.complete(op_id, owner, fencing_token, response)` as a conditional update; stale completion discards the response and does not overwrite state.
- [ ] B6.9 RED: `examples/reference-app/tests/e2e_register.rs` — retried `POST /register` with the identical `Idempotency-Key` and payload produces exactly one `UserRegistered` and one welcome-email effect (reference-service spec — **closes the `UserEntity` bug end to end**, distinct from and layered on top of B0's defensive fix).
- [ ] B6.10 GREEN: mark `RegisterUserImpl`'s handler(s) with `#[idempotent]`; verify B6.9 passes.
- [ ] B6.11 RED: reference-app test enumerating every mutating operation and asserting each carries the `#[idempotent]` marker (design.md Risks — mitigates the marker-completeness residual gap).
- [ ] B6.12 GREEN: add the enumeration/assertion helper and apply the marker to any operation the test finds missing it.

### Phase B7: Retention, Purge, Observability (needs B3, B6)

- [ ] B7.1 RED: `crates/integration-tests/tests/purge.rs` — reservation purge-eligibility is measured from `completed_at`, never `created_at`; an `InProgress` reservation is never TTL-purged.
- [ ] B7.2 GREEN: implement `purge_completed_before(cutoff, batch)` on `PostgresOperationReservationStore`, batched and observable.
- [ ] B7.3 RED: two-concurrent-workers test — overlapping eligible rows purged exactly once, no deadlock, no double-purge.
- [ ] B7.4 GREEN: implement the concurrency-safe purge query (e.g. `SELECT ... FOR UPDATE SKIP LOCKED` or equivalent row-claiming pattern).
- [ ] B7.5 RED: receipts survive the ordinary purge job — only an explicit aggregate/tenant deletion removes them (D5, idempotent-command-processing spec scenario).
- [ ] B7.6 GREEN: ensure the purge query never targets `operation_receipts`.
- [ ] B7.7 **Open-question task — purge worker ownership.** RED: test asserting the purge worker starts/stops under the existing CORE-017 lifecycle ordering contract (service-sdk spec: "Purge-Worker Lifecycle Follows Existing Ordering") and that shutdown never releases an in-progress lease. GREEN: register the purge worker as a runtime-owned background task in `RuntimeBuilder` (resolving the open question as **runtime-owned**, per design.md's stated assumption, not operator-scheduled); document the resolution and its rationale beside the `RetentionPolicy` config.
- [ ] B7.8 RED: cross-tenant replay test — tenant A's stored response never replays for tenant B, including the NULL-tenant systemwide scope (idempotent-command-processing spec — security-critical scenario).
- [ ] B7.9 GREEN: verify/harden the tenant-namespacing of every reservation/receipt lookup against B7.8.
- [ ] B7.10 RED: AD-10 observability tests — `idempotency.key.rejected`, `idempotency.reservation.outcome`, `idempotency.lease.event`, `idempotency.lease.stale_owner`, `idempotency.receipt.outcome`, `idempotency.purge.rows`/`.batch_duration`/`.oldest_completed_age` counters/spans/histogram emitted with the documented attributes; `operation_key` never appears raw, only as `idempotency.operation_key_hash` (first 16 hex chars of SHA-256), and never as a metric attribute.
- [ ] B7.11 GREEN: instrument the three spans (`idempotency.reserve`, `idempotency.takeover`, `idempotency.purge_batch`) and the counters/histogram/gauge on the existing CORE-012A/PROD-003 OTLP surface.

### Phase E1: Dual-Aggregate Recovery E2E (needs B6, B7 complete)

- [ ] E1.1 RED: `examples/reference-app/tests/register_user_partial_failure.rs` (existing hook, per user.rs comment) — kill the process after the `TenantOrganization` receipt confirms but before the `User` command executes; retry via lease takeover; assert org no-ops, user executes, zero duplicated `UserRegistered` events (D6 explicit non-promise; reference-service spec scenario).
- [ ] E1.2 GREEN: wire `RegisterUserImpl`'s recovery path so both aggregates are addressed under the one operation key/reservation (§9 non-atomicity honored explicitly, not silently).

### Phase DOC: Documentation and Rollout

- [ ] DOC.1 Update `README.md` with the declared PG14 floor and the `IdempotencyEnforcementMode` compatibility kill switch.
- [ ] DOC.2 Update `ROADMAP.md` §7.11 marking PROD-012 delivered; confirm no PROD-013 was created.
- [ ] DOC.3 Cross-reference the two-guarantee table (replay window vs. domain duplication protection) verbatim in user-facing docs, per proposal §2.1/§14 risk mitigation.

---

## Dependency Graph

```
A1 ──▶ A2 ──▶ A3 ─────────────┐
 │                            │
 ├──────────────▶ B3, B4      ├──▶ B5 (needs A2, A3, B4)
A4 ──▶ B2, B3                 │
                               │
B0 (independent, lands first) │
B1 ──┐                        │
B2 ──┼──▶ B6 (needs B1,B2,B3,B5) ──▶ B7 ──▶ E1
B3 ──┘
B4 ──▶ B5
```

- **No Block B slice merges before A1, A2, A3 merge — except B1 and B2**, which touch no schema and no real Postgres, and may proceed in parallel with Block A.
- **B0 has no dependency on anything** and should be the first slice to merge (D10: lands early, not at the end).
- A2 (`aggregate_id` split) gates the chain: nothing after it lands until verified against real Postgres in `crates/integration-tests`.

---

## Traceability

### `idempotent-command-processing` spec

| Requirement | Task(s) |
|---|---|
| Mandatory Key on Every External Mutable Command | B1.4, B1.5, B6.5 |
| No Server-Side Key Generation | B1.4, B1.5 |
| Operation-Scoped Identity, Reserved Before Dispatch | B1.9, B6.3, B6.4a, B6.4 |
| The Guarantee Is Protocol-Neutral, Demonstrated By Two Adapters | B1.4, B1.5, B1.6, B1.7, B1.11, B1.12, B1.13, B1.15 |
| Lease With Owner, Expiry, and Verified Fencing | B2.1–B2.6, B3.4, B3.5 |
| Per-Aggregate Receipts Confirmed Atomically With the Append | B5.1–B5.7 |
| Two Guarantees, Named Separately | B7.1, B7.5, DOC.3 |
| Fingerprint Determines Replay vs. Conflict | B2.8, B2.9, B5.4, B6.7 |
| Split Retention and Safe Purge | B7.1–B7.7 |
| The Dual-Aggregate Write Is Not Promised Atomic | E1.1, E1.2 |
| OperationKey Is Distinct From IdempotencyKey | B1.1–B1.3 |
| Cross-Tenant Replay Is Prohibited | B7.8, B7.9 |

### `event-store` spec

| Requirement | Task(s) |
|---|---|
| Effective Uniqueness on the Event Stream Identity | A3.1–A3.3 |
| Aggregate Type Is a Distinct Identity Component | A2.2, A2.4 |
| Append and Receipt Confirmation Share One Transaction | B4.2–B4.5, B5.3 |
| Event Metadata Carries the Operation Key | B4.6, B4.7 |
| The In-Memory Store Does Not Silently Diverge | B4.5 |

### `persistent-entity` delta

| Requirement | Task(s) |
|---|---|
| Receipt Consultation Gates Dispatch and Recovery | B5.4, B5.5 |
| Zero-Event Branch Opens a Transaction to Confirm a Receipt | B5.6, B5.7 |
| CommandContext Carries the Operation Key | B1.10 |
| Aggregate Identity Is Structurally Distinct, Not Concatenated | A2.2, A2.4 |

### `service-sdk` delta

| Requirement | Task(s) |
|---|---|
| ServiceContext Exposes the Operation Key | B1.8, B1.9 |
| RuntimeBuilder Registers the Reservation Store, Fail-Closed | B3.6 |
| RuntimeBuilder Registers a Single Injectable Clock | A4.4, A4.5 |
| RuntimeBuilder Registers Enforcement Mode and Retention Policy | B1.5, B7.7 |
| Purge-Worker Lifecycle Follows Existing Ordering | B7.7 |

### `http-transport` delta

| Requirement | Task(s) |
|---|---|
| Mandatory Idempotency-Key Extraction | B1.6, B1.7, B6.5, B6.6 |
| Replay and Conflict Responses Are Distinguishable | B6.7, B6.8 |

### `reference-service` delta

| Requirement | Task(s) |
|---|---|
| Retried RegisterUser Produces Exactly One UserRegistered Event | B0.1, B0.2, B6.9, B6.10 |
| Dual-Aggregate Recovery After Mid-Operation Process Death | E1.1, E1.2 |


### `testkit` delta

| Requirement | Task(s) |
|---|---|
| Reservation-Store Test Double | B2.2, B2.4 (implementation lives in `crates/testkit` per AD-7) |

**No gaps.** All 31 requirements across the 8 spec files have at least one covering task. B1.3's compile-fail test and A3.4/B5.8's schema-index assertion are cross-cutting checks that reinforce, rather than duplicate, their primary tasks.

## Open-Question Resolution Summary (D10)

| Question | Resolved by | Resolution recorded in |
|---|---|---|
| Lease renewal cadence and owner | B2.7 | Config-driven lease length; no automatic background renewal in this change; `renew()` remains caller-invocable for a future extension |
| Readiness during migrations / store unavailability | B3.7 | Startup fail-closed (no store registered) is distinct from readiness health-contributor (store registered but unreachable) |
| Purge worker ownership | B7.7 | Runtime-owned under `RuntimeBuilder`/CORE-017 ordering, not operator-scheduled |
