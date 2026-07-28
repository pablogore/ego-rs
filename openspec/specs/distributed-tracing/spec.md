# Distributed Tracing Specification

## Purpose

Distributed tracing (spans) over OTLP for `ego-rs`, v1 scope: one
interceptor-owned span per request boundary, HTTP-only outbound propagation,
and always-on OTLP export. Provides a transport-agnostic `Tracer` port and
`TraceContext` value in `ego-domain`, a `NoopTracer` default, and an
OTLP-backed adapter in `infrastructure` (sole `opentelemetry` consumer).

## Requirements

### Requirement: Start Span Returns Nothing; TraceContext.span_id Is Authoritative

`start_span(ctx: &TraceContext, name: &str, attrs: SpanAttributes)` MUST
return nothing. The authoritative span identity is `TraceContext.span_id()`
(`ctx.span_id()`); there is no separate returned handle/token type. A returned
`SpanId` would always equal `ctx.span_id()` and a stateless interceptor would
discard it, so the return is omitted to remove that contractual redundancy —
`end_span` re-derives the id from `&ctx` when the span is closed.

#### Scenario: start_span returns no handle; the span id comes from the context
- GIVEN a `TraceContext` `ctx` with `span_id` `S`
- WHEN `start_span(&ctx, "op", attrs)` is called, then a later separate call
  site reads `ctx.span_id()` from the same `ServiceContext`
- THEN `start_span` yields no value, and the id used to end the span is `S`
  obtained from `ctx.span_id()`, with no stored guard or ambient lookup
  involved

### Requirement: End Span Is Idempotent Per SpanId; Duplicate Start Is Ignored

`end_span(SpanId, SpanOutcome)` MUST be idempotent per `SpanId`: after the
first terminal outcome, subsequent calls for the same `SpanId` MUST have no
effect (the first call closes and exports the span; any later call for the
same `SpanId` is a no-op). An `end_span` for a `SpanId` that was never
started (unknown to the span table) MUST also be a no-op — the interceptor
may call `end_span` even when `on_request` never opened a span (e.g. a
failure earlier in the interceptor chain), and that MUST NOT raise or export
anything. This is why the interceptor may safely call `end_span` on both
`on_response` and `on_error`. A `start_span` for a `SpanId` still live in the
span table MUST be ignored (with a warning) rather than overwrite the
existing entry. The enforcing adapter contract test (`OtlpTracer`) lands in
PR5; the port here only states the normative contract.

#### Scenario: on_response and on_error race resolves to a single close
- GIVEN a span started for `SpanId` `S`
- WHEN `end_span(S, Ok)` and `end_span(S, Error{..})` are both invoked for `S`
- THEN only the first call closes and exports the span; the second is a
  no-op

#### Scenario: end_span for a never-started (unknown) SpanId is a no-op
- GIVEN no span was ever started for `SpanId` `S` (the request failed before `on_request` opened one)
- WHEN `end_span(S, ..)` is called
- THEN it is a no-op — nothing is closed or exported, and no error is raised

#### Scenario: Duplicate start_span for a live SpanId is ignored
- GIVEN `start_span` was already called for a `TraceContext` whose
  `span_id()` is `S`, and that span is still open in the table
- WHEN `start_span` is called again for a context with the same `S`
- THEN the existing table entry for `S` is unchanged and a warning is
  emitted

### Requirement: Lifecycle Is A Separate Concern From The Tracing Port

`shutdown` MUST NOT be a method on the `Tracer` port. It MUST live on a
separate `TracerLifecycle` trait, so `NoopTracer`, test spies, and future
tracers are not forced to implement an exporter/operational concern. The
runtime MUST own the `TracerLifecycle` (when present) and MUST call
`shutdown()` on teardown. `TracerLifecycle::shutdown()` MUST flush all
pending (unended) spans and MUST clear the span table afterward.

#### Scenario: shutdown exports orphaned spans and empties the table
- GIVEN spans started but never ended remain in the span table
- WHEN `TracerLifecycle::shutdown()` is called
- THEN each pending span is flushed/exported and the table is empty

#### Scenario: a plain Tracer implementation need not know about shutdown
- GIVEN a tracer implementation (e.g. `NoopTracer` or a test spy) that implements only `Tracer`
- THEN it MUST compile and be usable without implementing `TracerLifecycle`

### Requirement: In-Flight Spans Are Bounded By A Configured Limit (Soft Bound)

The live-span table MUST be bounded by a configured `max_in_flight_spans:
usize`. When the table is observed at (or above) capacity as a new span is
started, the implementation MUST **drop the new span and emit a diagnostic
warning**. It MUST NOT evict a live span, overwrite an existing entry, or grow
the table unbounded.

This is a **soft bound**: to avoid a global lock across span start (see the
non-blocking contract), the capacity check and the insert are not atomic, so
concurrent starts MAY briefly overshoot `max_in_flight_spans` by the number of
racing starts before the check takes effect. The table MUST stay `O(max_in_flight_spans + concurrent_starts)` — bounded and short-lived — never
unbounded. A hard cap is an explicit non-goal (it would require the very
global synchronization the port forbids).

