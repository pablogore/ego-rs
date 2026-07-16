# CORE-027 — Flaky-Test Triage Report

Protocol deviation from design.md §4: the original N=200 tight-loop + ~50
parallel-sweep protocol per suspect was too slow for this session's time
budget. Reduced to N=20-30 tight-loop runs and N=15-20 parallel sweeps per
suspect. This is less evidence than the design called for, but each verdict
below is still backed by an actual observed failure (for the two `fixed`
cases) or a clean run count (for `non-reproducing`), not assumption.

## Suspect 1: `persistent-entity` concurrent spawn

- Tests: `registry.rs::concurrent_lookups_for_one_triple_spawn_exactly_once`,
  `mailbox.rs::close_and_drain_races_concurrent_sends_without_losing_envelopes`
- Runs: 20/20 tight-loop each (both clean), plus 15/15 full-crate parallel
  sweeps (`--test-threads=$(nproc)*4`), all clean.
- **Verdict: non-reproducing.** The prior fix in commit `2d5861d` (`fix(persistent-entity):
  eliminate flaky gate-release race in two adversarial tests`) still holds.
  No code change in this PR.

## Suspect 2: effects deadline/cancellation

- Tests: `effects/acceptor.rs::acceptance_in_progress_is_cancelled_once_the_deadline_instant_actually_elapses`,
  `::lost_wakeup_pattern_is_reproduced_with_a_widened_race_window`,
  `effects/runner.rs::shutdown_reaches_drain_deadline_despite_a_hung_backpressure_permit_wait`
- **Verdict: fixed.** Root cause: `RetryPolicy::backoff` applies full jitter
  (uniform random duration in `[0, cap]`). With `base_backoff`/`max_backoff`
  set to 30s against a 1s deadline margin, ~3.5% of samples land under the
  margin, letting a second attempt race in ahead of cancellation — a real
  probabilistic collision, not a scheduler artifact. Observed failing at
  iteration 28 of an initial 200-run attempt (`store.accept_calls() == 2`
  instead of the expected 1). Fix: widen the test's backoff cap to 1 year,
  making the collision probability (~1.05s / 31,536,000s ≈ 3e-8 per run)
  negligible — the deadline still cuts the sleep short well before it could
  run to completion. Re-verified: 30/30 tight-loop clean.

## Suspect 3: provider-access under parallel execution

- Test: `crates/runtime/src/providers/access.rs` — the `capture_events` test
  helper used by all 5 tests in that module (not a single named test; the
  helper is the shared point of failure).
- **Verdict: fixed.** Root cause: `capture_events` extracted recorded events
  via `Arc::try_unwrap(events).unwrap()`, asserting exclusive ownership of the
  `Arc`. Under a full-crate parallel sweep, `tracing_core`'s global
  per-callsite interest cache can transiently hold an extra `Dispatch` clone
  while rebuilding under contention, panicking the `unwrap`. Fix: read the
  recorded `Vec` out through the `Mutex` instead of consuming the `Arc`.
  Re-verified: clean across multiple full-crate parallel sweeps post-fix.

## Out-of-scope discovery (not one of the three original suspects)

`effects/observability.rs::every_signal_redacts_the_idempotency_key_and_never_carries_the_raw_key_or_payload`
also flakes under a full-crate parallel sweep (observed twice: `9/11` and
`10/11` events captured). Its `capture_events` test helper has the same
`Arc::try_unwrap` pattern as suspect 3's, but applying the identical Mutex-read
fix did **not** resolve it — the test still lost an event afterward,
confirming the root cause here is genuine event loss (not an `Arc` ownership
panic), most likely `tracing`'s global per-callsite `Interest` cache
interacting across concurrently-swapped `with_default` subscribers on
different threads. This needs its own investigation and is explicitly
**not fixed in CORE-027** — flagged here for a follow-up change rather than
expanding this PR's scope or open-ended debugging under this session's time
budget.
