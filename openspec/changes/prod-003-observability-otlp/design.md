# Design: PROD-003 — Production Observability / OTLP (Distributed Tracing v1)

## Technical Approach

New domain `Tracer` span-lifecycle port (transport-agnostic), a domain `TraceContext`
value carried explicitly on `ServiceContext`, an unstubbed `TracingInterceptor` that
starts/ends spans from that context, and an OTLP adapter in `infrastructure` (the sole
`opentelemetry` consumer). Trace identity travels as data; the adapter bridges to
`opentelemetry` via an id-keyed span table — never `Context::current()`.

## Architecture Decisions

### ADR-1: Explicit trace-context vs ambient
**Choice**: `TraceContext` value on `ServiceContext`, passed by value.
**Alternatives**: `tracing`/`opentelemetry` task-local `Context::current()`.
**Rationale**: The enforced invariant (`config.yaml:7`, `ARCHITECTURE.md:140`) forbids
ambient/task-local state. Every hop already declares `ServiceContext` in its signature;
trace identity rides the same explicit channel.

### ADR-2: New `Tracer` port vs extending `Observability`
**Choice**: Separate `ego_domain::tracer::Tracer` trait; `NoopTracer` default.
**Alternatives**: Add `start_span`/`end_span` to `Observability`.
**Rationale**: `Observability` is a fire-and-forget *event* recorder on the CORE-012A
security-denial path and MUST stay untouched (proposal non-goal). Spans have a
lifecycle (start→end/error) and a different call surface. One trait per responsibility.

### ADR-3: start/end pair vs RAII span-guard
**Choice**: `start_span(&TraceContext, name, attrs)` + `end_span(&TraceContext, Outcome)`.
**Alternatives**: guard returned by `start_span` whose `Drop` closes the span.
**Rationale**: The proxy seam (`service-sdk-macros/src/lib.rs:475-486`) invokes
`on_request(&ctx)` and `on_response(&ctx)` as **two separate calls with no shared local
scope** — a guard has nowhere to live between them except ambient state (forbidden). The
pair reads the same explicit ids from `&ctx` in both phases; the adapter looks the span
up by those ids. Mirrors the non-blocking, stateless-trait `Observability` precedent.

### ADR-4: TraceContext location & migration
`TraceContext { trace_id, span_id, parent_span_id: Option<_> }` lives in `ego-domain`
(W3C `traceparent` parse/format, no `opentelemetry` types). `ServiceContext` gains
`trace_context: Option<TraceContext>` + `with_trace_context`, `trace_context()`,
`with_span(name)` (derives a child: new `span_id`, `parent = current span_id`).
`correlation_id` **stays** (distinct business-causal concept used by `SemanticEvent`);
flat `trace_id` is retained as a read-through mirror of `trace_context().trace_id` for
source compat, superseded by `trace_context()`.

### ADR-5: OTLP boundary (only `Span::current()` site)
Adapter holds `Map<SpanId, opentelemetry::Span>`. `start_span` mints an otel span with an
explicit parent from `TraceContext` and inserts it; `end_span` removes and closes it. The
key is an **argument-supplied id**, not thread-local — this is adapter bookkeeping, not
context propagation. `Context::current()`/`Span::current()` are forbidden for carrying
framework context; enforced by a `tenant_scoped_lint`-style test asserting neither symbol
appears outside the adapter module.

### ADR-6: Redaction rule
Single rule (reconciles to the existing `[REDACTED]` convention, `credential.rs:50`):
span attributes MUST be an allow-list of non-sensitive scalars (operation name, tenant
*hint presence*, outcome, duration). Tenant identifiers, credentials, principal subject,
and payloads MUST NOT become attributes. Any type already redacting in `Debug` renders
`[REDACTED]` if attached.

## Data Flow

    ingress ──build root TraceContext──▶ ServiceContext ──▶ proxy
       proxy: chain.on_request(&ctx) ─▶ handler ─▶ chain.on_response(&ctx)
       TracingInterceptor reads ctx.trace_context() ─▶ Tracer (domain trait)
       Arc<dyn Tracer> == OTLP adapter (infra) ─▶ OTLP gRPC|HTTP ─▶ collector

