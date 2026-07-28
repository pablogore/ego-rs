# Design: PROD-014 — Read-Side Production Observability

## Technical Approach

Lag is `target − processed`. The read-side records only the PROCESSED position
today: `Offset::Sequence(i64)` (`crates/domain/src/read_side/offset.rs:12-16`),
read/written through the `OffsetStore` SPI (`offset.rs:55-74`, `read_offset` at
`:59`). The read-side store SPI (`ReadSideStore`,
`crates/domain/src/read_side/store.rs:26`) exposes only `fetch(...)` (`:49-55`)
— an offset-paginated forward read. **No head/latest/target query exists
anywhere** on that SPI, so the TARGET half of lag is missing and lag is not
computable today. This design adds a target-checkpoint (head-version) query
(ADR-1), defines lag per read-side model (ADR-2), maps lag/stall state to the
PROD-005 health model with observable thresholds (ADR-3), and enumerates the
bounded metric-label set (ADR-4). The domain keeps only ports and value types;
OpenTelemetry adapters live in runtime — `ego-domain` stays OTel-free.

Two read-side engines exist and their position semantics differ, so lag is
defined for each:

- **Polling projection** — `TagSchedulerImpl::spawn`
  (`crates/runtime/src/read_side/scheduler.rs:247-315`) loops calling
  `start_projection`, and position is the `Offset::Sequence(i64)` last committed
  per `(projection_id, tag, tenant)`.
- **Reactive scheduler** — `crates/ego-scheduler/`, event-bus / gap-driven.
  Position is a per-entity `last_sequence_id: Option<u64>`
  (`crates/ego-scheduler/src/state.rs:25`) over bus `sequence_id`
  (`crates/ego-scheduler/src/event_bus.rs:77`); gaps are recorded as `GapInfo`
  per `EntityTriple` (`crates/ego-scheduler/src/gap.rs:17`). Its signals are
  tracing-only today (`crates/ego-scheduler/src/metric.rs:8`).

The PROD-005 health vocabulary is reused verbatim — `HealthContributor`
(`crates/domain/src/health/mod.rs:177`), `HealthStatus` (`:41`),
`DependencyRequirement` (`:55`), and `fold` (`:155`, which clamps
Optional+Unhealthy → Degraded at `:131`). The read-side contributor registers
through the same single authority the provider contributor uses
(`ProviderHealthContributor`, `crates/runtime/src/providers/health.rs:43`).

## Architecture Decisions

### ADR-1 (DECISION 1): Target-checkpoint query shape on the store SPI → **Option A, a `head_version` method on `ReadSideStore`**

**Choice**: add one method to the existing read-side store SPI —
`async fn head_version(&self, tenant: &str, tag: &EventTag) -> Result<Option<i64>, ReadSideStoreError>`
returning the highest `event_version` currently available for `(tenant, tag)`,
or `None` when the stream is empty. Processed is read from
`OffsetStore::read_offset` (`offset.rs:59`); lag = `head − processed`, clamped
at `0` (a processed offset can momentarily exceed a stale head read).
**Rejected**: (B) a separate `TargetCheckpointStore` SPI; (C) overloading
`fetch` to also return a head.

**Rationale**:

| Option | Tradeoff | Verdict |
|---|---|---|
| A `head_version` on `ReadSideStore` | Same trait already owns the `(tenant, tag, offset)` read model and the `Offset` type; a head query is the natural symmetric read. One method, one implementor surface. Cost: every `ReadSideStore` impl must provide it (documented SPI addition, default-less). | **Chosen** |
| B separate `TargetCheckpointStore` | Keeps the hot-path `fetch` trait minimal and lets lag observability be an opt-in SPI. Cost: a second store to wire, register, and correlate by `(tenant, tag)`; two SPIs to keep consistent for one derived number. | Rejected — splits one cohesive read model |
| C overload `fetch` to report head | No new method. Cost: conflates "give me the next batch" with "how far is the end"; forces a head computation on every hot-path fetch and muddies the `Vec<EventStreamElement>` contract. | Rejected — hot-path contamination |

The reactive scheduler needs no store query: its head is the highest observed bus
`sequence_id` already tracked in-process (`event_bus.rs:77`, `state.rs:25`); the
head-version SPI addition applies to the polling projection's store-backed model.

### ADR-2 (DECISION 2): Lag definition per read-side model → **two model-specific definitions + one shared wall-clock complement**

**Polling projection lag** (store-backed): for a `(projection_id, tag, tenant)`,
`version_lag = max(0, head_version − processed_sequence)` where `head_version`
comes from ADR-1 and `processed_sequence` from `Offset::Sequence`
(`offset.rs:15`, via `OffsetStore::read_offset`). Unit: **events** (version
delta). A projection at `version_lag == 0` is caught up.

