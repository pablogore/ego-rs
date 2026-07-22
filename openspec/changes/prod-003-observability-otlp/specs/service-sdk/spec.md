# Delta for service-sdk

## ADDED Requirements

### Requirement: ServiceContext Carries an Explicit TraceContext Value

`ServiceContext` MUST carry an explicit `TraceContext` value (trace-id,
span-id, optional parent), settable via `with_trace_context(TraceContext)`
and readable via `trace_context()`. No ambient, thread-local, or task-local
storage MUST be used to carry it between construction and read. The
existing flat `trace_id` field MUST become a read-through mirror of
`trace_context().trace_id` (source-compatible, not an independent value).
The existing `correlation_id` field is a distinct business-causal concept
and is NOT changed by this delta.

#### Scenario: Trace-context travels only via the explicit ServiceContext value
- GIVEN a call chain that passes `ServiceContext` explicitly across
  functions and `.await`/spawned-task boundaries
- WHEN the trace-context is needed at any point in that chain
- THEN it is obtained only by reading the passed `ServiceContext` value, with
  no ambient/thread-local/task-local lookup involved

#### Scenario: Flat trace_id mirrors trace_context().trace_id
- GIVEN a `ServiceContext` constructed via `with_trace_context(tc)`
- WHEN the flat `trace_id` field/accessor is read
- THEN its value equals `tc.trace_id`, with no independent trace-id storage

#### Scenario: correlation_id is unaffected by trace-context changes
- GIVEN a `ServiceContext` with both a `correlation_id` and a `trace_context`
  set
- WHEN the `trace_context` is read or replaced
- THEN `correlation_id` is unchanged, remaining the distinct business-causal
  identifier

### Requirement: Ambient Span/Context APIs Confined to the Infra OTLP Adapter

Any use of `Span::current()`, `Context::current()`, or equivalent
ambient-context APIs MUST be confined to the `infrastructure` crate's OTLP
adapter module. Service-author-facing code (services, interceptors,
handlers) MUST NOT call or rely on such ambient APIs to obtain or propagate
trace-context.

#### Scenario: Service code never touches ambient span/context APIs
- GIVEN service-author code outside the infrastructure OTLP adapter
- WHEN that code needs the current trace-context
- THEN it obtains it from the explicit `ServiceContext` value only, never
  from `Span::current()` or `Context::current()`

#### Scenario: Boundary lint fails if ambient APIs leak outside the adapter
- GIVEN a source-scan test (in the style of the existing
  `tenant_scoped_lint`) that scans all crates except the `infrastructure`
  OTLP adapter module
- WHEN it runs
- THEN it fails if `Context::current()` or `Span::current()` appears
  anywhere outside that module

### Requirement: TracingInterceptor Drives Span Lifecycle From ServiceContext

The built-in `TracingInterceptor` MUST, on `on_request`, call
`tracer.start_span(ctx.trace_context(), name, attrs)`, obtaining a `SpanId`
equal to `ctx.trace_context().span_id()`. On `on_response` it MUST call
`tracer.end_span(that SpanId, SpanOutcome::Ok)`. On `on_error` it MUST call
`tracer.end_span(that SpanId, SpanOutcome::Error { status_message })` with a
redaction-safe `status_message`. Exactly one span MUST be owned per request
boundary — the interceptor MUST NOT call `ServiceContext::with_span` (not
present in v1) or invoke `TraceContext::child()`.

#### Scenario: Successful invocation starts and ends exactly one span
- GIVEN `TracingInterceptor` is installed and `on_request` calls
  `start_span`, returning `SpanId` `S = ctx.trace_context().span_id()`
- WHEN `on_response` runs
- THEN it calls `end_span(S, Ok)`, closing exactly the span identified by `S`

#### Scenario: Failed invocation ends the span with a redaction-safe error message
- GIVEN `on_request` started a span with `SpanId` `S`
- WHEN the invocation fails and `on_error` runs
- THEN it calls `end_span(S, SpanOutcome::Error { status_message })` with a
  redaction-safe message, re-deriving `S` from `ctx` with no stored guard

#### Scenario: No with_span and no manual nested span in v1
- GIVEN `TracingInterceptor` is the only span owner in v1
- WHEN a request is handled
- THEN no `ServiceContext::with_span` call occurs, and `TraceContext::child()`
  is not invoked by any v1 code path

### Requirement: Trace-Context Originates At HTTP Ingress

Trace-context MUST be originated at HTTP ingress only, exactly once, at the
HTTP handler boundary: `TraceContext::from_inbound(traceparent)` MUST be
used when an inbound `traceparent` header is present, else
`TraceContext::root()`. The resulting `TraceContext` is then carried
explicitly via `ServiceContext` for the remainder of the call.
Message-consumer and actor/effect-runner trace-context origination are OUT
OF SCOPE for this delta.

#### Scenario: HTTP ingress with no traceparent uses root()
- GIVEN an inbound HTTP request with no `traceparent` header
- WHEN the HTTP handler constructs the `ServiceContext` for that request
- THEN `TraceContext::root()` is used, creating a new trace-id and root span

#### Scenario: HTTP ingress with a traceparent uses from_inbound()
- GIVEN an inbound HTTP request carrying a valid `traceparent` header for
  trace `T`, remote span `R`
- WHEN the HTTP handler constructs `ServiceContext` via
  `TraceContext::from_inbound(header)`
- THEN the resulting `TraceContext` has `trace_id` `T`, `parent_span_id`
  `Some(R)`, and a new local `span_id` distinct from `R`

## Out of Scope (Non-Goals for this Delta)

This delta does not add OTLP-exported metrics, OTLP-exported logs, or
tracing/propagation origination for actor/effect-runner execution or
message-consumer ingress (`persistent-entity`, `ego-scheduler`) — deferred,
no wire-header model exists for messaging. It does not add
`ServiceContext::with_span`; `TraceContext::child()` is retained only as a
future seam and is not exercised by any v1 requirement. It does not change
sampling (always-on, decided at the `Tracer` port level). It does not
change the `Observability` port or the CORE-012A macro-guard
denial-recording contract (`service-sdk/spec.md` — Observability for
Macro-Driven Security Enforcement section), which remain untouched. Spans
for macro-guard denials are not produced (guard denials short-circuit
before the interceptor chain); this is a documented v1 limitation.
