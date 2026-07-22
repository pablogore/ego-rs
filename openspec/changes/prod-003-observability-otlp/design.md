# Design: PROD-003 — Production Observability / OTLP (Distributed Tracing v1)

## Technical Approach

A domain `Tracer` span-lifecycle port (transport-agnostic, no `opentelemetry` types),
a domain `TraceContext` value carried explicitly on `ServiceContext`, an unstubbed
`TracingInterceptor` that owns exactly one span per request boundary, and an OTLP adapter
in `infrastructure` (the sole `opentelemetry` consumer) whose span table is keyed by
`SpanId`. Trace identity travels as data — never `Context::current()`. Redaction is
enforced in the domain at the port boundary via a typed `SpanAttributes` allow-list, so
nothing sensitive can even be expressed to the adapter.

## Span handle == the id already in the context (core equivalence)

`start_span` returns a `SpanId` that **is** the span handle **and is exactly**
`TraceContext.span_id` (`ctx.trace_context().span_id()`). One concept, not two. This
simultaneously satisfies "start returns a handle" and "no ambient / stateless
interceptor": the handle is the id already present on `ServiceContext`, so the proxy's
two separate calls (`on_request` starts, `on_response`/`on_error` ends) share **nothing**
in local or ambient scope — `end_span` re-derives the same `SpanId` from `&ctx`. The
adapter's span table is keyed by that `SpanId`.

## Architecture Decisions

### ADR-1: Explicit trace-context vs ambient
**Choice**: `TraceContext` value on `ServiceContext`, passed by value.
**Rejected**: `tracing`/`opentelemetry` task-local `Context::current()`.
**Rationale**: The enforced invariant (`config.yaml:7`, `ARCHITECTURE.md:140`) forbids
ambient/task-local state. Every hop already declares `ServiceContext`; trace identity
rides the same explicit channel.

### ADR-2: New `Tracer` port vs extending `Observability`
**Choice**: Separate `ego_domain::tracer::Tracer` trait; `NoopTracer` default.
**Rejected**: Add span methods to `Observability`.
**Rationale**: `Observability` is a fire-and-forget event recorder on the CORE-012A
denial path and stays untouched (non-goal). Spans have a start→end lifecycle and a
different call surface. One trait per responsibility.

### ADR-3: start/end handle pair vs RAII guard
**Choice**: `start_span(&ctx, name, attrs) -> SpanId` + `end_span(SpanId, SpanOutcome)`.
**Rejected**: a guard whose `Drop` closes the span.
**Rationale**: The proxy seam (`service-sdk-macros/src/lib.rs:478-486`) calls
`on_request(&ctx)` then `on_response(&ctx)`/`on_error(&ctx,e)` as **separate calls with
no shared local scope** — a guard has nowhere to live but ambient state (forbidden). The
returned `SpanId` handle equals `ctx.trace_context().span_id()`, so `end_span` needs no
stored guard; it looks the span up by that argument-supplied id.

### ADR-4: TraceContext, `ServiceContext` migration, no `with_span`
`TraceContext { trace_id, span_id, parent_span_id: Option<_> }` lives in `ego-domain`.
`ServiceContext` gains `trace_context: Option<TraceContext>`, `with_trace_context(..)`,
and `trace_context()`. **`with_span(name)` is removed from v1** — v1 is exactly one
interceptor-owned span per request boundary; the confusing sugar is dropped.
`TraceContext::child()` is retained as the seam for future manual/nested spans.
`correlation_id` stays (distinct business-causal concept used by `SemanticEvent`); flat
`trace_id` becomes a read-through mirror of `trace_context().trace_id` for source compat.

