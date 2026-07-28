# Tasks: PROD-014 — Read-Side Production Observability

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~650-800 (SPI head-version query + port extension in domain, read-side health + metric adapter in runtime, reactive-scheduler port wiring, builder registration, testkit fixtures, incl. tests) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Chain strategy | feature-branch-chain (PR1 domain SPI/port → PR2 runtime health+metrics+reactive → PR3 builder registration+testkit); only the tracker merged to develop |
| Delivery strategy | auto-forecast (no explicit ask-on-risk/auto-chain/single-pr label) — treated conservatively |

Decision needed before apply: Yes — resolve the head-version SPI shape fork
(ADR-1 Option A vs the separate-`TargetCheckpointStore` alternative) against the
actual set of `ReadSideStore` implementors.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 1 | Domain: `head_version` on `ReadSideStore`; `on_retry`/`last_progress_at` on the port | PR1 | `cargo test -p ego-domain read_side::store:: read_side::progress::` | Revert `store.rs` method + `progress.rs` port additions |
| 2 | Runtime: `ReadSideHealthContributor` (ADR-3), metric adapter (ADR-4), reactive-scheduler port wiring | PR2 | `cargo test -p ego-runtime read_side::health:: read_side::metrics:: && cargo test -p ego-scheduler` | Delete `runtime/read_side/{health,metrics}.rs`; revert `ego-scheduler/metric.rs` port wiring |
| 3 | Builder registration through the single authority; testkit fixtures | PR3 | `cargo test -p ego-service-sdk runtime::builder:: && cargo test -p ego-testkit read_side_observability::` | Revert `builder.rs` registration diff; delete testkit fixture |

## Phase 1: Domain — Target-Checkpoint Query (ADR-1)

- [ ] TASK-001 RED: failing test in `crates/domain/src/read_side/store.rs` for `ReadSideStore::head_version(tenant, tag)`: a stub store with events up to version N returns `Ok(Some(N))`; an empty stream returns `Ok(None)`; an empty tenant returns `Ok(None)` (fail closed). Traces read-side spec "Read-Side Store Exposes a Target Checkpoint Query".
- [ ] TASK-002 GREEN: add `async fn head_version(&self, tenant: &str, tag: &EventTag) -> Result<Option<i64>, ReadSideStoreError>` to the `ReadSideStore` trait (ADR-1 Option A). AC: TASK-001 green; `cargo build -p ego-domain` succeeds.

## Phase 2: Domain — Observability Port Surface (ADR-4)

- [ ] TASK-003 RED: failing test in `crates/domain/src/read_side/progress.rs` asserting the port exposes `on_retry(projection_id, outcome)` and `last_progress_at(projection_id) -> Option<Instant>` with default (no-op / `None`) bodies, and that `NoopProgressReporter` compiles against the extended trait unchanged.
- [ ] TASK-004 GREEN: add `on_retry` and `last_progress_at` default methods plus a bounded `RetryOutcome` enum (`{ Retried, Exhausted }` or equivalent closed set) to the port; document that `projection_id` is the only permitted key label and that tenant/tag/entity/sequence MUST NOT be labels. AC: TASK-003 green; existing `ProgressReporter` implementors compile unchanged.

## Phase 3: Runtime — Lag Computation Per Model (ADR-2)

- [ ] TASK-005 RED: failing test in new `crates/runtime/src/read_side/health.rs` (or a `lag` module) — polling lag = `max(0, head_version − processed_sequence)`; processed `Sequence(P)` and head `H` yield `H − P`; a processed value exceeding a stale head clamps to `0`. Traces read-side spec "Projection Lag Is Defined Per Read-Side Model".
- [ ] TASK-006 GREEN: implement polling lag from `OffsetStore::read_offset` (processed) and `ReadSideStore::head_version` (target), clamped at 0. AC: TASK-005 green.
- [ ] TASK-007 RED: failing test in `crates/ego-scheduler/` — reactive lag = `max(0, observed_head_sequence − last_sequence_id)` per entity stream, aggregated across entities (assert the exposed value is the aggregate, e.g. max, NOT a per-entity series), with an outstanding gap range counted as undelivered.
- [ ] TASK-008 GREEN: implement reactive lag aggregation over entity streams from bus `sequence_id` and `state.last_sequence_id`, counting `GapInfo` spans. AC: TASK-007 green.
- [ ] TASK-009 RED: failing test — time-since-last-progress = `now − last_batch_completed_at`; a lag-0 projection idle past the stall deadline is NOT flagged stalled, while a lag>0 projection idle past the deadline IS observably stalled. Traces "Time Since Last Progress Complements Lag".
- [ ] TASK-010 GREEN: implement the wall-clock progress complement over the port's `last_progress_at`, driven by an injectable clock. AC: TASK-009 green.

## Phase 4: Runtime — Health State Mapping (ADR-3)

- [ ] TASK-011 RED: failing test in `crates/runtime/src/read_side/health.rs` encoding the full ADR-3 table against configured `lag_degraded_threshold` and `stall_deadline`: (a) `L ≤ thr` AND `T ≤ deadline` ⇒ `Healthy/None`; (b) `L > thr` AND `T ≤ deadline` ⇒ `Degraded` (asserts NOT `Unhealthy`); (c) `L > 0` AND `T > deadline` ⇒ `Unhealthy/Unavailable`; (d) no processed offset ⇒ `Unhealthy/InitializationPending`; (e) `L == 0` AND `T > deadline` ⇒ `Healthy` (idle, not stalled). Only closed `HealthCode` values, no free-text.
- [ ] TASK-012 GREEN: implement the ADR-3 threshold mapping producing `HealthCheck { status, code }` from `(lag, time_since_last_progress, has_processed_offset)`, using existing `HealthCode` members only. AC: TASK-011 green.
- [ ] TASK-013 RED: failing test — a head-version query error resolves the mapping to `HealthCheck { Unhealthy, Some(HealthCode::Unavailable) }` (never a panic, never a leaked message).
- [ ] TASK-014 GREEN: map `Err(ReadSideStoreError)` on the target read to `Unhealthy/Unavailable`. AC: TASK-013 green.