**Reactive scheduler lag** (bus/gap-driven): for an entity stream,
`sequence_lag = max(0, observed_head_sequence − last_sequence_id)` over bus
`sequence_id` (`event_bus.rs:77`) and `last_sequence_id` (`state.rs:25`);
outstanding `GapInfo` ranges (`gap.rs:17`) count as undelivered events within
that span. Unit: **events** (sequence delta). Because per-entity is unbounded,
the exposed lag is an AGGREGATE across entity streams for the scheduler instance
(e.g. max sequence_lag), never a per-entity value (ADR-4).

**Shared complement — time-since-last-progress** (both models):
`time_since_last_progress = now − last_batch_completed_at`, a wall-clock duration
since the last successful commit/delivery. This is required because version/
sequence lag alone cannot distinguish "caught up and idle" (lag 0, no recent
progress → healthy) from "behind and stalled" (lag > 0, no recent progress →
unhealthy). The current port
(`ProgressReporter::on_batch_completed`, `progress.rs:27`) records that a batch
completed but exposes NO timestamp/clock and NO retry signal; both are added
(ADR-4 label rules apply).

**Rejected**: a single unified lag number across both engines — their positions
are structurally different (store version vs in-process bus sequence), and one
formula would misrepresent one engine.

### ADR-3 (DECISION 3): Health-state mapping thresholds → **observable event-count + duration boundaries, readiness-only**

Two configured, observable thresholds drive the mapping: `lag_degraded_threshold`
(events) and `stall_deadline` (duration). Let `L` = model lag (ADR-2) and
`T` = time-since-last-progress.

| Condition (observable) | HealthStatus | HealthCode | Rationale |
|---|---|---|---|
| `L ≤ lag_degraded_threshold` AND `T ≤ stall_deadline` | Healthy | None | Caught up / within budget |
| `L > lag_degraded_threshold` AND `T ≤ stall_deadline` | Degraded | None | Behind but still progressing (frozen decision 4) |
| `L > 0` AND `T > stall_deadline` | Unhealthy | `Unavailable` | Behind AND stalled — pipeline not advancing |
| no processed offset yet, initial replay in progress | Unhealthy | `InitializationPending` | Startup, not a failure |
| `L == 0` AND `T > stall_deadline` | Healthy | None | Caught up and idle — NOT a stall |

The `(HealthStatus, DependencyRequirement)` pair then folds through PROD-005's
`fold` (`crates/domain/src/health/mod.rs:155`): a Required projection's Unhealthy
becomes global Unhealthy; an Optional projection's Unhealthy is clamped to
Degraded (`:131`). Every code above is an EXISTING member of the closed
`HealthCode` set (`health/mod.rs:70-81`) — no new code is added.

**Readiness/liveness**: the read-side contributor participates only in readiness
and startup aggregation. It is structurally unable to affect liveness — the
PROD-005 `HealthContributor` trait exposes only `check()` and has no liveness
method (`health/mod.rs:177-190`), and liveness consults no contributor. This
enforces frozen decision 3: a lag/stall condition removes the instance from
rotation (readiness) but never restarts it (liveness).

**Rejected**: a status that flips on lag alone (no `stall_deadline`) — an idle,
caught-up projection would flap; and a wall-clock-only rule — a fast-cycling
stream that never falls behind but pauses briefly would false-positive.

### ADR-4 (DECISION 4): Cardinality-bounding of labels → **closed enumerated dimension set; tenant/tag/entity/sequence forbidden**

Metrics emitted through the observability port MUST carry labels ONLY from this
closed, enumerable set:

- `projection_id` — bounded: the set of projections is fixed at build/registration
  time (`ProjectionSpec.projection_id`, `scheduler.rs`).
- `read_side_model` — bounded: `{ polling, reactive }`.
- `outcome` / result class — bounded: e.g. `{ success, transient_error,
  fatal_error, poison }` for retry/error counters.

FORBIDDEN as labels (unbounded or sensitive): `tenant`, `tag` (CORE-018 makes
tags per-tenant → unbounded), any entity identifier (`EntityTriple`,
`gap.rs:17`), and any raw sequence/offset value. Where a dimension is unbounded,
the metric is AGGREGATED over it before emission (e.g. `max`/`sum` lag across
tenants for a `projection_id`), never labeled per key. This is the workspace
metrics constraint — bounded/low-cardinality labels only; no raw ids/keys/tenants
as labels — applied to the read-side.

