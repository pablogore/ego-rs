# Tasks: STOOLAP-S3 — Operation Reservation Store: Durability Signal, Stoolap Implementation, Production Gate

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~900-1050 (a≈150, b1≈350, b2≈250, c≈200, d≈100) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | WU1(a) → WU2(b1) → WU3(b2) → WU4(c) → WU5(d), sequential commits on `fix/stoolap-effect-01` |
| Delivery strategy | auto-chain |
| Chain strategy | stacked-to-main |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | (a) port durability signal + implementor sweep | WU1 | `cargo test -p ego-persistence-api -p ego-persistence-memory -p ego-persistence --lib` | N/A — pure unit tests | revert `reservation.rs` in the 3 crates; `false` default breaks nothing |
| 2 | (b1) Stoolap store core + AD-9 promotion | WU2 | `cargo test -p ego-persistence-stoolap --features operation-reservation` | N/A — colocated unit + conformance tests | delete `src/operation/` + feature flag; revert 3 AD-9 call sites |
| 3 | (b2) reopen/concurrency/tenant/purge tests | WU3 | `cargo test -p ego-persistence-stoolap --features operation-reservation --test reservation_conformance` | tempfile-backed Stoolap db, real close/reopen | revert new cases in `tests/reservation_conformance.rs` |
| 4 | (c) Production gate wiring | WU4 | `cargo test -p ego-service-sdk builder::` | N/A — builder validation unit tests | revert `validate_operation_reservation_profile` + 1 line in `validate_persistence_profile` |
| 5 | (d) cross-backend gate proof | WU5 | `cargo test --workspace && cargo test -p ego-persistence-stoolap --features operation-reservation` | real InMemory/Postgres/Stoolap stores through `RuntimeBuilder` | revert the new gate-integration test module only |

## Phase 1 (a): Shared-Port Durability Signal + Implementor Sweep

- [x] 1.1 RED `crates/persistence-api/src/operation/reservation.rs`: test bare impl reports `false` (spec `persistence-api-surface`: bare-implementation scenario).
- [x] 1.2 RED same file: test `Arc<Concrete>` used as generic `S: OperationReservationStore` reports `true` via forwarding (AD-2 — harness `testkit/src/reservation_conformance.rs:963-968` (read-only); pins the impl, not the vacuous `Arc<dyn _>` call site).
- [x] 1.3 GREEN same file: add `fn is_durable(&self) -> bool { false }` + doc, and `impl<T: OperationReservationStore + Send + Sync + ?Sized> OperationReservationStore for Arc<T>` forwarding all 7 methods incl. `oldest_completed`.
- [x] 1.4 RED `crates/persistence-memory/src/operation/reservation.rs`: test store reports `false` (spec `persistence-memory-adapter`: in-memory store scenario).
- [x] 1.5 GREEN same file: explicit `fn is_durable(&self) -> bool { false }` + honesty doc line.
- [x] 1.6 RED `crates/persistence/src/postgres/reservation.rs`: test store reports `true` (spec `idempotent-command-processing`: Postgres scenario).
- [x] 1.7 GREEN same file: override `fn is_durable(&self) -> bool { true }`.
- [x] 1.8 Review `crates/testkit/src/reservation.rs` (read-only): confirm it only re-exports the in-memory store; no other double implements the port.

## Phase 2 (b1): Stoolap Store Core + AD-9 Promotion

- [x] 2.1 RED `crates/persistence-stoolap/src/persistence/stoolap_common.rs`: test `dsn_declares_sync_full` rejects a path containing the text without the query param (AD-9; pattern from `crates/effect-store/src/stoolap/mod.rs:187-191` (read-only)).
- [x] 2.2 GREEN same file: promote the strict parser from effect-store; add its unit test.
- [x] 2.3 GREEN `crates/persistence-stoolap/src/persistence/snapshot.rs:74`, `crates/persistence-stoolap/src/event_sourcing/event_store.rs:213,263`: switch 3 call sites off `contains("sync=full")` to the strict parser (AD-9 — touches already-shipped S1/S2 files; 4 lines, no behavior change for any DSN `dsn_for` produces).
- [x] 2.4 `crates/persistence-stoolap/Cargo.toml`: add `operation-reservation` feature (`dep:tokio`, `dep:async-trait`, `dep:chrono`, `dep:ego-domain`, `dep:base64`) + `[[test]]` `required-features` (AD-7).
- [x] 2.5 Create `crates/persistence-stoolap/src/operation/mod.rs` (`pub mod reservation;`); gate + `pub use` in `crates/persistence-stoolap/src/lib.rs`.
- [x] 2.6 RED `crates/persistence-stoolap/src/operation/reservation.rs` (create): test `open()` refuses a non-`sync=full` engine.
- [x] 2.7 GREEN same file: `open(path, clock: Arc<dyn Clock>)`; `CREATE TABLE IF NOT EXISTS operation_reservations ... UNIQUE (tenant_id, operation_key)` — never `PRIMARY KEY` (unenforced composite key); `is_durable()` via `dsn_declares_sync_full`; base64 `response` (AD-8); local `token_for_storage`/`token_from_storage` (AD-10).
- [x] 2.8 RED same file: reserve fresh/replay/conflict/takeover per the AD-4 CAS table (spec `persistence-stoolap-operation-reservation`: "fresh reservation is exclusive", "takeover mints a strictly greater fencing token").
- [x] 2.9 RED same file: renew/complete/abandon stale-fence rejection leaves state unmodified (spec: "a stale fence is rejected without mutating state"); lost MVCC race maps to `Backend("...retry")`, never `StaleOwner` (AD-5).
- [x] 2.10 GREEN same file: implement `reserve`/`renew`/`complete`/`abandon`/`probe` (`SELECT 1 ... LIMIT 1`) and `oldest_completed` (`MIN(completed_at)`; NULL ⇒ `Empty`; fall back to `ORDER BY completed_at ASC LIMIT 1` if `MIN` unsupported, per Open Question).
- [x] 2.11 RED+GREEN `crates/persistence-stoolap/tests/reservation_conformance.rs` (create): run `assert_reservation_store_conformance` (`testkit/src/reservation_conformance.rs:963-968` (read-only)) against the new store + `TestClock`, `required-features = ["operation-reservation"]`.

