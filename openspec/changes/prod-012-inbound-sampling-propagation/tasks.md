# Tasks: PROD-012 — Inbound Sampling Propagation

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~250-350 (domain `tracer.rs` decision type + fields + parse/serialize changes, infra mapping + sampler, incl. tests) |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Chain strategy | single-pr (domain + infra land together; the `parse_traceparent` signature change spans both, so splitting would leave a non-compiling intermediate) |
| Delivery strategy | auto-forecast (no explicit ask-on-risk/auto-chain/single-pr/exception-ok label — treated conservatively; under budget, single PR) |

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 1 | Domain: `SamplingDecision`/`TraceState`, `TraceContext` fields, `parse_traceparent`/`from_inbound`/`root`/`to_traceparent` honoring | PR1 | `cargo test -p ego-domain tracer::` | Revert `crates/domain/src/tracer.rs`; restore `(TraceId, SpanId)` return and `-01` serialization |
| 2 | Infra: `SamplingDecision → TraceFlags`/`TraceState` mapping + parent-based sampler; remove hardcoded `SAMPLED`/`NONE` | PR1 | `cargo test -p ego-infrastructure tracing_otlp::` | Restore `TraceFlags::SAMPLED`/`TraceState::NONE` in `build_span_data` |

## Phase 1: Domain — Sampling Decision Value Types

- [ ] TASK-001 RED: failing test in `crates/domain/src/tracer.rs` asserting `SamplingDecision { Sampled, NotSampled }` and an opaque `TraceState` newtype exist and that `TraceContext` exposes `sampling_decision()` and `trace_state()` accessors. Assertion: `let c = TraceContext::root(); assert_eq!(c.sampling_decision(), SamplingDecision::Sampled);` (ADR-1, ADR-2 fork A local-root default).
- [ ] TASK-002 GREEN: add `SamplingDecision { Sampled, NotSampled }`, opaque `TraceState(String)`, `trace_flags`/`trace_state` fields on `TraceContext`, and `sampling_decision()`/`trace_state()` accessors in `crates/domain/src/tracer.rs`; `root()`/`child()` apply the local-root default `Sampled`; no `opentelemetry` type introduced. AC: TASK-001 green.

## Phase 2: Domain — parse_traceparent Retains The Sampled Bit

- [ ] TASK-003 RED: failing test in `crates/domain/src/tracer.rs` — `parse_traceparent("00-<tid>-<sid>-01")` returns `Ok((T, R, SamplingDecision::Sampled))` and `parse_traceparent("00-<tid>-<sid>-00")` returns `Ok((T, R, SamplingDecision::NotSampled))`; a malformed header still returns `Err(TraceParseError::InvalidFormat)`. (MODIFIED "TraceContext Distinguishes Inbound Origination From Raw Parsing".)
- [ ] TASK-004 GREEN: change `parse_traceparent` to decode the flags byte into a `SamplingDecision` and return `(TraceId, SpanId, SamplingDecision)`; keep the existing strict validation (version `00`, lowercase hex, non-zero ids). AC: TASK-003 green.

## Phase 3: Domain — from_inbound Preserves The Decision

- [ ] TASK-005 RED: failing test — `TraceContext::from_inbound("00-<tid>-<sid>-00")` yields `sampling_decision() == NotSampled`, `parent_span_id() == Some(R)`, and a fresh `span_id != R`; a sampled inbound header yields `sampling_decision() == Sampled`. (ADDED "Parent-Based Sampling Policy Honors The Inbound Decision" — VALID sampled / VALID not-sampled cases.)
- [ ] TASK-006 GREEN: update `from_inbound` to preserve the parsed `SamplingDecision` (and `tracestate` when present) onto the constructed `TraceContext`. AC: TASK-005 green.
- [ ] TASK-007 RED: failing test for the fallback cases — a malformed inbound header (`from_inbound` returns `Err`) followed by `TraceContext::root()` yields `sampling_decision() == Sampled`; `root()` with no parent also yields `Sampled` and consults no ambient state. (ADDED requirement — INVALID(malformed) and ABSENT(no parent) cases.)
- [ ] TASK-008 GREEN: confirm/implement the local-root default so both the malformed-parent fallback path and `root()` resolve to `Sampled` deterministically. AC: TASK-007 green.

