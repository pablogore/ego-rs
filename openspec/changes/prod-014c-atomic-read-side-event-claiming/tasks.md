# Tasks: PROD-014C — Atomic Read-Side Event Claiming

> Canonical / source of truth. Spanish companion: `tasks.es.md` (1:1 identifiers).
> Strict TDD (AD-10): the contention suite (Phase 4) is written RED against
> `PostgreSQLReadSideClaimStore`, which does not exist yet, before the adapter body
> (Phase 3 GREEN steps). Every error assertion names the specific `ClaimError` variant,
> never `is_err()`. `cargo clippy --workspace -- -W clippy::cognitive-complexity` after
> each split.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1450 total — PR1 ~320 (port + types + migration), PR2 ~580 (adapter + real-PG contention suite), PR3 ~300 (session + scheduler wiring), PR4 ~250 (gate + docs) |
| 400-line budget risk | High for PR2 only — an accepted deviation, same shape as PROD-014B PR2 (D-7 mandates the real-PostgreSQL suite; it is never split from the adapter it proves). PR1, PR3, PR4 stay at or under budget |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (port + types + migration) → PR 2 (PostgreSQL adapter + real-PG contention suite) → PR 3 (session + scheduler wiring) → PR 4 (Production gate + docs) |
| Delivery strategy | auto-chain (session preflight) |
| Chain strategy | stacked-to-main — mirrors PROD-014A/PROD-014B topology; each PR branches from the previous one and merges to `develop` in order |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Deviation from design.md's suggested slicing (must justify)

Proposal R-4's mitigation text (and design's closing forecast note) suggests three slices:
"port + adapter, then session/scheduler wiring, then gate + docs." Estimating each
slice's own line count shows "port + adapter" combined — once migration `016` and the
mandatory real-PostgreSQL contention suite (D-7, IS-7) are included — runs to roughly
900 lines, well past even PROD-014B PR2's already-accepted ~500-line deviation. Splitting
the port + migration (schema-shaped, foundational, no adapter behavior to prove) from the
adapter + contention suite (the one slice D-7 forbids trimming) mirrors PROD-014B's own
PR1/PR2 split exactly, keeps PR1 comfortably under budget, and leaves only PR2 as an
accepted deviation — the same deviation PROD-014B already established as acceptable for
this exact reason (adapter never separated from the tests that prove it). This is a
four-slice plan, not the three design.md's closing note names; PROD-014C's Approach and
Required Semantics are unaffected — the deviation is delivery-slicing only.

### Suggested Work Units

| Unit | Goal | PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----|----------------------|-----------------|-------------------|
| 1 | `ReadSideClaimStore` port + `ClaimId`/`ClaimFence`/`ClaimError` + `Arc<T>` forwarding; migration `016_create_projection_claims.sql` registered | PR 1 | `cargo test -p ego-persistence-api read_side::claim`, `cargo test -p ego-persistence migrations` | N/A — schema + trait shape only, no adapter behavior to prove yet | Delete `claim.rs`, its re-export, migration `016` + registry entry, revert `reservation.rs`'s visibility change; nothing else references any of it yet |
| 2 | `PostgreSQLReadSideClaimStore` (`try_claim`/`renew`/`release`, fencing, error mapping) + the full real-PG contention suite (RED before GREEN, D-7) | PR 2 | `cargo test -p ego-persistence postgres::` (unit `is_fatal`/`claim_error` reuse) + `cargo test -p ego-integration-tests --test read_side_claiming_postgres` | Real PostgreSQL via `isolated_database()`, separate `PgPoolOptions` pools per contender, `SettableClock` | Delete `read_side_claim.rs`, its re-export, the contention suite file; PR 1's port + migration stay valid and unused |
| 3 | `ReadSideSession::execute()` claim/renew/release wiring; `ProjectionSpec::claims` scheduler knob | PR 3 | `cargo test -p ego-domain read_side::session`, `cargo test -p ego-runtime read_side::scheduler` | N/A — scripted doubles, no pool; unit-level only per AD-10 | Revert `session.rs`'s `ReadSideClaiming`/`with_claiming`/`run_batch` split and `scheduler.rs`'s `claims` knob; PR 1–2 remain valid for any manual host wiring |
| 4 | `Profile::Production` fail-closed gate; `AppBuilder::read_side_claims` + dup guard; reference-app + `ARCHITECTURE.md` docs | PR 4 | `cargo test -p ego-service-sdk read_side_claim`, `cargo test -p reference-app` | `examples/reference-app` composing under `Profile::Production` with a real Postgres pool | Revert `builder.rs`'s slot/validator, `app/mod.rs`'s method + dup guard, `app/error.rs`'s variant, reference-app wiring, `ARCHITECTURE.md`; PR 1–3 remain functionally complete but unused by the gate |

