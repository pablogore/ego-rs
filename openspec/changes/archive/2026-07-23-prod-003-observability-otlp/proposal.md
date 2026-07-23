# Proposal: PROD-003 — Production Observability / OTLP (Distributed Tracing v1)

## Intent

`ego-rs` has no production trace export: no `opentelemetry` deps, no installed subscriber, `tracing` call sites emit into the void, and `ServiceContext.trace_id`/`correlation_id` are unused flat strings. Operators cannot follow a request across command → event → projection boundaries. PROD-003 v1 delivers **distributed tracing (spans) over OTLP** while preserving the enforced *no ambient/thread-local state* invariant (`ARCHITECTURE.md:140`, `config.yaml:7`).

## Scope

### In Scope
- New dedicated span-lifecycle port (`Tracer`) in `ego-domain`, separate from `Observability`; `NoopTracer` default mirroring `NoopObservability`.
- OTLP-backed implementor in `infrastructure`, transport **configurable (gRPC or HTTP)**; `opentelemetry`/`opentelemetry-otlp` confined to infra.
- Explicit trace-context value on `ServiceContext` (trace-id + span-id + parent, W3C traceparent-compatible), extending existing fields.
- Unstub `TracingInterceptor` to own span start/end/error via the interceptor chain (`on_request`/`on_response`/`on_error`). `start_span` returns the span handle (`SpanId`), which is the identity already carried in `TraceContext`; `end_span(SpanId, outcome)` is idempotent.
- **Inbound** origination at request ingress: `TraceContext::from_inbound(traceparent)` when a W3C `traceparent` is present (same trace-id, remote span-id becomes parent, **new** local span-id), else `TraceContext::root()`.
- **Outbound propagation-only** at supported transport boundaries (HTTP; gRPC and messaging where the transport already defines header/metadata propagation): the outbound transport injects `ctx.trace_context().to_traceparent()`, obtained **explicitly** from `ServiceContext` (ambient lookup forbidden). The transport does **not** create its own span — the span stays owned by the request boundary/interceptor.
- Redaction enforced at the **port boundary** (domain span-attribute allow-list of non-sensitive scalars), not deep in the adapter.

### Out of Scope (Non-Goals / Follow-ups)
- **OTLP metrics** — deferred (future change).
- **OTLP-exported logs** — deferred (future change).
- **Actor/effect-runner tracing** (`persistent-entity`, `ego-scheduler`) — non-request/response origination deferred.
- **Guard-denial spans** — CORE-012A macro-guard denials short-circuit before the interceptor chain; v1 **accepts "guard denied → no span"** as a documented known limitation.
- **Client/server transport spans** — v1 outbound is propagation-only; transport-created (client/server) spans deferred.
- **Sampling configuration** — v1 is always-on (no sampler); configurable/ratio sampling deferred.
- **Manual nested child spans** (`ServiceContext::with_span`) — deferred; v1 is exactly one interceptor-owned span per request boundary (`TraceContext::child()` retained as the seam).
- `Observability` port and CORE-012A security-denial contract (`service-sdk/spec.md:1348-1429`) — left UNTOUCHED.

## Capabilities

### New Capabilities
- `distributed-tracing`: `Tracer` span-lifecycle port, `NoopTracer`, OTLP infra adapter, interceptor-driven span lifecycle, ingress trace-context origination.

### Modified Capabilities
- `service-sdk`: `ServiceContext` gains an explicit trace-context value; `TracingInterceptor` unstubbed in the builtin chain.

## Approach

Port stays in domain, OTLP in infra (hexagonal). Trace context travels **as data on `ServiceContext`** — never ambient. Any `Span::current()`/`Context::current()` usage is strictly an infra OTLP-adapter implementation detail, never exposed to service authors or used to carry context. Interceptor chain wraps the handler body to start/end/error spans reading the explicit context.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/domain/src/observability.rs` (new `tracer.rs`) | New | `Tracer` port + `NoopTracer` |
| `crates/infrastructure` | New | OTLP adapter (gRPC/HTTP), `opentelemetry-otlp` |
| `service-sdk/src/context/mod.rs` | Modified | Explicit trace-context value |
| `service-sdk/src/interceptor/builtin/mod.rs` | Modified | Unstub `TracingInterceptor` |
| `service-sdk/src/runtime/builder.rs` | Modified | `with_tracer(...)` wiring |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Ambient-state idiom reintroduced | Med | Confine `Context::current()` to infra adapter; explicit context on `ServiceContext` |
| Guard-denial coverage hole | High | Accept for v1, document as follow-up |
| Sensitive data in span attributes | Med | Reuse redaction convention; forbid unredacted tenant/credential attributes |
| `opentelemetry` leaking into domain | Low | Deps infra-only; port is transport-agnostic |

## Rollback Plan

Feature-gated and **Noop-default**: `NoopTracer` ships as default, OTLP adapter behind config/feature flag. Disabling reverts to no-op with zero behavioral change (spans are diagnostic only). No schema/migration impact; revert = drop the infra adapter wiring.

## Dependencies

- `opentelemetry`, `opentelemetry-otlp` (infra crate only).
- Deployment collector assumed present.

## Success Criteria

- [ ] `Tracer` port + `NoopTracer` in domain; no `opentelemetry` symbols in `ego-domain`.
- [ ] OTLP adapter exports spans over configurable gRPC/HTTP to a collector.
- [ ] Trace-context propagates explicitly via `ServiceContext` across request/response.
- [ ] `TracingInterceptor` starts/ends/errors spans around the handler.
- [ ] Span attributes redacted; no tenant/credential leakage.
- [ ] `cargo test --workspace` green; disabling tracer yields no-op.
