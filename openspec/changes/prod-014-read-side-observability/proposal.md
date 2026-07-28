# Proposal: PROD-014 — Read-Side Production Observability

## Intent

`ROADMAP.md:623` lists "Projection lag" as an unchecked P0 item under §7.3
Production Observability, but the read-side today is **operationally blind**:
there is no way to compute lag, no way to observe read-side health, and no
production metrics. Concretely — the only position a projection records is a
PROCESSED offset (`Offset::Sequence(i64)`, `crates/domain/src/read_side/offset.rs:12-16`)
written through the `OffsetStore` SPI; **there is no head/latest/target query
anywhere** on the read-side store SPI, so lag (target − processed) is not
computable. The read-side is also invisible to the PROD-005 health model: the
only production `HealthContributor` is `ProviderHealthContributor`
(`crates/runtime/src/providers/health.rs:43`), so a stalled or badly-lagging
projection never affects `/ready` or `/startup`. Finally, the `ProgressReporter`
observability port exists (`crates/domain/src/read_side/progress.rs:25-46`) but
its only implementation is `NoopProgressReporter` (`:54`) — no throughput, retry,
error, or time-since-last-progress signal is emitted, and the reactive scheduler's
signals are tracing-only (`crates/ego-scheduler/src/metric.rs`).

PROD-014 defines production observability for the read-side: a **target-checkpoint
(head-version) query** added to the read-side store SPI, a **per-model lag
definition** (the polling projection and the reactive scheduler have different
position semantics), **operational metrics through a port** with bounded
cardinality, and a **read-side `HealthContributor`** wired into the single
PROD-005 aggregator. Hexagonal boundaries are preserved: the domain stays
OpenTelemetry-free; all metric emission flows through ports whose adapters live
in runtime/infrastructure.

## Scope

### In Scope

- A target-checkpoint (head-version per `(tenant, tag)`) query on the read-side
  store SPI — the missing half of a lag computation.
- Lag definition for **each** read-side model: the polling projection
  (`Offset::Sequence` vs head version) and the reactive scheduler
  (consumed sequence vs observed head sequence, gap-driven), plus
  time-since-last-progress as the wall-clock complement for both.
- Operational metrics: throughput, retries, errors, time-since-last-progress —
  emitted through the observability port, with bounded/low-cardinality labels
  only.
- A read-side `HealthContributor` that maps read-side lag/stall state to the
  PROD-005 `(HealthStatus, DependencyRequirement)` model and registers through
  the existing single aggregation authority.
- Readiness-vs-liveness discipline for the read-side (read-side health is a
  readiness/startup concern; it never touches liveness).

### Out of Scope (Non-Goals / Follow-ups)

- Concrete OpenTelemetry / OTLP exporters, dashboards, alerting rules, or SLO
  definitions (those consume the ports/metrics defined here).
- `/live` `/ready` `/startup` transport endpoints or Kubernetes probe wiring
  (PROD-005 out-of-scope; unchanged here).
- Broker lag, outbox metrics, saga metrics, external-effect metrics — separate
  ROADMAP §7.3 items.
- Changing polling/dedup/offset/ordering semantics of either scheduler engine.
- Backfilling per-tenant or per-tag dashboards (forbidden as unbounded label
  cardinality; only aggregated dimensions are exposed).

## Frozen Decisions (decided constraints, not open questions)

1. **Lag is target − processed, and the target must be queryable.** Processed is
   read from `OffsetStore::read_offset`; target is the head version obtained from
   a NEW head-version query on the read-side store SPI. No lag requirement may
   assume a head query that does not exist.
2. **Bounded labels only.** Metric labels MUST come from a closed, enumerable set
   (`projection_id`, `read_side_model`, and result/outcome classes). `tenant`,
   `tag`, entity identifiers, and raw sequence/offset values MUST NEVER be labels
   — CORE-018 makes tag/tenant unbounded (one tag stream per tenant). Unbounded
   dimensions are aggregated (e.g. max lag over tenants), never labeled.
3. **Read-side health is readiness, never liveness.** A lagging or stalled
   projection MUST NOT be able to fail liveness (that would restart the pod over
   a dependency blip). The read-side participates only in readiness/startup
   aggregation — structurally guaranteed because the PROD-005 `HealthContributor`
   trait has no liveness method.
4. **Lagging-but-progressing is Degraded; stalled-and-behind is Unhealthy.** A
   Required projection that is behind but still advancing SHOULD surface as
   `Degraded`; a Required projection that is behind AND has made no progress past
   its stall deadline surfaces as `Unhealthy`. Thresholds MUST be observable
   (configured event-count and duration), not vague.
5. **Hexagonal boundary preserved.** The domain (`crates/domain`) holds only
   ports/value types and stays OpenTelemetry-free. Metric adapters live in
   runtime/infrastructure. The read-side `HealthContributor` maps to the existing
   closed `HealthCode` set (`crates/domain/src/health/mod.rs`) without adding new
   codes.
6. **Reuse the PROD-005 single-model registration authority.** The read-side
   contributor registers through the one runtime-owned aggregator exactly as the
   provider contributor does — no parallel read-side readiness model.

## Open Fork for DESIGN (do not resolve here)