Metrics defined (all through the port, adapter maps to OTel):

| Metric | Kind | Labels | Source |
|---|---|---|---|
| `read_side.throughput` | counter | `projection_id`, `read_side_model` | `on_batch_completed` count (`progress.rs:27`) |
| `read_side.retries` | counter | `projection_id`, `outcome` | retry surface (added to port) |
| `read_side.errors` | counter | `projection_id`, `outcome` | `on_error` (`progress.rs:38`) |
| `read_side.lag` | gauge | `projection_id`, `read_side_model` | ADR-2 lag (aggregated over tenants/entities) |
| `read_side.time_since_last_progress` | gauge (seconds) | `projection_id`, `read_side_model` | ADR-2 wall-clock (added to port) |

**Rejected**: per-tenant lag gauges (unbounded cardinality, tenant leakage) and
free-form error-message labels (redaction violation).

## Data Flow

    Lag (polling):   OffsetStore.read_offset ─▶ processed:i64
                     ReadSideStore.head_version ─▶ target:i64        [ADR-1, NEW]
                     lag = max(0, target − processed)                [events]

    Lag (reactive):  bus sequence_id (max observed) ─▶ head:u64
                     state.last_sequence_id ─▶ consumed:u64          [state.rs:25]
                     lag = max(0, head − consumed) + open GapInfo    [aggregated over entities]

    Both:            now − last_batch_completed_at ─▶ time_since_last_progress

    Health:  (lag, time_since_last_progress) ──ADR-3 thresholds──▶ (HealthStatus, HealthCode)
             requirement(Required|Optional) ──▶ HealthCheck ──▶ fold() ──▶ HealthReport(Readiness|Startup)
             [liveness path never reaches here — trait has no liveness method]

    Metrics: port(on_batch_completed | on_error | retry | progress_clock)
             ──runtime adapter (bounded labels only)──▶ OTel   [domain stays OTel-free]

### Sequence: read-side readiness contribution

    Aggregator      ReadSideHealthContributor      OffsetStore      ReadSideStore
      │─check()────────▶│
      │                 │─read_offset(t,tag,ten)──────▶│
      │                 │◀── processed:i64 ────────────┤
      │                 │─head_version(ten,tag)───────────────────▶│   [ADR-1]
      │                 │◀── target:i64 ───────────────────────────┤
      │                 ├ lag = max(0, target−processed)
      │                 ├ T = now − last_progress
      │                 ├ ADR-3: (lag,T) ⇒ (HealthStatus, HealthCode)
      │◀─ HealthCheck{status, code} ──┤
      ├ fold over (status, requirement)  [Optional+Unhealthy ⇒ Degraded]
      └ tag ProbeKind(Readiness|Startup) ⇒ HealthReport
      (Liveness path never invokes check — no liveness method on the trait)

## File Changes

All production files are FUTURE work (planned by this change, not implemented
here).

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/read_side/store.rs` | Modify (FUTURE) | Add `head_version(tenant, tag) -> Option<i64>` to `ReadSideStore` (ADR-1) |
| `crates/domain/src/read_side/progress.rs` | Modify (FUTURE) | Add retry-observed and last-progress-timestamp surface to the port; document bounded-label contract |
| `crates/runtime/src/read_side/health.rs` | Create (FUTURE) | `ReadSideHealthContributor` — reads processed/target, applies ADR-3 thresholds, maps to `HealthCheck` |
| `crates/runtime/src/read_side/metrics.rs` | Create (FUTURE) | Port adapter emitting the ADR-4 metrics with bounded labels; OTel mapping |
| `crates/ego-scheduler/src/metric.rs` | Modify (FUTURE) | Route reactive-model throughput/lag/errors through the port instead of tracing-only |
| `crates/service-sdk/src/runtime/builder.rs` | Modify (FUTURE) | Register the `ReadSideHealthContributor` through the single PROD-005 authority |
| `crates/testkit/src/read_side_observability.rs` | Create (FUTURE) | Deterministic lag/health fixtures (fixed processed/target/clock) |

## Interfaces / Contracts

```rust
// ego-domain::read_side::store — target-checkpoint query (ADR-1)
#[async_trait]
pub trait ReadSideStore<E> {
    async fn fetch(/* unchanged */) -> Result<Vec<EventStreamElement<E>>, ReadSideStoreError>;

    /// Highest `event_version` currently available for `(tenant, tag)`,
    /// or `None` when the stream is empty. The TARGET checkpoint; lag =
    /// max(0, head_version − processed). An empty `tenant` MUST return
    /// `Ok(None)` (fail closed, mirroring `fetch`).
    async fn head_version(
        &self,
        tenant: &str,
        tag: &EventTag,
    ) -> Result<Option<i64>, ReadSideStoreError>;
}

