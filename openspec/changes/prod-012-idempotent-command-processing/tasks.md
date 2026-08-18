# Tasks: PROD-012 — End-to-End Idempotent Command Processing

> Strict TDD: every task's RED step lands before its GREEN step, one focused
> commit each, per `skills/work-unit-commits`. Verification default:
> `cargo test --workspace`; per-slice overrides noted where narrower.
>
> **116 tasks total** — 86 complete and 30 pending. Complete: B0.1–B0.3 (merged as
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
>    asserted by one, and **102** with the feature-state check that a repository
>    without CI needs in order for gated code not to rot. See the note under Phase B1b.
>
> An earlier revision briefly recorded 103 and 104 while A2.3 and A2.4 were split into
> halves. Review then showed the switch-over cannot be split without forking history, so
> the halves collapsed back and the total returned to 102. Recorded because the number
> moved and then moved back, which is worth being able to reconstruct.
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
| E1 | `RegisterUserImpl` dual-aggregate recovery E2E | PR 14 | `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` | Real child process killed by SIGABRT between the aggregates; retry as a different owner past the lease | Revert the test; no wiring to revert (AD-12) |
| DOC | README PG14 floor, spec/design cross-links, ROADMAP update | PR 15 | N/A (docs) | N/A | Revert doc commit |

---

## Block A — Persistence Foundations

### Phase A1: Integration-Test Infrastructure

- [x] A1.1 RED: write `crates/integration-tests/tests/event_store_characterization.rs` asserting today's synchronous `EventStore::append` (`crates/persistence/src/postgres/event_store.rs:76-129`) behavior against a real Postgres testcontainer — pins current append/version/23505-unreachable behavior before any contract change.
- [x] A1.2 GREEN: create `crates/integration-tests/Cargo.toml` (testcontainers, testcontainers-modules, sqlx dev-deps per `assets/integration-crate-template.toml`), add it as a workspace member, make A1.1 pass.
- [x] A1.3 GREEN: add `crates/integration-tests` entry to the `layers.toml` layer map at the repository root — that file is the registry, and `xtask/src/layers.rs` is the reader that validates against it (activates `skills/testing`'s `PENDING [INFRA-CRATE]` block; satisfies `foundation-integrity` FR-001).
- [x] A1.4 Document declared minimum PostgreSQL version (14) in `README.md` and the integration-test container image tag.

### Phase A2: `aggregate_type` Real Column (AD-9 — reversible both directions)

> **Two review slices, one coordinated transition.**
>
> The first slice is **purely additive Rust**: an `aggregate_type()` accessor on the
> entity identity, plus tests that record why the joined form cannot be reversed. No
> schema change, no SQL, no `EventStore` change, and `aggregate_id()` keeps its current
> meaning. It is deployable on its own with no traffic coordination because it changes
> no persisted behaviour at all.
>
> Everything that touches the schema or the data is the **second** slice, landing as one
> transition: the column, the preflight and its four aborts, the backfill, the switch-over
> of read and write identity, the post-verification, `SET NOT NULL`, the reverse
> operation, the report, the runbook, and the real-PostgreSQL suite.
>
> **Why the boundary sits exactly there.** An earlier attempt put the `EventStore`
> signature change in the first slice, reasoning that new writes would then record the
> type immediately. That is unsafe. `load` and `append`'s version check would query
> `aggregate_type = $1 AND aggregate_id = $2` while historical rows still hold `NULL` and
> a joined `"user-7"`. Neither condition matches — one of them for the same
> three-valued-logic reason recorded elsewhere in this change — so every historical stream
> reads as absent, `append` computes version 0, and the actor writes a **second, forked
> stream** under the split identity while the original rows are orphaned. That is not a
> window of temporary unavailability; it is history divergence with no clean revert once
> traffic has passed through.
>
> **The tell, and it was missed once.** In the unsafe version the inherited
> characterization tests had to be edited to keep compiling. A characterization test that
> needs adapting is reporting that the behaviour it characterises has changed — that was
> the alarm, and it was read as mechanical fallout. In the corrected first slice those
> tests pass untouched, and that is the evidence the slice is additive.
>
> The second slice must not assume the first cleaned anything. All four aborts run before
> any `UPDATE`.

- [x] A2.1 RED: `crates/integration-tests/tests/aggregate_type_backfill.rs` — given ambiguous stored `aggregate_id` strings under a registered entity-type list (e.g. `user-account`/`7` vs `user`/`account-7`), the offline backfill tool MUST abort and name the ambiguous rows rather than guess (AD-9, verified constraint 5).
- [x] A2.2 RED: `EntityTriple::aggregate_id()` (`crates/persistent-entity/src/scheduler.rs:30`) test asserting it exposes `aggregate_type`/`aggregate_id` as distinct fields, not a hyphen-joined string.
- [x] A2.3 GREEN: add migration `crates/persistence/src/postgres/migrations/007_add_aggregate_type_to_events.sql` — nullable `aggregate_type` column; write the operator backfill tool that verifies longest-prefix-match non-ambiguity and aborts on any ambiguous row (**forward direction, not derivable from data alone**).
- [x] A2.4 GREEN **(one coordinated step, after the rows are transformed)**: extend `EventStore`'s surface synchronously to carry the type alongside the id, switch the `actor.rs` write path to the structural identity, and `SET NOT NULL` on `aggregate_type`. These cannot be separated: activating the identity before the data is transformed forks history, and making the column mandatory before the switch-over would reject writes from the old path. Deliberately **not** async and **no** unit-of-work handle — this changes what the identifier is, not how the transaction is shaped.
- [x] A2.5 GREEN: implement the **exact, lossless reverse migration** — `UPDATE events SET aggregate_id = aggregate_type || '-' || aggregate_id` rejoining precisely what was split, then drop the column. State explicitly in the migration file's down-comment: forward is abort-on-ambiguity (not derivable), reverse is exact and total.
- [x] A2.7 RED/GREEN: **preflight, before a single row is written.** Three checks, each aborting the whole run and naming the offending rows:
      (a) an `aggregate_id` matching **no** registered entity type — distinct from ambiguity, which is matching more than one, and not covered by A2.1/A2.3;
      (b) an `aggregate_id` that is empty or whitespace-only, since the column is `NOT NULL` but not non-empty;
      (c) a post-split identity `(tenant_id, aggregate_type, aggregate_id, version)` that would collide with another row's — detected here rather than discovered when A3 tries to build its unique index.
      No write happens unless all three pass, alongside A2.1's ambiguity check.
- [x] A2.8 RED/GREEN: **post-verification, after the `UPDATE`s and before `SET NOT NULL`, inside the same transaction.** Row count unchanged; the post-split identity is unique across the table; and per identity the `version` sequence is gap-free and starts at 1. Referential integrity is deliberately **not** checked: no migration in this repository declares a foreign key, so there is nothing to verify and a check would only imply otherwise. Stream integrity is the meaningful analogue and is what A2.8 asserts. These checks read the rows **as written**, which is a different claim from the preflight ones computed in memory; failing any of them rolls the whole transaction back and reports which of the two stages refused.
- [x] A2.9 GREEN: the backfill tool emits a machine-readable report — rows scanned, rows rewritten, and every abort with its reason and offending row identifiers — and the exact command sequence is recorded as a runbook so the external pipeline can execute it unchanged when it exists. Evidence is a deliverable of this slice, not a by-product of running it locally.
- [x] A2.10 RED/GREEN **(second slice)**: a fail-closed open-time check — the Postgres event store refuses to open while any row has `aggregate_type IS NULL`, after the migration and before any store operation is possible, on every open with no cached flag. A cached answer goes stale exactly when an old writer inserts one more untyped row mid-transition, which is the case worth catching. Proven against real PostgreSQL: refuses with a row present, opens after the backfill completes, and the refusal precedes any read or write because the constructor returns a result and no store value exists on the error path. The runbook records the order — quiesce old writers, migrate and backfill, verify, make the column mandatory, then start the new binary — and the check exists because that order cannot be enforced by a document.
- [x] A2.6 Gate: nothing in A3/B5 lands until A2.1–A2.5 and A2.7–A2.10 all pass under `crates/integration-tests` against real Postgres (per design.md AD-9 cost statement). A failing precondition leaves data untouched by construction, so a red gate here is a stop, never a partial migration.

### Phase A3: Effective Event Uniqueness (AD-1)

- [x] A3.0 RED/GREEN **(first slice — prerequisite for A3.2)**: the store's three `tenant_id` comparisons are null-safe. `resolve_tenant(None)` binds SQL NULL, and `tenant_id = NULL` is unknown rather than true for every row, so a systemwide stream was invisible to its own version check, its own `load`, and `list_aggregate_ids`. Every systemwide append therefore read an empty history and wrote version 1 again — duplicating history silently. Fixed with `IS NOT DISTINCT FROM`, which compares two NULLs as equal while keeping NULL distinct from any concrete tenant. **This must land before A3.2**: the unique indexes cannot be built over a table that already holds the duplicates this defect produces. Proven against real PostgreSQL, including that the fix does not over-match — a systemwide stream and a tenant stream sharing type and id stay separate.
- [x] A3.0a GREEN **(debt closed with the slice)**: a shared `EventStore` conformance harness in `crates/testkit`, run against **both** implementations of the port — the in-memory one hermetically, the PostgreSQL one against a real database. The two disagreed about the systemwide partition while both satisfying the trait's signature, and nothing compared them. Proven to have teeth: with the null-safe comparison temporarily reverted, the harness fails on the PostgreSQL store with `Conflict { expected: 1, actual: 0 }` while the in-memory store still passes.
- [x] A3.0b GREEN **(debt closed with the slice)**: the migration registry is verified against the filesystem, bidirectionally. Three migration files numbered into the applied sequence had never been executed by any code path; `include_str!` only binds the files that are named, so an unnamed file is inert while looking exactly like one that ships. The orphans are removed and the check makes the omission impossible to repeat.
- [x] A3.1 RED: `crates/integration-tests/tests/uniqueness.rs` — duplicate `(tenant_id, aggregate_type, aggregate_id, version)` rejected for a real tenant AND for `tenant_id IS NULL` systemwide mode (event-store spec scenarios).
- [x] A3.2 GREEN: migration `008_events_stream_identity_unique.sql` — two partial unique indexes, `ux_events_identity_tenant` over `(tenant_id, aggregate_type, aggregate_id, version) WHERE tenant_id IS NOT NULL` and `ux_events_identity_systemwide` over `(aggregate_type, aggregate_id, version) WHERE tenant_id IS NULL`. A single conventional `UNIQUE` treats every NULL as distinct, so it would permit unlimited duplicates in the tenant-less partition — the one where silent duplication was already found. `NULLS NOT DISTINCT` would express it in one index but arrived in PostgreSQL 15, and the declared floor is 14: verified empirically against the pinned `14-alpine` image, where the syntax is a parse error and `pg_index` has no `indnullsnotdistinct` column. The systemwide half omits `tenant_id` because its predicate already fixes that column to NULL, so including it would index a constant.
- [x] A3.3 GREEN: the now-reachable `23505` maps to `PersistenceError::Conflict`, reporting the version the stream **really** has. The mapping already existed but reported `current`, which the in-process check has already proven equal to `expected_version` — a conflict asserting expected and actual are the same number. Since the transaction is aborted and unqueryable, the stream is re-read on another connection. Reachability is proven deterministically by a competing uncommitted row rather than by timing.
- [x] A3.4 RED: `crates/integration-tests/tests/schema_index_assertion.rs` — asserts against PostgreSQL's catalog, not the migration source: exact ordered column list, uniqueness, the partial predicate that carries the NULL semantics, and the stable index names. Plus a check that the two predicates partition the table with no gap and no overlap, evaluated by the server rather than inferred from the predicate text.
- [x] A3.5 GREEN: pair completeness is enforced two ways. A hand-maintained registry names the tables that must carry a pair — `events` today, receipts and reservations when their slices land — because a table that should have a pair and has no index at all is invisible to discovery. And a discovery query fails any table carrying one half of a tenant-partitioned pair without the other, which needs no one to remember to extend the registry. The discovery half is inert today and says so; it also asserts it matched `events`, so it cannot pass vacuously.

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
> code, and it is recorded as B6.4a below. This note originally said the bridge
> belonged in generated slot-3 code, since that was then the only point which had
> read the resolved key. The reservation boundary changed that: the runtime now
> stamps the accepted key and fingerprint onto `ServiceContext`, so the service
> body reads an already-authorised identity and transfers it into each
> `CommandContext` it creates. See B6.4a for the split.
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

**Placement — decided.** `GrpcMetadataCarrier` lives in `crates/transport`, with
`tonic` as an **optional** dependency behind a `grpc` feature, so the carrier compiles
only when that feature is on. A separate `crates/transport-grpc` was rejected: a new
workspace member and layer-map entry for a small amount of code, and it would
fragment the adapters. Keeping both in one crate keeps them side by side — that crate
already hosts `GrpcServerConfig` — while an HTTP-only build still never compiles tonic.

**Consequence that needs a task, because this repository has no CI to catch it.** Code
behind a feature nothing builds rots silently: it stops compiling and nobody learns
until someone enables the feature months later. The default build does not cover it,
and `cargo check --workspace --all-targets` does not enable non-default features.

- [ ] B1.16 GREEN: exercise both feature states as part of this slice's own gate —
      the default build and `--features grpc` — and record both commands in the
      slice's evidence. A feature that is only ever verified by the person who wrote
      it is not verified.

Two further transports are explicitly **not** in this change: Kafka record headers,
and any real gRPC server binding. Both would consume the same carrier contract
unchanged, which is the point of stopping here.

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

- [x] B3.1 RED/GREEN: `crates/integration-tests/tests/reservation_store_postgres.rs` runs the **shared** conformance contract from `ego-testkit` — not a copy of B2's scenarios. Both implementations execute the identical definitions, which is what the B3-0 extraction was for. Each scenario gets a store over a freshly truncated table and a clock at the shared epoch, matching the isolation the in-memory factory gives by allocating a new map.
- [x] B3.2 GREEN: migration `010_create_operation_reservations.sql` — the table plus the AD-1 partial pair on `(tenant_id, operation_key)`, the systemwide half covering the key alone because its predicate already fixes the tenant to NULL. Also two CHECK constraints: the state is one of the two the contract defines, and a completion carries both its timestamp and its response or neither. Purge eligibility is measured from `completed_at`, so a completed row without one would be unpurgeable forever and an in-progress row with one purgeable while still held — the database refuses both rather than trusting writers. A catalog assertion pins the index shape, because the behavioural scenarios would keep passing against a single conventional `UNIQUE`.
- [x] B3.3 GREEN: `PostgresOperationReservationStore` implements **all five** methods — `reserve`, `renew`, `complete`, `abandon`, `purge_completed_before` — and satisfies the whole shared contract. Parameterised `$N` binds only, never interpolation: an operation key is client-supplied. Every `tenant_id` comparison is `IS NOT DISTINCT FROM`, so a systemwide reservation is visible to its own lookup; verified by neutralising it, which fails four of the six conformance tests. The three mutators share one shape that puts the full triple *and* the lease bound in the `WHERE`, so verification and mutation are one statement rather than a read-then-write with a window in it.
- [x] B3.4 RED/GREEN: two concurrency tests, because one was not enough. Six contenders racing one expired lease yield exactly one winner whose token advanced by exactly one — but each contender re-reads before updating, so that test never exercises the `UPDATE`'s own guards, which was verified by neutralising them and watching it stay green. The second forces the window open instead of racing for it: another transaction locks the row, the takeover's `UPDATE` blocks, the holder extends the lease and commits, and the blocked update must re-check and report the current holder. Neutralising the lease predicate makes that one report a takeover of a renewed lease.
- [x] B3.5 GREEN: the atomic conditional-update takeover. `lease_until <= $N` is the load-bearing predicate — it is what judges a waiting caller against the row that exists rather than the row it read. `fencing_token = $N` is a compare-and-swap on the row version and is **redundant given** that predicate: every path that changes the token also pushes the lease into the future. No test distinguishes them, checked rather than assumed, and the code says so instead of claiming the token guard carries the guarantee.
- [x] B3.6 GREEN: `RuntimeBuilder::with_operation_reservation_store(...)` and `with_idempotency_enforcement_mode(...)`, with the fail-closed check the spec requires — under the default `MandatoryKey`, building without a registered store fails, naming both the registration that fixes it and the explicit opt-out. **One** validation, two consumers: `build()` panics on it, `try_build()` returns it as a structured error. `try_build` validates *before* delegating to `build`, because `build` panics and a check afterwards could never return the error. A new `RuntimeError` variant rather than `DependencyNotFound`: none of the existing `DependencyKind`s describes a reservation store, and each carries a fix hint naming its own registration method, so reusing one would tell the reader to call `.adapter(...)`. **Two consequences of this requirement, not separate ones:** `AppBuilder` forwards both methods, because its `build()` delegates to `try_build()` and it exposes no route to the builder it wraps — without them no application could adopt enforcement or decline it, so the requirement would not be usable. And 48 construction sites across 24 files declare `Compatibility` explicitly, which is the mechanical consequence of a fail-closed default; none is handed the in-memory store, since registering the double would make a build succeed and look adopted while giving no durability.
- [x] B3.7 GREEN: `OperationReservationStoreHealthContributor`, following the existing `health/mod.rs` `check() -> HealthCheck` pattern, registered by `build()` from the **same** `Arc` it then hands `RuntimeInner` — `Arc::clone` of the field, never a second construction and never a second read of the configuration, because two instances built from one config would mean readiness reporting truly about the wrong object. The registration is keyed on the store being **present**, not on the enforcement mode: a `Compatibility` runtime that registered one is still dispatching through it. The two failure modes stay distinct and neither covers the other — B3.6's startup refusal covers "no store registered at all" and never reaches readiness; this covers "store registered, process started, store now unreachable", which no startup check could decide. Readiness only: `Runtime::liveness` consults no contributor and stays healthy, because restarting on a lost database gives a new process the same unreachable database and, under a restart-on-failure supervisor, a crash loop that clears no state. The port gains `probe()` with **no default implementation** — a default `Ok` would let a store that forgot to write one report itself reachable forever, and an incomplete `impl` is the cheapest place to catch that; the Postgres one reads the reservation table under `LIMIT 1`, so a pool connected to an unmigrated database is caught rather than answering `SELECT 1` happily and discovering the missing table on the first real reserve. The error's text is read for nothing: `HealthCode` is a closed set that structurally cannot carry a payload, so a driver error quoting a DSN cannot reach an unauthenticated probe response. Teeth on all four risks — omitting the contributor, reading a store error as ready, marking `Compatibility`-without-store as failed, and pointing the contributor at a second instance — each killed a distinct test, and only the probe-count identity test caught the last one. **Verification status: implemented and contractually tested; real PostgreSQL not verified.** The durable evidence closing this task is entirely in-process and needs no infrastructure: contributor mapping, builder wiring, `Arc` identity through the probe counter, `Compatibility` with and without a store, credential redaction, and the readiness-versus-liveness separation. A companion test driving healthy → unreachable → healthy against a real database was written alongside this work but does not survive the constitution's ban on infrastructure in this workspace (CC-R11 No Infrastructure Dependency, UT-R4 No Testcontainers), so the durable `probe()`'s behaviour against PostgreSQL — the `LIMIT 1` read, and recovery after an outage — rests on the adapter's SQL and on reasoning rather than on a passing test. Reconstruction in a separate Testcontainers workspace is tracked as **issue #275**, where the measured constraint is recorded: Docker re-allocates a dynamically published port on restart, so the outage must be driven through a TCP forwarder the test owns rather than by restarting the container. The task stays complete because the requirement it names is the contributor and its registration, both proven here; the deferred verification is transverse debt, not an unfinished part of B3.7.

### Phase B4: Async `EventStore` + Unit-of-Work Contract (needs A1 characterization tests)

- [x] B4.1 RED: `crates/domain/src/persistence/event_store.rs` — trait-level test (via a mock/double) asserting the new async `EventStore::begin() -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError>` shape compiles and is callable behind `Arc<dyn EventStore<E>>`.
- [x] B4.2a GREEN **(first slice)**: `EventStore` becomes asynchronous (AD-2) — `append`, `load` and `list_aggregate_ids` are `async`; `stream_version_offset` stays synchronous because it reports a static property of the store's configuration with no fallible path and no I/O. Uses `#[async_trait]`, not native `async fn` in trait: the trait is consumed as `dyn EventStore<E> + Send` behind a shared lock, and a native `async fn` makes a trait non-dyn-compatible. `PostgreSQLEventStore` loses its `block_in_place` + `block_on` bridge entirely. Behaviour unchanged: the full pre-existing suite passes untouched in substance.
- [x] B4.2b GREEN **(second slice)**: add the `EventStoreUnitOfWork` trait with `append` and `commit`. **`confirm_receipt` deliberately deferred to B5**: nothing backs it today — no `operation_receipts` table, no receipt type, no caller — so shipping it now would mean a trait method whose every implementation answers "not yet". That is the same premise-without-backing already trimmed from A4 in this change. It lands in B5 alongside migration `009` and the semantics it needs. Recorded as B5.3a.
- [x] B4.2c GREEN: `commit` takes `self: Box<Self>`, so a committed unit of work cannot be reused — the compiler refuses rather than an implementation discovering a spent transaction at runtime. There is deliberately **no `rollback`**: dropping is the rollback, which makes the safe outcome the one that happens on an early return, a cancellation, or a panic — exactly the paths where an explicit call gets missed.
- [x] B4.3 RED: `crates/integration-tests/tests/event_store_uow.rs` — dropping a UoW without calling `commit()` rolls back (real Postgres transaction).
- [x] B4.4 GREEN: implement `PostgresEventStoreUnitOfWork` in `crates/persistence/src/postgres/event_store.rs`. `begin` takes `&self`, not `&mut self`: handing out a transaction does not mutate the store, so requiring exclusive access would force every caller behind a lock it does not need.
- [x] B4.4b GREEN: `EventStore::append` narrows from `&mut self` to `&self`, and `PersistenceFacade` drops the lock it only ever held to manufacture that exclusive borrow. Appending is not a mutation *of the store*: every implementation reaches whatever state it owns through a pool handle or an interior lock, because a store shared between actors cannot be exclusively borrowed across a database round trip anyway. The lock was held for the whole append, so every entity sharing a facade queued behind every other one. `EventStoreUnitOfWork::append` keeps `&mut self` — it owns a transaction, which is a genuine exclusive borrow. The property is asserted behaviourally rather than by reading the struct: two appends through one facade must overlap, proven with a store that blocks until a second append arrives, and verified to fail (a 10-second timeout) when serialisation is reintroduced.
- [x] B4.5a GREEN: implement the `EventStoreUnitOfWork` equivalent for **`ego-infrastructure`'s** in-memory store, and run the shared conformance harness against it. That store partitions by tenant — its `StreamKey` carries `Option<String>` — so it is judged against the same tenant-scoped assertions as the durable store, unit of work included.
- [x] B4.5b GREEN: implement the `EventStoreUnitOfWork` equivalent for **`persistent-entity`'s** in-memory store, with version arithmetic matching its own direct append path — `offset + committed + staged`. Version offsets are part of that arithmetic, not an exception to it: the direct path adds them, so a unit of work that left them out would reject an append the direct path accepts on the same stream with the same argument. Pinned by `in_memory_version_offset_parity.rs`, which compares the two paths to each other rather than restating either one's numbers.
- [x] B4.5d RED/GREEN **(unplanned — a live defect the B4.5c investigation surfaced)**: recovery of an aggregate that has never been persisted must succeed. `PersistenceFacade::load_for_recovery` propagated the store's error, and the durable store reports an absent stream as `PersistenceError::NotFound`, so **no entity could be activated for the first time against PostgreSQL**. Every recovery test used the in-memory store, which returns an empty stream instead of reporting absence, so nothing could see it. Reproduced against a real database before fixing. `NotFound` is now absorbed as "no history"; every other error still fails recovery, because those mean the history could not be *read* — and treating an unreadable stream as a fresh entity would append from version zero over history it never saw. Both halves are pinned, the second hermetically.
- [x] B4.5c GREEN: `persistent-entity`'s in-memory store — the one the runtime builder installs when none is supplied — is now tenant-partitioned and inside the shared conformance harness. Its `StreamKey` carries `Option<String>`; keeping the `Option` in the key rather than flattening it is what makes the tenant-less scope its own partition, since `None == None` in a keyed collection while `None` never equals `Some(_)` — the same semantics the durable store expresses with `IS NOT DISTINCT FROM`. Its `load` now reports an unseen stream as `NotFound` instead of an empty list, which the harness also requires and which B4.5d's recovery fix is what made safe. Both divergences are pinned: flattening the tenant fails the harness with `Conflict { expected: 0, actual: 2 }`, and returning an empty stream fails it on the absent-stream assertion.
- [x] B4.5e GREEN **(enabling, behaviour-preserving)**: `resolve_tenant` moves to `crates/domain/src/persistence/tenant.rs`. It had been copied into four adapters — the PostgreSQL module and three in-memory ones — and B4.5c needed a fifth call site, so copying again would have knowingly worsened a duplication that already existed four times. The four copies were compared before consolidating: two textual variants differing only in `match` arm order, semantically identical. The rule is a domain statement about what a tenant identifier means, not an implementation detail of any adapter.
- [x] B4.6 RED/GREEN: `crates/domain/src/persistence/stored_event.rs` unit tests pin what that file can pin — that neither constructor attaches an operation key, that attaching one leaves the other fields alone, and that attaching twice keeps the last rather than refusing or accumulating. The **round trip through storage** is not provable there and is asserted by the shared conformance harness instead, against all three implementations: a key attached to one event in a batch comes back exactly, and the event appended beside it does not acquire one. Both write paths are covered, since the direct append and the unit of work bind it through separate code.
- [x] B4.7 GREEN: migration `009_add_operation_key_to_events.sql` adds a nullable `operation_key VARCHAR(255)`, and **both** Postgres write paths bind it — the direct append and the unit of work, which write through separate code. `load` reads it back and parses it, surfacing an unparseable stored value as an error rather than returning the event as though it had no operation behind it. A dedicated column rather than a generic `metadata` blob: the operation is a first-class identity here, not incidental annotation, and a JSON blob would be unqueryable without expression indexes while inviting anything at all to be dumped in. Deliberately unindexed — nothing queries events by operation key, and an index for a query that does not exist costs every write to serve none.
- [~] B4.7a **WITHDRAWN — superseded by AD-13 and B4.7b.** `StoredEvent::correlation_id` behaved differently per store and the shared contract pinned neither behaviour. **Neither becomes normative — the field is withdrawn. See AD-13.**

      Not `[x]`: this task's own criterion was to decide the contract *and* correct the implementation. AD-13 decided the contract; B4.7b implemented the removal afterwards, landing in #323. Marking B4.7a done would claim work that was superseded rather than performed — the same standard applied to E1.2.

      The audit that decided it, all measured rather than assumed:

      | Store | `append` | `load` | Survives? |
      |---|---|---|---|
      | `InMemoryEventStore` (infrastructure) | `stream.extend(events)` | returns the stored values | yes, verbatim |
      | `InMemoryEventStore` (persistent-entity) | `stream.push(event)` | returns the stored values | yes, verbatim |
      | `PostgreSQLEventStore` | `INSERT` has no such column | `without_correlation(event)` | no |
      | `PostgresEventStoreUnitOfWork` | the same `INSERT` | — | no |

      **The deciding fact is not the divergence.** At the time of that audit, `StoredEvent::new` — then the only constructor that could set a correlation id — had exactly **two call sites, both inside the type's own unit tests**. No productive code in `crates/`, `examples/` or `integration-tests/` ever supplied one. Persisting it would add schema and contract to transport absence forever.

      Two corrections to this task's own earlier text, both worth recording, and both describing the state measured during the audit that preceded #323. First, the in-memory stores preserved the field **by accident** — they kept whole `StoredEvent` values — which is not a specification. Second, `persistent-entity/src/persistence.rs` also constructed through `without_correlation`, but that is a *facade* receiving raw `&[E]` with no correlation id to carry, not a store discarding one; conflating them would have meant fixing the wrong place. Also unstated before: the `events` table has **never** had a `correlation_id` column across all eight migrations, so making it durable needs a migration, not just a bind.

      `ServiceContext::correlation_id` is untouched and remains the correlation that exists — set, read and propagated across services. Nothing bridges it to events; no file in the codebase mentions both.

- [x] B4.7b Implementation: remove `StoredEvent::correlation_id` and the two-argument constructor whose only parameter beyond the event is that field. `StoredEvent::new` itself is **not** removed — the two prior constructors, the two-argument `new` and `without_correlation`, collapse into the single `StoredEvent::new(event)`, so `without_correlation` disappears as a name while construction keeps happening through `new`. No accessor to remove; it was a public field. Update the 24 `without_correlation` call sites across 8 files, the single read (its own test), and three prose references: the type's doc comment and two lines in `persistence/mod.rs`'s module map, which would otherwise describe a field that no longer exists. `SemanticEvent::correlation_id` in `observability.rs` shares the name and is a different type — leave it alone.

      **The harness gap is fixed in the same slice, not left behind** — the removal and the prevention of another divergence are one contractual repair. Before this slice, `crates/testkit/src/event_store.rs` built 13 fixtures and called the then-existing two-argument `StoredEvent::new` zero times; every fixture used `without_correlation`, so the harness could not exercise the field — the same shape as the systemwide comparison, the unit-of-work offsets and the absent-stream report.

      The fix is **structural exhaustiveness, not a dynamic assertion**. The harness destructures a loaded event with no `..`:

      ```rust
      let StoredEvent {
          event,
          operation_key,
      } = loaded_event;
      ```

      A future field then **breaks the harness's compilation** until someone states how it must be verified. This matters because no runtime assertion can discover a new field in Rust: a round-trip check can only compare what it was written to look at, so it would stay green over exactly the kind of silent addition that produced this debt. `..` would defeat it entirely and must not appear here.

      Acceptance: no reference to `correlation_id` survives on `StoredEvent` or any store, both workspaces build and their gates pass, and the round-trip assertion fails if a store drops any remaining field.

      **Landed as `e41edc912b2f4ae771ea252b0d25aa97448020a1`**, squashed from PR #323 — 9 files, +90/−101. `StoredEvent::correlation_id` is gone, and so is `without_correlation`: the two constructors collapsed into the single `StoredEvent::new(event)`, and its 24 call sites across 8 files now go through that one. `ServiceContext::correlation_id` and `SemanticEvent::correlation_id`, which share the name and are a different concept, are untouched. The remaining acceptance conditions hold: no reference to the removed field survives on `StoredEvent` or any store, and the single read and the three prose references are updated.

      **The harness's exhaustiveness was proved, not asserted.** `crates/testkit/src/event_store.rs` destructures a loaded `StoredEvent` with no `..`, so the claim worth testing is that a future field breaks compilation instead of passing unnoticed. Temporarily adding `probe_field: Option<String>` to the struct made the build fail with exactly `error[E0027]: pattern does not mention field probe_field` — the destructuring caught it, not a runtime check, which is the distinction this task turns on. The probe was then reverted and the file restored byte-identically; the tree identity below is what shows nothing from it remained.

      All eight gates concluded with exit code 0 — `build`, `clippy`, `test` and `fmt` for the root workspace, and `build`, `clippy`, `fmt` and the suite for `integration-tests`. The integration suite passed 17 of 17 on its first attempt. The evidence is bound to the content rather than to a description of it: the tree identity computed before the first gate, after the last one, and in the merged commit is the same `c940da7e132c07e8e87a35f44adfbc2f29c7443a`.
- [x] B4.8 Update every existing `EventStore` caller (`EntityActor`, in-memory persistence adapter) for the new async signature; run full `cargo test --workspace` to catch ripple. **Run, not skipped**: 112 suites, 1 540 passed, 0 failed, 0 ignored, exit 0. This is the one task in the change whose text names that command, and it names it because a trait change of this shape is exactly what ripples somewhere nobody thought to look — a per-crate selection would have been the reviewer choosing which crates could break.

### Phase B5: Per-Aggregate `operation_receipts` (needs A2, A3, B4)

- [x] B5.1 RED **(retargeted by #274)**: extend `ego-testkit`'s shared `EventStore` conformance harness with the zero-event receipt scenario — a success producing no events still confirms its receipt inside the same transaction, and dropping that unit of work without committing leaves no receipt behind. Exercised in-process by `crates/infrastructure/tests/in_memory_event_store_conformance.rs` and `crates/persistent-entity/tests/default_store_conformance.rs`. It belongs in the shared harness rather than in a test of its own so the durable store answers the same definition later under #275, instead of a parallel copy of it (event-store + idempotent-command-processing spec scenarios). The original target `crates/integration-tests/tests/receipts.rs` no longer exists; #274 removed that crate. Real-PostgreSQL atomicity of the empty transaction is inventoried in #275 §6 and is not claimed here.
- [x] B5.2 GREEN: migration for `operation_receipts` at the next free number — `009` is taken by the `operation_key` column, and the numbers named in this plan are indicative rather than reserved, since B3 and B5 can land in either order + AD-1 partial-index pair on `(tenant_id, aggregate_type, aggregate_id, operation_key)`, storing the fingerprint.
- [x] B5.3a GREEN **(deferred here from B4.2b)**: add `confirm_receipt` to the `EventStoreUnitOfWork` trait. It was left out of B4-ii because nothing backed it — no table, no receipt type, no caller — and a trait method every implementation answers "not yet" is a premise without backing.
- [x] B5.3 GREEN: implement `confirm_receipt` on `EventStoreUnitOfWork` for both implementors, joining the same transaction as `append`.
- [x] B5.4 RED **(reframed by AD-3b/AD-3e)**: `crates/persistent-entity` test — the actor consults the receipt before dispatch, keyed on `(operation_key, fingerprint)` taken from `CommandContext` and never recomputed. A matching fingerprint replays without invoking `handle_command`; a mismatched one returns a permanent conflict without invoking `handle_command`. The replay must be observably a **replay**: the test asserts that post-commit effect acceptance is not re-entered, since a hit that rebuilt an ordinary `CommandResult::Events` would dispatch side effects the first execution already dispatched — the receipt would then prevent the state transition while permitting the duplicate it exists to stop (persistent-entity spec scenarios).
- [x] B5.5 GREEN **(reframed by AD-3c/AD-3e)**: add receipt-consultation gating in `crates/persistent-entity/src/actor.rs` before the `handle_command` call at line ~214, via `EventStore::find_receipt`. Return `CommandResult::Replayed { outcome }` — a public variant carrying the `AggregateOutcome` and **no state**, per the corrected AD-3c/AD-3e. It reconstructs neither the original result nor the original state: AD-3c records why neither is recoverable. Update every exhaustive match in the workspace explicitly, never with a `_` that could hide semantics, and give `RegisterUserImpl` concrete behaviour for it rather than letting current state stand in implicitly. A receipt that cannot be read is an internal error and never a re-execution; gap detection inside a range is **not** promised, because `EventStore::load` infers versions from positions and cannot prove it.
- [x] B5.6 RED **(reframed by AD-3c)**: zero-event branch test — `actor.rs:220`'s `CommandResult::NoEvents` path now opens a **real** unit of work to confirm `AggregateOutcome::NoEvents`, appending nothing (today it opens none — verified constraint 1). `NoEvents` is the only valid encoding of an empty range, so the test must also reject an `Events` range that describes nothing.
- [x] B5.7 GREEN **(reframed by AD-3c)**: change the zero-event branch to `begin()` → `confirm_receipt(AggregateOutcome::NoEvents)` → `commit()`, with no `append`. The `Ok(events)` branch must move off `persistence.persist_events(...)`, which owns and closes its own transaction, onto the same `begin()` → `append` → `confirm_receipt` → `commit()` sequence — otherwise the receipt cannot share the events' transaction, which is the whole point of B5. A command carrying no `operation_key` keeps the existing path and pays for no extra transaction.
      **Ordering is a static guarantee, not a tested one.** Four mutations were
      run against this slice and each died on observable behaviour: swallowing a
      confirmation error, omitting the confirmation on the `NoEvents` branch, and
      confirming in a second unit of work after the first committed (that one
      kills four tests). The fifth — committing *before* confirming, on the same
      unit of work — could not be written: `commit(self: Box<Self>)` consumes it,
      so the attempt fails to compile with `error[E0382]: borrow of moved value`.
      Recorded here as a guarantee the type system already provides rather than a
      mutation that was skipped, so a later change to `commit`'s receiver is
      understood to be removing a check, not tidying a signature.
- [x] B5.7a **(debt created by AD-3b, on already-merged code)**: `OperationReceipt` currently stores `ego_domain::operation::reservation::StoredResponse` — literally the reservation's type, imported across the two scopes AD-3b separates. Replace it with `AggregateOutcome`, rename the reservation's own to `StoredServiceResponse`, and rename migration `011`'s `response` column accordingly. This touches `crates/domain/src/operation/receipt.rs`, `011_create_operation_receipts.sql`, the Postgres adapter and the shared conformance harness — landed in `febeaaa`/`d5cd752` before the two scopes were distinguished. The two may share a byte representation; they share neither semantics nor ownership, and the shared name is how the scopes were merged in the first place.
- [x] B5.8 Restore A3.4/A3.5's schema-index assertion and extend it to the `operation_receipts` pair.

      **The target this task named no longer existed.** A3.4/A3.5's assertion lived in `crates/integration-tests/tests/schema_index_assertion.rs`, and `f8f5c37` (#274) deleted it wholesale along with the rest of that crate — every target that reached outside the process. So this was a reconstruction, not the edit the original wording ("update … to include the `operation_receipts` pair") presumed. Recorded because a reader checking the task against the tree would otherwise find nothing to update.

      Rebuilt as `integration-tests/tests/infrastructure/schema_index_assertion.rs`, in the independent `integration-tests/` workspace that now owns infrastructure-backed tests, and registered as a module of the consolidated `integration-tests/tests/infrastructure.rs` target. It takes the run's single PostgreSQL from the runner (`cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`) and a migrated, empty database of its own from `ego_integration_tests::isolated_database()`. It starts no container and does not reinstate the deleted `start_pool()`.

      **The registry went from one table to three.** The old version covered `events` alone, because the reservation and receipt tables did not exist when it was written. `EXPECTED_PAIRS` now pins `events`, `operation_reservations` and `operation_receipts`, asserting for each half the stable name, `indisunique`, the exact column list, the exact column order and the exact partial predicate — read from `pg_index`, `pg_class` and `pg_attribute`. The `unnest(i.indkey) WITH ORDINALITY` join is kept: without ordinality the column list comes back in join order and the ordering assertion would be vacuous rather than merely weaker.

      **The catalog, not the migration text.** Nothing here reads a `.sql` file. Matching migration source would only prove a file says what it says; these queries ask the server what it is actually enforcing, which is what the stores depend on.

      **Both A3.5 defences kept, because they catch opposite failures.** The explicit registry catches a table that should carry a pair and carries *no* index at all — invisible to discovery, since there is nothing to discover. Catalog discovery catches any table carrying only one half of a tenant/systemwide pair, including a table nobody registered. Discovery cannot pass vacuously: it asserts it found a pair on every registered table, so a query that silently matched nothing fails instead of reporting success over an empty map.

      **Mutation evidence.** Each mutation was applied to `011_create_operation_receipts.sql`, with its anchor text proven to match exactly once before being applied, and the file restored afterwards and confirmed byte-identical by SHA-256 and `diff -q`. Exit codes are the runner's, captured rather than inferred from screen output.

      | Mutation | Exit | Failed | Assertion that fired |
      |---|---|---|---|
      | both receipts indexes removed | 101 | 3 | registry: missing `ux_operation_receipts_identity_tenant`; discovery: non-vacuity anchor; complementarity: no catalog predicate to partition by |
      | systemwide half removed only | 101 | 3 | registry: missing `ux_operation_receipts_identity_systemwide`; discovery: lopsided pair; complementarity: missing half |
      | two tenant-half columns transposed | 101 | 2 | registry: exact key order; discovery: normalised identities differ |
      | tenant half made non-unique | 101 | 2 | registry: must be UNIQUE; discovery: lopsided pair, the half having dropped out of the `indisunique` filter |
      | systemwide predicate flipped to `IS NOT NULL` | 101 | 3 | registry: exact partial predicate; discovery: lopsided pair; complementarity: non-zero overlap |
      | expression key `lower(fingerprint)` appended to the systemwide half | 101 | 2 | registry: keys must be plain columns; discovery: normalised identities differ |
      | `INCLUDE (fingerprint)` added to the tenant half | **0** | 0 | none — **negative control**, uniqueness is unchanged so green is the correct answer |
      | an **unregistered** table given two halves guarding different identities | 101 | 1 | discovery alone: an index under each predicate, but no two guarding the same identity |
      | `tenant_id` added to the **registered** systemwide half | 101 | 2 | registry: exact keys; discovery: a systemwide half may not key on the discriminator |
      | an **unregistered** table whose *both* halves key on `tenant_id` | **0 → 101** | 1 | discovery alone; **was a false green** before this round |
      | an **unregistered** table with a valid pair plus an orphan tenant half | **0 → 101** | 1 | discovery alone; **was a false green** before this round |

      The last two rows are recorded as `0 → 101` because both were **measured green** against the previous candidate's test code before the fix and 101 after it, with the migration restored byte-identically between runs. A claim that a mutation "would have passed" is worth nothing unless it was run; these were.

      Unmutated, the suite is green at 20 tests (exit 0) — the 17 that preceded this slice plus these three. The last three rows carry the argument the others cannot: the expression-key row is a false green that a shape assertion has to close, the `INCLUDE` row is the mirror false red it must not create, and the unregistered-table row is the only one the registry cannot see at all, so it is discovery alone earning its place in the file.

      **Two review rounds, and the matrix above is what caught both.**

      *Round two — what a catalog read has to look at.* The key list was assembled by `unnest(i.indkey) WITH ORDINALITY` joined to `pg_attribute` on `attnum`. PostgreSQL stores **`0`** in `indkey` for a key that is an expression, and `pg_attribute` has no `attnum = 0`, so an inner join dropped that element without a trace. Appending `lower(fingerprint)` to a pair half therefore narrowed the identity — two receipts alike in every registered column could coexist — while the reported column list was unchanged and the "exact columns" assertion passed. A false green against the guarantee this file exists to pin, not merely thin coverage for a kind of index the schema does not yet use. The join is now outer, the element holds its position as `(expression)`, `indexprs` is read as a second independent signal, and `pg_get_indexdef` is carried so a failure shows the server's real definition. The same read also separates key columns from `INCLUDE` payload by `indnkeyatts`: only the first `indnkeyatts` entries enforce uniqueness, so comparing the whole vector would have failed on an added `INCLUDE` column that changes nothing — the mirror mistake, and the negative-control row above proves it is not made.

      *Round two — what pairing has to compare.* Discovery asked only whether *some* index existed under each tenancy predicate. That admitted a table whose two halves guard **different** identities — `UNIQUE (tenant_id, aggregate_id) WHERE tenant_id IS NOT NULL` beside `UNIQUE (operation_key) WHERE tenant_id IS NULL` — two halves, one per predicate, and no coherent guarantee between them. The registry catches that on the three tables it names, which is exactly why discovery has to catch it on the tables nobody named: that is discovery's entire reason to exist, and it was the part not working. Halves are now paired by identity — strip `tenant_id` from the tenant half and require what remains to equal the systemwide half's keys exactly — and the final matrix row exercises it on a table the registry has never heard of.

      *Round three — normalising both sides the same way, and pairing existentially.* Two further false greens, both in discovery. The normaliser stripped `tenant_id` from whichever half it was handed, so `UNIQUE (tenant_id, aggregate_id) WHERE tenant_id IS NULL` normalised to the same list as its tenant partner and was accepted as a pair. But that systemwide half keys on a column its own predicate fixes to NULL for every row it contains, and PostgreSQL treats NULLs as **distinct** in a unique index — so it collides nothing and duplicate identities are permitted across the entire systemwide partition. The mirror mistake passed too: a tenant half with no `tenant_id` key normalises identically while enforcing one identity globally across all tenants instead of once per tenant. Each side is now checked on its own terms — the tenant half requires `tenant_id` exactly once and leading, and removes only that position; the systemwide half must not carry it at all. Separately, pairing asked only whether *some* matching pair existed, so one good pair satisfied it and hid every orphan beside it, contradicting the test's own name. Every half must now have a counterpart, in both directions, and unpaired halves are named in the failure.

      *Round one — the tautological partition test.* The complementarity test originally hardcoded `tenant_id IS NOT NULL` and `tenant_id IS NULL` into its `FILTER` clauses. Those are exhaustive and mutually exclusive by definition, so its two partition assertions were tautologies that held for any table with a `tenant_id` column — including one with no indexes at all — while its doc comment claimed the check ran against the server rather than by reading the predicates. The evidence was already visible here: an earlier version of this matrix attributed every detection to the registry and discovery tests and **none** to the complementarity test, which is exactly what an inert assertion looks like from outside. The predicates are now read from `pg_get_expr` for the table's two named indexes and substituted back into the query, which is why the predicate-flip row now reports three failures instead of two.

      One thing the reconstruction states that the original could not: the runner provisions PostgreSQL 16, while the declared floor of 14 is the entire reason the two-index strategy exists. The assertions pin the strategy rather than the server, and read no version-specific catalog column, so they mean the same thing on either.

### Phase B6: `#[idempotent]` Marker + Slot-3 Wiring — Closes the Live Bug (needs B1, B2, B3, B5)

- [x] B6.1 RED: `crates/service-sdk-macros/src/tests.rs` — `#[idempotent]` outside `#[service]` is a compile error; `#[idempotent]` without `#[operation]` is a compile error (mirrors the existing check at `lib.rs:528`).
- [x] B6.2 GREEN: add the inert `#[idempotent]` marker attribute in `crates/service-sdk-macros/src/lib.rs`, read by the `#[service]` generator alongside `#[authorize]` (`lib.rs:808`) and `#[tenant_scoped]` (`lib.rs:824`).
      **Found while implementing, and part of this task rather than a follow-up:**
      `OperationDescriptor::idempotent` already existed and was emitted as a
      literal `false` for every operation the generator produced. The field is
      serialised and exposed through `ServiceContract`, so leaving it hardcoded
      would have made the new marker exist syntactically while remaining
      invisible to every consumer of the contract. It is now populated from the
      marker, with `crates/service-sdk/tests/idempotent_descriptor.rs` covering
      both directions — marked reports `true`, unmarked still reports `false` —
      so neither a dead flag nor a default-everything-idempotent regression can
      pass unnoticed.
- [x] B6.3 RED: macro-expansion test asserting generated slot ordering — slot 1 `#[authorize]`, slot 2 `#[tenant_scoped]` (`enforce_tenant`), slot 3 the new reservation call — and that slot 3 never runs before a passing guard (design.md AD-5, spec scenario "Reservation happens after authorization and tenant scoping").
      The blocking behavioural criteria this box was held open for are now green
      in `crates/service-sdk/tests/idempotent_dispatch.rs`, and every one of them
      is stated as an observed count rather than as a shape: authorization denied
      -> `reserve` called 0 times; tenant rejected -> 0 times; both guards passing
      -> exactly 1; the definitive key, canonical tenant and fingerprint are read
      back off the request the store actually received; a refused reservation
      leaves the handler body at 0 calls.
      **The mutation that closes it.** Reverting `idempotency_slot` to
      `quote! {}` — the empty seam this box previously described — fails 9 of the
      19 tests, each by an observed count or a returned value, none by a list
      comparison. The 10 that survive are the guard-ordering ones (which pass
      trivially when nothing reserves at all), the pure fingerprint unit tests,
      and the two "legitimately did not reserve" cases — which is why they are
      not the evidence and the other 9 are.
- [x] B6.4a GREEN: bridge the service and aggregate contexts — after a successful
      reservation, the service body reads the definitive `OperationKey` and
      `OperationFingerprint` from `ServiceContext` and threads those exact values
      into every `CommandContext` it creates. **The generated slot does not
      construct aggregate contexts and must not recompute or independently
      propagate the identity.**
      **This closes the gap B1 left open**: B1 made both contexts able to hold the
      key and proved traversal from the command envelope onward, but nothing
      joined the two halves.
      **Where the responsibility now sits**, fixed by the reservation boundary that
      landed with the slot. An earlier revision of this box put the bridge in
      generated slot-3 code, which was correct only while the slot was the sole
      place that had read the resolved key. It is not any more, and the split is:
      - slot 3 computes the fingerprint over the typed arguments (AD-3f);
      - the runtime reserves, and stamps the accepted key and fingerprint onto
        `ServiceContext` — only after the store accepts;
      - the service body reads that already-authorised identity back;
      - the service body transfers it into each `CommandContext` it creates;
      - the receipt gate consumes exactly those values.
      The slot cannot do the transfer: it does not know how many aggregates a
      service body will touch, or when. Putting it back there would mean the macro
      constructing aggregate contexts on the body's behalf.
      **The bridge must not live in a transport adapter.** It belongs to the dispatch
      path every transport shares, so each adapter decides only how to *extract* the
      key while everything from `ServiceContext` inward is one identical path.
      **The criteria, each stated as something observed rather than present:**
      - multi-aggregate: every `CommandContext` receives the *same* key and
        fingerprint, and both match what the store was actually handed — asserted
        against the store's recorded request, not against a value the test computed;
      - the fingerprint is never recomputed downstream; the only value that reaches
        an aggregate is the one the reservation stamped;
      - `Replay` and every rejection construct **zero** `CommandContext` — the body
        does not run, so nothing downstream is reached at all;
      - a dispatch that legitimately did not reserve (no key, or no reservation
        store) leaves both values absent and does not activate the receipt gate;
      - the transfer is killed by mutation, per the scenario's own criterion below.
      **BLOCKING ACCEPTANCE CRITERION — the multi-aggregate recovery scenario.**
      Promoted here from B5, and deliberately not left as a generic follow-up: it
      is the scenario that justifies the whole receipt layer, and it cannot run
      until this bridge exists. B5 proved the mechanism *locally* — prior lookup,
      replay, permanent conflict, error propagation, no fallback and no writes,
      all in `crates/persistent-entity/tests/receipt_gating.rs`. What it could not
      prove is the integration, because `RegisterUserImpl` builds
      `CommandContext::new(..)` for both entities with no key and no fingerprint
      (`application.rs:250`, `:287`), so the gate never fires there. Wiring that by
      hand would have tested a transient integration different from the specified
      architecture: slot 3 → the reservation's stamp on `ServiceContext` → the
      service body → each entity's `CommandContext` →
      the receipt gate. The scenario:
      - one service `operation_key`, with the per-aggregate identity derived from it;
      - an existing receipt for `tenant_organization`, a miss for `user`;
      - organization returns `Replayed` and its handler does not run;
      - its read-side is not republished and its effects are not accepted again;
      - the workflow continues to the user step;
      - the user handler runs **exactly once** and confirms its own receipt;
      - `RegisterOutput` completes without presenting current state as a historical result;
      - **a mutation dropping the transfer into either `CommandContext` must kill
        the test — and the *second* aggregate is the one that matters.** The first
        is what a partial implementation gets right by accident; only the second
        proves the transfer is systematic rather than a single wired-up call site.
        Without this, the test proves the bridge exists rather than that it works.
      Implementing it inside the axum layer would make the actor's idempotency
      accidentally HTTP-shaped, and the second adapter would then need its own copy.
      **Done.** The identity travels as one indivisible value:
      `ego_domain::operation::OperationIdentity` pairs the key with the
      fingerprint, `CommandContext` carries one `Option<OperationIdentity>` in
      place of the two fields it had, and `ServiceContext::operation_identity()`
      answers `Some` only when a reservation actually stamped one. The two
      defensive `(Some(key), Some(fingerprint))` pairings the actor used to do at
      each read site are gone — the type made them unnecessary, which is the
      evidence it named a real concept rather than adding one.
      Two runtime tests that asserted the gate ignored a half identity were
      replaced by a compile-fail fixture: the state is no longer expressible,
      which is stronger than checking it is tolerated.
      `RegisterUser::register` is now `#[idempotent]`, which is what makes any of
      this observable end to end, and its error type gained
      `Refused(ReservationRejection)` split by who can act — 409 for contention
      and a conflicting fingerprint, 503 for an unreachable store, 500 for the
      two that reproduce identically on retry.
      `build_runtime` keeps its `Compatibility` declaration and registers no
      store; the scenario wires its own, which is not the app claiming adoption.
      **Both mutations run.** Dropping the transfer into the organization fails
      the scenario on its receipt lookup (0 vs 1); dropping it into the user
      fails on the user's. The second is the one that matters.
      Verified: fmt, integration guard, `clippy -D warnings` and
      `cargo test --workspace --no-fail-fast` all 0 — 115 targets, 1597 tests.
- [x] B6.4 GREEN: emit slot-3 codegen: `store.reserve(CanonicalTenant, OperationKey, fingerprint, owner, lease_until)`; branch on `Fresh`/`TakenOver` → continue, `Succeeded` → return stored response without invoking the handler, `Conflict` → permanent conflict, `*InProgress` → contention response.
      **Done.** Slot 3 emits one `?`-terminated call to
      `RuntimeInner::reserve_idempotent_operation`; the store access and the
      six-way branching stayed in `ReservationConfig::reserve` (AD-3g). The
      fingerprint is a SHA-256 digest of a tagged, length-prefixed canonical
      encoding of the typed arguments, built in `operation_fingerprint` rather
      than borrowed from `serde_json`'s map ordering — that ordering is not a
      stable property of this workspace, because `preserve_order` is an additive
      feature any crate in the graph can switch on, and a `HashMap` argument
      field would then hash in random iteration order. The digest is also what
      keeps the value inside `fingerprint VARCHAR(255)` for any payload size.
      `#[idempotent]` gained a third public obligation — every fingerprinted
      argument must be `Serialize` — with its own isolated `compile_fail`
      fixture, so no one of the three can be dropped and stay green.
      **Two things this task forced, recorded as AD-3j amendment 2:** a sixth
      rejection, `RequestNotFingerprintable`, because serialising a user type is
      fallible and the alternative was running a marked operation unreserved;
      and `Option<ReservationDecision>` on the success side, so "this runtime
      does not reserve" cannot be confused with a `Proceed` that carries a fence.
      **Not covered here, and stated so it is not mistaken for covered:** the
      keyless request. Slot 3 reserves only when the context carries a key; the
      missing-key policy stays with `resolve_operation_key`, and the exposure of
      a transport that never calls it closes in B6.5/B6.6.
      **Fingerprint contract fixed by AD-3f — read it before writing the
      canonicalisation.** The fingerprint is computed here, in slot 3, over the
      operation's already-deserialised typed arguments: not raw transport bytes,
      not JSON shape or field order, and not after the handler's own
      transformations. It covers the semantic input only and excludes
      `operation_key`, owner, lease, trace and correlation ids — those describe
      the attempt, not the request, and folding them in would make every retry
      look like a different request. The property to test: two syntactically
      different requests that deserialise to the same typed values produce the
      same fingerprint, and two different typed values produce different ones.
      **Shape fixed by AD-3g.** Slot 3 emits one `?`-terminated call to a public
      runtime method; the store access and the six-way outcome branching live in
      `service-sdk`, not in generated code — one source of truth, testable where
      it lives, mirroring `enforce_tenant`. The method must expose a
      dispatch-oriented result rather than the store's own outcome type, so how
      each outcome is translated stays private. The runtime receives tenant, key
      and fingerprint already definitive; canonicalisation and fingerprinting
      belong to the generated code under AD-3f.
      While implementing, update the now-obsolete annotation on
      `RuntimeInner::operation_reservation_store` — its
      `expect(dead_code, reason = "called by #[idempotent] dispatch, landing in
      B6")` describes a call that AD-3g means will never happen. It stays
      `pub(crate)`.
      **Six outcomes, not five — see AD-3h.** `ReservationOutcome` has
      `Fresh`, `TakenOver`, `OwnedInProgress`, `OtherInProgress`, `Succeeded` and
      `Conflict`. Only the first two continue. `OwnedInProgress` blocks like
      `OtherInProgress`: fencing proves ownership, not exclusion between two
      executions of the same owner, so it cannot tell a legitimate recovery from
      a concurrent retry or from the previous execution still running. Recovery
      happens by waiting for the lease to expire and taking over with a greater
      token, not by re-entering. The runtime's branching test must cover all six.
      **Runtime state this needs first — AD-3i.** `ReserveRequest` demands an
      `owner_id` and a `lease_until` that `RuntimeInner` does not hold. Add
      a single `Option<ReservationConfig>` holding the store, an `Arc<dyn Clock>`
      (injectable, real by default), an `OwnerId` (UUID minted once in `build()`,
      unique per instance, injectable for tests) and a `lease_duration`
      (configurable, strictly positive, default 30s). **No `Option` inside the
      struct**: two representable states, not sixteen. It also keeps
      `new_with_logger` at eleven positional parameters instead of sixteen, where
      transposing two `Option<Arc<dyn …>>` arguments compiles and fails at
      runtime. Seven constructor call sites, not the 71 that `build` has. Without an injectable clock and owner,
      `OwnedInProgress`, `OtherInProgress` and `TakenOver` cannot be exercised
      deterministically — the branching test would depend on wall time, which is
      what A4 generalised the clock out of auth to avoid.
      **Boundary types fixed by AD-3j.** The runtime method returns
      `Result<ReservationDecision, ReservationRejection>`, and `#[idempotent]`
      requires `UserError: From<ReservationRejection>` at compile time — a
      `trybuild` case must prove that bound is enforced with a precise message,
      or the requirement lives only in the design doc. The `Ok` side has two
      shapes because `Succeeded` is a replay, neither permit nor rejection:
      `Proceed(permit)` for Fresh/TakenOver, `Replay(response)` for Succeeded.
      `ReservationRejection` carries four distinguishable cases —
      `SelfInProgress`, `OtherInProgress`, `FingerprintConflict`,
      `StoreUnavailable` — not flattened to a string, because "retry shortly"
      and "never retry" must not require parsing prose to tell apart.
      **The replay path is blocked on AD-3k and must not be half-wired.**
      `Replay` promises a typed `UserOutput`; the stored response is bytes. One
      codec owns `encode` and `decode`, JSON with an explicit envelope tag, and
      B6.8's epilogue must use that same codec rather than a parallel
      serialisation — the reader lives here and the writer lives there, so
      defining either alone fixes the format from the side with less
      information, and a mismatch fails on the first real retry in production
      rather than at compile time. `#[idempotent]` gains a second public bound,
      `UserOutput: Serialize + DeserializeOwned`. AD-3j is amended: a fifth
      rejection, `StoredResponseIncompatible`, because an undecodable stored
      response is neither `StoreUnavailable` (the store answered correctly) nor
      `FingerprintConflict` (the request is the one that succeeded). Permanent
      for the caller, recoverable by an operator.
- [x] B6.5 RED: HTTP-level test (`crates/transport`) — missing/invalid `Idempotency-Key` rejected before the guarded operation runs; valid key surfaces identically on `ServiceContext` (http-transport spec scenarios).
      **A prerequisite neither box named, found while wiring it.** The builder
      validated `IdempotencyEnforcementMode` at startup — refusing `MandatoryKey`
      with no reservation store — and then **discarded the value**. Nothing
      downstream could read the policy the build had been checked against, so a
      transport had no way to apply it. Retaining it on `RuntimeInner` is not
      accidental scope: it is the minimum state required for the boundary to
      enforce the same policy the runtime promised, rather than a second copy of
      the configuration that could drift from it.
      **The policy table keeps its single owner.** The extractor reads the mode
      and passes it to `resolve_operation_key`; it never matches on it. A
      `MandatoryKey`/`Compatibility` match inside the transport would be a second
      definition of the rule, and the copy deciding whether a real request is
      rejected would not be the one the builder validated.
      **Correction to a stated criterion:** the default is **`MandatoryKey`**,
      not `Compatibility` — fail-closed, so a caller who never considered
      idempotency does not silently get none. A bare `RuntimeBuilder::new()`
      cannot even `build()`: the validation refuses it for having nowhere to
      reserve. Pinned in `the_runtime_reports_the_mode_it_was_built_under`.
- [x] B6.6 GREEN: wire the HTTP carrier + `resolve_operation_key` at the axum layer ahead of the guarded operation.
      **Done.** `OperationKeyExtractor` (`crates/transport/src/operation_key.rs`)
      joins the three pieces that already existed and were never connected: the
      header carrier, the shared policy table, and the runtime's retained mode.
      Shaped after `TraceContextExtractor` — boundary work done once, handlers
      declare it — except that its rejection cannot be `Infallible`, since a
      missing key under an enforcing runtime must stop the request before the
      operation. Every rejection maps to `400`: the request is unusable, and
      `401`/`403` would send a caller looking for a fix in identity or
      permission, where nothing failed.
      The handler only transfers the result, and only when there is one — a
      `None` stays `None`, because inventing a key would manufacture an identity
      the caller never supplied.
      **Both mutations bite.** Hardcoding `Compatibility` in the extractor
      instead of reading the runtime kills the mandatory-key test; dropping
      `.with_operation_key(..)` in the handler kills the carriage test.
      The second one is the reason a second test file exists: the extractor's own
      tests all still pass under that mutation, because the extractor is not what
      broke. Only an assertion on what the **service received** — the real router
      driven end to end against a recording `RegisterUser` — can tell a working
      transfer from a dropped one.
      **The same separation applies to the mode, and had to be closed too.**
      Review found the central claim — a missing key under `MandatoryKey` is
      refused before the operation runs — proven only by calling
      `from_request_parts` directly. That establishes the extractor's policy, not
      the router's behaviour, and says nothing about whether the operation ran.
      The router-level file is now parameterised by mode and covers it with three
      observations, because a status code alone cannot distinguish "refused
      before dispatch" from "refused after running": **400**, the service
      recorder still `None`, and **zero** reservations — a refusal at the
      boundary leaves no lease behind for a legitimate retry to contend with.
      Its control uses the same enforcing runtime and the same store with a key
      present, so the refusal is attributable to the missing header rather than
      to an enforcing runtime rejecting everything. That control also observes
      one `reserve` **and one `complete`**, which is B6.8's epilogue running
      through the HTTP path: the header a client sent reaching the reservation,
      the operation, and the completion that makes the next identical request
      replayable.
      Hardcoding `Compatibility` in the extractor now fails at the **router**
      level, so one mutation demonstrates the whole chain rather than an isolated
      component.
      Verified: fmt, integration guard, `clippy -D warnings` and
      `cargo test --workspace --no-fail-fast` all 0 — 117 targets, 1618 tests.
- [x] B6.7 RED: replay vs. conflict HTTP response test — same key/same fingerprint returns the original stored response unexecuted; same key/different fingerprint returns a distinguishable permanent-conflict response (http-transport spec scenarios).
      **Done**, in `examples/reference-app/tests/http_replay_and_conflict.rs`,
      driving the real router with a scripted reservation store.
      **Response bodies are not the evidence, and must not be.** `RegisterOutput`
      copies `input.user_id` and `input.tenant_id` verbatim, so two identical
      requests produce byte-identical responses whether the body ran once, twice
      or not at all — an assertion that cannot fail. Instead the store returns a
      response **the handler could not have produced from this input**
      (`user_id: "REPLAYED-FROM-THE-STORE"`), so its arrival over HTTP is proof
      the answer came from the store rather than from a second execution.
      **Scope widened deliberately.** B6.7 asks for replay versus conflict, but
      the six-way refusal mapping landed in #280 with no HTTP-level test at all,
      so the whole public table is closed here rather than leaving branches
      uncovered in the same file. Every row asserts a status **and** three
      counts — reserves, handler invocations, completions — because a status
      alone cannot show whether the operation ran first:
      replay → 201 with the marked value, 0 body, 0 complete;
      `Conflict`, `OwnedInProgress`, `OtherInProgress` → 409;
      `StoreUnavailable` → 503; `StoredResponseIncompatible` → 500; each with 1
      reserve, 0 body, 0 complete.
      A `Fresh` control runs the same wiring permitted — 1 reserve, 1 body,
      1 complete — so the zeros above are attributable to the refusal rather
      than to a fixture that rejects everything.
      **`RequestNotFingerprintable` is absent from the router-level file** — it is
      raised before the store is reached, when an operation's arguments fail to
      serialise, and `RegisterInput` always does, so no store script can provoke
      it. Its translation is proven directly against the mapper instead, in
      `handlers.rs`'s own unit test, which enumerates **all six** rejections
      rather than only the unreachable one: a table with one entry proven
      elsewhere and five assumed is how a mapping drifts. That test also pins
      `StoredResponseIncompatible` → 500, the one row the router file and the
      mapper both cover, so the two cannot disagree silently.
      **Three mutations bite:** mapping `FingerprintConflict` to 500 kills the
      table; mapping `StoreUnavailable` to 500 kills the 503; and letting the
      replay branch fall through to the handler kills the marked-value test —
      returning the recomputed `user-1` instead, which is exactly what a
      body-comparison assertion would have missed.
      **A fourth was not fabricated.** "Complete on a replay" is not writable:
      `ReservationDecision::Replay` carries a `StoredServiceResponse` and no
      permit, so there is no `OwnerFence` to complete under. That zero is
      guaranteed by the split #279 introduced precisely to make a permit-less
      completion unrepresentable. The assertion stays as a regression tripwire
      against a future degradation of the type, not as primary evidence.
      **Corrected during review:** the production comment grouping
      `StoredResponseIncompatible` with `RequestNotFingerprintable` claimed
      retrying either "reproduces it exactly". True of stored bytes, which do not
      change; not true of a fingerprint failure, where `Serialize` is satisfied
      at compile time and what failed is *this value* — a hand-written impl may
      fail on one value and succeed on the next. Same overstatement withdrawn
      from the epilogue's docs in #281. Both still map to 500, chosen for the
      action they imply rather than for a prediction about recurrence.
      Verified: fmt, integration guard, `clippy -D warnings` and
      `cargo test --workspace --no-fail-fast` all 0 — 118 targets, 1622 tests.
- [x] B6.8 GREEN: implement the slot-3 epilogue — `store.complete(op_id, owner, fencing_token, response)` as a conditional update; stale completion discards the response and does not overwrite state.
      **Done.** The epilogue is one call to a public runtime method, same shape
      as the reservation itself (AD-3g): `RuntimeInner::complete_idempotent_operation`
      encodes through the AD-3k codec — the same one the replay path reads with,
      asserted by round-trip rather than by pinning bytes — and completes under
      the fence the permit carries. It runs only on `Ok`: a failed operation has
      no answer to record, and recording one would tell the next identical
      arrival the work is done. Its lease is left to expire so a retry can take
      it over, which is why the test store still panics on `abandon`.
      **A failed completion does not fail an operation that succeeded.** By the
      time the epilogue runs the handler returned `Ok` and every aggregate
      committed, so reporting an error would describe successful work as a
      failure and invite a retry of something that must not run twice.
      **What that costs, stated precisely** — not "the retry just re-runs the
      body", because the reservation stays open and there is a window:
      the durable work stays successful; the immediate replay is lost, since the
      reservation never reached `Succeeded`; **while the lease is still valid,
      retries are refused as in progress** (`SelfInProgress`/`OtherInProgress`,
      AD-3h) and do not reach the body, which is a real unavailability window as
      long as the configured lease; once it expires a retry takes ownership
      (`TakenOver`) and does reach the body; and the per-aggregate receipts then
      stop each already-confirmed durable step from happening twice.
      **Scope.** That last step protects what the receipt protocol covers —
      durable aggregate writes and effects made idempotent through it. It says
      nothing about an arbitrary external side effect performed outside that
      protocol, and nothing here makes such an effect safe to repeat.
      Because the loss is invisible to the caller by design, it is emitted as an
      `idempotency.completion_lost` semantic event through the same
      `Observability` sink and panic isolation as `record_security_denial`. Each
      carries a `reason` and an `action`, because the three call for different
      responses and an operator should not have to infer that from the tag. What
      travels is the action, deliberately **not** a claim about whether the
      failure would recur — that is not something this code can establish.
      `store_unavailable` → `monitor_rate`: the ordinary contingency, where one
      occurrence is noise and a rate is a problem.
      `stale_owner` → `review_lease_duration`: this response was discarded
      because the caller no longer held a current fence. That is all the contract
      guarantees. It does **not** say another owner completed the operation —
      another owner may have taken over and still be running, or already
      completed, or the lease may simply have expired with no takeover at all,
      and what the reservation looks like now is not knowable from this error.
      Worth acting on regardless, because every path here means the lease elapsed
      before the work did. The lease window described above also does not apply
      as written to this case: the reservation is not necessarily still open
      under this owner.
      `not_encodable` → `investigate`: this *value* failed to serialise.
      `T: Serialize` is satisfied at compile time, so this is not "the type
      cannot be serialised" — a hand-written `Serialize` may fail on one value
      and succeed on the next. It is a judgement that waiting is not a justified
      recovery strategy — the failure requires investigation — not a proof that
      the failure recurs. Until someone looks, that operation does not reach
      `Succeeded`.
      **Where the chain is proven.** Each link lives in the layer that owns it:
      a failed completion leaving the handler's `Ok` intact, in this unit's
      `a_stale_completion_does_not_fail_the_operation` and
      `an_unreachable_store_does_not_fail_a_completed_operation`; mid-lease
      refusal and post-expiry takeover, in the shared
      `assert_reservation_store_conformance` suite every store implementation
      runs; in-progress stopping dispatch, in
      `every_refusal_stops_dispatch_and_arrives_as_itself`; and a `TakenOver`
      retry not repeating an already-receipted step, in
      `register_user_multi_aggregate_recovery`.
      **The two `expect(dead_code)` annotations this was waiting for are gone.**
      `ReservationPermit::fence()` and `ReservationConfig::store()` now have
      production callers, and because they were `expect` rather than `allow`,
      `unfulfilled_lint_expectations` forced their removal rather than leaving
      them as stale notes.
      Seven behavioural tests, each an observed count on the store: a completed
      operation records once under the permit's fence and round-trips; a failed
      one records nothing though its body ran; replay, refusal and an unreserved
      dispatch record nothing; and neither `StaleOwner` nor an unreachable store
      turns a succeeded operation into a failure.
      **Mutation.** Emptying the epilogue kills the three tests that assert a
      completion happened. The four "records nothing" tests survive by design —
      they are negative controls and cannot detect an absent epilogue, which is
      why they are not the evidence.
      Verified: fmt, integration guard, `clippy -D warnings` and
      `cargo test --workspace --no-fail-fast` all 0 — 115 targets, 1607 tests.
- [x] B6.9 RED: `examples/reference-app/tests/e2e_register.rs` — retried `POST /register` with the identical `Idempotency-Key` and payload produces exactly one `UserRegistered` and one welcome-email effect (reference-service spec — **closes the `UserEntity` bug end to end**, distinct from and layered on top of B0's defensive fix).
      **A third property, observable only since B6.8: the second `POST` must be
      answered by a replay, not by a second execution.** Until the epilogue
      existed no reservation reached `Succeeded`, so a retry could only ever be
      answered by re-entering the body — "one `UserRegistered`" and "one effect"
      were both satisfiable with zero replay, because each aggregate's receipt
      answered for its own step.
      **Comparing the two response bodies does not establish this, and must not
      be the evidence.** `RegisterOutput` is built by copying `input.user_id` and
      `input.tenant_id` verbatim (`application.rs:357`), so two identical
      requests produce byte-identical responses whether the body ran once, twice,
      or not at all. That assertion cannot fail, which makes it worth nothing.
      What must be observed instead:
      - `RegisterUserImpl::register`'s body runs **exactly once** across both
        requests — counted in the implementation, not inferred from the response;
      - the store records **one** `complete()` after the first `POST`;
      - the second `POST` reads `Succeeded` and issues **no** second `complete()`;
      - and the replayed value is tied to the store rather than to the request:
        either compare it against the bytes the store itself recorded, or have
        the store return a marked response the handler could not have produced.
      A test that asserts only "both responses are equal" has replaced one
      un-failable box with another, which is the specific failure this note
      exists to prevent.
- [x] B6.10 GREEN: prove the marker governs the **real HTTP path**, and verify
      B6.9 passes.
      **Restated, because its original work is already done.** This box used to
      read "mark `RegisterUserImpl`'s handler(s) with `#[idempotent]`". That
      happened in #280: B6.4a needed the marker for the reservation to stamp an
      identity at all, so without it the multi-aggregate scenario had nothing to
      observe. Leaving the box worded that way would invite someone to re-apply
      an attribute that is already there and call the unit closed, which proves
      nothing about the transport.
      What remains is the part the marker alone never established: that a request
      arriving over HTTP is governed by it. B6.9's e2e is the evidence — the
      claim is not "the attribute is present" but "the second identical `POST`
      reserved under the key the header carried, replayed rather than executed,
      and produced no second effect". A mutation removing `#[idempotent]` from
      `register` must break that e2e, or it demonstrates the transport reaches
      the operation rather than that idempotency governs it.
      **The store the e2e wires is the test's, not the app's.** `build_runtime`
      declares `Compatibility` and registers no reservation store on purpose —
      recorded at `examples/reference-app/src/lib.rs:286`, with a migration
      behind it, because an in-memory store there "would make this look adopted
      while giving no durability at all". The e2e must inject its own, the way
      `register_user_multi_aggregate_recovery` does, and must not edit that
      declaration. A test wiring a store is not the application claiming
      adoption, and B6.9/B6.10 must not blur the two.
- **B6.9/B6.10 closing note.** Both boxes are satisfied by `integration-tests/tests/replay_from_postgres.rs`, not by the originally named `examples/reference-app/tests/e2e_register.rs`, which no longer exists — the scenario moved to where the replay is genuine instead of scripted.
  **B6.9's two remaining observations, added as independent ones.** A recording `ReadSideSink` over a `SharedReadSideStore` counts exactly one `UserRegistered` published by the write side, read back through the `ReadSideStore` trait the store already implements (nothing new exposed; tags discovered via `known_tags()` because the tag helper is crate-private). A decorator over `EffectAcceptor` counts exactly one accepted `user.welcome_email` keyed `welcome-email:user-1`, delegating to the real acceptor. Both stay at one after the replay.
  **Why acceptance and not delivery.** `accept()` runs synchronously in post-commit, so the count is settled before the response returns. Delivery goes through the deferred runner on its own reclaim tick, so counting there would mean waiting and then asserting nothing else arrived — an absence claim dressed as a timeout.
  **Two observations, not one wearing two hats.** Both descend from the same commit, which is the causal claim itself, but neither substitutes for the other: one says the event was committed and published, the other says that commit produced one accepted effect.
  **Measured.** With `#[idempotent]` removed from `register`, the scenario fails, and a probe past the reservation assertions recorded `bodies=2 events=2 effects=2` — both counts double, which is the duplicate the marker prevents. Marker restored, suite green.
- [x] B6.11 RED: reference-app test enumerating every mutating operation and asserting each carries the `#[idempotent]` marker (design.md Risks — mitigates the marker-completeness residual gap). Landed as `examples/reference-app/tests/idempotent_marker_completeness.rs`.
- [x] B6.12 GREEN: add the enumeration/assertion helper and apply the marker to any operation the test finds missing it. **No operation was missing it** — the application publishes one service trait with one operation, already marked. The value delivered is the guard against the next one, not a defect found today, and that is worth stating rather than reporting a fix that did not happen.
  **What the guard establishes, and where it stops.** The marker is read from the generated contract (`ServiceContract::operations()`), so it reflects what the macro produced rather than the presence of an attribute that may not have expanded. Operation-level completeness is compared against that contract in both directions, so a new operation on an existing trait fails until classified and a stale entry fails until removed. Trait-level completeness is only a **source-text tripwire**: nothing in the runtime enumerates registered services, so the test counts `#[service` declarations under `src/` and requires the count to match the inventory. That correlates with completeness instead of establishing it, and it is documented as such in the test rather than left to be discovered. Closing it properly needs a registration mechanism operations enrol themselves in — a larger change than recording the judgement.
  **Measured, four mutations.** Removing `#[idempotent]` from `register` fails the marker assertion; emptying the inventory fails the published-but-unlisted assertion; adding a phantom entry fails the stale-entry assertion; adding a second `#[service]` trait fails the tripwire at `2 != 1`.
  **Bearing on B6.10, not B6.9.** The marker-removal requirement — "a mutation removing `#[idempotent]` from `register` must break that e2e, or it demonstrates the transport reaches the operation rather than that idempotency governs it" — is stated in **B6.10**'s block, and an earlier version of this note misattributed it to B6.9. That condition is demonstrably met: the mutation fails `integration-tests/tests/replay_from_postgres.rs` (`one execution records one answer`, 0 vs 1) and all three in-process HTTP replay tests.
  **What is still missing, so neither box is checked.** B6.9 asks for exactly one `UserRegistered` **and** one welcome-email effect, counted directly. `replay_from_postgres` observes one body execution, one `complete()`, no second `complete()`, and a replay tied to stored bytes — it does not count the domain event or the effect. So one important condition of B6.10 is satisfied and B6.9's own observations are not. Both stay open until the event and effect counts are added, and they should be **added to the existing end-to-end scenario rather than as another infrastructure test** if they fit there — the end-to-end budget is spent.

### Phase B7: Retention, Purge, Observability (needs B3, B6)

- [x] B7.1 RED **(retargeted and narrowed by #274)**: add the discriminating `completed_at`-vs-`created_at` scenario to `assert_purge_conformance` in `crates/testkit/src/reservation_conformance.rs` — reserve at `t0`, advance the clock, complete at `t0 + Δ`, then purge with a cutoff falling between the two: the reservation must survive, because eligibility reads `completed_at`. The seven existing scenarios cannot catch a store that reads `created_at` instead: the `completed_at` helper positions the clock and completes at the same instant, so both columns hold the same value in every one of them. The second half of this task's original claim — an `InProgress` reservation is never TTL-purged, however old — is **already covered** by scenario 3 of that same harness, landed with B3-i, and needs no new test. The original target `crates/integration-tests/tests/purge.rs` no longer exists; #274 removed that crate, and the durable store answers this same harness later under #275.
  **Landed** as scenario 8 of `assert_purge_conformance`: reserved at `t0` with a generous lease, completed at `cutoff + 200s`, purged with `cutoff` between the two. Eligibility is defined on completion, so the reservation must survive.
  **Measured, and the harness gap it closes is real.** Mutating the in-memory store to measure eligibility from a creation timestamp instead of `completed_at` makes the suite fail **at scenario 8** — so the seven earlier scenarios ran and passed under that mutation, exactly as this task claimed. The message is `left: 1, right: 0`: a reservation still inside its retention window was purged.
  **One thing the mutation exposed about writing it.** A first attempt re-stamped the creation timestamp inside `complete()`, which made creation and completion coincide again and let the mutation pass. A mutation that quietly repairs the defect it is meant to introduce proves nothing; the timestamp has to be preserved through completion for the two columns to differ at all.
  **Not covered here:** the durable store does not answer this harness in this workspace. Nothing in `crates/` or `integration-tests/` calls the conformance functions against `PostgresOperationReservationStore` — the only caller is the in-memory store's own test. So scenario 8 currently pins the contract and the double, not the SQL.
- [x] B7.2 **(reframed twice: by B3-i, then by measurement)**: establish, by measuring the existing query before changing it, which multi-worker property is actually missing — separating **correctness** (an eligible row removed at most once; coherent counts; ineligible rows untouched) from **progress** (a worker can fill its batch from unlocked eligible rows without waiting). Result: correctness was already satisfied, and by PostgreSQL rather than by our SQL — a second `DELETE` blocks on the row lock, re-evaluates under READ COMMITTED, finds the row gone and removes zero. Progress was **not**: with two of four eligible rows held and a batch of two, the statement waited on a locked tuple until its timeout while free eligible rows sat untouched (`canceling statement due to statement timeout … while deleting tuple (0,2)`) — head-of-line blocking.
      **This box no longer claims instrumentation.** Its earlier wording bundled "observable" batched execution into the same task; nothing here instruments anything, and claiming it would be false. Purge observability belongs to B7.10/B7.11 and stays open there.
- [x] B7.3 RED **(reframed by B7.2's measurement)**: one durable test that forces a contended window rather than racing for it — a transaction holds two eligible rows while two others stay free, and a purge asking for a batch of two must fill it **within a deadline**, leaving the held rows and the ineligible row intact.
      **Deliberately not asserting "no deadlock" or "no double-purge".** Neither discriminates our query: double-purge is prevented by PostgreSQL regardless, and no circular wait was reproduced — "it could happen" is not a property worth asserting. The original wording claimed both; it was measured and dropped rather than satisfied on paper.
- [x] B7.4 GREEN **(reframed by B7.2's measurement)**: add `FOR UPDATE SKIP LOCKED` to the selection subquery as a **progress** fix. Not "concurrency-safe" — the earlier wording said that, and the measurement showed the query was already safe. Guarded by `integration-tests/tests/purge_progress_postgres.rs`: removing the clause restores the stall and fails the test on its deadline, and the same test pins that a skipped row is only deferred — a later call drains it once released.
  **B7.2/B7.3/B7.4 — what the three boxes above no longer claim.** Their original wording asked for multi-worker *safety*, *observable* batched execution, and "no deadlock, no double-purge". Each of those was measured and either found already true without our change, or not reproducible — so the boxes were reworded to the property that was actually established, rather than ticked against text the evidence does not support. Purge observability remains open at B7.10/B7.11.
  **Ledger:** PostgreSQL concurrency invariants 2/2, six infrastructure tests. Admitted under the budget's existing new-infrastructure-risk clause.
  **Debt kept explicit:** the full conformance harness still does not run against PostgreSQL — the only caller is the in-memory double's own test — so B7.1's `completed_at`-vs-`created_at` scenario still pins the contract and the double, not the SQL.
- [x] B7.5 **(reframed: no executable path to the defect)**: structural analysis rather than a behavioural test. The only deletion paths in `PostgresOperationReservationStore` are two `DELETE FROM operation_reservations` statements, and `operation_receipts` declares no foreign key, no `REFERENCES` and no `ON DELETE CASCADE` toward them. So no execution path reaches a receipt from a purge, and there is nothing dynamic to observe. Standing up PostgreSQL to watch an unrelated table survive would test the engine, not this code; adding a mutant foreign key to manufacture a RED would invent an architecture the system does not have.
- [x] B7.6 **(satisfied by table separation)**: no code to correct. The purge query never named `operation_receipts`, and nothing relates the two tables, so the guarantee held before this task was opened.
      **Guarded at its precondition, not its behaviour.** `migrations.rs`'s test module gains one bounded assertion, scanning **every registered migration** — any statement mentioning `operation_receipts` must contain no `REFERENCES`, `FOREIGN KEY` or `CASCADE`. Checked against the migration text the crate already embeds via `include_str!`, scoped per statement so an unrelated foreign key elsewhere in the same file is not flagged, and needing no infrastructure. Measured at the place the regression would actually appear: registering a later `012` migration whose `ALTER TABLE operation_receipts … REFERENCES … ON DELETE CASCADE` relates the two fails it by name. A first version inspected only the creating migration `011`, which that mutation would have passed — its own failure message promised to stop an author it could not have stopped, and a guard whose message describes a case it does not cover is worse than none.
      **Deferred obligation, stated where it will be read.** If a future migration does relate receipts to reservations, this assertion becomes the wrong shape and must be replaced by a catalogue check that the constraint carries no cascading delete, plus the behavioural test that only becomes meaningful once a path exists. That obligation is born with the foreign key, not before it — recorded in the test's own doc comment as well as here.
      **What this is not.** A guarantee by construction, not demonstrated dynamic behaviour. No infrastructure test was spent; the ledger stays at 4/4 end-to-end plus 2/2 concurrency invariants.
- [x] B7.7 **Open question resolved: runtime-owned.** The purge worker starts explicitly (`Runtime::start_retention`, the same shape as `start_effects`, so nothing begins deleting rows as a side effect of `build()`) and stops through a registered async teardown hook, in the same ordered, panic-isolated mechanism provider teardown already uses.
      **The task was widened, and why.** Its text said to "document the resolution beside the `RetentionPolicy` config" — a config that did not exist anywhere in `crates/`. A worker without a retention window, an interval and a batch has no defined behaviour, so `RetentionPolicy` was introduced as part of this unit rather than presupposed by it.
      **`RetentionPolicy` is optional and absent by default**, validated in its constructor (all three values > 0, one error variant each) following `ReservationConfig`'s precedent so no caller can hold a degenerate one. Enabling it without an `OperationReservationStore` is refused at `build()`, matching the existing refusal for a missing store: a retention schedule with nothing to purge cannot mean what it says.
      **The cutoff comes from the runtime's own clock** (`now - retention`), the same clock the store stamps rows with — a worker reading wall time would disagree with it under test and under skew.
      **Shutdown is bounded and panic-isolated, and touches no lease.** It cancels the loop, waits under a deadline, and never calls `abandon`, `complete` or `renew`; a purge worker holds no lease of its own. A panic or an overrun is isolated so the hooks after it still run, and is *returned* as `RuntimeInfraError::Teardown` rather than logged — the same "all hooks run, failure surfaced afterwards" shape as provider teardown. Richer reporting is B7.10/B7.11's subject and is not claimed here.
      **Tests, no infrastructure** (`crates/service-sdk/tests/retention_worker_lifecycle.rs`): no policy starts no worker; each degenerate value is refused; a policy without a store is refused at build; a configured worker purges with the exact cutoff and batch; shutdown ends later ticks; and an in-progress reservation survives shutdown untouched, asked through the port. **Two defects found in review and fixed.** On timeout the `JoinHandle` was dropped, and dropping one *detaches* the task in Tokio rather than cancelling it — shutdown returned an error while the worker carried on purging. It now aborts and awaits, so the call returns only once the task is gone. And `start_retention` had no idempotence guard: a second call spawned a second loop and a second hook, while its documentation claimed otherwise — that documentation had been inherited by an editing slip that left a neighbouring doc comment attached to this method — which happened three times before it was fixed properly: `start_effects`', then `effect_acceptor`'s, then `data_provider_access`'. Moving the method after one accessor only relocated the problem to the next; it is now last in the `impl`, where there is no following item whose doc it can take. Both are now guarded by the same compare-and-exchange the effects subsystem uses.
      **A third finding, about a test rather than the code.** The first version of the timeout test could not tell `abort` from `drop`: shutdown leaves a cancel permit, so even a detached loop exits at its next `select!` and never enters a second purge. Counting entries was un-failable. It now counts whether the parked purge *resumes* after release — a cancelled task is dropped at its await point and cannot. Verified: with `abort` replaced by `drop`, it fails at `1 != 0`.
      **Measured:** removing the teardown registration fails the lifecycle test at `7 != 1`; removing the idempotence guard fails the double-start test at `2 != 1` — the worker kept ticking after the runtime that owns it shut down. Ledger unchanged: 4/4 end-to-end plus 2/2 concurrency invariants.
- [x] B7.8 RED: cross-tenant replay test — tenant A's stored response never replays for tenant B, including the NULL-tenant systemwide scope (idempotent-command-processing spec — security-critical scenario).
      **This was not a preventive tripwire. It was a reachable cross-tenant leak, and it reproduced with no mutation applied.**
      Both surfaces were measured separately before anything was written, and the stores turned out to be isolated *by construction* — so the defect is not where the task text implies. What guarantees them: all six tenant predicates in `PostgresOperationReservationStore` compare with `tenant_id IS NOT DISTINCT FROM` (eleven more in the Postgres event store); migrations 010 and 011 each carry the AD-1 complementary partial unique-index pair, so the tenant-less partition is as unique as the tenant-scoped one on the PG14 floor; the in-memory reservation store keys on `OperationId`, whose `Hash`/`Eq` carry `Option<TenantId>`; in-memory receipts key on a four-tuple carrying the scope; and `resolve_tenant` refuses `Some("")` rather than coercing a misconfiguration into the shared partition. `reserve` is also insert-first, so `current()`'s predicate is only reached when the same `(tenant, key)` already exists — a single-predicate mutation there is unreachable on its own.
      **The executable crossing was one layer up**, at `RuntimeInner::reserve_idempotent_operation`: `ctx.canonical_tenant().and_then(|r| r.tenant_id().cloned())` flattened *no scope was resolved* and *the scope resolved to systemwide* into one `None`. `enforce_tenant` is the sole writer of `canonical_tenant` and runs only for `#[tenant_scoped]`, so any operation marked `#[idempotent]` without it was filed in the shared tenant-less partition for every caller.
      **Observed through generated dispatch**, real testkit store, two authenticated principals, one operation key: with an identical payload the handler ran **once** and tenant B received tenant A's stored response bytes — the information disclosure the requirement names as security-critical; with a differing payload tenant B was refused `FingerprintConflict` against tenant A's reservation. The control with resolution on its path ran once per tenant.
      Three tests keep those three findings: `crates/service-sdk/tests/cross_tenant_reservation_isolation.rs`. Each refusal is asserted by an observed `reserve` count as well as by the returned variant, because a refusal issued *after* reserving would satisfy the error assertion having already taken the lease.
      **No infrastructure test was added.** The defect is in runtime composition, not in PostgreSQL, and it reproduces in-process in milliseconds — the ledger stays at 4/4 end-to-end plus 2/2 concurrency invariants. The separate matter of the conformance harnesses never being driven against PostgreSQL is recorded below as its own debt and does not block this.
- [x] B7.9 GREEN: verify/harden the tenant-namespacing of every reservation/receipt lookup against B7.8.
      **Nothing in either store needed hardening** — see the measurement above. The change is the collapse: an explicit `match` over three states, refusing an unresolved scope with the new `ReservationRejection::TenantUnresolved` *before* the store is reached.
      The contract enforced is **"a scope was resolved before reserving"**, not "a particular attribute is present". Nothing inspects markers, so a `#[tenant_scoped]` requirement in the macro was deliberately not added: an operation arrives with a resolved scope or it does not, and how it got one is not the runtime's business.
      **The systemwide scope stays legitimate and still replays inside its own namespace.** Verified as its own case, and it had to live in-crate: `CanonicalTenant::systemwide()` is `pub(super)` and `set_resolved_tenant` is `pub(crate)`, so a test in `tests/` can build "no scope" and "scope with a tenant" but not the third state. Worth recording — `TenantResolver::resolve` never actually returns `Systemwide` today; every success path returns `scoped(...)`, and `CanonicalTenant::systemwide()` is constructed in exactly one place, its own unit test. So the refusal removed no working path.
      **Measured, mutation applied and reverted:** restoring the `and_then` fails `an_identical_payload_across_two_tenants_never_replays_cross_scope_bytes` and `a_differing_payload_across_two_tenants_never_produces_a_cross_scope_conflict` on their refusals, and the in-crate `an_unresolved_scope_is_refused_rather_than_filed_as_systemwide` on the recorded scope — the store is reached and handed `None`. The three systemwide/tenant tests stay green under that mutation, correctly: they pin that the fix did not break the legitimate scopes, not the separation itself.
      **Fallout handled rather than suppressed.** `idempotent_dispatch.rs` used the unguarded `settle` as a convenient vehicle for the outcome matrix and the epilogue — those cases were relying on an unresolved context being silently namespaced, which is the defect. `settle` now carries `#[tenant_scoped]` (still no `#[authorize]`, so the authorization fixture is still avoided) and its keyed cases pass an authenticated context; the three "legitimately did not reserve" cases get a keyless authenticated one, so the absence under test stays the key's rather than the scope's. `a_dropped_runtime_refuses_rather_than_running_unreserved` no longer names `StoreUnavailable`: the tenant guard needs the runtime too and now notices first, so pinning the variant there would pin guard ordering under a name about fail-open behaviour — it keeps both observed counters, which are what a fail-open regression has to get past.
      **Debt recorded, not silently carried:** neither conformance harness is ever driven against PostgreSQL (`assert_reservation_store_conformance` runs in-memory only, and its `ReserveRequest` fixture hardcodes `tenant: None`, so it exercises exactly one scope; the event-store harness's receipt section uses one tenant and no NULL-tenant receipt). The durable `complete`/`renew`/`abandon` tenant predicates therefore have no test reaching them with two scopes. That is a coverage gap in the durable adapter, not a known defect, and closing it needs an integration slot the ledger does not currently have.
- [x] B7.10 RED: AD-10 observability tests — `idempotency.key.rejected`, `idempotency.reservation.outcome`, `idempotency.lease.event`, `idempotency.lease.stale_owner`, `idempotency.receipt.outcome`, `idempotency.purge.rows`/`.batch_duration`/`.oldest_completed_age` counters/spans/histogram emitted with the documented attributes; `operation_key` never appears raw, only as `idempotency.operation_key_hash` (first 16 hex chars of SHA-256), and never as a metric attribute.
- [x] B7.11 GREEN: instrument the **two** spans (`idempotency.reserve`, `idempotency.purge_batch`) and the counters/histogram/gauge on the existing CORE-012A/PROD-003 OTLP surface. Originally three: `idempotency.takeover` was amended out of AD-10 as **AD-10a**, which records that a counter (`idempotency.reservation.outcome{outcome=taken_over}`) preserves only **occurrence and frequency** — not duration, start instant, causal position, or any circumstance, since `outcome` is the sole attribute it carries. That is a change to the design decision, and it lives there — in the normative source — rather than as a note under this box.

      **Measured before implementing, and AD-10 assumes a richer surface than exists.** Recorded here because it changes what these two boxes can honestly claim.
      - **As measured, and since resolved.** `Observability::metric(&self, name: &str, value: f64)` had **no attribute parameter**, so none of AD-10's metric attributes (`reason`, `carrier`, `outcome`, `event`, `operation`, `aggregate_type`) was expressible as written, and there was no separate histogram or gauge kind. The signals that landed under that surface fold their attribute into the metric name — every one of them is a bounded enum, which is what made folding survivable rather than correct. **The port no longer has that shape**: attributes became expressible, and then `record_metric(MetricObservation)` made the kind expressible with them. The folded names are therefore no longer a deviation forced by the port — they are call sites not yet migrated, and migrating them is a named slice below. Do not read this bullet as a standing justification for folding: it records what was true when those signals were written, not what the port permits now.
      - **As measured, and since resolved.** `metric` had **zero production call sites**; its only implementations were `NoopObservability` (empty body) and test spies, and there was no OTLP *metrics* exporter — the spans side had one (`crates/infrastructure/src/tracing_otlp.rs`), the metrics side had none. So "verify the telemetry actually emitted" could only mean against a recording double, and that was stated rather than implied. **Both halves are now false**: every signal has a production emitter, and `crates/infrastructure/src/metrics_otlp.rs` exports them over OTLP. Recording doubles remain the right tool for asserting *what* a signal carries — that is where AD-10's contracts are proven — but they are no longer the only thing standing between a metric and a backend.
      - `SpanAttributes` was a closed two-field allow-list whose own docs call it "the ONLY way to attach attributes to a span", with the restriction "enforced structurally at this type rather than by a runtime filter" (ADR-6). `idempotency.operation_key_hash` was therefore **not expressible**, and admitting it is a deliberate widening of that list — decided explicitly rather than assumed, and kept to one bounded curated concept, the way `tenant_present` already is.
      - Two of the three properties asked for held **by construction, not by test**, under the surface as measured: the raw key is not expressible as a span attribute, and nothing could be a metric attribute at all. The second half of that no longer holds by construction — metric attributes now exist — so "the raw key is never a metric attribute" becomes a property that must be *tested* from the slice that introduces the first real metric attribute onward. The first half still holds structurally: `SpanAttributes` accepts an `OperationKeyHash`, never a `String`. What is mutable, and is guarded, remains the hash computation and the adapter's mapping.
      **Sliced into three PRs against this tracker**, because the whole thing exceeds the 400-line review budget and the first slice is independently valuable:
      1. **The correlation-token primitive** (405 changed lines). `OperationKeyHash` in `crates/domain/src/operation/key.rs` — sole constructor hashes an `OperationKey`; `SpanAttributes::with_operation_key_hash` takes that type, never a `String`; the OTLP adapter maps it to `idempotency.operation_key_hash`. Ordered first on purpose: the type lands before any instrumentation exists that could emit a key, so there is never a window in between. The guarantee is deliberately narrow and documented as such. The token is a **diagnostic correlator for attempts of one operation key across traces** — a job the trace id cannot do, since a retry arrives on a new trace — and never an identity, a lookup key, or an input to a security decision. What is guaranteed is only that the raw key is never emitted literally. That is **not** anonymisation: low-entropy keys remain vulnerable to dictionary enumeration. And the 16-hex truncation is a real cost, not a free one — it reduces uniqueness and collision resistance, so a full digest would correlate strictly better; what it does not do is add confidentiality, since computing SHA-256 costs the same and comparing a prefix only weakens an attacker's ability to discard candidates. Six mutations measured — truncating to 8 hex, uppercase hex, emitting the raw key, hashing a constant, dropping the attribute before export, and renaming the attribute — each kills at least one test. `sha2` added to `ego-domain`: the rule is a domain rule (CORE-019 §12), and a hash computed in an adapter could be bypassed by a caller that already has a string.
         **Scope stated rather than implied:** this slice makes the digest *representable* and the raw key *not* representable where an `OperationKeyHash` is required, and shows the adapter transports a hash to the exporter faithfully. It is **not** evidence that idempotency telemetry redacts anything — no idempotency span is emitted yet. The behavioural claim needs the full chain `OperationKey` → `OperationKeyHash` → `SpanAttributes` → observed exporter driven from a real reservation, and belongs to slice 2.
      2. **The spans.** Originally scoped as "the three spans", and that turned out to be wrong on two counts once measured, so it is now three PRs covering **one** span between them.
         - **2a — the plumbing and `idempotency.reserve`.** Blocked on plumbing, which is why it could not be folded anywhere else: **`RuntimeInner` held no tracer.** The tracer was consumed at `build()` solely to construct a `TracingInterceptor` and then dropped, so `reserve_idempotent_operation` had nothing to open a span with. Retaining it is a new field through `new_with_logger`, whose 13 positional parameters become 14 across 9 construction sites.
         - **2b — the behavioural chain**, test-only: a real `Idempotency-Key` header through the generated proxy and guards, asserting the token the runtime hands the `Tracer` against one derived from the header. Stops at the port; the port → exporter hop is already covered where the adapter lives. Reaching the exporter from an example would mean making `OtlpTracer::from_processor` public and giving an example an OpenTelemetry SDK dependency — widening a production API and inverting the layering to move an assertion already made.
         - **2c — `idempotency.purge_batch`**, a root span per tick: a background tick has no request boundary to descend from, and a parent would attribute shared batch work to one caller's request. A fresh root per tick rather than one for the worker's lifetime, since a single trace spanning every tick would never close. It reuses `OpenSpan` rather than a second guard, and the reason is sharper here than at the reservation: `RetentionWorker::stop` **aborts the task when a purge overruns the shutdown deadline**, which is a drop mid-`await` — so without the guard every such shutdown leaked an entry in the adapter's bounded table, and that table drops *new* spans at capacity rather than evicting. A failing purge also produces a signal for the first time — the loop discarded the `Result` outright before this — but only **when tracing is configured**: without a tracer the failure is still discarded and retention behaviour is unchanged, since the span is the sole reporter. A failure counter is a metric, and metrics are what remains.
         - **`idempotency.takeover` is not emitted as a span**, and that is settled in the normative source rather than here: see **AD-10a** in `design.md`, which amends AD-10 from three spans to two and records why the third is not implementable as written. Its coverage moves to `idempotency.reservation.outcome{outcome=taken_over}`, which carries occurrence and frequency only — AD-10a names what that loses.
      3. **The eight counters/histogram/gauge**, name-folded as above.

      **What actually closed these two boxes, and when.** The work ran as nine reviewed PRs against this tracker — six implementation slices and three normative amendments, each amendment written *before* the code that assumed it:

      | Slice | What it closed |
      |---|---|
      | #307 | `MetricAttribute` — dimensions became expressible |
      | #308 | **AD-10b** — `key.rejected.reason` admits `unreadable` |
      | #309 | `MetricKind`/`MetricObservation` — the kind became expressible; `purge.batch_duration` became a real histogram |
      | #310 | **AD-10c** — `lease.event` withdraws `renewed` and `expired` |
      | #311 | `idempotency.receipt.outcome`, from the four places the receipt gate reaches a verdict |
      | #312 | `idempotency.purge.oldest_completed_age`, and `OldestCompleted`'s three states |
      | #313 | **AD-10d** — `lease.stale_owner` withdraws `renew` and `abandon` |
      | #314 | Every folded value moved into a dimension; all twelve folded names deleted |
      | #315 | The OTLP metrics exporter, and one sink wired to both halves of the host |

      **B7.10** closes because every signal in AD-10's table is emitted with its documented kind and attributes, and the redaction property is *tested* rather than assumed. That last point is a change in kind: before dimensions existed, "the raw key is never a metric attribute" held because nothing could be one. It now holds because each emitter is scanned for it — across every observation of every channel, and both halves of every attribute.

      **B7.11** closes because those signals *can* reach a productive OTLP backend, and because both halves of the system provably reach the same export pipeline. Two pieces of evidence, and neither is the whole claim on its own: `OtlpMetrics::new` builds the productive OTLP pipeline — exporter for the configured endpoint and protocol, reader, provider — and the acceptance test builds the reference app through `build_runtime_observed` and proves the SDK's own signal and an entity actor's `receipt.outcome` arrive at **one** provider with kinds and dimensions intact. That test runs a real provider over an in-memory exporter, not a collector across a network, so what it establishes is the topology and the translation, not delivery over the wire. Its negative control performs the wiring mistake and shows the SDK half still reporting normally — which is what would have made the gap invisible.

      **Four values the table asked for are not emitted, and will not be.** `lease.event`'s `renewed` and `expired` (AD-10c) and `lease.stale_owner`'s `renew` and `abandon` (AD-10d) were **withdrawn**, because the runtime produces no independently observable event with those meanings — not because they were left unwired. The distinction was held to deliberately: everything that *was* merely unwired (`receipt.outcome`, `oldest_completed_age`, real attributes, the exporter) stayed required and was built. Each amendment records the condition that would reopen it, and requires the value and the behaviour producing it to arrive in the same unit of work.

### Phase E1: Dual-Aggregate Recovery E2E (needs B6, B7 complete)

- [x] E1.1 Acceptance: a real crash between the two aggregates is recovered by takeover. `integration-tests/tests/infrastructure/dual_aggregate_crash_recovery_postgres.rs` — a child process dies by **SIGABRT** after the `TenantOrganization` receipt confirms and before the `User` command is sent; the parent proves the partial durable state, then retries as a different owner with a clock past the lease.

      Landed as an acceptance test rather than the specified RED/GREEN pair, because **the expected scenario passes with no new recovery logic** — see AD-12. Written to fail, and it does not.

      Two findings from writing it, both worth keeping:

      **The event-count assertion was a vacuous green.** Asserting "no second `OrganizationEnsured` event" was measured to survive a mutation that breaks the receipt gate outright, because `TenantOrgCommand::Ensure` is idempotent in the *domain* too — an existing org yields `NoEvents`. The count is guaranteed twice over and cannot say which guarantee produced it. What distinguishes them is `idempotency.receipt.outcome`: the gate answers **without running the handler** and counts `already_applied`, while domain idempotency runs it and counts `confirmed`. The test now asserts `[(tenant_organization, already_applied), (user, confirmed)]`, and the count stays with its message saying out loud that it is the weaker claim.

      **A third identical attempt exercises a different layer.** It counts no receipt outcome at all: the reservation holds the key as completed and replays its stored response, so the service body never runs. Asserted against positive evidence from the guard's own counter first, because "nothing was counted" is also what an unwired recorder reports.

      Eight mutations, eight killed, six distinct killing assertions. One declared coverage limit: the empty-receipts assertion is never the first to fail, since `ReservationOutcome::Succeeded`'s label and its `Replay` decision derive from the same match arm.

      The interruption is an explicit dependency (`DualAggregateFailpoint`, one method fixing its own position) behind the reference app's `crash-test-failpoint` feature, **off by default**. An earlier version read the environment unconditionally, which meant an inherited variable could abort an ordinary host mid-`register` — fixed, and verified by measuring the artifact: the variable's name appears 0 times in `libreference_app.rlib` with default features and 3 times with the feature on. Unix-only by `#[cfg(unix)]`, because the signal *is* the evidence.

- [~] E1.2 **WITHDRAWN — superseded by evidence (AD-12).** Not implemented, and not implementable as specified: there is no recovery path left to wire. Recovery is the composition of takeover on an expired lease, a rising fencing token, durable per-aggregate receipts, the receipt gate ahead of the handler, and one operation identity carried to both aggregates. E1.1 **observes that composition** end to end; its mutation battery **directly mutates three** of the five — the fencing token's advance, the receipt gate, and the shared operation identity — and additionally validates the real boundary of the crash and the distinction between resumption and replay. That all five are necessary is an architectural conclusion, not a per-guarantee mutation result: takeover and receipt durability are observed rather than mutated (the durable row changes owner; a different process reads the receipt the crashed one wrote). AD-12 states which is which.

      Deliberately **not** marked complete: that would claim work never performed and attribute the behaviour to a slice that wrote no code. Deliberately not reconverted into another task either — a slice invented to preserve a RED/GREEN shape is not a requirement.

      Reopens if any of those five guarantees changes, or if a multi-aggregate case arises that cannot be resumed by receipts and takeover alone. AD-12 states both conditions.

      **Phase E1 is closed:** E1.1 delivered, E1.2 withdrawn. Nothing in E1 remains open.

      **PROD-012 as a whole is not closed, and this note exists so that is not mistakenly inferred.** Eleven tasks remain unchecked outside E1: six in **B1** (the gRPC metadata carrier and the cross-adapter equivalence work, B1.11–B1.16), two in **F1** (`EffectRunner`'s injected clock, F1.1–F1.2), and the three **DOC** items (DOC.1–DOC.3). E1 was the last *recovery* phase, not the last phase.

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
| The Dual-Aggregate Write Is Not Promised Atomic | E1.1 (E1.2 withdrawn — AD-12) |
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
| Dual-Aggregate Recovery After Mid-Operation Process Death | E1.1 (E1.2 withdrawn — AD-12) |


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