## Phase 1: Port & Types (Foundation) — PR 1

- [x] 1.1 RED `crates/persistence-api/src/read_side/claim.rs` `#[cfg(test)]`: `Arc<T>` forwards `is_durable()` to the wrapped store, not the trait's `false` default (AD-3 — the PROD-014A EC-2 landmine); `ClaimId` equality/hash by the full triple; `ClaimFence` equality by the full triple.
- [x] 1.2 GREEN: define `ReadSideClaimStore` (`try_claim`/`renew`/`release`, `is_durable() -> bool { false }` default), `ClaimId { projection_id, tag, tenant }`, `ClaimFence { claim_id, owner_id, fencing_token }`, `ClaimError { StaleOwner, FencingExhausted, Transient, Fatal }` (AD-1, AD-2).
- [x] 1.3 GREEN: `impl<T: ReadSideClaimStore + Send + Sync + ?Sized> ReadSideClaimStore for Arc<T>` forwarding all three methods and `is_durable()` explicitly (AD-3).
- [x] 1.4 GREEN: `crates/persistence-api/src/read_side/mod.rs` — add `pub mod claim;` and re-export `ReadSideClaimStore`, `ClaimId`, `ClaimFence`, `ClaimError`.
- [x] 1.5 GREEN: `crates/persistence/src/postgres/reservation.rs` — change `token_from_storage` from private to `pub(crate)` (AD-3); no behavior change, reused unchanged by Phase 3.

## Phase 2: Migration — PR 1

- [x] 2.1 Create `crates/persistence/src/postgres/migrations/016_create_projection_claims.sql`: `projection_claims(projection_id, tag, tenant, owner_id, fencing_token, lease_until, claimed_at)`, `PRIMARY KEY (projection_id, tag, tenant)`, `CHECK (fencing_token > 0)`, no `state` column, no index on `lease_until` (AD-8, D-6). Traces: "Claim Identity Is `(projection_id, tag, tenant)`".
- [x] 2.2 Register as an `include_str!` const + one ascending entry in `migrations.rs::migrations()`. No new test needed — run `cargo test -p ego-persistence migrations` to confirm the existing registry tests cover `016`.

## Phase 3: PostgreSQL Adapter — PR 2

- [x] 3.1 GREEN `crates/persistence/src/postgres/read_side_claim.rs`: `PostgreSQLReadSideClaimStore { pool: PgPool, clock: Arc<dyn Clock> }`, manual `Debug` (pool only), `is_durable() -> true`; `try_claim` as the single `INSERT … ON CONFLICT (projection_id, tag, tenant) DO UPDATE … WHERE projection_claims.lease_until <= $now RETURNING fencing_token` statement — no check-then-act window (AD-5).
- [x] 3.2 GREEN: shared `mutate_owned`-shaped private helper for `renew`/`release`, verifying `(projection_id, tag, tenant, owner_id, fencing_token, lease_until > now)` in one `WHERE` per statement; `release` sets `lease_until = now`, never `DELETE`, keeping the fencing token strictly monotone across the release boundary (AD-5 criteria).
- [x] 3.3 GREEN: `claim_error` mapping reuses PROD-014B's `pub(crate) is_fatal` verbatim for the `Transient`/`Fatal` split, with SQLSTATE `22003` (`numeric_value_out_of_range`) checked first → `ClaimError::FencingExhausted`.
- [x] 3.4 GREEN: `crates/persistence/src/postgres/mod.rs` — `pub use read_side_claim::PostgreSQLReadSideClaimStore;`.