### Sequence: request → span → export

    Ingress      Proxy         TracingIntc      Tracer(OTLP adapter)     Collector
      │  build root TC on ctx                                              
      ├──ctx──▶ │                                                          
      │         ├─on_request(&ctx)─▶│                                      
      │         │                   ├─start_span(ctx.trace_context())─▶│   
      │         │                   │        insert Map[span_id]=otel   │   
      │         ├────── handler(ctx) ─────────────────────────────────┐│   
      │         │                   │        (async work)             ││   
      │         ├─on_response(&ctx)─▶│                                 ││   
      │         │                   ├─end_span(ctx.trace_context(),Ok)▶│   
      │         │                   │   remove+close → batch export ──▶│──▶│
      │         │  on_error(&ctx,e): end_span(..,Err) records status   │   

Trace-context is **read** from `&ctx` in every phase; it is **written** only at ingress
(root) and by `with_span` (child derivation). It crosses into infra only as the
`&TraceContext` argument to `Arc<dyn Tracer>`.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/tracer.rs` | Create | `Tracer` trait, `TraceContext`, `TraceId`/`SpanId`, `SpanOutcome`, `NoopTracer` |
| `crates/domain/src/lib.rs` | Modify | `pub mod tracer;` + re-exports |
| `crates/service-sdk/src/context/mod.rs` | Modify | `trace_context` field, `with_trace_context`, `trace_context()`, `with_span()` |
| `crates/service-sdk/src/interceptor/builtin/mod.rs` | Modify | Unstub; export `TracingInterceptor` |
| `crates/service-sdk/src/interceptor/builtin/tracing.rs` | Create | `TracingInterceptor { tracer: Arc<dyn Tracer> }` on_request/response/error |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `with_tracer(Arc<dyn Tracer>)` → registers `TracingInterceptor` in chain |
| `crates/infrastructure/src/tracing_otlp.rs` | Create | `OtlpTracer` adapter + `OtlpConfig { endpoint, protocol: Grpc\|Http }` |
| `crates/infrastructure/Cargo.toml` | Modify | Add `opentelemetry`, `opentelemetry-otlp` (infra only) |

## Interfaces / Contracts

```rust
// ego-domain: transport-agnostic
pub struct TraceContext { /* trace_id, span_id, parent_span_id */ }
impl TraceContext {
    pub fn root() -> Self;                     // mint new trace + root span
    pub fn child(&self) -> Self;               // parent = self.span_id
    pub fn parse_traceparent(s: &str) -> Result<Self, TraceParseError>;
    pub fn to_traceparent(&self) -> String;    // W3C format
}
pub enum SpanOutcome { Ok, Error }
pub trait Tracer: Send + Sync {                // non-blocking, like Observability
    fn start_span(&self, ctx: &TraceContext, name: &str, attrs: &[(&str, &str)]);
    fn end_span(&self, ctx: &TraceContext, outcome: SpanOutcome);
}
pub struct NoopTracer;                          // default; all methods no-op
```

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | `TraceContext` W3C parse/format round-trip; `child()` parent linkage | domain tests |
| Unit | `with_span` derives child, keeps `correlation_id`; flat `trace_id` mirrors | context tests |
| Unit | `NoopTracer` discards; `TracingInterceptor` calls start/end/error via spy `Tracer` | interceptor tests |
| Unit | **Boundary lint**: no `Context::current()`/`Span::current()` outside adapter | source-scan test |
| Unit | Redaction: sensitive attrs rejected/`[REDACTED]` | adapter tests |
| Integration | OTLP gRPC & HTTP export to a stub collector; disable ⇒ no-op | infra `#[tokio::test]` |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, or executable-file
classification. OTLP is outbound network egress only; its data-exposure risk is covered
by ADR-6 redaction, not the process-integration matrix.

## Migration / Rollout

`NoopTracer` is the default; behavior is byte-for-byte unchanged until `with_tracer` is
called. OTLP adapter is opt-in via wiring/config. No schema/migration. Rollback = drop
the `with_tracer` wiring. `correlation_id`/`trace_id` fields preserved (no breaking API).

## Known v1 limitations & seams

- **Guard-denial spans**: CORE-012A macro-guard denials short-circuit before the chain →
  no span (accepted). Seam: `Tracer` is reachable from `RuntimeInner`; a future change can
  emit a denial span there without touching this port.
- **Actor/effect-runner origination**: deferred. Seam: `TraceContext::child()` +
  `Arc<dyn Tracer>` are usable from `persistent-entity`/`ego-scheduler` when those add a
  `TraceContext` to their own contexts — no port change required.

## Open Questions

- [ ] Sampling policy (always-on vs ratio) — default always-on for v1; revisit if volume warrants.
