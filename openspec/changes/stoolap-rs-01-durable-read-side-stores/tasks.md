# Tasks: STOOLAP-RS-01 — Durable Read-Side Stores on Stoolap

> Canonical / English. Spanish companion: `tasks.es.md` (1:1 numbering).
> TDD is on (`openspec/config.yaml`): every new behavior lands RED before GREEN. PR slicing,
> line estimates, and file paths are fixed by `design.md`'s "Migration / Rollout" and "File
> Changes" sections — not re-derived here.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1240 total — PR1 ~280, PR2 ~210, PR3 ~380, PR4 ~250, PR5 ~120 (design.md "Migration / Rollout") |
| 400-line budget risk | Medium — every slice forecast under 400 with margin; PR3 closest, measure once written |
| Chained PRs recommended | Yes |
| Suggested split | PR1 → PR2 → PR3 → PR4 → PR5, feature-branch chain |
| Delivery strategy | ask-on-risk |
| Chain strategy | feature-branch-chain — PR1 targets the tracker branch, each later PR targets its predecessor (design.md) |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | Offset store + `read-side` feature scaffold | PR1 | `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` | tempfile-backed on-disk Stoolap db, real close/reopen | delete `src/read_side/` + the `read-side` feature entry; feature off leaves `cargo build`/`cargo test --workspace` unchanged |
| 2 | Dedup store | PR2 | same binary, dedup section | same, dedup table | delete `dedup.rs` + its `pub mod`/`pub use`; revert dedup section of `tests/read_side_stores.rs` |
| 3 | AD-11 hoist + claim store construction + unit tests | PR3 | `cargo test -p ego-persistence-stoolap --features read-side` (claim unit tests) + `cargo test -p ego-persistence-stoolap --features operation-reservation` (regression) | `ego_testkit::TestClock` advanced explicitly, tempfile db, never a real sleep | revert the AD-11 hoist (restore the two functions in `operation/reservation.rs`); delete `claim.rs` |
| 4 | Claim concurrency race + reopen durability + shared-engine tests | PR4 | same binary, concurrency/reopen/shared-engine sections, `#[tokio::test(flavor = "multi_thread")]` | real concurrent `tokio::spawn` tasks against one real tempfile Stoolap db | revert the new test cases only; PR1-3 remain valid |
| 5 | Production composition + negative control | PR5 | `cargo test -p ego-service-sdk --test read_side_progress_composition` | real `App::builder()`/`try_build()` over a tempfile-backed on-disk Stoolap db under `Profile::Production` | revert the composition test additions + the one dev-dependency feature word; PR1-4 remain valid |

## Phase 1: Offset Store + `read-side` Feature Scaffolding — PR1

- [x] 1.1 `crates/persistence-stoolap/Cargo.toml`: add `read-side = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain"]` (AD-1, all four deps already declared, zero manifest additions) + `[[test]] name = "read_side_stores" required-features = ["read-side"]`.
- [x] 1.2 Run `cargo tree -p ego-persistence-stoolap --features read-side -e normal` and record the output: confirm zero new transitive dependency edges versus the feature-off tree (proposal/design claim, not yet run in explore).
- [x] 1.3 Create `src/read_side/mod.rs` (`pub mod offset; pub mod dedup; pub mod claim;`) with a module doc stating the same-process-only concurrency scope (design "Concurrency Scope"); gate `#[cfg(feature = "read-side")] pub mod read_side;` in `src/lib.rs` + crate-root `pub use` of the three store types (AD-2).
- [x] 1.4 RED `src/read_side/offset.rs`: unit test `read_offset_of_a_never_written_key_is_none` — fails to compile, `StoolapOffsetStore` does not exist yet.
- [x] 1.5 GREEN same file: `StoolapOffsetStore::open(path: &Path) -> Result<Self, OffsetStoreError>` via `stoolap_common::dsn_for`; refuse with `Fatal` if `dsn_declares_sync_full` is false before `CREATE TABLE` (AD-9); `CREATE TABLE IF NOT EXISTS projection_offsets (... UNIQUE (projection_id, tag, tenant))` — `UNIQUE`, never `PRIMARY KEY` (AD-3); private `run_blocking` via `tokio::task::spawn_blocking`, never `block_in_place` (AD-10); `fn is_durable(&self) -> bool { dsn_declares_sync_full(self.db.dsn()) }` — never a hardcoded `true` (AD-9).
- [x] 1.6 RED same file: `a_write_is_isolated_to_its_key` — two distinct `(projection_id, tag, tenant)` keys, each read returns only its own value.
- [x] 1.7 RED same file: `a_repeat_write_overwrites_without_ordering_enforcement` — `write_offset` twice for the identical key, `read_offset` returns the value just written (last-write-wins, no CAS).
- [x] 1.8 GREEN same file: implement `write_offset`'s three-step UPDATE-first / INSERT `ON CONFLICT DO NOTHING` / re-UPDATE (AD-4); implement `read_offset`.
- [x] 1.9 RED+GREEN `crates/persistence-stoolap/tests/read_side_stores.rs` (create): offset reopen test — write, **drop every store handle for the path**, `open()` the same file again, `read_offset` for the written key returns the identical value and a never-written sibling key still returns `None` (spec "Offset And Dedup State Survive Close And Reopen"); `is_durable()` returns `true` only once this is proven.
- [x] 1.10 Verification: `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` green; `cargo build --workspace` and `cargo test --workspace` with the feature off are unchanged (spec/proposal success criterion).
- [x] 1.11 Verification: `rg block_in_place crates/persistence-stoolap/src/read_side` returns nothing; manual review of `crates/persistence-stoolap/src/read_side` and `crates/persistence-stoolap/tests/read_side_stores.rs` confirms no positive claim of multi-process, multi-node, Kubernetes, or distributed-coordination support — explicit documentation stating these modes are unsupported is required and permitted, not prohibited (a blind word-ban grep would incorrectly flag that required documentation).

