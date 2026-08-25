# Proposal: PROD-012A — Idempotency Closure Hardening

## Why This Is Its Own Change

PROD-012 ("End-to-End Idempotent Command Processing") was archived as
**Delivered** on 2026-08-20
(`openspec/changes/archive/2026-08-20-prod-012-idempotent-command-processing/`,
promoted to `openspec/specs/idempotent-command-processing/spec.md`). An
archived change is frozen history, not a working document. When a fresh
audit against an already-"Delivered" guarantee finds real gaps, the correct
move is a new atomic follow-up change, not a retroactive edit of the closed
PROD-012 folder or its archive report. This is that follow-up.

## What Was Audited

A 2026-08-25 audit re-examined the PROD-012 guarantee end to end: fingerprint
comparison, `NoEvents` receipts, fencing CAS, dual-aggregate crash recovery,
protocol-neutral key extraction, and tenant/aggregate isolation. The core
mechanism held up — it is proven against real PostgreSQL where PROD-012
claimed it was. The audit's job was to find where the proof was thinner than
the claim.

## What Was Found

One structural bypass and three test-coverage gaps, all now closed:

1. **Structural bypass.** `#[operation]` hardcodes `mutating: true`
   (`crates/service-sdk-macros/src/lib.rs:258-259`), and `#[idempotent]` was
   a fully optional attribute — nothing at the SDK level enforced
   `mutating ⇒ idempotent`. Only a narrower reference-app-specific test
   existed.
2. **Multi-node race gap.** The existing concurrent-replicas test proved
   reservation-ownership fencing against real Postgres, but each replica's
   actual event/aggregate writes went through a private in-memory store, so
   "two nodes racing to commit the same operation, only one durable write
   survives" was never proven end to end.
3. **Single-aggregate crash-recovery gap.** Crash-after-commit recovery was
   proven for the dual-aggregate case only; the single-aggregate case — a
   real process killed after a real commit, then recovered by a fresh
   process — had no equivalent proof.
4. **Isolation-scope gap.** Cross-tenant/type/id isolation for receipts was
   proven structurally/at the catalog level, never functionally against
   real Postgres receipts varying one identity field at a time.

A fifth item was corrected as documentation drift, not a code gap: the
ROADMAP and spec claimed "two conforming adapters — HTTP and gRPC" for
command dispatch. Only HTTP dispatches real commands; the gRPC adapter is
carrier/extraction-only (`GrpcMetadataCarrier` passes the shared harness for
reading the key out of metadata, but no gRPC service/socket/command dispatch
path exists in the workspace — `crates/transport/src/lib.rs:10-32`).

## Why This Is Atomic

This change adds no new capability and no new scope. It closes gaps against
an already-accepted guarantee: one new structural lint, three new
integration tests against real Postgres, and documentation corrections to
match what the code actually proves. No new architectural decision was made
— `design.md`/`decisions.md` are intentionally absent from this change
folder. What remains open (dual-write atomicity, first-value-wins on
duplicate keys, the generic reservation-conformance harness not being
Postgres-parametrized, and `EntityRuntimeBuilder`'s silent in-memory
default) stays open, documented as such, and is explicitly out of scope
here.