## Phase 4: Domain — to_traceparent Reflects The Decision

- [ ] TASK-009 RED: failing test — `to_traceparent()` ends in `-01` for a `Sampled` context and `-00` for a `NotSampled` context; a parse→serialize round-trip preserves the sampled bit. (ADDED "Outbound Propagation And Export Reflect The Actual Decision" + "Inbound Sampling Interoperates With W3C And OpenTelemetry".)
- [ ] TASK-010 GREEN: change `to_traceparent` to serialize the actual `trace_flags` (`-01`/`-00`) instead of the hardcoded `-01`; update the doc line that reads "Always sampled (`01`)". AC: TASK-009 green.
- [ ] TASK-011 RED: failing test — A→B→C chain: a not-sampled decision emitted by A survives through B and C (each `to_traceparent()` ends in `-00`). (Compatibility scenario "The decision round-trips across a service hop chain".)
- [ ] TASK-012 GREEN: ensure `from_inbound` + `to_traceparent` compose so the decision round-trips across hops. AC: TASK-011 green.

## Phase 5: Infrastructure — OTel Mapping And Parent-Based Sampler

- [ ] TASK-013 RED: failing test in `crates/infrastructure/src/tracing_otlp.rs` (or a new `sampling.rs`) — `to_otel_trace_flags(SamplingDecision::NotSampled) == TraceFlags::default()` (0x00) and `to_otel_trace_flags(SamplingDecision::Sampled) == TraceFlags::SAMPLED`; the domain `TraceState` maps to a matching `opentelemetry::trace::TraceState`. (ADR-1 — mapping confined to infra.)
- [ ] TASK-014 GREEN: implement `to_otel_trace_flags` and the domain-`TraceState`→`opentelemetry::TraceState` mapping in `crates/infrastructure`. AC: TASK-013 green.
- [ ] TASK-015 RED: failing test — `build_span_data` derives `TraceFlags`/`TraceState` from the `TraceContext` decision/tracestate rather than the hardcoded `TraceFlags::SAMPLED` (`tracing_otlp.rs:288`) / `TraceState::NONE` (`tracing_otlp.rs:290`); a `NotSampled` record yields non-sampled flags. (ADDED "Outbound Propagation And Export Reflect The Actual Decision".)
- [ ] TASK-016 GREEN: replace the hardcoded `TraceFlags::SAMPLED`/`TraceState::NONE` in `build_span_data` with the domain-driven mapping; add the parent-based sampler mapping (`Sampled ⇒ export`, `NotSampled ⇒ drop`) as `ParentBased{ root: AlwaysOn }`. AC: TASK-015 green.
- [ ] TASK-017 RED: failing `#[tokio::test]` — a `NotSampled` context is not exported (dropped by the parent-based policy) while a `Sampled` context is exported with faithful flags. (ADDED export/compat scenarios.)
- [ ] TASK-018 GREEN: wire the parent-based drop for `NotSampled` in the adapter's export path. AC: TASK-017 green.

## Phase 6: Caller Update — parse_traceparent Signature

- [ ] TASK-019: update every `parse_traceparent` caller for the new `(TraceId, SpanId, SamplingDecision)` return (primary caller `from_inbound`, `crates/domain/src/tracer.rs:166`; any transport/test caller). AC: `cargo build --workspace` succeeds; grep confirms no caller destructures the old two-tuple.

## Phase 7: Cross-Cutting Guarantees & Verification

- [ ] TASK-020: grep-verify the hexagonal boundary — no `opentelemetry`/`opentelemetry-otlp` symbol appears anywhere in `crates/domain` (including the new `SamplingDecision`/`TraceState` types). AC: grep clean.
- [ ] TASK-021: grep-verify no remaining hardcoded sampled flag — `crates/domain/src/tracer.rs` no longer serializes a constant `-01`, and `crates/infrastructure/src/tracing_otlp.rs` no longer references `TraceFlags::SAMPLED`/`TraceState::NONE` as unconditional constants. AC: grep confirms both are decision-driven.
- [ ] TASK-022: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions.