## Phase 4: Real-Postgres Contention Suite — RED before Phase 3 GREEN (`integration-tests/tests/infrastructure/read_side_claiming_postgres.rs`) — PR 2

Written against a not-yet-existing `PostgreSQLReadSideClaimStore` per D-7/AD-10; compile
failure is the expected RED state. Harness mirrors `takeover_fencing_postgres.rs` /
`concurrent_replicas_postgres.rs`: `isolated_database()` per test, separate pools per
contender, `SettableClock` moved by hand, `tokio::sync::Barrier`, `AtomicUsize` observers,
bounded `WAIT_LIMIT` assertions, final state read back with raw `sqlx::query_as` never
through the port under test.

- [x] 4.1 RED — SC-1 exclusion: two workers, two pools, two `OwnerId`s, released together onto one `(projection_id, tag, tenant)`; exactly one gets `Some(fence)`, the refused worker's fetch/handler counters are both 0. Control case: the same two workers on two different tenants both obtain a fence and both run. Traces: "Acquisition Excludes A Concurrent Second Claimant".
- [x] 4.2 RED — SC-2 takeover: A claims and never releases (session dropped mid-batch); clock advanced past `lease_until`; B's `try_claim` returns `Some`, `fencing_token` strictly greater, row's `owner_id` is B's. Traces: "An Expired Lease Enables Takeover Without Operator Action".
- [x] 4.3 RED — SC-3 stale-owner rejection: after B's takeover, A's `renew`/`release` both `Err(StaleOwner)`, row still holds B's owner and token unchanged; plus a token-isolation probe — B's `owner_id` paired with A's stale `fencing_token` is also refused, so the refusal is never attributable to `owner_id` alone. Traces: "Takeover Fences Out The Stale Owner".
- [x] 4.4 RED — renewal prevents takeover: A renews before expiry; B's concurrent `try_claim` attempt during the renewed lease is refused. Traces: "A Valid Claim May Be Renewed To Extend Processing".
- [x] 4.5 RED — SC-5 ordering: one worker holds the claim across a batch of at least three events; the handler's received slice is asserted strictly ascending by `event_version`. Traces: "Claiming Preserves Existing Per-Stream Ordering".
- [x] 4.6 RED — immediate reclaim on release: a worker releases normally; a second `try_claim` immediately after succeeds without waiting for lease expiry. Traces: "Normal Release Makes the Stream Immediately Reclaimable".
- [x] 4.7 Mutation checks, recorded in the suite's module doc rather than assumed: deleting `AND projection_claims.lease_until <= $6` from `try_claim`'s `WHERE` must make 4.1 fail with both workers claiming; deleting `AND fencing_token = $6` from the shared fence `WHERE` must make 4.3's token probe fail. Confirmed by hand once, documented, not left in the delivered diff as a broken state.
- [x] 4.8 GREEN: confirm 4.1–4.6 pass once Phase 3's adapter lands.

## Phase 5: Session Wiring — PR 3

