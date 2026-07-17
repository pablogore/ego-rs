# CORE-027 — Flaky-Test Triage Report

Protocol per design.md §4: `N` scaled to whether the root cause has a known
probability model. Suspects with no analytical model (persistent-entity,
provider-access) ran the full `N = 200` tight-loop + `~50` full-crate
parallel sweeps. The effects suspect, whose fix reduces a *quantified*
collision probability to a negligible ~3×10⁻⁸/run (not eliminated in
principle, just far below what any practical N could resample — not just
empirically rare), ran `N = 30` tight-loop — see design.md §4 for the full
reasoning. Every verdict below is backed by actual run counts, not
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
  negligible, though not zero — the deadline overwhelmingly cuts the sleep
  short before it could run to completion, but the race is reduced in
  probability, not removed in principle. Re-verified: 200/200 tight-loop clean
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

**First fix attempt (rejected on review, both blocking):** `ensure_interest_cache_race_immune()`
in `effects::observability::tests` tried installing one always-enabled
subscriber as `tracing`'s process-wide default via
`tracing::subscriber::set_global_default`, called lazily from inside
`capture_events`. Review caught two real problems:

- **F-01 (blocking):** the doc comment claimed this installs "before any test
  runs", but it only ran the first time *some* `capture_events` call fired —
  `effects::runner`'s and `effects::acceptor`'s own tests call the same
  production `log_*` callsites directly with no subscriber at all, and
  nothing ordered those ahead of or behind the lazy install. A prior attempt
  at this same fix additionally relied on an undocumented internal
  `tracing-core` field (`has_just_one`) and leaked (`mem::forget`) dispatch
  guards to flip it — also rejected, for depending on non-contractual
  internal behavior.
- **F-02 (non-blocking but real):** `let _ = set_global_default(...)` silently
  ignored failure — if anything else had already installed a global default
  first, the fix would report as "applied" without actually having installed
  the always-on subscriber, and the calling code had no way to know.

**Actual fix:** stopped depending on `tracing`'s dispatch/interest-cache
machinery for correctness at all. Each per-effect signal's field
construction and redaction (`effect_fields`, in `effects::observability`) is
now a pure, deterministic function, called by both the `log_*` functions
(which pass its output straight into `tracing::info!`/`warn!`) and by tests
directly — so redaction, payload-absence, and correct values are asserted
by calling `effect_fields`/`oldest_pending_age_ms` directly, never by
capturing through a subscriber. This closes F-01 by removing its
precondition entirely (correctness no longer depends on harness ordering)
and closes F-02 by removing `set_global_default` (and
`ensure_interest_cache_race_immune`, and the shared `CAPTURE_EVENTS_GUARD`)
outright — there is no global state left to install or silently fail to
install. `providers::access`'s `capture_events` reverted to its simpler,
already-correct pre-investigation form (Mutex-read, no shared guard).

A small `tracing`-capturing "wiring" test was tried as a belt-and-braces
smoke test alongside the deterministic tests, then deliberately deleted: 5x
`--test-threads=64` sweeps showed it still flaked (1/5) via the exact same
interest-cache race, since it necessarily still called a shared `log_*`
callsite through `with_default`. It added no coverage beyond what
`tracing`'s own macro-expansion already guarantees at compile time (the
field names/values passed to `info!`/`warn!` match what was computed), so
removing it was strictly better than chasing its last flake.

**Re-verified:** `cargo test -p ego-runtime` green (144 tests); 5x
`--test-threads=64` full-crate sweeps clean, zero flakes; full 50-sweep
protocol result below.

## Full-crate sweep (all four fixes in place)

- `cargo test -p ego-runtime -- --test-threads=$(nproc)*4`, 50 runs: see
  `RESULT ego-runtime full-sweep (post-fix, includes observability.rs)` in
  the protocol run log for the final pass count.