## Phase 3 (b2): Reopen / Concurrency / Tenant / Purge Tests

- [x] 3.1 RED `crates/persistence-stoolap/tests/reservation_conformance.rs`: reopen-durability — reserve, drop store, reopen same path, owner/fencing/lease/tenant intact, stale fence still rejected (spec: "Reservations Survive Close And Reopen").
- [x] 3.2 RED same file: TS-4 same-process, two store instances at one path — A reserves, lease expires, B (separate instance) takes over with strictly greater token, A's stale fence then fails (spec: "Scoped To Same-Process Concurrency"). Also added a genuine real-concurrency test (`tokio::spawn`/`join!`, two racing tasks) for "a fresh reservation is exclusive" under actual concurrent execution, tolerating AD-5's documented `Backend("...retry")` signal on a genuinely lost MVCC race as a valid, non-corrupted loser outcome alongside `OtherInProgress`.
- [x] 3.3 RED same file: tenant isolation — identical operation key under tenant A / tenant B / systemwide are 3 distinct reservations, none cross-observable (spec: "Two tenants with the identical operation key remain isolated").
- [x] 3.4 RED same file: purge two-step select-then-delete proves nonzero rows deleted despite the `DELETE...WHERE IN (SELECT...LIMIT)` zero-row trap (`crates/effect-store/src/stoolap/mod.rs:292-301` (read-only)); in-progress reservation never removed (spec: "An in-progress reservation is never purged").
- [x] 3.5 GREEN: no implementation gap surfaced by 3.1-3.4 (the existing `reserve`'s honest `Backend("...retry")` classification on a lost MVCC race, per AD-5, is what 3.2's genuine-concurrency test observes and asserts — not a bug); `cargo test -p ego-persistence-stoolap --features operation-reservation` (run serially, `--test-threads=1`, for a deterministic baseline — see Issues Found re: a pre-existing Phase 2 test-isolation flake under default parallel threads).

## Phase 4 (c): Production Gate Wiring

- [x] 4.1 RED `crates/service-sdk/src/runtime/builder.rs`: matrix `{Dev, Production} × {no store, volatile store, durable store}` — only `Production` + volatile refuses (spec `production-composition-hardening`: gate scenarios); clone `validate_read_side_claim_profile_matrix`'s shape with `compat()` and `StubReservationStore(bool)`.
- [x] 4.2 RED same file: `Production` + volatile store under `Compatibility` mode still refuses (AD-3 — the one test distinguishing the registration trigger from the rejected `MandatoryKey` trigger).
- [x] 4.3 RED same file: rejection names the capability and `with_operation_reservation_store` (spec: "Rejections Are Actionable"; mirrors `validate_read_side_claim_profile_rejects_volatile_claim_store`).
- [x] 4.4 RED same file: `build()` panics and `try_build()` returns the same refusal.
- [x] 4.5 GREEN same file: implement `validate_operation_reservation_profile()` — fires on registration only (AD-3), routes through `persistent_entity::profile::require_durably_configured`; append to `validate_persistence_profile()`.
- [x] 4.6 REFACTOR: `cargo test -p ego-service-sdk`; confirm no existing gate check weakened.

## Phase 5 (d): Cross-Backend Gate Verification

- [x] 5.1 RED `crates/service-sdk` (builder integration tests): real `InMemoryOperationReservationStore` registered under `Profile::Production` → rejected (spec `persistence-memory-adapter`: "a production-profile composition still rejects it").
- [x] 5.2 RED same suite: real `PostgresOperationReservationStore` under `Profile::Production` → accepted (spec `idempotent-command-processing`: durable scenario).
- [x] 5.3 RED same suite: real `StoolapOperationReservationStore` (Phase 2/3, `--features operation-reservation`) under `Profile::Production` → accepted — honest only because 3.1's reopen test already proved the durability claim.
- [x] 5.4 GREEN+REFACTOR: wire the three backend fixtures onto 4.5's gate (no new gate logic); `cargo test --workspace`, `cargo test -p ego-persistence-stoolap --features operation-reservation`, and `cargo build --workspace --all-targets` (feature off) all green.