#### Scenario: starting a span over the in-flight limit drops it with a warning
- GIVEN the live-span table already holds `max_in_flight_spans` unended spans and no other start is in flight
- WHEN a new span is started
- THEN the new span is dropped, a diagnostic warning is emitted, and no existing live span is evicted or overwritten

### Requirement: End Span With Error Records A Redaction-Safe Status Message

`SpanOutcome::Error { status_message: String }` MUST carry a
`status_message` that is recorded on the closed span, and that message MUST
be redaction-safe (no raw sensitive values).

#### Scenario: Error outcome records the given status_message
- GIVEN a span started for `SpanId` `S`
- WHEN `end_span(S, SpanOutcome::Error { status_message: "msg" })` is called
- THEN the closed span's recorded status includes `status_message` equal to
  `"msg"`, not merely a generic error flag

### Requirement: TraceContext Distinguishes Inbound Origination From Raw Parsing

`TraceContext::from_inbound(traceparent: &str) -> Result<TraceContext,
TraceParseError>` MUST produce a `TraceContext` with the SAME `trace_id` as
the parsed header, the remote span-id as `parent_span_id`, and a NEW
locally-generated `span_id`. This MUST be distinct from `parse_traceparent`,
which only decodes `(TraceId, SpanId)` and constructs no `TraceContext`.

#### Scenario: from_inbound creates a new local span with remote parent linkage
- GIVEN a valid `traceparent` header with `trace_id` `T` and remote
  `span_id` `R`
- WHEN `TraceContext::from_inbound(header)` is called
- THEN the result has `trace_id` `T`, `parent_span_id` `Some(R)`, and a
  freshly-generated `span_id` different from `R`

#### Scenario: parse_traceparent performs raw decode only
- GIVEN the same valid `traceparent` header
- WHEN `parse_traceparent(header)` is called
- THEN it returns `(T, R)` only — no new span-id, no `TraceContext`

#### Scenario: A→B→C parent linkage chains correctly
- GIVEN service A emits `to_traceparent()` for trace `T`, span `1`
- WHEN B calls `from_inbound` on that header (new span `2`, parent `1`) and
  emits its own `to_traceparent()`, and C calls `from_inbound` on B's header
- THEN C's `TraceContext` has `trace_id` `T`, `parent_span_id` `Some(2)`, and
  a span-id distinct from `1` and `2`

### Requirement: Outbound HTTP Propagation Injects TraceContext Without Creating A Span

