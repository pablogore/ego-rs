# Tasks: PROD-003 — Production Observability / OTLP (Distributed Tracing v1)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1250-1500 (13 files: 6 new, 7 modified, incl. tests) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 domain → PR2 context+interceptor → PR3 runtime wiring → PR4 outbound propagation → PR5 OTLP adapter → PR6 boundary lint + spec reconciliation |
| Delivery strategy | auto-forecast (not a recognized ask-on-risk/auto-chain/single-pr/exception-ok label — treated conservatively) |
| Chain strategy | pending — orchestrator must confirm with user |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | `Tracer`/`TracerLifecycle`/`TraceContext`/`SpanAttributes`/`NoopTracer` in `ego-domain` | PR1 | `cargo test -p ego-domain tracer::` | N/A — pure domain unit, no runtime | Delete `tracer.rs`, revert `lib.rs` mod line |
| 2 | `ServiceContext` trace-context + `TracingInterceptor` | PR2 | `cargo test -p ego-service-sdk context:: interceptor::builtin::tracing::` | N/A — interceptor not yet runtime-wired | Revert `context/mod.rs`; delete `builtin/tracing.rs`; re-stub `builtin/mod.rs` |
| 3 | `RuntimeBuilder::with_tracer` wiring | PR3 | `cargo test -p ego-service-sdk runtime::builder::` | reference-app boot with `with_tracer(NoopTracer)` | Revert `builder.rs`/`runtime_builder.rs` tracer additions |
| 4 | Outbound HTTP `traceparent` propagation helper + reference-app call site | PR4 | `cargo test -p ego-transport propagation:: && cargo test -p reference-app --test outbound_trace_propagation` | reference-app outbound call site test asserting injected header | Delete `transport/src/propagation.rs`; revert `transport/lib.rs` mod line; revert reference-app call site |
| 5 | OTLP adapter (`infrastructure`) + lossless id conversion | PR5 | `cargo test -p ego-infrastructure tracing_otlp::` | `#[tokio::test]` vs stub OTLP collector | Delete `tracing_otlp.rs`; revert `Cargo.toml` deps |
| 6 | Boundary lint + AC-9 reconciliation | PR6 | `cargo test -p ego-service-sdk otlp_boundary_lint` | N/A — static source scan | Delete `otlp_boundary_lint.rs`; revert spec.md note |

## Phase 1: Domain — Tracer Port, TraceContext, SpanAttributes

- [x] TASK-001 RED: failing tests for `TraceContext::root()`/`from_inbound()`/`child()` parent linkage incl. A→B→C chain.
- [x] TASK-002 GREEN: implement `TraceContext`, `TraceId`, `SpanId` newtypes, `root()`/`from_inbound()`/`child()`. AC: TASK-001 green.
- [x] TASK-003 RED: failing tests for W3C `traceparent` parse/format round-trip incl. invalid-input error; `parse_traceparent` returns raw `(TraceId,SpanId)` only.
- [x] TASK-004 GREEN: implement `parse_traceparent`/`to_traceparent`/`TraceParseError`. AC: TASK-003 green.
- [x] TASK-005 RED: failing tests — `SpanAttributes` allow-list has no constructor/field for tenant id, credential, or arbitrary payload; `Tracer` trait (`start_span`/`end_span` only) + `NoopTracer` (zero observable effect, returns `ctx.span_id()`, implements `Tracer` ONLY — does not implement `TracerLifecycle`) + `SpanOutcome`.
- [x] TASK-006 GREEN: implement `SpanAttributes` (operation, tenant-present bool, duration), `SpanOutcome`, `Tracer` trait (`start_span`/`end_span`), a SEPARATE `TracerLifecycle` trait (`shutdown`, ADR-9), and `NoopTracer` (impl `Tracer` only). AC: TASK-005 green, no `opentelemetry` symbol in signatures.
- [x] TASK-007: wire `pub mod tracer;` + re-exports in `crates/domain/src/lib.rs`. AC: `ego_domain::{Tracer, TracerLifecycle, TraceContext, SpanAttributes, NoopTracer, SpanOutcome}` importable.

## Phase 2: ServiceContext Trace-Context Threading

- [x] TASK-008 RED: failing tests in `context/mod.rs` for `with_trace_context`/`trace_context()` round-trip, flat `trace_id` mirror, `correlation_id` unaffected. No `with_span` test (removed from v1).
- [x] TASK-009 GREEN: add `trace_context: Option<TraceContext>` field + accessors to `ServiceContext`; flat `trace_id` becomes read-through mirror. AC: TASK-008 green; existing context tests unaffected; no `with_span` method exists.

## Phase 3: TracingInterceptor

- [x] TASK-010 RED: failing tests in new `interceptor/builtin/tracing.rs` with a spy `Tracer`: `on_request` starts span (handle == `ctx.trace_context().span_id()`), `on_response` ends `Ok`, `on_error` ends `Error{status_message}` (redaction-safe), double-end race resolves to one close.
- [x] TASK-011 GREEN: implement `TracingInterceptor { tracer: Arc<dyn Tracer> }` impl `Interceptor`; no `TraceContext::child()` call. AC: TASK-010 green.
- [x] TASK-012: unstub `builtin/mod.rs` (`pub mod tracing; pub use tracing::TracingInterceptor;`). AC: crate compiles, symbol exported.

## Phase 4: Runtime Wiring