- [ ] 5.1 RED `crates/domain/src/read_side/session.rs` `#[cfg(test)]`, scripted doubles, no pool: a refused `try_claim` (`Ok(None)`) ⇒ `fetch` never called, handler never invoked, `execute()` returns `Ok(None)` (IS-4, AD-4).
- [ ] 5.2 RED: `renew` returning `StaleOwner` ⇒ no `mark_seen`, no `write_offset`, error propagates as `ProjectionError::transient` naming the withheld writes (AD-6).
- [ ] 5.3 RED: `release` is called on the success path, both empty-early-return paths (`events.is_empty()`, `unique_events.is_empty()`), and the handler-error path.
- [ ] 5.4 GREEN: add `ReadSideClaiming { store, owner, clock, lease }` and `with_claiming(...)` as an optional knob — every existing `ReadSideSession::new` call site compiles unchanged; split `execute()` into the `try_claim` gate + extracted `run_batch` body, with `release` called unconditionally on every exit path (AD-6).
- [ ] 5.5 GREEN: insert the `renew` call between `handler.handle()` and the commit loop inside `run_batch`; map `StaleOwner` to `ProjectionError::transient` with AD-6's exact wording, other errors to `ProjectionError::transient(format!("claim renew failed: {other}"))`.
- [ ] 5.6 Rustdoc on `ReadSideClaiming::owner`: state `OwnerId` per-process-instance uniqueness is the host's obligation, the port cannot verify it, and name the consequence of violating it (documented Open Question — not a code gap to close).
- [ ] 5.7 `crates/domain/src/read_side/mod.rs`: re-export `claim` types at the module's existing path shape, mirroring `offset`/`dedup`.

## Phase 6: Scheduler Wiring — PR 3

- [ ] 6.1 RED `crates/runtime/src/read_side/scheduler.rs`: `ProjectionSpec::claims(claiming)` sets the knob, absent by default (mirrors `reporter`/`interval`/`on_error`); `TagSchedulerImpl::start_projection` attaches it to each session it constructs (AD-7).
- [ ] 6.2 GREEN: add `pub fn claims(mut self, claiming: ReadSideClaiming) -> Self` to `ProjectionSpec`; move `spec.claiming` onto `TagSchedulerImpl` inside `spawn`; `start_projection` reads `self.claiming` and calls `.with_claiming(...)` when present. `TagScheduler::start_projection`'s public signature stays unchanged — no external implementor breaks.
- [ ] 6.3 Confirm `start_projection` remains today's sequential for-loop — no cross-tag concurrency added (D-12, OOS-5); no cross-tick claim state, no in-memory fence cache.

## Phase 7: Production Gate — PR 4