## Phase 2: Dedup Store — PR2

- [ ] 2.1 RED `src/read_side/dedup.rs`: unit test `seen_of_an_unmarked_triple_is_false` — fails to compile, `StoolapDedupStore` does not exist yet.
- [ ] 2.2 GREEN same file: `StoolapDedupStore::open(path: &Path) -> Result<Self, DedupStoreError>`; `CREATE TABLE IF NOT EXISTS projection_dedup (... UNIQUE (projection_id, tag, event_id))` — **no** tenant column, matching the port's own no-tenant identity (AD-3); same fail-closed `open()`/real `is_durable()` pattern as offset (AD-9); own private `run_blocking` (AD-10).
- [ ] 2.3 RED same file: `mark_seen_is_idempotent` — repeat `mark_seen` for the identical triple succeeds without error, `seen()` still returns `true`.
- [ ] 2.4 RED same file: `no_dedup_entry_is_ever_pruned` — a mark written arbitrarily long ago (simulated by no time-based cleanup path existing) still returns `true` from `seen()`; no `seen_at` column, no TTL, no retention (spec Non-Goal).
- [ ] 2.5 RED same file: `the_same_event_id_under_a_different_projection_and_tag_is_independent` — isolation across the full key.
- [ ] 2.6 GREEN same file: implement `mark_seen` (`INSERT ... ON CONFLICT (projection_id, tag, event_id) DO NOTHING`, AD-4) and `seen` (`SELECT 1 ... LIMIT 1`, presence is the answer).
- [ ] 2.7 RED+GREEN `tests/read_side_stores.rs`: dedup reopen test — mark seen, **drop every store handle for the path**, reopen the same file, `seen()` for the marked triple still returns `true` (spec "A dedup mark survives a close/reopen cycle").
- [ ] 2.8 Verification: `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` green for both offset and dedup sections; repeat the `block_in_place` grep and the no-positive-multi-process-claim review from 1.11 over `dedup.rs`.

## Phase 3: AD-11 Helper Hoist + Claim Store Construction + Unit Tests — PR3

