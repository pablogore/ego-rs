# Exploration — PROD-003: Production Observability / OTLP

**Change:** `prod-003-observability-otlp`
**Phase:** explore (investigation only — no implementation, no proposal)
**Status:** ready for proposal

## Current-state map

### 1. The `Observability` port (domain contract)
`crates/domain/src/observability.rs:154-202`
```rust
pub trait Observability: Send + Sync {
    fn trace(&self, event: SemanticEvent);
    fn metric(&self, name: &str, value: f64);
    fn log(&self, level: Level, message: &str);
}
```
- `SemanticEvent` (`observability.rs:47-61`): flat, string-keyed — `event_name`, `correlation_id`, `actor_id`, `lifecycle_state`, `timestamp`, `metadata: HashMap<String,String>`. **No span tree** (no span-id/trace-id pair, no parent-id, no start/end duration, no span kind).
- Trait doc mandates implementations be **non-blocking, O(1), stateless** — good seam for an enqueue-and-return OTLP adapter, but the port has **no span lifetime concept**.
- Only implementor today: `NoopObservability` (`crates/infrastructure/src/observability.rs:7-17`) — discards everything. **No production implementor exists.** `RecordingObservability`/`PanickingObservability` (`service-sdk/src/test_support.rs`) are test doubles.
- Wiring: `RuntimeBuilder::with_observability(Arc<dyn Observability>)` (`service-sdk/src/runtime/builder.rs:283`) → `Option<Arc<dyn Observability>>` on `RuntimeInner`. **Only production call site**: `RuntimeInner::record_security_denial` (CORE-012A) — fires one `SemanticEvent` on macro-guard denials. Nothing else calls the port.
- `ServiceContext` has **no `observability` field** — only `logger: Option<Arc<KITLogger>>` (`context/mod.rs:91`).

### 2. Metrics — do not really exist
`ego-scheduler/src/metric.rs`, `runtime/src/effects/observability.rs` are **`tracing`-macro wrappers**, not a metrics port. They emit structured log events with numeric fields (`total_events_consumed`, effect lifecycle: `accepted`/`dispatch_started`/`attempt`/`success`/`retry_scheduled`/`terminal_failed`/`deduplicated`/`queue_depth`/`oldest_pending_age`/…). **No `metrics` crate, no exporter, no aggregation.** Docs: "Metrics are purely diagnostic — no behavioral role."

### 3. Logging — two disconnected mechanisms
- **`KITLogger`** (external git dep) — host-facing, explicitly propagated, threaded through `ServiceContext::with_logger`. Structured JSON, but **no trace/span correlation of its own**.
- **`tracing` macros** (`tracing = "0.1"` in 6+ crates) — used purely as a logging facade. **No `tracing-subscriber` installed in production** (only declared as a dep + used in archived docs). So these call sites currently emit into the void.

### 4. Tracing / spans — effectively absent
No `tracing::Span`, `#[instrument]`, `span.enter()`, `opentelemetry`/`otlp`/`otel` anywhere (0 hits across 18 manifests). Only "trace" vocabulary: `Observability::trace(SemanticEvent)` (fire-and-forget event) and `ServiceContext.trace_id`/`.correlation_id` (`context/mod.rs:67,69`) — **flat strings, no W3C traceparent parsing, nothing in the runtime reads/propagates them automatically**.

### 5. Interceptors — stub
`service-sdk/src/interceptor/builtin/mod.rs` is a 5-line stub with `TracingInterceptor` commented out. `InterceptorChain::on_request/on_response/on_error` are the natural span start/end/error hooks — BUT CORE-012A found the **macro-guard denial paths short-circuit before the interceptor chain runs**, so an interceptor-only span misses guard denials.

### 6. Context propagation — the central constraint
`ServiceContext` is an **explicit-propagation value type** — "no ambient or thread-local fallback." This is load-bearing and previously fought: dedicated past change `archive/2026-06-22-remove-ambient-service-context/` removed ambient/task-local access; `ARCHITECTURE.md:140` and `openspec/config.yaml:7` encode "no ambient/thread-local/task-local state" as an **enforced invariant**.
- **Direct tension with OTLP**: `tracing::Span::current()` / `opentelemetry::Context::current()` are thread-local/task-local ambient state. Any OTLP design leaning on that idiom reintroduces exactly what this codebase tore out. Precedent: thread trace context **explicitly as data on `ServiceContext`**.

