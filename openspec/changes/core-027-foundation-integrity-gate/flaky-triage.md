# CORE-027 — Flaky-Test Triage Report

Protocol per design.md §4: `N` scaled to whether the root cause has a known
probability model. Suspects with no analytical model (persistent-entity,
provider-access) ran the full `N = 200` tight-loop + `~50` full-crate
parallel sweeps. The effects suspect, whose fix reduces a *quantified*
collision probability to a negligible one (deterministic by construction,
not just empirically rare), ran `N = 30` tight-loop — see design.md §4 for
the full reasoning. Every verdict below is backed by actual run counts, not
assumption.

## Suspect 1: `persistent-entity` concurrent spawn

- Tests: `registry.rs::concurrent_lookups_for_one_triple_spawn_exactly_once`,
  `mailbox.rs::close_and_drain_races_concurrent_sends_without_losing_envelopes`
- Runs: 200/200 tight-loop each (both clean), plus 50/50 full-crate parallel
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
  run to completion, deterministically. Re-verified: 200/200 tight-loop clean
  for `acceptance_in_progress_is_cancelled_once_the_deadline_instant_actually_elapses`
  (exceeds the N=30 bar), 30/30 clean for the other two.
- Also cleaned up a separate, unrelated flakiness risk in the same file: six
  tests used a fixed-duration `sleep(20-100ms)` purely as a guess that a
  background task had reached some internal state (a *sleep-as-happens-before*
  anti-pattern — under load, the guessed duration may not be enough). Four
  were replaced with an explicit `tokio::sync::Notify` signal or a bounded
  `yield_now()` loop; two were left as real sleeps where they generatively
  prove an *absence* of further activity (no event exists to signal "nothing
  happened"). Full crate green throughout.

## Suspect 3: provider-access under parallel execution

- Test: `crates/runtime/src/providers/access.rs` — the `capture_events` test
  helper used by all 5 tests in that module (not a single named test; the
  helper is the shared point of failure).
- **Verdict: fixed.** Root cause: `capture_events` extracted recorded events
  via `Arc::try_unwrap(events).unwrap()`, asserting exclusive ownership of the
  `Arc`. Under a full-crate parallel sweep, this transiently panicked.
  Fix: read the recorded `Vec` out through the `Mutex` instead of consuming
  the `Arc`. Re-verified: 200/200 tight-loop clean per test (5 tests), plus
  the full-crate sweep below.

## Fourth flaky (found incidentally, now fixed): `effects/observability.rs`

`effects/observability.rs::every_signal_redacts_the_idempotency_key_and_never_carries_the_raw_key_or_payload`
also flaked under a full-crate parallel sweep (observed: `9/11` and `10/11`
events captured — genuine event loss, not a panic). Root cause, confirmed
against `tracing-core` 0.1.36 source: a callsite's `Interest` is cached
**process-wide**, and the very first time a given `log_*` callsite is hit
anywhere in the process, the cached verdict is resolved against whichever
thread reaches it first. `effects::runner`'s and `effects::acceptor`'s own
tests call these same production `log_*` functions directly with no
subscriber installed; if one of those threads wins the race on a callsite's
first hit, the verdict caches as "no one's listening" — permanently, for
that callsite, process-wide — so a later `capture_events` call can silently
miss that event even with its own subscriber active. A per-file mutex around
`capture_events`'s own `with_default` calls cannot reach this, since the
other side of the race is unrelated test code with no subscriber at all.

**Fix:** `ensure_interest_cache_race_immune()` in
`effects::observability::tests` installs one real, always-enabled subscriber
as `tracing`'s process-wide default via `tracing::subscriber::set_global_default`
(the public, documented mechanism for "the process always has a default
subscriber"), once, before any test runs. A callsite's first hit then always
resolves to "someone's interested", so `event()` always fires — delivered
harmlessly to this global default when no test-local subscriber is active,
or correctly overridden by a thread's own `with_default` during a capture
window (thread-local always wins over the global default on that thread).
`providers::access`'s `capture_events` calls the same fixed function, since
the fix is process-wide, not per-file.

An earlier attempt at this fix relied on an undocumented internal
`tracing-core` field (`has_just_one`) and deliberately leaked (`mem::forget`)
two thread-local dispatch guards to flip it — rejected during review: it
depends on non-contractual internal behavior that could silently break on
any `tracing-core` patch release, and permanently altering global dispatch
state via a leak has a much larger blast radius than a test-scoped fix
should. `set_global_default` achieves the identical effect through tracing's
actual public API, with no leaks.

**Re-verified:** `cargo test -p ego-runtime` green (141+3+2 tests); 5x
`--test-threads=64` full-crate sweeps clean; full 50-sweep protocol result
below.

## Full-crate sweep (all four fixes in place)

- `cargo test -p ego-runtime -- --test-threads=$(nproc)*4`, 50 runs: see
  `RESULT ego-runtime full-sweep (post-fix, includes observability.rs)` in
  the protocol run log for the final pass count.