- [ ] 3.1 Hoist `token_for_storage`/`token_from_storage` out of `crates/persistence-stoolap/src/operation/reservation.rs:76-93` into `crates/persistence-stoolap/src/persistence/stoolap_common.rs` as `pub(crate)` (AD-11) — behavior-preserving, not bumped to `pub(crate)` in place, because `operation/reservation.rs` sits behind its own `#[cfg(feature = "operation-reservation")]` and a cross-feature in-place reference would break under `--features read-side` alone; update `reservation.rs`'s import to the hoisted path.
- [ ] 3.2 `cargo test -p ego-persistence-stoolap --features operation-reservation` green — confirms the AD-11 move is behavior-preserving, no regression on the shipped reservation store.
- [ ] 3.3 Create `src/read_side/claim.rs`: `CREATE TABLE IF NOT EXISTS projection_claims (... UNIQUE (projection_id, tag, tenant))` — `UNIQUE`, never `PRIMARY KEY`, no `CHECK` (AD-3); a `to_claim_error(ReservationError) -> ClaimError` shim mirroring `crates/persistence/src/postgres/read_side_claim.rs:51-57` (AD-11).
- [ ] 3.4 GREEN same file: `StoolapReadSideClaimStore::open(path: &Path, clock: Arc<dyn ego_domain::Clock>) -> Result<Self, ClaimError>` — injected `Clock` (AD-7); same fail-closed `open()`/real `is_durable()` pattern (AD-9); own private `run_blocking` (AD-10).
- [ ] 3.5 RED same file: `try_claim_grants_a_fresh_claim_with_no_live_lease` — `Ok(Some(fence))` with `FencingToken::initial()`.
- [ ] 3.6 RED same file: `a_live_claim_refuses_a_second_claimant` — `Ok(None)`, the existing holder's fence remains valid.
- [ ] 3.7 GREEN same file: implement `try_claim`'s two-statement CAS — `INSERT ... ON CONFLICT (projection_id, tag, tenant) DO NOTHING`, then a conditional `UPDATE` re-verifying the live row's `fencing_token` and `lease_until` (AD-5), the proven `StoolapOperationReservationStore::reserve()` pattern, not Postgres's single-statement `DO UPDATE ... RETURNING`.
- [ ] 3.8 RED same file: `takeover_of_a_lapsed_lease_mints_a_strictly_greater_token` — `Ok(Some(fence))` with a strictly greater `fencing_token`, the lapsed holder's fence no longer verifies.
- [ ] 3.9 GREEN same file: complete the takeover branch of `try_claim`'s CAS (AD-5 step 4).
- [ ] 3.10 RED same file: `fencing_exhaustion_is_reported_not_wrapped` — `FencingToken::next() == None` at takeover surfaces `ClaimError::FencingExhausted`, never a wrapped or truncated token.
- [ ] 3.11 GREEN same file: wire the exhaustion check into `try_claim` before the takeover `UPDATE`.
- [ ] 3.12 RED same file: `renew_and_release_reject_a_stale_or_lapsed_fence_without_mutating_state` — a fence that no longer matches the live claim, and separately a fence whose lease has already lapsed, both fail `StaleOwner`, stored claim unchanged.
- [ ] 3.13 GREEN same file: implement one private `fn set_lease(&self, fence, new_lease_until) -> Result<(), ClaimError>` — one statement, `UPDATE ... WHERE claim_id AND owner_id AND fencing_token AND lease_until > $now` (AD-6); `renew` calls it with the caller's `lease_until`; `release` calls it with `clock.now()` (never a `DELETE`).
- [ ] 3.14 RED same file: `release_marks_the_claim_expired_not_deleted` — after `release`, a subsequent `try_claim` for the identical `claim_id` succeeds immediately, and the row still exists with an expired lease (fencing token unchanged by release itself).
- [ ] 3.15 GREEN: confirm 3.14 passes against 3.13's `set_lease` (no additional production code — `release` setting an already-expired `lease_until` is the whole mechanism).
- [ ] 3.16 GREEN same file: error classification — a raw `stoolap::Error` for which `stoolap_common::is_write_conflict` is `true` maps to `ClaimError::Transient`, everything else to `Fatal`; `affected == 0` on a fence-verified mutation maps to `StaleOwner`, never `Transient` (AD-8).
- [ ] 3.17 Document + flag (no production code): a module-doc line on `claim.rs` states the satisfiable reading of "lease expiry is caller-computed" per design AD-7 — the *lease bound* (`lease_until`) is always the caller's, and the store's own "now" comes only from the injected `Clock`, never ambient system time; the store never reads `Utc::now()`/`SystemTime::now()`/SQL `now()`. Record in the PR description that `spec.md`/`spec.es.md`'s "Lease Expiry Is Always Caller-Computed" wording ("never on a clock read performed inside the store") is literally unimplementable against `try_claim`'s real signature (no `now` parameter) and flag it to `sdd-verify`/a human for a follow-up clarification pass — do not silently reinterpret without this paper trail.
- [ ] 3.18 Verification: `cargo tree -p ego-persistence-stoolap --features read-side -e normal` unchanged from 1.2 (AD-11 only moves code, adds no dependency); `cargo test -p ego-persistence-stoolap --features read-side` (claim unit tests) green; `cargo test -p ego-persistence-stoolap --features operation-reservation` green (no regression); repeat the `block_in_place` grep and the no-positive-multi-process-claim review over `claim.rs`.

## Phase 4: Claim Concurrency Race + Reopen Durability + Shared-Engine Tests — PR4