- [x] 7.1 RED `crates/service-sdk/src/runtime/builder.rs`: matrix {Dev, Production} × {no progress / no claim store, progress registered / no claim store, progress registered / volatile claim store, progress registered / durable claim store}; Production + zero progress registered + no claim store ⇒ `Ok` (the early-return-inside-the-function shape, PROD-014A EC-1); `build()`/`try_build()` agree (SC-4).
- [x] 7.2 GREEN: add `read_side_claims: Option<Arc<dyn ReadSideClaimStore + Send + Sync>>` field; `validate_read_side_claim_profile` returns `Ok(())` early when `self.read_side_progress.is_empty()`, else calls `require_durably_configured(self.profile, self.read_side_claims.as_ref().is_some_and(|c| c.is_durable()), "durable read-side claim store (ReadSideClaimStore)", "AppBuilder::read_side_claims(store) (or RuntimeBuilder::with_read_side_claim_store(..))")` verbatim; called from `validate_persistence_profile` after the existing two validators (AD-9).
- [x] 7.3 RED `crates/service-sdk/src/app/error.rs`: `CompositionError::DuplicateReadSideClaimStore` message names the offending call, suggests no replace API (mirrors `DuplicateReadSideProgress`, PROD-014A 3.1).
- [x] 7.4 GREEN: add the variant; `crates/service-sdk/src/app/mod.rs` — `AppBuilder::read_side_claims(store)` with a fail-closed duplicate guard; `RuntimeBuilder` stays last-write-wins (mirrors `effect_store`'s split, AD-9 criteria d).

## Phase 8: Reference-App & Docs — PR 4

- [x] 8.1 `examples/reference-app/src/read_side/mod.rs:118-126`: retire the "PROD-014C is the unenforced gap" comment; wire (or explicitly document the absence of) a claim store registration, reflecting the now-enforced mechanism.
- [x] 8.2 `ARCHITECTURE.md:211-219`: replace the single-writer-unenforced language with the enforced-claiming description, naming `read-side-event-claiming`.
- [x] 8.3 Confirm `openspec/changes/prod-014c-atomic-read-side-event-claiming/specs/{read-side-event-claiming,read-side}/spec.md` (already drafted) are the exact deltas `sdd-archive` merges — no further edit needed at this task.
- [x] 8.4 Grep-gate (SC-6, R-1): confirm no file touched by this change asserts this capability's own guarantee as "exactly-once" — a hit inside OOS-2/D-8's own non-goal wording is expected; a hit claiming achieved exactly-once is not and must be fixed before merge.

## Phase 9: Final Verification — PR 4

- [x] 9.1 `cargo test --workspace` zero new failures (SC-5); `cargo clippy --workspace -- -D warnings` clean; confirm no touched function exceeds cognitive-complexity 10.
- [x] 9.2 Re-run `cargo test -p ego-integration-tests --test read_side_claiming_postgres`; confirm 4.1–4.6 all GREEN, and 4.7's ablation checks are documented but never left broken in the delivered diff.
- [x] 9.3 Diff-read confirmation (no code change): every SQL statement across `read_side_claim.rs` and `016_create_projection_claims.sql` binds via `$N`, zero string interpolation (Threat Matrix — Rules 1/2 closed by construction).

## Traceability Audit

All ADDED (`read-side-event-claiming`) and MODIFIED (`read-side`) requirements mapped to at
least one covering task:

| Requirement | Capability | Covering task(s) |
|---|---|---|
| Claim Identity Is `(projection_id, tag, tenant)` | `read-side-event-claiming` | 1.2, 2.1, 4.1 |
| Acquisition Excludes A Concurrent Second Claimant | `read-side-event-claiming` | 3.1, 4.1 |
| A Valid Claim May Be Renewed To Extend Processing | `read-side-event-claiming` | 3.2, 4.4 |
| An Expired Lease Enables Takeover Without Operator Action | `read-side-event-claiming` | 3.1, 4.2 |
| Takeover Fences Out The Stale Owner | `read-side-event-claiming` | 3.2, 5.2, 4.3 |
| Normal Release Makes the Stream Immediately Reclaimable | `read-side-event-claiming` | 3.2, 4.6 |
| Claiming Preserves Existing Per-Stream Ordering | `read-side-event-claiming` | 5.4, 4.5 |
| Expiry Is Evaluated Consistently, Never Against An Individual Worker's Own Clock | `read-side-event-claiming` | 3.1 (injected `Clock`), 5.4, 5.5 |
| `Profile::Production` Fails Closed Without A Durable Claim Mechanism | `read-side-event-claiming` | 7.1, 7.2 |
| This Capability Bounds Handler-Execution Count, Never External Side-Effect Count | `read-side-event-claiming` | 5.6, 8.4 |
| Prevention of Double Handler Execution Is Enforced By Atomic Claiming Across Replicas | `read-side` (MODIFIED) | 5.4, 5.5, 4.1 |
| The Concurrency Gap Named In PROD-014B Is Discharged By Atomic Claiming | `read-side` (MODIFIED) | 8.1, 8.2 |

**Scope-boundary cross-check against proposal's Out of Scope and design's OOS references —
zero findings.** No task in this list adds: distributed consensus, leader election, or a
broker (OOS-1); an exactly-once external-effect guarantee (OOS-2 — 8.4 grep-gates the
wording instead); retry/backoff for `Transient` errors (OOS-3 — untouched); cross-table
dedup/offset atomicity (OOS-4 — 5.4/5.5 name the residual window, never close it); intra-
process cross-tag concurrency (OOS-5 — 6.3 confirms `start_projection` stays sequential); or
any backend other than PostgreSQL (OOS-6 — every adapter task targets
`crates/persistence/src/postgres/`). The three Open Questions in design.md (residual
fence/write window, `OwnerId` per-process uniqueness, ungoverned out-of-composition-root
projections) are documented as accepted limitations (5.6, AD-6's own wording) — no task
attempts to close them.