The **shape of the head-version query** on the read-side store SPI:
**(A)** a dedicated `head_version(tenant, tag) -> Option<i64>` method on the
existing `ReadSideStore` trait, **(B)** a separate `TargetCheckpointStore` SPI so
lag observability does not widen the hot-path fetch trait, or **(C)** overloading
`fetch` semantics to also report a head. The design MUST choose consciously,
weighing SPI-surface growth against forcing every `ReadSideStore` implementor to
provide a head query.

## Capabilities

### New Capabilities

None. This change is entirely additive to two existing capabilities.

### Modified Capabilities

- `read-side`: adds observability requirements to the capability whose canonical
  spec (`openspec/specs/read-side/spec.md`) is CORE-026 lifecycle only — a
  target-checkpoint query, per-model lag definition, bounded-cardinality
  operational metrics, and a read-side health-state mapping.
- `runtime-health-model`: adds the read-side as a `HealthContributor` participant
  in the single PROD-005 model (readiness/startup only; lagging→Degraded,
  stalled-Required→Unhealthy, initial-replay→InitializationPending).

## Approach

Add a head-version (target checkpoint) query to the read-side store SPI so lag
becomes `head − processed`. Define lag independently for the polling projection
(sequence offsets) and the reactive scheduler (bus sequences / gaps), plus a
shared time-since-last-progress wall-clock. Extend the observability port so
throughput, retries, errors, and progress timestamps are emitted through a
domain-neutral seam whose adapter (runtime) maps to OpenTelemetry; enforce
bounded labels at the port contract. Add one read-side `HealthContributor` that
reads processed and target, applies the observable health-state thresholds, and
reports `(HealthStatus, DependencyRequirement)` into the single PROD-005
aggregator via the existing registration authority.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/domain/src/read_side/store.rs` | Modified (SPI) | Target-checkpoint / head-version query added (shape per design fork) |
| `crates/domain/src/read_side/progress.rs` | Modified (port) | Retry + time-since-last-progress surface; bounded-label contract |
| `crates/runtime/src/read_side/` | New/Modified | Read-side `HealthContributor`; lag/metric adapter over the port |
| `crates/ego-scheduler/src/` | Modified | Reactive-model lag/metrics routed through the port, not tracing-only |
| `crates/service-sdk/src/runtime/builder.rs` | Modified | Register the read-side contributor via the existing single authority |
| `crates/testkit/src/` | New | Deterministic read-side health/lag fixtures |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Head-version query becomes a mandatory SPI break for all `ReadSideStore` impls | Med | Design fork weighs a separate SPI (B) vs widening `fetch`'s trait; document migration/compat |
| Unbounded labels (tenant/tag/entity) leak into metrics | Med | Normative bounded-label requirement enumerating the closed dimension set; forbidding tenant/tag/entity/sequence labels |
| Read-side wrongly folded into liveness, causing restart storms | Med | Frozen: read-side contributes only to readiness/startup; PROD-005 trait has no liveness method (structural) |
| Vague health thresholds make "lagging" non-testable | Med | Observable thresholds (configured event-count + stall duration); Given/When/Then encodes each boundary |
| Two models diverge in lag semantics, confusing operators | Low | Lag defined explicitly per model with a shared time-since-last-progress complement |
| OTel creeps into `ego-domain` | Low | Hexagonal clause; domain holds only ports; adapters in runtime |

## Rollback Plan

Additive and behavior-neutral until an exporter/adapter consumes it. Rollback =
remove the head-version query, the port extensions, the read-side
`HealthContributor`, and its builder registration; the read-side returns to its
current PROCESSED-only, contributor-invisible state. `OffsetStore`,
`ReadSideStore::fetch`, and both scheduler engines' existing semantics are
untouched, so revert has no correctness impact. No schema/migration beyond
whatever a concrete head-version query implementation adds in a later change.

## Dependencies

- Builds on the PROD-005 Runtime Health Model (already merged to develop via
  PR #243): `HealthContributor`, `HealthStatus`, `DependencyRequirement`, `fold`,
  and the single runtime-owned aggregator / registration authority.
- Reuses the existing `OffsetStore` (`offset.rs:55-74`) and `ReadSideStore`
  (`store.rs:26`) SPIs and the `ProgressReporter` port (`progress.rs:25-46`).
- Independent of other in-flight changes otherwise.

## Success Criteria

- [ ] The read-side store SPI exposes a target-checkpoint (head-version) query;
      lag = head − processed is computable for the polling projection.
- [ ] Lag is defined for BOTH read-side models (polling sequence-offset lag and
      reactive consumed-vs-head-sequence/gap lag), with a shared
      time-since-last-progress complement.
- [ ] Operational metrics (throughput, retries, errors, time-since-last-progress)
      are emitted through a port with a bounded, enumerated label set; tenant,
      tag, entity, and sequence values are provably never labels.
- [ ] A read-side `HealthContributor` participates in the single PROD-005 model
      for readiness/startup only, never liveness; lagging-but-progressing yields
      Degraded, a stalled Required projection yields Unhealthy.
- [ ] Domain stays OpenTelemetry-free; adapters in runtime; `cargo test
      --workspace` green.
