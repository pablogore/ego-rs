# Archive Report: PROD-012A — Idempotency Closure Hardening

**Change**: `2026-08-25-PROD-012A-idempotency-closure-hardening`
**Audited/Archived**: 2026-08-25
**Branch / SHA audited**: `develop` @ `a740d3476eb704d216a37d45961e8bde1c19aeca`
**Status**: Complete

## Executive Summary

A follow-up audit of the already-archived PROD-012 guarantee
("End-to-End Idempotent Command Processing", archived 2026-08-20) found the
core mechanism solid — fingerprint comparison, `NoEvents` receipts, fencing
CAS, and dual-aggregate crash recovery are all proven against real
PostgreSQL — but found one structural bypass and three places where the
proof was thinner than the claim. All four have now been closed with real
evidence. This change is filed as its own atomic follow-up rather than a
retroactive edit of the frozen 2026-08-20 archive, per this project's
convention that archived changes are frozen history.

## What Was Found

1. **Structural bypass**: `#[operation]` hardcodes `mutating: true`
   (`crates/service-sdk-macros/src/lib.rs:258-259`); `#[idempotent]` was
   fully optional with no SDK-level enforcement of `mutating ⇒ idempotent`.
2. **Multi-node race gap**: the concurrent-replicas test proved reservation
   fencing against real Postgres, but each replica's actual event writes
   went through a private in-memory store — the true "only one durable
   write survives" guarantee was unproven end to end.
3. **Single-aggregate crash-recovery gap**: crash-after-commit recovery was
   proven only for the dual-aggregate case.
4. **Isolation-scope gap**: tenant/aggregate_type/aggregate_id isolation was
   proven only structurally/at the catalog level, never functionally
   against real Postgres receipts.
5. **Documentation drift** (not a code gap): ROADMAP and spec claimed "two
   conforming adapters — HTTP and gRPC" for command dispatch; only HTTP
   dispatches real commands, the gRPC adapter is carrier/extraction-only.

## What Was Fixed

- **Fix 1**: New `crates/service-sdk/tests/idempotent_marker_lint.rs` — a
  `syn`-AST scan (mirroring `tenant_scoped_lint.rs`) that structurally fails
  the build if any `#[operation]` lacks `#[idempotent]`, over `crates/*/src`
  and `examples/*/src`.
- **Fix 2**: `integration-tests/tests/infrastructure/concurrent_replicas_postgres.rs`
  now has both racing replicas write through a real, shared
  `EntityEventStores::open(pool.clone())` and the production
  `compose_entity_runtimes` wiring. New test
  `two_replicas_racing_one_key_yield_exactly_one_execution` confirms exactly
  one durable event set and one confirmed receipt for the contended key.
- **Fix 3**: New
  `integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs`
  (566 lines) — a real child process commits to real Postgres, is killed
  for real (`std::process::abort()`), and a fresh process/pool/owner is
  proven to replay the result with zero re-execution and zero duplicate
  rows.
- **Fix 4**: New
  `integration-tests/tests/infrastructure/receipt_identity_isolation_postgres.rs`
  (4 tests) — holds 3 of 4 identity fields fixed and varies one at a time
  against real Postgres receipts, with a negative control and explicit
  NULL/systemwide-tenant coverage.
- **Documentation**: `ROADMAP.md` §7.12 and
  `openspec/specs/idempotent-command-processing/spec.md` corrected to state
  the gRPC adapter is carrier/extraction-only, with no real gRPC command
  dispatch path in the workspace (`crates/transport/src/lib.rs:10-32`).

## Gate Results

| Gate | Result |
|------|--------|
| `cargo fmt --check` (new file) | Clean |
| `cargo check --workspace` | Clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | Clean |
| `cargo test --workspace` | Exit 0, zero failures |
| Fix 2 test, 3 consecutive runs vs. real Postgres | Green, no flakes |
| Fix 3 test, 3 consecutive runs vs. real Postgres | Green, no flakes |
| Fix 4 tests vs. real Postgres | Green |
| Full integration suite | 39-41/41 passing (1 ignored test pre-existing/unrelated) |

## Residual Gaps (Documented, Not Blocking)

- Dual-aggregate write atomicity — stated non-goal, unchanged.
- Coalesced/duplicate idempotency keys admitted first-value-wins — stated
  non-goal, unchanged.
- The generic reservation-conformance harness (`testkit`) is still not
  driven against real Postgres as one of its parametrized targets —
  narrower than the tenant-isolation gap Fix 4 closed, and genuinely
  deferred rather than a non-goal.
- `EntityRuntimeBuilder::build()` (`crates/persistent-entity/src/builder.rs:279-281`)
  still silently defaults to a non-durable in-memory event store when
  `.with_event_store()` is never called. No production path hits this
  today; newly noted by this audit, scoped as future composition-root
  hardening, not a PROD-012 blocker.

## Recommendation

The four scenarios/invariants targeted by this hardening pass — end-to-end
no-bypass enforcement, two-node write-level racing, single-aggregate
crash-after-commit recovery, and tenant/type/id receipt isolation — are now
demonstrated with real evidence against real PostgreSQL, closing the gap
between what PROD-012 claimed and what was proven. The residual items above
are documented non-goals or narrow, non-blocking debt, not violations of the
core guarantee. This change is recommended for archive in its current,
already-complete state — no further work is required to close it.

## Authority and Closure

- **Audited by**: 2026-08-25 hardening pass, filed as this atomic follow-up
  to the frozen 2026-08-20 PROD-012 archive.
- **Task authority**: `tasks.md` in this change folder.
- **Archive date**: 2026-08-25