### ADR-5: OTLP boundary + span-table operational semantics
Adapter holds a thread-safe `Map<SpanId, opentelemetry::Span>` (`Mutex`/`DashMap`). The
key is an **argument-supplied `SpanId`**, not thread-local — adapter bookkeeping, not
context propagation. `Context::current()`/`Span::current()` are forbidden for carrying
framework context, enforced by a `tenant_scoped_lint`-style source-scan test asserting
neither symbol appears outside the adapter module. Operational rules (also MUST specs):
- `end_span` is **idempotent per `SpanId`**: first end wins; the `on_response`+`on_error`
  race resolves to a single close — the second call is a no-op.
- A duplicate `start_span` for a still-live `SpanId` is **ignored with a warning**.
- **Orphaned** spans (started, never ended) are bounded by `OtlpConfig.max_in_flight_spans`
  (a `usize` cap on the live-span table). On overflow, a new `start_span` **drops the new
  span and emits a diagnostic warning** — it never evicts a live span, overwrites, or grows
  unbounded. `TracerLifecycle::shutdown()` flushes the remaining pending spans and clears
  the table.

### ADR-6: Redaction at the port boundary (not in the adapter)
`start_span` takes a domain `SpanAttributes` — an allow-list of non-sensitive typed
scalars (operation name, tenant-hint **presence** bool, outcome, duration) — **not**
free-form `&[(&str,&str)]`. Tenant ids, credentials, principal subject, and payloads
**cannot be expressed** as `SpanAttributes`, so redaction is structurally enforced in the
domain before anything reaches infra. The adapter no longer redacts — it maps
already-safe attributes to otel key/values. Any redacting type attached renders the
single existing `[REDACTED]` convention (`auth/credential.rs:50`).

### ADR-7: Outbound propagation-only (v1) + transports in scope
Every supported outbound transport MUST propagate the current `TraceContext` as a W3C
`traceparent`, obtained **explicitly** from `ServiceContext`
(`ctx.trace_context().to_traceparent()`) — ambient lookup forbidden. `to_traceparent()`
serializes the **current local span** (`trace_id`/`span_id`) so it becomes the remote
parent of the next service. The transport MUST NOT create its own span in v1
(client/server span semantics deferred); the span stays owned by the request
boundary/interceptor.

**Transports actually in ego (investigated) and scope:**

| Transport | Exists? | In scope? | Evidence |
|-----------|---------|-----------|----------|
| HTTP | Yes (`crates/transport`, axum) | **Yes** — inject `traceparent` header outbound | `transport/src/{lib,server}.rs` |
| gRPC | No client (`tonic` absent); `GrpcServerConfig` is config-only | **No** — transport does not exist | `transport/src/config.rs`, `lib.rs` ("no gRPC transport") |
| Messaging | In-process tokio mpsc only, no wire headers | **No** — no header/metadata propagation model | `ego-scheduler/src/event_bus.rs`, `persistent-entity/src/publisher.rs` |
| Actor/effect-runner | — | **No** (non-goal) | — |
| `reqwest` (OIDC) | Yes, but does not carry `ServiceContext` | **No** — not a service-to-service boundary | `security-jwt/src/{jwks,introspection,discovery}.rs` |

Because ego ships no framework outbound HTTP client, v1 provides the **mechanism** and a
propagation helper in `crates/transport` that builds the header from an explicitly-passed
context; application outbound call sites apply it.

**Causal chain A→B→C:** A emits `traceparent AAA/111`. B `from_inbound` → trace `AAA`,
new local span `222`, parent `111`; B `to_traceparent()` emits `AAA/222`. C `from_inbound`
→ trace `AAA`, new local span `333`, parent `222`.

### ADR-8: Sampling — always-on (decided)
v1 is **always-on, no sampler** — a decided behavior, not an open question. Configurable
ratio/parent-based sampling is an explicit deferred non-goal.

### ADR-9: `shutdown` on a separate `TracerLifecycle` trait
**Choice**: `Tracer` = `start_span`/`end_span` only; a separate `TracerLifecycle::shutdown()`.
**Alternatives**: keep `shutdown` on `Tracer` (v1 simplification).
**Rationale**: `shutdown` is exporter/implementor lifecycle, not a domain tracing operation.
Splitting it means `NoopTracer`, test spies, and future tracer impls are not forced to know
an OTLP operational concern. The runtime owns the lifecycle and calls `shutdown()` on
teardown; the OTLP adapter implements both `Tracer` and `TracerLifecycle`.

