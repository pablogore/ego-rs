# Archive Report — PROD-003: Production Observability / OTLP (Distributed Tracing v1)

**Archived:** 2026-07-23
**Status:** complete — all tasks 001-030 done; delivered in 6 stacked PRs.

## Delivered
- **Domain** `Tracer` port (start/end, no return; SpanId = authoritative identity) + separate `TracerLifecycle` (ADR-9); `TraceContext` (root/from_inbound/child/to_traceparent, W3C-strict version-00 inbound); redaction-safe typed `SpanAttributes`; `NoopTracer` default. Vendor-neutral (no `opentelemetry` in domain).
- **service-sdk** explicit `trace_context` on `ServiceContext` (authoritative-by-construction over the legacy `trace_id`); `TracingInterceptor` (one span per request boundary; redaction-safe error status); `RuntimeBuilder::with_tracer`/`with_traced` + single `TracerLifecycle` shutdown on teardown.
- **transport** HTTP ingress origination via a `TraceContextExtractor` (boundary-owned) + outbound `traceparent` propagation (propagation-only, no client span).
- **infrastructure** `OtlpTracer` (DashMap bookkeeping, direct `SpanData` construction, no lock across exporter work; idempotent/unknown-id/duplicate-start/soft-overflow/shutdown semantics; configurable gRPC/HTTP) — exported span fully domain-identified. Real gRPC+HTTP wire round-trip tests.
- **Enforcement** OTLP boundary lint (no `Context::current()`/`Span::current()` outside the adapter).

## PRs
#209 (domain) → #211 (context+interceptor) → #214 (runtime wiring) → #216 (HTTP ingress+outbound) → #218 (OTLP adapter) → #220 (boundary lint + AC-9).

## Notable decisions & findings
- Explicit trace-context, NO ambient state (honors EGO's enforced invariant); ambient OTel API confined to the adapter and CI-linted.
- `start_span` returns nothing (SpanId is the identity carried on `TraceContext`).
- Non-blocking contract refined to: no sync I/O / no contended-or-global lock across exporter/SDK work; bookkeeping bounded and short-lived (soft in-flight bound).
- Real wire tests caught a production bug: the async reqwest HTTP exporter panics "no reactor" on the batch thread → switched to `reqwest-blocking-client`.

## Deferred follow-ups
- Operation-level span naming (issue #212) — interceptor uses a fixed `"request"` name in v1.
- Preserve inbound sampling flags — v1 is always-on (ADR-8); `to_traceparent` emits `01`.
- OTLP metrics, OTLP-exported logs, actor/effect-runner tracing, guard-denial spans, configurable sampling — explicit v1 non-goals.

## Spec merge
- New capability spec: `openspec/specs/distributed-tracing/spec.md`.
- `openspec/specs/service-sdk/spec.md`: appended the ADDED trace-context / interceptor / ingress requirements + the AC-9 (Pure DTO) scope note.
