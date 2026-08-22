# PROD-002 — Architecture Reconciliation Baseline (G10–G15)

> Audit evidence, not normative specification. This document records what five
> reconciliation audits found while comparing PROD-002's stashed WIP
> (`opsx/prod-002-durable-external-effect-store`, forked at `cbc0187`) against
> `develop` and against `feat/prod-012-idempotency-tracker` — a change that
> matured horizontal infrastructure (Clock, integration-test layout, retention
> lifecycle, observability) while PROD-002 sat stashed. It does not modify
> `design.md`, `tasks.md`, or `spec.md`. It exists so that any future design
> work — starting with G15 — has a frozen picture of what is already resolved
> and must not be reopened.

**Status: PROD-002 — ARCHITECTURE FROZEN**

G15 is closed (causal gating in `abandon_and_release`, `crates/runtime/src/effects/runner.rs`) without widening `EffectDedupStore`, without a schema change, without reopening AD-6, and without altering the accepted G2 window. The Fault/Crash Semantics audit was re-run against the fixed code: all 11 scenarios are SAFE, bounded only by the already-documented G2/at-least-once residual. The exit condition set below is met.

**Frozen core**: `EffectStateStore`, `EffectDedupStore`, at-least-once delivery semantics, the PostgreSQL claim model, Stoolap's local-durable model, the Tier 1/2/3 conformance structure, the separation from `OperationReservationStore`, and AD-6. None of these reopen except on a bug that violates a spec guarantee.

**G10–G13** remain reconciliation/implementation work (not architectural). **G14** remains non-blocking API cleanup. **G15** is CLOSED.

**Reopening criterion**: only effect loss, state corruption, a spec violation, or a new execution window outside the explicitly accepted guarantees reopens this freeze.

**Discipline change from this point on**: no more exploratory design for PROD-002. Remaining work is execution — reconcile with `develop`, implement G10–G13, resolve G14 when convenient, and resume the pending PR sequence (PR4/PR5).

~~**Exit condition: resolve G15 → update `design.md`/`tasks.md` → re-run the
Fault/Crash Semantics audit → all 11 scenarios must come back SAFE**~~ — met.

---

## G10–G13 — Horizontal infrastructure (do not touch the domain model)