## Data Flow

    remote traceparent? ─yes─▶ TraceContext::from_inbound(header)  ┐
                        ─no──▶ TraceContext::root()               ├─▶ ServiceContext ─▶ proxy
      proxy: chain.on_request(&ctx) ─▶ handler ─▶ on_response(&ctx) / on_error(&ctx,e)
      TracingInterceptor reads ctx.trace_context() ─▶ Arc<dyn Tracer> (OTLP adapter, infra)
      outbound HTTP call: header["traceparent"] = ctx.trace_context().to_traceparent()

### Sequence: inbound → span → outbound injection → export

    Ingress        Proxy          TracingIntc        Tracer(OTLP adapter)   OutboundHTTP  Collector
      │ from_inbound|root → ctx                                                            
      ├──ctx──▶ │                                                                          
      │         ├─on_request(&ctx)─▶│                                                      
      │         │                   ├ start_span(&ctx,attrs)→SpanId(==ctx.span_id)         
      │         │                   │        insert Map[SpanId]=otel span                  
      │         ├── handler(ctx) ──────────────────────────────▶ inject to_traceparent()─▶│
      │         ├─on_response(&ctx)─▶│                                                     │
      │         │                   ├ end_span(ctx.span_id, Ok)  remove+close → export ───────▶│
      │         │  on_error: end_span(ctx.span_id, Error{status_message})  (idempotent)    

`SpanAttributes` are built in-domain before `start_span`; sensitive data is unrepresentable.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/tracer.rs` | Create | `Tracer`, `TraceContext`, `TraceId`/`SpanId`, `SpanAttributes`, `SpanOutcome`, `NoopTracer`, `parse_traceparent` |
| `crates/domain/src/lib.rs` | Modify | `pub mod tracer;` + re-exports |
| `crates/service-sdk/src/context/mod.rs` | Modify | `trace_context` field, `with_trace_context`, `trace_context()`; flat `trace_id` mirror (no `with_span`) |
| `crates/service-sdk/src/interceptor/builtin/mod.rs` | Modify | Unstub; export `TracingInterceptor` |
| `crates/service-sdk/src/interceptor/builtin/tracing.rs` | Create | `TracingInterceptor { tracer: Arc<dyn Tracer> }` start/end/error; `on_error` → redacted `status_message` |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `with_tracer(Arc<dyn Tracer>)` registers the interceptor; runtime owns the optional `Arc<dyn TracerLifecycle>` and calls `shutdown()` on teardown |
| `crates/transport/src/propagation.rs` | Create | Outbound `traceparent` header builder from an explicit `TraceContext` (HTTP injection point) |
| `crates/transport/src/lib.rs` | Modify | `pub mod propagation;` |
| `crates/infrastructure/src/tracing_otlp.rs` | Create | `OtlpTracer` impl `Tracer` + `TracerLifecycle` (SpanId-keyed table, idempotent end, bounded by `max_in_flight_spans` → drop-new+warn, shutdown flush) + `OtlpConfig { endpoint, protocol: Grpc\|Http, max_in_flight_spans: usize }` |
| `crates/infrastructure/Cargo.toml` | Modify | Add `opentelemetry`, `opentelemetry-otlp` (infra only) |

## Interfaces / Contracts

```rust
// ego-domain: transport-agnostic, NO opentelemetry types.
pub struct TraceId(/* 16-byte W3C */);
pub struct SpanId(/* 8-byte W3C */);            // the span handle == TraceContext.span_id