- [ ] 4.1 RED `tests/read_side_stores.rs`, `#[tokio::test(flavor = "multi_thread")]` + `tokio::spawn`: `concurrent_claimants_yield_exactly_one_winner` — several tasks race `try_claim` on one fresh `claim_id` with no existing live lease, mirroring `tests/reservation_conformance.rs:192-240`'s `two_concurrent_reserves...` shape.
- [ ] 4.2 GREEN: confirm exactly one task receives `Ok(Some(fence))` and every other receives `Ok(None)` or `ClaimError::Transient` (classified via `is_write_conflict`, AD-8) — never a second `Ok(Some(fence))` for the same live lease.
- [ ] 4.3 RED same file: `takeover_after_expiration_under_real_concurrency_mints_a_strictly_greater_token` — two real concurrent tasks race a takeover from one lapsed lease.
- [ ] 4.4 GREEN: confirm the loser resolves to `Ok(None)` (a peer took over or the owner renewed in the window) or a retry-safe `Transient` — never a third outcome, never two grants.
- [ ] 4.5 RED same file: `claim_state_survives_close_and_reopen` — hold a claim under a valid fence, **drop every store handle for the path**, reopen the same file; a different owner's `try_claim` on the identical `claim_id` still returns `Ok(None)` and the held fence still verifies through `renew`; separately, a fence released before the drop reopens as immediately reclaimable with a strictly greater token on the next takeover. Test name and module comment state explicitly this is drop-and-reopen, **not** crash/power-loss safety (spec "Claim Durability Is Drop-And-Reopen, Not Crash Recovery").
- [ ] 4.6 GREEN: confirm 4.5 passes; review the test's doc comment and assertions to confirm no crash-safety language is implied anywhere.
- [ ] 4.7 RED+GREEN same file: `three_stores_at_one_path_share_one_engine` — offset, dedup, and claim stores opened at one identical path all observe the same database (Stoolap's process-global-engine-per-DSN property), mirroring `tests/reservation_conformance.rs:258-311`'s precedent; re-proved here, not assumed (design "Concurrency Scope").
- [ ] 4.8 Verification: `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` green; if flaky under default parallel threads, re-run with `--test-threads=1` (S3 precedent) and record the finding; grep sweep confirms no `block_in_place`; manual review confirms no positive claim of multi-process/multi-node/Kubernetes/distributed-coordination support anywhere in this PR's new test names, comments, or docs (unsupported-mode documentation remains required and permitted).

## Phase 5: Production Composition + Negative Control — PR5

- [ ] 5.1 `crates/service-sdk/Cargo.toml`: add `"read-side"` to the existing `ego-persistence-stoolap` dev-dependency feature list (AD-13, one word).
- [ ] 5.2 RED `crates/service-sdk/tests/read_side_progress_composition.rs` (extend, existing file): a real `Profile::Production` composition over a `tempfile::tempdir()`-backed on-disk Stoolap database, using real `StoolapOffsetStore`, `StoolapDedupStore`, and `StoolapReadSideClaimStore` through `App::builder()` / `try_build()` — not solely `is_durable()` on an isolated store (spec "A Real Profile::Production Composition Exercises The Gate").
- [ ] 5.3 GREEN: confirm the durable Stoolap composition builds successfully under the unmodified gate.
- [ ] 5.4 RED same file: negative control — the identical composition with exactly one store swapped for the file's existing `VolatileOffsetStore` (or an equivalent volatile claim/dedup store) under `Profile::Production`.
- [ ] 5.5 GREEN: confirm the same gate, unmodified, rejects the negative-control composition as `CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_))` — no gate code is touched by this PR.
- [ ] 5.6 Verify-don't-touch: read `openspec/specs/real-infrastructure-verification/spec.md`'s Purpose, Requirements, and Non-Goals; confirm all five requirements remain PostgreSQL-specific and none is engaged by this change (design AD-14); record the confirmation in the PR description; make **zero** edits to that file.
- [ ] 5.7 Verification: `cargo test -p ego-service-sdk --test read_side_progress_composition` green; `cargo test --workspace` with the `read-side` feature off on `persistence-stoolap` unaffected; final `cargo tree -p ego-service-sdk --features read-side -e normal` (or equivalent workspace-wide check) confirms zero new transitive dependencies introduced end to end by the whole change.

## Cross-Cutting Acceptance Criteria (apply to every PR above, not stated once)

- No PostgreSQL dependency introduced anywhere in this change.
- No `block_in_place` anywhere — only `tokio::task::spawn_blocking` via each store's own `run_blocking()`.
- No positive claim of multi-process, multi-node, Kubernetes, or distributed-coordination support in any doc, comment, or test name shipped by this change. Explicit documentation stating these modes are unsupported is required and permitted — this criterion prohibits false claims of support, not the words themselves.
- Every store's `is_durable()` reports `true` only when backed by a real, fail-closed, `sync=full`-verified Stoolap connection — never a hardcoded literal.
- `cargo tree` (or equivalent) confirms zero new transitive dependencies for the `read-side` feature — recorded at PR1 (1.2), reconfirmed at PR3 (3.18) and PR5 (5.7).

## Out of Scope (reaffirmed, not re-litigated)

No task above touches `EventStore`, `Snapshot`, `Repository`, `EffectStateStore`, `EffectDedupStore`, or `OperationReservationStore` production code (reused as a pattern template only); no task changes a Postgres file or any of the three trait contracts in `crates/persistence-api/src/read_side/`; no task adds dedup pruning/TTL/retention or offset compare-and-swap/monotonicity (proposal Non-Goals, unchanged).