// ego-domain::read_side::progress — bounded-label observability surface (ADR-4)
pub trait ProgressReporter: Send + Sync {
    fn on_batch_completed(&self, projection_id: &str, tag: &EventTag, count: usize, offset: &Offset);
    fn on_error(&self, projection_id: &str, error: &str);
    fn on_state_transition(&self, projection_id: &str, from: ProjectionState, to: ProjectionState);
    /// A retry attempt was made. `outcome` MUST be a bounded class, never a
    /// free-form message; `projection_id` is the only key label permitted.
    fn on_retry(&self, projection_id: &str, outcome: RetryOutcome) { let _ = (projection_id, outcome); }
    /// Wall-clock instant of the last successful progress, for
    /// time-since-last-progress. Never labeled by tenant/tag/entity.
    fn last_progress_at(&self, projection_id: &str) -> Option<std::time::Instant> { let _ = projection_id; None }
}

// ego-runtime::read_side::health — read-side contributor (ADR-3), reuses PROD-005 types
pub struct ReadSideHealthContributor { /* projection_id, stores, clock, thresholds, requirement */ }

#[async_trait]
impl HealthContributor for ReadSideHealthContributor {
    fn name(&self) -> &str;                       // == projection_id (bounded)
    fn requirement(&self) -> DependencyRequirement;
    async fn check(&self) -> HealthCheck;         // ADR-3 mapping; probe-independent, no liveness
}
```

## Error Model

A failing head-version query MUST NOT crash aggregation: an
`Err(ReadSideStoreError)` while reading target resolves the contributor to
`HealthCheck { Unhealthy, Some(HealthCode::Unavailable) }` (the store cannot
report progress), never a panic or a leaked message. Metric emission is
best-effort and diagnostic — a reporter error MUST NOT affect projection
correctness (mirrors `progress.rs:22-23`). No free-text crosses the health
boundary; only closed `HealthCode` values.

## Observability

This change IS the observability surface. It emits the ADR-4 metrics through the
port and contributes read-side health to the single PROD-005 report. No metric
label carries tenant, tag, entity, offset, or sequence values. Redaction:
`on_error`'s message is used for logs/traces only and is never promoted to a
label or to the public `HealthReport`.

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | `head_version` shape: empty stream ⇒ `None`; empty tenant ⇒ `None` (fail closed) | domain test |
| Unit | Polling lag = `max(0, head − processed)`; negative clamps to 0 | domain/runtime test |
| Unit | Reactive lag = `max(0, head_seq − last_sequence_id)`; aggregated over entities, never per-entity | ego-scheduler test |
| Unit | ADR-3 table: each of the 5 rows maps to the exact `(HealthStatus, HealthCode)` at the threshold boundary | runtime test |
| Unit | Bounded labels: emitted metric label set ⊆ `{projection_id, read_side_model, outcome}`; no tenant/tag/entity/sequence label | runtime test (assert label keys) |
| Structural | Read-side contributor has no liveness path — trait has only `check()`; liveness consults it zero times | runtime/service-sdk test |
| Integration | Required stalled projection ⇒ global Unhealthy; Optional stalled ⇒ Degraded (fold clamp); lagging-but-progressing ⇒ Degraded | `#[tokio::test]` |
| Integration | head-query error ⇒ `Unhealthy/Unavailable`, aggregation completes without hang | `#[tokio::test]` |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, or process-integration
boundary. The one data-exposure risk (tenant/key leakage via metric labels or
health text) is structurally closed by the bounded-label requirement (ADR-4) and
the closed `HealthCode` set (no free-text field), not by a process matrix.

## Migration / Rollout / Compatibility

Adding `head_version` to `ReadSideStore` is an SPI addition every implementor
must satisfy — the design fork (proposal Open Fork) records the alternative of a
separate `TargetCheckpointStore` to avoid the trait break; whichever is chosen,
the change documents the migration for existing implementors. Port additions
(`on_retry`, `last_progress_at`) ship with defaults so `NoopProgressReporter`
(`progress.rs:54`) and existing reporters compile unchanged. The read-side
contributor is additive: with no read-side registered, the aggregator behaves
exactly as PROD-005 shipped (empty fold ⇒ Healthy). Nothing is wired to a
transport here.

## Open Questions

None blocking. The head-version SPI shape (A/B/C) is the single conscious fork
recorded above and is resolved at ADR-1 (Option A) pending apply-time
confirmation that no `ReadSideStore` implementor requires the separate-SPI
escape hatch.
