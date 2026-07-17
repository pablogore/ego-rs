# Tasks: CORE-027 — Foundation Integrity Gate

Strict TDD Mode is enabled (`cargo test --workspace`). Phases are ordered per
design.md §1: **fix `layers.toml` first** (data), **then build `xtask`
against it** (enforcement), so the checker is green on day one. `xtask`
unit tests ship with the checker (design.md §6). Flaky-test triage
(design.md §4) is evidence-driven and has no dependency on `xtask` — it runs
as a separable, parallelizable track.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~360 (`layers.toml` ~30, `xtask/Cargo.toml` + `xtask/src/main.rs` + unit tests ~260, workspace `Cargo.toml` +1, `flaky-triage.md` ~60, conditional fix commits excluded) |
| 400-line budget risk | Medium — close to budget; a reproducing flaky fix pushes it over |
| Chained PRs recommended | Yes |
| Suggested split | PR1 → PR2 (parallel) → PR3 (conditional) |
| Delivery strategy | ask-on-risk → resolved: chained (2-3 PRs) |
| Chain strategy | stacked-to-main — PR2 targets PR1's branch; PR3 (if needed) targets PR2's branch |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Fix `layers.toml` + build `xtask` (`verify-layers`, `verify-isolation`, `verify-hygiene`) with unit tests — Phases 1-3 | PR1 | `cargo test -p xtask` | `cargo run -p xtask -- verify-layers \| verify-isolation \| verify-hygiene` against real workspace | Revert `layers.toml`, delete `xtask/`, revert workspace `Cargo.toml` member add — no other file touched |
| 2 | Flaky-test triage, all three suspects — Phase 4 | PR2 (parallel with PR1) | `cargo test -p persistent-entity`, `cargo test -p runtime`, per suspect | 200-run loop + 50 parallel sweeps per suspect (design.md §4 protocol) | Delete/revert `flaky-triage.md`; independent of PR1 |
| 3 | Conditional fix commit(s) if a suspect still reproduces | PR3 (only if triggered) | Suspect's own focused test, re-run 200x clean | N/A unless a fix changes runtime timing — re-run full triage protocol post-fix | Revert the individual fix commit; PR1/PR2 unaffected |

## Phase 1: Fix `layers.toml` (Data, Before Checker)

Spec ref: FR-001 (complete/accurate map). Design ref: §2 Layer Assignments,
§1 ordering.

- [x] 1.1 Remove the dead `"runtime-slice" = "domain"` entry from
  `layers.toml`.
- [x] 1.2 Add all 8 missing crates with the exact layer assignments from
  design.md §2's table: `ego-persistence` → infrastructure,
  `ego-event-adapter` → infrastructure, `persistent-entity` → foundation,
  `ego-security-sdk` → cross-cutting, `security-apikey` → infrastructure,
  `ego-service-sdk` → sdk, `ego-service-sdk-macros` → tooling, `ego-testkit`
  → tooling.
- [x] 1.3 Update `layers.toml`'s header comment: replace the
  `scripts/verify-layers.sh` reference with the real tool
  (`cargo run -p xtask -- verify-layers`); add `cross-cutting` / `sdk` /
  `tooling` to the documented allowed-layers list and the direction rules
  from design.md §2's allowed-dependency matrix.
- [x] 1.4 Verify manually: `layers.toml` now has exactly 16 entries (one per
  `crates/*` workspace member, excluding `examples/reference-app` and the
  not-yet-added `xtask`), each naming a real crate — confirms FR-001 by
  inspection ahead of the automated check built in Phase 2.

## Phase 2: `xtask` Crate — `verify-layers` (Direction + Cycles + Completeness)

Spec ref: FR-002 (direction), FR-003 (cycles), FR-001 (completeness via
tool), FR-004 (single local command). Design ref: AD-1, AD-2, AD-3.

- [x] 2.1 Create `xtask/Cargo.toml` (package `xtask`, deps: `serde`,
  `serde_json`, `toml`, `anyhow`/similar); add `"xtask"` to the workspace
  `Cargo.toml` `members` list.
- [x] 2.2 RED: unit test with a synthetic layer map + fixture dependency
  graph asserting the direction rule fails an edge whose target layer is
  not in `allowed[layer(source)]` (design.md §2 matrix), and passes when it
  is.
- [x] 2.3 GREEN: implement direction-rule check consuming `cargo metadata
  --format-version 1` output — iterate `workspace_members`, take each
  package's normal + build dependencies whose name is also a workspace
  member (AD-2; dev-dependencies excluded).
- [x] 2.4 RED: unit test asserting Tarjan SCC over a fixture graph with a
  3-crate cycle reports that cycle, and a fixture graph with no cycle
  reports none.
- [x] 2.5 GREEN: implement Tarjan SCC cycle detection over the normal-dep
  graph; any SCC of size >1 is a failure (AD-3b).
- [x] 2.6 RED: unit test asserting completeness fails both ways — a real
  workspace member missing from the layer map, and a layer-map entry naming
  a crate absent from the workspace (the dead-`runtime-slice` shape).
- [x] 2.7 GREEN: implement completeness check against `layers.toml` +
  `cargo metadata` workspace members (excludes `examples/*`, per design.md
  §2).
- [x] 2.8 GREEN: wire `verify-layers` subcommand — runs all three checks,
  collects violations by class into one human-readable report, exits `0`
  clean / `1` any violation (AD-3, FR-004).
- [x] 2.9 Verify: `cargo run -p xtask -- verify-layers` passes against the
  now-corrected `layers.toml` (Phase 1) with zero violations.

## Phase 3: `xtask` — `verify-isolation` + `verify-hygiene`