## Phase 5: Runtime — Read-Side HealthContributor

- [ ] TASK-015 RED: failing test — `ReadSideHealthContributor` implements `HealthContributor`, is object-safe (`Arc<dyn HealthContributor>`), `name()` returns the bounded `projection_id`, `requirement()` returns the configured `DependencyRequirement`, and `check()` is probe-independent (identical result regardless of probe). Traces runtime-health-model "The Read-Side Participates as a Health Contributor".
- [ ] TASK-016 GREEN: implement `ReadSideHealthContributor` over the lag/mapping from Phases 3-4. AC: TASK-015 green.
- [ ] TASK-017 RED: failing structural test — the read-side contributor has no liveness path: it exposes only `check()`; liveness (`Runtime::liveness`) consults zero contributors, so a Required read-side reporting `Unhealthy` leaves liveness unaffected while readiness aggregates `Unhealthy`. Traces "Read-Side Health Never Affects Liveness".
- [ ] TASK-018 GREEN: confirm (no production change beyond Phase 5) that the contributor participates only in `readiness()`/`startup()` aggregation, never a liveness call. AC: TASK-017 green.
- [ ] TASK-019 RED: failing `#[tokio::test]` — folding through the existing aggregator: Required stalled read-side ⇒ global `Unhealthy`; Optional stalled read-side (no Required unhealthy) ⇒ global `Degraded`; lagging-but-progressing ⇒ `Degraded`; initial-replay ⇒ `Unhealthy` with `InitializationPending` distinguishable from a Required `DependencyFailure` at the same status. Traces "Read-Side Status Folds by Requirement Like Any Contributor".
- [ ] TASK-020 GREEN: no new fold logic — assert the read-side `HealthCheck` values from Phase 4 fold correctly through the existing PROD-005 `fold`. AC: TASK-019 green.

## Phase 6: Runtime — Bounded-Cardinality Metrics (ADR-4)

- [ ] TASK-021 RED: failing test in new `crates/runtime/src/read_side/metrics.rs` — the emitted metric label set for throughput/retries/errors/lag/time-since-last-progress is a subset of `{ projection_id, read_side_model, outcome }`; assert NO `tenant`, `tag`, entity, offset, or sequence label key is ever present. Traces "Operational Metrics Are Emitted Through a Port With Bounded Labels".
- [ ] TASK-022 GREEN: implement the metric adapter over the observability port, emitting the ADR-4 metrics with bounded labels only, aggregating per-tenant/per-entity lag before emission. AC: TASK-021 green.
- [ ] TASK-023 RED: failing test in `crates/ego-scheduler/src/metric.rs` — reactive throughput/lag/error signals are emitted through the observability port, not tracing exclusively (assert the port is invoked). Traces "The reactive scheduler's signals flow through the port".
- [ ] TASK-024 GREEN: route reactive-scheduler signals through the port in addition to (not instead of) existing tracing. AC: TASK-023 green.

## Phase 7: Service-SDK — Registration Through the Single Authority

- [ ] TASK-025 RED: failing test in `crates/service-sdk/src/runtime/builder.rs` — `RuntimeBuilder::build()` registers a read-side projection's `ReadSideHealthContributor` into the one runtime-owned aggregator via the SAME single authority that registers provider contributors; a runtime with no read-side registered aggregates identically to before. Traces "does not create a parallel readiness model" and "No read-side registered leaves aggregation unchanged".
- [ ] TASK-026 GREEN: wire the read-side contributor registration into the existing construction-phase authority (no second registration channel, no mutable global aggregator mutated by a subsystem). AC: TASK-025 green.

## Phase 8: TestKit — Same-Contract Fixtures

- [ ] TASK-027 RED: failing test in new `crates/testkit/src/read_side_observability.rs` — a fixture with fixed `(processed, target, last_progress_at)` and configured thresholds drives a real aggregator deterministically to `Degraded` (lagging-but-progressing) and to `Unhealthy` (stalled Required), matching ADR-3 exactly.
- [ ] TASK-028 GREEN: implement the deterministic read-side observability fixture over the real `HealthContributor` contract; wire `mod read_side_observability;` + re-export in `crates/testkit/src/lib.rs`. AC: TASK-027 green.

## Phase 9: Cross-Cutting Guarantees & Verification

- [ ] TASK-029: grep-verify hexagonal boundary — no OpenTelemetry or telemetry-backend symbol referenced in `crates/domain/src/read_side/`; metric emission crosses the port only. AC: grep clean.
- [ ] TASK-030: grep-verify bounded labels — no `tenant`/`tag`/entity/sequence value reaches a metric label constructor in `crates/runtime/src/read_side/metrics.rs` or the reactive port wiring. AC: grep/audit clean.
- [ ] TASK-031: confirm the zero-read-side default runtime path is behaviorally unchanged (empty read-side contributes nothing; aggregation identical to PROD-005). AC: pre-existing health suite passes unmodified.
- [ ] TASK-032: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions.