pub struct TraceContext { /* trace_id, span_id, parent_span_id: Option<SpanId> */ }
impl TraceContext {
    pub fn root() -> Self;                                   // new trace + root span
    pub fn child(&self) -> Self;                             // same trace, parent = self.span_id (future nesting seam)
    pub fn from_inbound(tp: &str) -> Result<Self, TraceParseError>; // same trace_id, parent = remote span, NEW local span
    pub fn to_traceparent(&self) -> String;                 // serialize CURRENT LOCAL span
    pub fn span_id(&self) -> SpanId;                         // handle already carried
}
pub fn parse_traceparent(s: &str) -> Result<(TraceId, SpanId), TraceParseError>; // raw decode only

pub struct SpanAttributes;                       // allow-list of non-sensitive typed scalars
impl SpanAttributes {                            // sensitive data is UNREPRESENTABLE here
    pub fn new(operation: &str) -> Self;
    pub fn with_tenant_present(self, present: bool) -> Self;
    pub fn with_duration(self, d: std::time::Duration) -> Self;
}
pub enum SpanOutcome { Ok, Error { status_message: String } } // status_message must be redaction-safe

pub trait Tracer: Send + Sync {                  // non-blocking, stateless-trait like Observability
    fn start_span(&self, ctx: &TraceContext, name: &str, attrs: SpanAttributes) -> SpanId; // returns ctx.span_id()
    fn end_span(&self, span: SpanId, outcome: SpanOutcome);   // idempotent per SpanId
}
// Exporter/operational lifecycle — SEPARATE from the domain tracing calls (ADR-9) so
// NoopTracer, test spies, and future tracers need not know an OTLP operational concern.
// The runtime owns it and calls shutdown() on teardown; the OTLP adapter implements both.
pub trait TracerLifecycle: Send + Sync {
    fn shutdown(&self);                                        // flush pending + clear table
}
pub struct NoopTracer;   // default; implements Tracer ONLY (start_span returns ctx.span_id(), end no-op)
```

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | `from_inbound` (new local span, parent = remote), `root`, `child`, `to_traceparent` round-trip; A→B→C parent linkage | domain tests |
| Unit | `start_span` returns `ctx.span_id()`; `SpanAttributes` cannot carry tenant/credential/payload | domain tests |
| Unit | `end_span` idempotent (double-end = one close); duplicate `start_span` warns; `SpanOutcome::Error{status_message}` redaction-safe | adapter tests |
| Unit | `TracerLifecycle::shutdown` flushes orphaned spans and clears table; at `max_in_flight_spans` a new span is dropped + warned (no eviction/overwrite/unbounded growth) | adapter tests |
| Unit | `NoopTracer` no-ops; `TracingInterceptor` start/end/error via spy `Tracer`; no `with_span` | interceptor tests |
| Unit | **Boundary lint**: no `Context::current()`/`Span::current()` outside adapter | source-scan test |
| Integration | Outbound HTTP injects `traceparent`; OTLP gRPC & HTTP export to stub collector; disable ⇒ no-op | infra `#[tokio::test]` |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, or executable-file
classification. OTLP is outbound network egress; its data-exposure risk is structurally
closed by ADR-6 (redaction at the port boundary), not the process-integration matrix.

## Migration / Rollout

`NoopTracer` is the default; behavior is byte-for-byte unchanged until `with_tracer` is
called. OTLP adapter is opt-in via wiring/config. No schema/migration. Rollback = drop
the `with_tracer` wiring. `correlation_id`/flat `trace_id` preserved (no breaking API).

## Known v1 limitations & seams

- **Guard-denial spans**: CORE-012A macro-guard denials short-circuit before the chain →
  no span (accepted). Seam: `Tracer` is reachable from `RuntimeInner` for a future denial span.
- **Actor/effect-runner & messaging origination**: deferred. Seam: `TraceContext::child()`
  + `Arc<dyn Tracer>` are usable once those contexts carry a `TraceContext` — no port change.
- **Client/server transport spans**: deferred; v1 outbound is propagation-only (ADR-7).

## Open Questions

None. Sampling is decided (ADR-8: always-on for v1).