- [x] TASK-013 RED: failing test — `RuntimeBuilder::with_tracer(Arc<dyn Tracer>)` registers `TracingInterceptor`; omitted ⇒ `NoopTracer` default, behavior byte-identical.
- [x] TASK-014 GREEN: implement `with_tracer` in `builder.rs` + thread `tracer` through `runtime_builder.rs` (mirror `with_observability`); the runtime owns an optional `Arc<dyn TracerLifecycle>` and calls `shutdown()` on teardown (not a `Tracer` method). AC: TASK-013 green.

## Phase 5: HTTP Trace-Context — Ingress Origination + Outbound Propagation

### Ingress origination (SS-4 — added post-verify: `service-sdk/spec.md` "Trace-Context Originates At HTTP Ingress" had no covering task; without it the feature originates no trace in production)

- [x] TASK-014a RED: failing test at the HTTP ingress boundary — the request→`ServiceContext` path originates a `TraceContext` exactly once: `TraceContext::from_inbound(traceparent)` when an inbound `traceparent` header is present, else `TraceContext::root()`, attached via `ServiceContext::with_trace_context`. No ambient lookup.
- [x] TASK-014b GREEN: implement ingress origination at the HTTP handler boundary and attach the `TraceContext` to the `ServiceContext` threaded downstream. AC: TASK-014a green — a request carrying an inbound `traceparent` continues that trace (parent linkage); one without starts a fresh root.

### Outbound propagation

- [x] TASK-015 RED: failing test in `crates/transport` — helper builds `traceparent` header from an explicitly-passed `TraceContext` (`ctx.trace_context().to_traceparent()`), no ambient lookup, and starts no span.
- [x] TASK-016 GREEN: implement `crates/transport/src/propagation.rs` header-builder helper + `pub mod propagation;` in `lib.rs`. AC: TASK-015 green.
- [x] TASK-017 RED: failing test at a reference-app outbound call site (new `examples/reference-app/tests/outbound_trace_propagation.rs`) proving the outgoing request carries `traceparent` equal to `ctx.trace_context().to_traceparent()` and no new span is started.
- [x] TASK-018 GREEN: wire the reference-app outbound call site to apply the propagation helper. AC: TASK-017 green.

## Phase 6: Infrastructure OTLP Adapter

- [x] TASK-019: add `opentelemetry`, `opentelemetry-otlp` to `crates/infrastructure/Cargo.toml` only. AC: `cargo build -p ego-infrastructure` succeeds; no other crate gains the dep.
- [x] TASK-020 RED: failing unit tests — `SpanId`-keyed span table bookkeeping; idempotent `end_span` (double-end = one close); duplicate `start_span` for a live `SpanId` ignored+warns; at `max_in_flight_spans` a new `start_span` is **dropped + warns** (no eviction/overwrite/unbounded growth); `TracerLifecycle::shutdown()` flushes orphaned spans and clears table.
- [x] TASK-021 GREEN: implement `crates/infrastructure/src/tracing_otlp.rs`: `OtlpConfig { endpoint, protocol: Grpc|Http, max_in_flight_spans: usize }`, `OtlpTracer` impl `Tracer` + `TracerLifecycle`, bounded span table with drop-new-on-overflow, maps already-safe `SpanAttributes` with no redaction step. AC: TASK-020 green.
- [x] TASK-022 RED: failing test — lossless `SpanId`/`TraceId` ↔ otel span/trace id conversion round-trip (encode then decode yields identical bytes).
- [x] TASK-023 GREEN: implement the conversion functions used by `OtlpTracer`. AC: TASK-022 green.
- [x] TASK-024 RED: failing tests — protocol-selection construction (`#[tokio::test]`, gRPC/HTTP, no live collector) and export-reaches-exporter verification (see deviation note: in-memory `SpanExporter` used in place of a stub gRPC/HTTP collector server).
- [x] TASK-025 GREEN: wire config-driven protocol selection into `OtlpTracer` construction. AC: TASK-024 green.

## Phase 7: Boundary Lint & Spec Reconciliation

- [x] TASK-026 RED: source-scan test `crates/service-sdk/tests/otlp_boundary_lint.rs` (mirrors `tenant_scoped_lint.rs`): fails against a fixture using `Context::current()`/`Span::current()` outside `tracing_otlp.rs`.
- [x] TASK-027 GREEN: confirm zero real violations workspace-wide (no production code change). AC: `cargo test -p ego-service-sdk otlp_boundary_lint` green.
- [x] TASK-028: reconcile `openspec/specs/service-sdk/spec.md` "ServiceContext Remains a Pure DTO" (AC-9) — scoping note: AC-9 governs the CORE-015 change and does not preclude additive data-only fields (e.g. PROD-003's `trace_context`). AC: doc-only diff, no code change.

## Phase 8: Verification

- [x] TASK-029: run `cargo test --workspace` and `cargo build --workspace`. AC: exit 0, no regressions.
- [x] TASK-030: confirm default runtime (no `with_tracer`) is behaviorally unchanged (NoopTracer). AC: pre-existing test suite passes unmodified.

## Non-Goals (no tasks generated)

OTLP metrics, OTLP logs, actor/effect-runner and messaging tracing origination, guard-denial spans, client/server transport spans, configurable sampling (always-on per ADR-8), and manual nested `with_span` are out of scope for v1 — seams only (`TraceContext::child()`, `Tracer` reachable from `RuntimeInner`).