| Gap | Severity | Finding | Resolution direction | Doc impact |
|---|---|---|---|---|
| **G10** — Clock authority | HIGH | `claim_due`/`recover_in_flight` already take a caller-supplied `Timestamp` — deterministic. `mark_in_flight` computes a hardcoded `Utc::now()`; the `mark_*` guards validate lease validity against SQL-side `now()` — two independently-drifting clocks deciding the same lease invariant. | Inject `Arc<dyn Clock>` (mature, `crates/domain/src/time`) into the effect-store backends and `DeliveryRunner`; replace both call sites with the same injected instant. | `design.md` + `tasks.md` |
| **G11** — Integration Tests layout | HIGH | Real-Postgres tests live in `crates/integration-tests/` (a root workspace member) — this actively fails the pre-existing `detect-integration-tests.sh` guard (confirmed by running it: FAIL, check 3, `testcontainers` dependency). | Migrate to the independent top-level `integration-tests/` workspace PROD-012 built (#274/#285); reuse its Postgres factory, template-DB-per-test isolation, cleanup, and concurrency semaphore. | `design.md` + `tasks.md` |
| **G12** — Retention lifecycle | HIGH | `run_retention` (Postgres/Stoolap) is fully implemented but never invoked anywhere in production code — no scheduler exists. | New optional capability `RetentionMaintenance` (2 methods: `purge_before`, `oldest_terminal`), runtime-owned worker in `service-sdk` (same lifecycle shape as PROD-012's `RetentionWorker`), without touching `EffectStateStore`/`EffectDedupStore`. | `design.md` (AD-9 rewrite) + `tasks.md` |
| **G13** — Observability/Metrics | MED | AD-14 is 100% `tracing` macros, zero metrics. Confirmed with evidence (not opinion) that events and metrics are not substitutes — PROD-012's own AD-10a states a counter gives "occurrence and frequency... not enough to investigate one." | ADAPT: add metrics alongside the existing log calls, at the same call sites. Names fixed: `effect.claim.event` (counter, `event ∈ {acquired, reclaimed_after_expiry}`), `effect.recovery.rows` (counter), `effect.cleanup.rows` + `effect.cleanup.batch_duration` (counter + histogram). Owner/epoch/expires_at stay log-only permanently (unbounded by nature); `reason` stays log-only until closed to a typed enum (unbounded only because untyped today). | `design.md` (extend AD-14, don't replace) + `tasks.md`. No `spec.md` change. |

## G14 — API surface cleanup (non-blocking)

`ExternalEffectExecutor::honors_idempotency_key()` (`crates/runtime/src/effects/executor.rs:48-53`) has zero real callers anywhere in the workspace outside its own unit tests — fails Rule of Two. Candidate for DROP or completion, not REPLACE/ADAPT. Does not gate the freeze decision.

## G15 — CRITICAL — the only functional/architectural blocker

`abandon_and_release()` (`crates/runtime/src/effects/runner.rs:835-842`) does not gate `dedup.release()` on `mark_terminal()`'s result — a `Conflict` is logged, never stops the flow. `EffectDedupStore::release()` (`crates/effect-store/src/postgres/mod.rs:737-749`) deletes by `DedupScope` alone — no ownership, epoch, or state guard. A superseded worker's stale attempt can delete a dedup reservation another worker already flipped to `succeeded`, violating AD-8's explicit invariant ("a different later submission must observe `OtherSucceeded`, never `Fresh`"; "a reservation is never deleted while its effect is non-terminal"). This is not the accepted at-least-once tradeoff (the same effect executing twice) — it's dedup memory loss causing a *future, independent* submission to be treated as if it never existed.

Root cause: `EffectDedupStore::release`/`commit_success` only take a `DedupScope` — no `EffectId`, owner, epoch, or reservation identity. The port cannot implement fencing today even if an implementation wanted to. Full evidence, breaking sequence, and solution constraints are in the G15 ledger entry (engram memory, architecture/prod-002-audit-g15-critical-core).

---

## Decisions frozen by this reconciliation — do not reopen

- **`EffectStateStore`** — all 7 methods (`accept`/`mark_in_flight`/`mark_succeeded`/`mark_retryable`/`mark_terminal`/`claim_due`/`recover_in_flight`): universal, zero leakage, third-party-implementable — proven by three structurally distinct implementations (in-memory, Stoolap, Postgres) passing the same Tier 1 conformance harness unmodified.
- **At-least-once delivery** — the accepted tradeoff; not to be strengthened to exactly-once.
- **Postgres claim model** (`FOR UPDATE SKIP LOCKED` + the G1 guard) — atomic, race-free, proven by `claim_due_never_re_stamps_a_row_already_carrying_a_live_claim` and `superseded_worker_write_is_conflict_live_worker_succeeds`.
- **Stoolap's single-owner model** — correct for its documented scope (no lease/ownership columns needed).
- **Tier 1/2/3 conformance structure and placement** — Tier 1 + Tier 2-Stoolap stay crate-local (`crates/effect-store/tests/conformance.rs`); Tier 2/3-Postgres move to the top-level `integration-tests/` per G11.
- **AD-6** (no lease/fencing token on `EffectStateStore`; a single `DeliveryRunner` per process owns the claim→dispatch→mark lifecycle) — reconfirmed valid after G10–G14, independently corroborated by PROD-012's own AD-10a/AD-10c rejecting the identical coupling (a cross-cutting concern onto a persistence port) for a different port, arrived at independently.
- **Separation from PROD-012's `OperationReservationStore`** — its callers aren't funneled through one owning runtime component (different topology), which justifies putting Lease/OwnerFence/FencingToken inside *that* port. This does not mean AD-6's premise is wrong for `EffectStateStore` — nothing in G10–G14 changes that premise.

## Constraint carried into the G15 design work

Do not touch `EffectStateStore`'s trait, the at-least-once contract, or AD-6. Resolve `release`/`commit_success` ownership without contaminating `EffectStateStore` and without importing PROD-012's full fencing model wholesale — different caller topology, different problem shape.
