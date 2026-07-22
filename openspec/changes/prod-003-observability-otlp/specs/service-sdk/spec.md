# Delta for service-sdk

## ADDED Requirements

### Requirement: ServiceContext Carries an Explicit Trace-Context Value

`ServiceContext` MUST carry a trace-context value composed of a trace-id, a
span-id, and an optional parent span reference, compatible with the W3C
`traceparent` format. This value MUST be propagated explicitly by value as
part of `ServiceContext` — no ambient, thread-local, or task-local storage
MUST be used to carry trace-context between the point it is created and the
point it is read.

#### Scenario: Trace-context is readable from a ServiceContext value
- GIVEN a `ServiceContext` constructed with a trace-context (trace-id,
  span-id, optional parent)
- WHEN the trace-context is read from that `ServiceContext`
- THEN the same trace-id, span-id, and parent reference are returned

#### Scenario: Trace-context travels only via the explicit ServiceContext value
- GIVEN a call chain that passes `ServiceContext` explicitly between
  functions and across an `.await` / spawned task boundary
- WHEN the trace-context is needed at any point in that chain
- THEN it is obtained only by reading the passed `ServiceContext` value, with
  no ambient/thread-local/task-local lookup involved

### Requirement: Ambient Span/Context APIs Confined to the Infra OTLP Adapter

Any use of `Span::current()`, `Context::current()`, or equivalent
ambient-context APIs from the `opentelemetry`/`tracing` ecosystem MUST be
confined to the `infrastructure` crate's OTLP adapter implementation detail.
Service-author-facing code (services, interceptors, handlers) MUST NOT call
or rely on such ambient APIs to obtain or propagate trace-context.

#### Scenario: Service code never touches ambient span/context APIs
- GIVEN service-author code outside the infrastructure OTLP adapter
- WHEN that code needs the current trace-context
- THEN it obtains it from the explicit `ServiceContext` value only, never
  from `Span::current()` or `Context::current()`

#### Scenario: OTLP adapter internally bridges to the vendor SDK
- GIVEN the infrastructure OTLP adapter is exporting a span
- WHEN it interacts with the underlying `opentelemetry` SDK
- THEN any ambient-API usage required by that SDK is confined inside the
  adapter and never leaks into `ServiceContext` or the `Tracer` port contract

### Requirement: TracingInterceptor Drives Span Lifecycle From ServiceContext

The built-in `TracingInterceptor` MUST start a span on `on_request`, end that
span successfully on `on_response`, and, on `on_error`, record the error on
the span and end it. In all cases the interceptor MUST read the
trace-context from the `ServiceContext` value passed to it, rather than from
any ambient source.

#### Scenario: Successful invocation starts and ends one span
- GIVEN `TracingInterceptor` is installed in the interceptor chain
- WHEN a service invocation runs `on_request` followed by `on_response`
- THEN exactly one span is started on `on_request` and ended successfully on
  `on_response`, using the trace-context read from the given `ServiceContext`

#### Scenario: Failed invocation ends the span with error status
- GIVEN `TracingInterceptor` is installed in the interceptor chain and
  `on_request` has started a span
- WHEN the invocation fails and `on_error` runs instead of `on_response`
- THEN the error is recorded on the span and the span is ended with error
  status

### Requirement: Root Trace-Context Originates at Request/Response Ingress

Root trace-context (a new trace-id and root span, or an extracted parent
from an inbound `traceparent`) MUST be originated at request/response ingress
points — the HTTP handler and the message consumer — and then carried
explicitly on `ServiceContext` for the remainder of the call. No other layer
MUST originate a root trace-context for an already-in-flight request.

#### Scenario: HTTP ingress originates trace-context for an inbound request
- GIVEN an inbound HTTP request with no `traceparent` header
- WHEN the HTTP handler constructs the `ServiceContext` for that request
- THEN a new root trace-context is created and set on that `ServiceContext`

#### Scenario: HTTP ingress extracts an inbound traceparent
- GIVEN an inbound HTTP request carrying a valid `traceparent` header
- WHEN the HTTP handler constructs the `ServiceContext` for that request
- THEN the trace-context on that `ServiceContext` reflects the extracted
  parent trace-id and span-id

#### Scenario: Message consumer ingress originates trace-context
- GIVEN an inbound message with no propagated trace-context
- WHEN the message consumer constructs the `ServiceContext` for handling it
- THEN a new root trace-context is created and set on that `ServiceContext`

## Out of Scope (Non-Goals for this Delta)

This delta does not add OTLP-exported metrics, OTLP-exported logs, or
tracing for actor/effect-runner execution (`persistent-entity`,
`ego-scheduler`) outside request/response ingress. It does not change the
`Observability` port or the CORE-012A macro-guard denial-recording contract
(`service-sdk/spec.md` — Observability for Macro-Driven Security Enforcement
section), which remain untouched. Spans for macro-guard denials are not
produced by this delta (guard denials short-circuit before the interceptor
chain runs); this is a documented v1 limitation, not a defect.
