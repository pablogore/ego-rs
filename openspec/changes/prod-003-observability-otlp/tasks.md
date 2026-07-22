# Tasks: PROD-003 — Production Observability / OTLP (Distributed Tracing v1)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~950-1150 (10 files: 4 new, 6 modified, incl. tests) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 domain → PR2 context+interceptor → PR3 runtime wiring → PR4 OTLP adapter → PR5 boundary lint + spec reconciliation |
| Delivery strategy | auto-forecast (not a recognized ask-on-risk/auto-chain/single-pr/exception-ok label — treated conservatively) |
| Chain strategy | pending — orchestrator must confirm with user |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | `Tracer`/`TraceContext`/`NoopTracer` in `ego-domain` | PR1 | `cargo test -p ego-domain tracer::` | N/A — pure domain unit, no runtime | Delete `tracer.rs`, revert `lib.rs` mod line |
| 2 | `ServiceContext` trace-context + `TracingInterceptor` | PR2 | `cargo test -p ego-service-sdk context:: interceptor::builtin::tracing::` | N/A — interceptor not yet runtime-wired | Revert `context/mod.rs`; delete `builtin/tracing.rs`; re-stub `builtin/mod.rs` |
| 3 | `RuntimeBuilder::with_tracer` wiring | PR3 | `cargo test -p ego-service-sdk runtime::builder::` | reference-app boot with `with_tracer(NoopTracer)` | Revert `builder.rs`/`runtime_builder.rs` tracer additions |
| 4 | OTLP adapter (`infrastructure`) | PR4 | `cargo test -p ego-infrastructure tracing_otlp::` | `#[tokio::test]` vs stub OTLP collector | Delete `tracing_otlp.rs`; revert `Cargo.toml` deps |
| 5 | Boundary lint + AC-9 reconciliation | PR5 | `cargo test -p ego-service-sdk otlp_boundary_lint` | N/A — static source scan | Delete `otlp_boundary_lint.rs`; revert spec.md note |

## Phase 1: Domain — Tracer Port & TraceContext

- [ ] TASK-001 RED: failing tests in `crates/domain/src/tracer.rs` for `TraceContext::root()`/`child()` parent linkage.
- [ ] TASK-002 GREEN: implement `TraceContext`, `TraceId`, `SpanId`, `root()`/`child()`. AC: TASK-001 green.
- [ ] TASK-003 RED: failing tests for W3C `traceparent` parse/format round-trip incl. invalid-input error.
- [ ] TASK-004 GREEN: implement `parse_traceparent`/`to_traceparent`/`TraceParseError`. AC: TASK-003 green.
- [ ] TASK-005 RED: failing tests for `Tracer` trait + `NoopTracer` (zero observable effect) + `SpanOutcome`.
- [ ] TASK-006 GREEN: implement `Tracer`, `SpanOutcome`, `NoopTracer`. AC: `cargo test -p ego-domain` passes, no `opentelemetry` symbol in signatures.
- [ ] TASK-007: wire `pub mod tracer;` + re-exports in `crates/domain/src/lib.rs`. AC: `ego_domain::{Tracer, TraceContext, NoopTracer, SpanOutcome}` importable.

## Phase 2: ServiceContext Trace-Context Threading

- [ ] TASK-008 RED: failing tests in `context/mod.rs` for `with_trace_context`/`trace_context()` round-trip, `trace_id()` mirror, `with_span(name)` child derivation.
- [ ] TASK-009 GREEN: add `trace_context: Option<TraceContext>` field + accessors/`with_span` to `ServiceContext`; `correlation_id` untouched. AC: TASK-008 green; existing context tests unaffected.

## Phase 3: TracingInterceptor

- [ ] TASK-010 RED: failing tests in new `interceptor/builtin/tracing.rs` with a spy `Tracer`: `on_request` starts span, `on_response` ends Ok, `on_error` records+ends Err, all reading `ctx.trace_context()`.
- [ ] TASK-011 GREEN: implement `TracingInterceptor { tracer: Arc<dyn Tracer> }` impl `Interceptor`. AC: TASK-010 green.
- [ ] TASK-012: unstub `builtin/mod.rs` (`pub mod tracing; pub use tracing::TracingInterceptor;`). AC: crate compiles, symbol exported.

## Phase 4: Runtime Wiring

- [ ] TASK-013 RED: failing test — `RuntimeBuilder::with_tracer(Arc<dyn Tracer>)` registers `TracingInterceptor`; omitted ⇒ `NoopTracer` default, behavior byte-identical.
- [ ] TASK-014 GREEN: implement `with_tracer` in `builder.rs` + thread `tracer` through `runtime_builder.rs` (mirror `with_observability`). AC: TASK-013 green.

## Phase 5: Infrastructure OTLP Adapter

- [ ] TASK-015: add `opentelemetry`, `opentelemetry-otlp` to `crates/infrastructure/Cargo.toml` only. AC: `cargo build -p ego-infrastructure` succeeds; no other crate gains the dep.
- [ ] TASK-016 RED: failing unit tests — `Map<SpanId, otel span>` bookkeeping; redaction rejects tenant-id/credential-shaped attributes ([REDACTED] convention).
- [ ] TASK-017 GREEN: implement `crates/infrastructure/src/tracing_otlp.rs`: `OtlpConfig { endpoint, protocol: Grpc|Http }`, `OtlpTracer` impl `Tracer` with allow-list attribute redaction. AC: TASK-016 green.
- [ ] TASK-018 RED: failing `#[tokio::test]` integration tests — gRPC export and HTTP export to a stub collector; adapter absent ⇒ no-op.
- [ ] TASK-019 GREEN: wire config-driven protocol selection into `OtlpTracer` construction. AC: TASK-018 green.

## Phase 6: Boundary Lint & Spec Reconciliation

- [ ] TASK-020 RED: source-scan test `crates/service-sdk/tests/otlp_boundary_lint.rs` (mirrors `tenant_scoped_lint.rs` pattern): fails against a fixture using `Context::current()`/`Span::current()` outside `tracing_otlp.rs`.
- [ ] TASK-021 GREEN: confirm zero real violations workspace-wide once TASK-017 lands (no production code change). AC: `cargo test -p ego-service-sdk otlp_boundary_lint` green.
- [ ] TASK-022: reconcile `openspec/specs/service-sdk/spec.md` "ServiceContext Remains a Pure DTO" (AC-9) — add a scoping note: AC-9 governs the CORE-015 change (authorization provider) and does not preclude additive data-only fields (e.g. PROD-003's `trace_context`, data not behavior). AC: doc-only diff, no code change.

## Phase 7: Verification

- [ ] TASK-023: run `cargo test --workspace` and `cargo build --workspace`. AC: exit 0, no regressions.
- [ ] TASK-024: confirm default runtime (no `with_tracer`) is behaviorally unchanged (NoopTracer). AC: pre-existing test suite passes unmodified.

## Non-Goals (no tasks generated)

OTLP metrics, OTLP logs, actor/effect-runner tracing origination (`persistent-entity`, `ego-scheduler`), and CORE-012A guard-denial spans are explicitly out of scope for v1 — seams only, per design "Known v1 limitations & seams".