The HTTP transport (`crates/transport`) MUST propagate the current
`TraceContext` on outbound calls as a W3C `traceparent` header, built from a
`TraceContext` obtained EXPLICITLY from `ServiceContext`
(`ctx.trace_context()`) — ambient/task-local lookup is forbidden. The
outbound call MUST NOT create its own client span in v1; the span remains
owned by the request-boundary interceptor. gRPC and messaging transports are
explicitly OUT OF SCOPE for outbound propagation (no gRPC client exists and
`ego-rs`'s in-process messaging has no wire-header/metadata model).

#### Scenario: Outbound HTTP call injects the traceparent header
- GIVEN a `ServiceContext` whose `trace_context()` is set
- WHEN an outbound HTTP call is made from a reference-app call site using the
  `crates/transport` propagation helper
- THEN the outgoing request carries a `traceparent` header equal to
  `ctx.trace_context().to_traceparent()`, and no new span is started for the
  outbound call

#### Scenario: gRPC and messaging are not required to propagate traceparent
- GIVEN `ego-rs` has no gRPC client transport and no wire-header model for
  its in-process messaging
- WHEN evaluating v1 conformance
- THEN absence of `traceparent` propagation over gRPC or messaging is NOT a
  defect

### Requirement: Span Attributes Are A Redaction-Safe Allow-List Enforced In Domain

`start_span` MUST take a typed `SpanAttributes` value — an allow-list of
non-sensitive scalars (tenant-hint presence boolean via
`with_tenant_hint_present`, duration) — never a free-form key/value map. The
span's operation is its **name** (the `name` argument to `start_span`), not a
`SpanAttributes` field. The tenant-hint-presence attribute reflects only the inbound
caller-supplied hint (set pre-enforcement from `has_tenant_hint()`), NOT the
resolved/canonical tenant. Tenant ids, credentials, principal subject, and
payload data MUST NOT be expressible as `SpanAttributes`. The
`infrastructure` adapter MUST NOT redact; it only maps already-safe
`SpanAttributes` to OTel key/values.

#### Scenario: SpanAttributes cannot carry a tenant id, credential, or payload
- GIVEN the `SpanAttributes` builder's public API
- WHEN attempting to construct attributes carrying a raw tenant id, a
  credential/token, or an arbitrary payload
- THEN no such constructor or field exists — the value cannot be expressed

#### Scenario: Adapter maps already-safe attributes without redacting
- GIVEN `SpanAttributes::new().with_tenant_hint_present(..).with_duration(..)`
- WHEN the OTLP adapter exports the span
- THEN it maps the given attributes directly to OTel key/values with no
  redaction step applied

### Requirement: Request Span Is Named After The Operation

The request span opened by the tracing interceptor MUST be named after the
real operation being served, not a fixed literal. The name is carried on the
request context (`ServiceContext.operation_name`), populated by the
macro-generated service proxy from the method it dispatches, before the
interceptor chain runs; the tracing interceptor MUST use that operation name
as the span name. When a context carries no operation name (e.g. a
hand-constructed context or a non-proxied call path), the interceptor MUST
fall back to a single stable default name and MUST NOT panic or emit an empty
name. The trait signature of the interceptor MUST NOT change to carry the
name — it travels as additive context data.

(Implemented: PROD-003 follow-up #212 — `ServiceContext.operation_name`,
the `#[service]` proxy binding, and the `TracingInterceptor` name selection
with a `"request"` fallback all shipped. This requirement records the
already-delivered behavior that the v1 spec did not yet capture.)

#### Scenario: Request span uses the operation name when present
- GIVEN a request dispatched through the macro-generated proxy for operation
  `create_user`
- WHEN the tracing interceptor opens the request span
- THEN the span name is `create_user`, sourced from
  `ServiceContext.operation_name`

#### Scenario: Missing operation name falls back to a stable default
- GIVEN a hand-constructed `ServiceContext` with no operation name
- WHEN the tracing interceptor opens the request span
- THEN the span name is the stable default (`request`), with no panic and no
  empty name

### Requirement: Tracer Port Is Transport-Agnostic, Non-Blocking

The `Tracer` trait signature (methods, parameters, return types) MUST NOT
reference `opentelemetry` or any other vendor type; all such types MUST be
confined to `infrastructure`. Implementations MUST NOT perform synchronous I/O
or network calls inside any trait method, and MUST NOT hold a contended/global
lock across exporter/SDK work; span bookkeeping MUST stay bounded and
short-lived (a per-shard concurrent map is fine; a global mutex held across
span construction/export is not).

#### Scenario: Domain crate has no opentelemetry symbols
- GIVEN the `ego-domain` crate source, including `tracer.rs`
- WHEN its public signatures are inspected
- THEN no `opentelemetry`/`opentelemetry-otlp` type appears anywhere

#### Scenario: Span start/end calls do not block the caller
- GIVEN a `Tracer` implementor with expensive export work to perform
- WHEN a span is started or ended on a request-critical path
- THEN the call returns without waiting on network I/O or export completion

### Requirement: NoopTracer Is A Zero-Effect Default

`ego-domain` MUST provide a `NoopTracer` implementation of `Tracer` (it does
NOT implement `TracerLifecycle`). `start_span` MUST return nothing and have no
observable side effect; `end_span` MUST be a no-op. `NoopTracer` MUST be the
default when no tracer is wired.

#### Scenario: NoopTracer start_span is a no-op with no side effects
- GIVEN a `NoopTracer` instance and a `TraceContext` `ctx`, with no tracer
  wired in a fresh runtime
- WHEN `start_span(&ctx, ..)` then `end_span(ctx.span_id(), ..)` are called
- THEN `start_span` returns nothing with no exported/observable span data,
  the id passed to `end_span` is obtained from `ctx.span_id()`, and the
  unwired runtime uses `NoopTracer` by default

### Requirement: OTLP Adapter Exports Spans Over A Configurable Transport

The `infrastructure` crate MUST provide a `Tracer` implementor exporting
completed spans over OTLP. The export wire transport (gRPC or HTTP) MUST be
selectable via `OtlpConfig`, not hardcoded, and MUST NOT change the `Tracer`
port's signature. This is distinct from outbound trace propagation.

#### Scenario: OTLP adapter exports over the configured protocol
- GIVEN the OTLP adapter is configured with `protocol: Grpc` or
  `protocol: Http`
- WHEN spans are started and ended through it
- THEN completed spans are exported to the collector over the configured
  protocol

### Requirement: Sampling Is Always-On In v1

v1 MUST export every started/ended span (always-on sampling); no sampler or
sampling-decision hook MUST exist in the `Tracer` port or its adapter.
Configurable/ratio/parent-based sampling MUST NOT be implemented in v1.

#### Scenario: Every ended span is exported, no sampling decision applied
- GIVEN any span started and ended through the OTLP adapter
- WHEN the adapter processes it
- THEN the span is exported (subject to `shutdown`/flush timing) with no
  sampling ratio or decision applied to skip it

### Requirement: Out of Scope for v1

NOT covered by this capability in v1: OTLP-exported metrics, OTLP-exported
logs, actor/effect-runner and messaging trace origination/propagation,
client/server transport spans (outbound is propagation-only), configurable
sampling, and spans for CORE-012A macro-guard denials (denials short-circuit
before the interceptor chain; accepted v1 limitation). `Observability` and
the CORE-012A denial-recording contract remain unchanged.

#### Scenario: A macro-guard denial produces no span
- GIVEN an operation denied by `#[authorize]` or `#[tenant_scoped]` before
  reaching the interceptor chain
- WHEN the denial occurs
- THEN no span is started or ended, and the existing `Observability`
  denial-recording contract (CORE-012A) still records the denial exactly as
  before