Spec ref: FR-005 (isolation), FR-006 (hygiene). Design ref: AD-4, AD-5.

- [x] 3.1 GREEN: implement `verify-isolation` — loop `cargo check -p
  <crate> --no-default-features` over every `crates/*` workspace member
  (AD-4); report any crate that fails only in isolation.
- [x] 3.2 Verify: `cargo run -p xtask -- verify-isolation` passes for all 16
  crates.
- [x] 3.3 RED: unit test asserting hygiene fails when an un-archived
  `openspec/changes/<name>` dir case-insensitively suffix-matches an
  `archive/<date-prefix>-<name>` dir, and passes when no such duplicate
  exists (AD-5).
- [x] 3.4 GREEN: implement `verify-hygiene` — for each dir under
  `openspec/changes/` (excluding `archive/`), strip the `YYYY-MM-DD-`
  prefix from every `archive/*` name and compare.
- [x] 3.5 Verify: `cargo run -p xtask -- verify-hygiene` passes (no known
  un-archived duplicates remain).
- [x] 3.6 Verify (deliberate-failure check, proposal success criteria):
  temporarily inject a wrong-direction edge, a cycle, and an unmapped crate
  one at a time; confirm `verify-layers` fails on each with the correct
  report, then revert the injections — no injected state is committed.
  (Wrong-direction and unmapped-crate were injected directly into
  `layers.toml` and reverted; each produced the correct `verify-layers`
  failure report. The cycle class could not be injected into a real
  Cargo-resolved workspace: adding a real path-dependency edge to close a
  2-crate loop made `cargo` itself refuse to resolve the workspace before
  `xtask` ever ran (`error: cyclic package dependency`), which is Cargo's
  own structural guarantee, not `xtask`'s. Cycle detection is instead
  proven by the `cycles::tests` unit tests over synthetic fixtures — the
  only reachable place a cycle can exist for this checker to catch.)

## Phase 4: Flaky-Test Triage (Parallel Track, No `xtask` Dependency)

Spec ref: FR-007 (resolved verdicts). Design ref: §4 protocol.

- [x] 4.1 Run the persistent-entity suspect (full protocol, design.md §4 —
  no analytical model for this suspect's failure mode): N=200 tight-loop +
  N=50 full-crate parallel sweeps:
  `registry.rs::concurrent_lookups_for_one_triple_spawn_exactly_once` and
  `mailbox.rs::close_and_drain_races_concurrent_sends_without_losing_envelopes`;
  verdict recorded: non-reproducing.
- [x] 4.2 Run the effects suspect (N=30 tight-loop — design.md §4's
  analytically-bounded-root-cause bar, since the fix makes the race
  deterministic by construction):
  `effects/acceptor.rs::acceptance_in_progress_is_cancelled_once_the_deadline_instant_actually_elapses`,
  `::lost_wakeup_pattern_is_reproduced_with_a_widened_race_window`, and
  `effects/runner.rs::shutdown_reaches_drain_deadline_despite_a_hung_backpressure_permit_wait`;
  verdict recorded: fixed.
- [x] 4.3 Grepped `crates/runtime/src/providers/access.rs` — the failure
  point is the shared `capture_events` test helper, not a single named test;
  ran the full N=200 protocol against it (no analytical model); verdict
  recorded: fixed.
- [x] 4.4 Created `openspec/changes/core-027-foundation-integrity-gate/flaky-triage.md`
  recording, per suspect: test name(s), actual run counts, verdict, plus one
  out-of-scope discovery (see below).

## Phase 5: Conditional Fixes (Only If a Suspect Reproduces)

- [x] 5.1 persistent-entity suspect did not reproduce — no fix needed.
- [x] 5.2 Effects suspect reproduced: fixed in `crates/runtime/src/effects/acceptor.rs`
  (widened test backoff cap to remove a real jitter/deadline collision
  probability). Re-run clean (30/30).
- [x] 5.3 Provider-access suspect reproduced: fixed in
  `crates/runtime/src/providers/access.rs` (`capture_events` reads through
  the `Mutex` instead of `Arc::try_unwrap`). Re-run clean across multiple
  parallel sweeps.
- [x] 5.4 Neither fix changes `persistent-entity` or `external-effects`
  spec-level behavior — both are test-helper/test-fixture corrections, not
  behavior changes. No spec delta needed.
- [x] 5.5 (Not in original scope, added during triage) A fourth flaky test —
  `effects/observability.rs::every_signal_redacts_the_idempotency_key_and_never_carries_the_raw_key_or_payload` —
  was discovered incidentally during the same parallel sweeps. Root cause
  confirmed against `tracing-core` 0.1.36 source: a callsite's `Interest` is
  cached process-wide on first hit, resolved against whichever thread
  reaches it first — if that thread has no subscriber installed (true for
  `effects::runner`/`effects::acceptor`'s own tests, which call the same
  production `log_*` functions directly), the callsite permanently caches
  "no one's listening", so a later `capture_events` call can silently miss
  it. Fixed in `effects::observability::tests::ensure_interest_cache_race_immune`:
  installs one real, always-enabled subscriber as `tracing`'s global default
  via the public `tracing::subscriber::set_global_default`, once, before any
  test runs — no undocumented internals, no leaked guards. Re-verified: 5x
  clean `--test-threads=64` full-crate sweeps plus the full 50-sweep
  protocol (see `flaky-triage.md`).

## Phase 6: Final Verification

- [x] 6.1 `cargo test --workspace` passes with `xtask`'s unit tests
  included.
- [x] 6.2 All three `xtask` subcommands pass against the final workspace
  state: `verify-layers`, `verify-isolation`, `verify-hygiene`.
- [x] 6.3 Confirmed no `.github/workflows/` or Dagger files were added or
  modified.