### 7. Dependencies
No `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry` anywhere. `tracing`/`tracing-subscriber` present but unused for subscriber/export. Adding OTLP is **infra-layer only** — must not leak into `ego-domain`.

### 8. Prior art in specs
No dedicated observability domain spec. Only `openspec/specs/service-sdk/spec.md:1348-1429` (CORE-012A) — narrow to security-denial recording, redaction, `with_observability(...)`. Explicitly **not** OTLP. `ROADMAP.md:609-629` carries this as PROD-003: OTLP traces/metrics/logs, correlation propagation, tenant-safe telemetry, actor lifecycle / entity activation / projection-lag / effect / outbox / saga / broker-lag metrics, cardinality policy, sensitive-data redaction.

## Hard constraints
- **Hexagonal**: port stays in `ego-domain`; OTLP wiring lives in `infrastructure`. No `opentelemetry*` in domain.
- **Explicit propagation (invariant)**: trace context must be threaded as data on `ServiceContext`, NOT via ambient span-stack. Any ambient usage = architectural exception needing explicit sign-off.
- **Non-blocking port**: `Observability` impls must stay O(1)/enqueue-and-return.
- **Redaction across a network boundary**: OTLP data leaves the process; redaction stakes are higher than a local `Debug` string. Two conventions already coexist (Display-redacted vs Debug-redacted) — must reconcile.

## Candidate approaches (not decided — for proposal)
1. **OTLP-backed `Observability`/span adapter in infra + explicit trace-context on `ServiceContext`.** Extend `ServiceContext` with an explicit span/trace-context value; unstub `TracingInterceptor` to start/end spans reading that value; infra adapter exports via `opentelemetry-otlp`. Pros: preserves invariant + hexagonal boundary, builds on existing fields. Cons: current port shape can't express span lifetime (needs new methods/wrapper); guard-bypass hole. Effort: Med-High.
2. **Bridge via `tracing` + `tracing-opentelemetry`, keep ambient span machinery.** Install global subscriber exporting to OTLP; instrument with `#[instrument]`/`Span::current()`. Pros: minimal new abstraction, industry-standard, `tracing` already a dep. Cons: **reintroduces ambient/task-local state** — conflicts with the enforced invariant; needs an explicit exception or a hybrid. Effort: Low-Med + architectural-consistency risk.
3. **Separate `Telemetry`/span port distinct from `Observability`.** Keep `Observability` as-is; add a narrow span-lifecycle port (start/end or RAII handle) carried explicitly on `ServiceContext`, OTLP-implemented. Pros: doesn't force spans into `SemanticEvent`'s flat shape; leaves CORE-012A untouched. Cons: two overlapping ports risk confusion. Effort: Med.

**Recommendation:** favor 1 or 3 (or a merge) over 2. The "no ambient state" invariant is enforced, not a preference — reaching for `tracing::Span::current()` as the primary mechanism is an exception requiring sign-off.

## Open questions (for proposal)
- Does `metric(name, f64)` suffice for OTLP counters/histograms/gauges with attributes, or must the port grow?
- Where does trace-context originate at true ingress (HTTP handler, message consumer)? Do actor/effect-runner paths (`persistent-entity`, `ego-scheduler`) — which aren't request/response — need their own origination?
- Do macro-guard denial paths (interceptor-bypass) get direct instrumentation, or is "guard denied, no span" acceptable for v1?
- Is `tracing` kept as an internal logging facade forever, or does PROD-003 adopt `tracing-opentelemetry` as the export bridge (resolving the ambient tension by confining ambient usage strictly inside one adapter/interceptor detail)?
- Redaction/cardinality policy: piggyback CORE-012A `Display`-redaction, or the `[REDACTED]`-Debug convention? Reconcile the two.
- OTLP transport target (gRPC vs HTTP exporter; collector assumed present) — needed before design.

## Risks
- Ambient-vs-explicit conflict is not cosmetic — contradicting the enforced invariant must be flagged in the proposal, not discovered in review.
- `tracing` used unexported in 6+ crates — OTLP bridge must decide adopt-existing-call-sites (scope creep) vs new mechanism.
- CORE-012A interceptor-chain bypass → interceptor-only spans have a documented coverage hole at guard denials.
- No metrics aggregation exists — the metrics scope needs counters/histograms designed from scratch, not an export pipe on existing log lines.
- Redaction split across two conventions — higher stakes over a network boundary.
- No dedicated observability domain spec — proposal must create one or delta `service-sdk/spec.md` without conflicting CORE-012A.
