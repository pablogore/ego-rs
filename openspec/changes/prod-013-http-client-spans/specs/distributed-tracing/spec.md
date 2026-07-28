# Delta for distributed-tracing

This delta adds the outbound HTTP **client-span** instrumentation contract and
supersedes the v1 clauses that forbade a client span. It DEPENDS ON PROD-012
(Inbound Sampling Propagation): the client span honors the sampling decision that
PROD-012 makes `TraceContext` carry, so this change MUST land after PROD-012.

## ADDED Requirements

### Requirement: Outbound HTTP Request Is Wrapped In A Client Span

An outbound HTTP request MUST be wrapped in exactly one span of role
`SpanRole::Client` (mapped to OpenTelemetry `SpanKind::Client` only in the
`infrastructure` adapter). The client span MUST be a child of the current
request span carried by `TraceContext`. The client-span contract MUST be
expressed as a vendor-neutral instrumentation port in `ego-domain`; no
`opentelemetry` type and no concrete HTTP client type (reqwest/hyper/axum) MUST
appear in the domain contract.

#### Scenario: An outbound HTTP request produces one client span
- GIVEN an outbound HTTP request instrumented through the framework's
  outbound-HTTP instrumentation port
- WHEN the request is executed
- THEN exactly one span of role `Client` is created for it, parented by the
  current request span in `TraceContext`

#### Scenario: The client-span contract carries no vendor or client type
- GIVEN the `ego-domain` outbound-HTTP instrumentation port and its neutral
  request/response value types
- WHEN their public signatures are inspected
- THEN no `opentelemetry` type and no reqwest/hyper/axum type appears; only the
  `infrastructure` adapter maps `SpanRole::Client` to `SpanKind::Client`

#### Scenario: A non-HTTP transport is not required to create a client span
- GIVEN `ego-rs` has no gRPC client transport and no wire-header model for its
  in-process messaging
- WHEN evaluating conformance
- THEN absence of a client span for gRPC or messaging is NOT a defect — this
  requirement covers outbound HTTP only

### Requirement: Client Span Carries HTTP Semantic-Convention Attributes At A Pinned Version

The client span MUST carry HTTP semantic-convention attributes for the request
method, request URL, target server address, response status, and error. The
attribute **names** MUST follow an explicitly pinned OpenTelemetry Semantic
Conventions version derived from the pinned `0.32` OpenTelemetry crate stack; the
design MUST state that version, and attribute keys MUST be taken from the
semantic-conventions crate constants rather than hand-typed literals. The
deprecated pre-stability keys (`http.method`, `http.url`, `http.status_code`)
MUST NOT be emitted. Attribute values MUST be redaction-safe: the recorded URL
MUST NOT carry credentials or secret query values, and the error attribute MUST
carry a closed error **kind**, never a free-text backend message or payload. The
semantic-convention **naming** MUST live only in `infrastructure`; the domain
carries neutral typed fields.

#### Scenario: Client span uses the pinned stable attribute keys
- GIVEN an outbound HTTP request whose method, URL, server address, and response
  status are known
- WHEN the client span is exported
- THEN it carries `http.request.method`, `url.full`, `server.address`,
  `http.response.status_code` (and `server.port` when known), named per the
  pinned semantic-conventions version stated in the design

#### Scenario: Deprecated attribute keys are never emitted
- GIVEN the exported client span
- WHEN its attribute keys are inspected
- THEN none of `http.method`, `http.url`, or `http.status_code` appears — only
  the stable pinned-version keys

#### Scenario: The recorded URL and error carry no sensitive free text
- GIVEN an outbound request whose URL contains credentials/secret query values
  and whose failure carries an internal backend message
- WHEN the client span records `url.full` and the error attribute
- THEN the URL is redaction-safe (no credentials/secret query) and the error is
  a closed `error.type` kind, with no free-text backend message field

### Requirement: Client Span Status Reflects Success Or Error And Records Duration

The client span's status MUST reflect the outcome of the outbound request:
success sets an Ok/unset status, and a transport failure or an error HTTP
response status sets an error status. The span's duration MUST be recorded (start
to end of the outbound attempt).

#### Scenario: A successful outbound request records Ok status and a duration
- GIVEN an outbound HTTP request that completes successfully
- WHEN its client span ends
- THEN the span status is Ok/unset, `http.response.status_code` is recorded, and
  the span has a recorded duration

#### Scenario: A failed outbound request records an error status
- GIVEN an outbound HTTP request that fails at the transport layer or returns an
  error HTTP status
- WHEN its client span ends
- THEN the span status is Error, `error.type` is recorded as a closed kind, and
  the span still has a recorded duration

### Requirement: Context Propagation Continues Under Client Spans

The outbound HTTP call MUST continue to inject the W3C `traceparent` header,
obtained EXPLICITLY from `TraceContext` (never via ambient/task-local lookup).
When a client span is created, the injected `traceparent` MUST reflect that
client span as the parent for the downstream service. Introducing the client
span MUST NOT remove or weaken existing outbound propagation.

#### Scenario: Outbound call still injects an explicitly-sourced traceparent
- GIVEN a `TraceContext` set on the current `ServiceContext`
- WHEN an instrumented outbound HTTP call is made
- THEN the outgoing request carries a `traceparent` header sourced explicitly
  from `TraceContext` (no ambient lookup)

#### Scenario: The injected traceparent reflects the client span as parent
- GIVEN a client span created for the outbound request
- WHEN the `traceparent` header is injected
- THEN the header's parent span id reflects the client span, so the downstream
  service links to the client span, not to the caller's request span directly

### Requirement: Client Span Honors The Completed Inbound Sampling Decision

The client span MUST honor the sampling decision that `TraceContext` carries
after PROD-012. It MUST NOT make an independent sampling decision, re-sample, or
override the inbound decision: a not-sampled inbound trace MUST yield a
not-sampled client span and a not-sampled outbound `traceparent`, and a sampled
inbound trace MUST yield a sampled client span. This requirement depends on
PROD-012, which makes `TraceContext` carry a faithful decision; PROD-013 MUST
land after it.

#### Scenario: A not-sampled inbound trace yields a not-sampled client span
- GIVEN an inbound trace whose completed sampling decision (PROD-012) is
  not-sampled, carried on `TraceContext`
- WHEN an outbound HTTP client span is created for that trace
- THEN the client span and its injected outbound `traceparent` both reflect the
  not-sampled decision — no independent sampling is applied

#### Scenario: The client span applies no sampler of its own
- GIVEN any outbound HTTP request under a `TraceContext` with a decided sampling
  flag
- WHEN the client span is created
- THEN the decision is inherited from `TraceContext` unchanged; no new
  sampler/ratio/parent-based decision is computed at the client span

### Requirement: Double Instrumentation Is Avoided

There MUST be exactly one client span per outbound HTTP request. When the
underlying HTTP client or framework already emits its own OTel client span, the
framework MUST NOT create a second one; whether the framework creates the client
span MUST be a deterministic property of how the client is bound (opt-in per
client binding declaring whether it self-instruments), NOT an ambient runtime
inspection of the active span. A self-instrumented client MUST still have
`traceparent` propagation applied.

#### Scenario: A self-instrumented client gets no second span
- GIVEN an underlying HTTP client bound as already emitting its own client span
- WHEN an outbound request is made through it
- THEN the framework starts no additional client span (exactly one client span
  total), while `traceparent` propagation is still applied

#### Scenario: A non-instrumented client gets exactly one framework client span
- GIVEN an underlying HTTP client bound as not self-instrumented
- WHEN an outbound request is made through it
- THEN the framework creates exactly one `SpanRole::Client` span for the request

### Requirement: Retries Produce One Client Span Per Attempt

When an outbound HTTP request is retried, each attempt (the initial send and each
resend) MUST produce its own `SpanRole::Client` span. A single merged span for
the whole retried operation MUST NOT be used. Each resend attempt MUST carry the
resend index as the semantic-convention `http.request.resend_count` attribute
(the first attempt omits it or records 0). The retry **policy** (when and how
many times to retry) is out of scope; only per-attempt instrumentation is fixed.

#### Scenario: Each retry attempt gets its own client span with a resend count
- GIVEN an outbound HTTP request that is sent once and then retried once
- WHEN the attempts execute
- THEN two `SpanRole::Client` spans are produced; the retried attempt carries
  `http.request.resend_count = 1`, each with its own status and duration

#### Scenario: A single attempt produces a single client span with no resend count
- GIVEN an outbound HTTP request that succeeds on its first attempt with no retry
- WHEN it executes
- THEN exactly one client span is produced and `http.request.resend_count` is
  absent (or 0), not a merged multi-attempt span

### Requirement: Outbound Instrumentation Contract Is Decoupled From Any Concrete HTTP Client

The outbound instrumentation contract MUST be a hexagonal port that hides the
concrete HTTP client behind a neutral abstraction: the outbound call MUST be
supplied to the port as a neutral operation producing a neutral response value,
so no concrete client type enters the contract. The domain contract MUST remain
free of `opentelemetry` and of reqwest/hyper/axum types; adapters supply the
concrete client and the OTel mapping.

#### Scenario: Swapping the concrete client requires no domain contract change
- GIVEN two different concrete HTTP clients wired behind the same
  outbound-instrumentation port
- WHEN each is used to make an instrumented outbound request
- THEN both satisfy the same domain port unchanged; the domain contract names
  neither concrete client

#### Scenario: The domain instrumentation contract has no OTel or client symbols
- GIVEN the `ego-domain` outbound-instrumentation port and value types
- WHEN their source is inspected
- THEN no `opentelemetry` symbol and no reqwest/hyper/axum symbol appears

## MODIFIED Requirements

### Requirement: Outbound HTTP Propagation Injects TraceContext And Creates A Client Span

The HTTP transport (`crates/transport`) MUST propagate the current
`TraceContext` on outbound calls as a W3C `traceparent` header, built from a
`TraceContext` obtained EXPLICITLY from `ServiceContext` (`ctx.trace_context()`)
— ambient/task-local lookup is forbidden. The outbound call MUST ALSO be wrapped
in a `SpanRole::Client` span per the PROD-013 client-span requirements above; the
request-boundary interceptor still owns the inbound server span, and the client
span is its child. The injected `traceparent` reflects the client span as the
parent for the downstream service. gRPC and messaging transports remain OUT OF
SCOPE for outbound propagation and client spans (no gRPC client exists and
`ego-rs`'s in-process messaging has no wire-header/metadata model).

(Previously: "The outbound call MUST NOT create its own client span in v1; the
span remains owned by the request-boundary interceptor." PROD-013 supersedes the
no-client-span clause: outbound HTTP now creates a client span. Explicit
`traceparent` injection and the gRPC/messaging out-of-scope clause are
unchanged.)

#### Scenario: Outbound HTTP call injects the traceparent header and starts a client span
- GIVEN a `ServiceContext` whose `trace_context()` is set
- WHEN an outbound HTTP call is made through the instrumented outbound path
- THEN the outgoing request carries a `traceparent` header sourced explicitly
  from `TraceContext`, AND a `SpanRole::Client` span is started for the outbound
  call as a child of the current request span

#### Scenario: Propagation is still explicit, never ambient
- GIVEN the outbound propagation helper
- WHEN it builds the `traceparent` value
- THEN the `TraceContext` is obtained explicitly from `ServiceContext`, with no
  ambient/task-local lookup, exactly as before

#### Scenario: gRPC and messaging are not required to propagate or span
- GIVEN `ego-rs` has no gRPC client transport and no wire-header model for its
  in-process messaging
- WHEN evaluating conformance
- THEN absence of `traceparent` propagation and of a client span over gRPC or
  messaging is NOT a defect

### Requirement: Out of Scope for This Capability

NOT covered by this capability: OTLP-exported metrics, OTLP-exported logs,
actor/effect-runner and messaging trace origination/propagation, gRPC and
messaging client spans, configurable/ratio sampling (parent-based honoring of an
inbound decision is PROD-012), retry/backoff POLICY (only per-attempt
instrumentation is in scope), and spans for CORE-012A macro-guard denials
(denials short-circuit before the interceptor chain; accepted limitation).
`Observability` and the CORE-012A denial-recording contract remain unchanged.

(Previously: the out-of-scope list included "client/server transport spans
(outbound is propagation-only)". PROD-013 removes outbound HTTP client spans from
the out-of-scope list — they are now IN scope. gRPC/messaging client spans and
server-transport spans remain out of scope.)

#### Scenario: A macro-guard denial produces no span
- GIVEN an operation denied by `#[authorize]` or `#[tenant_scoped]` before
  reaching the interceptor chain
- WHEN the denial occurs
- THEN no span is started or ended, and the existing `Observability`
  denial-recording contract (CORE-012A) still records the denial exactly as
  before

#### Scenario: An outbound HTTP client span is no longer out of scope
- GIVEN an outbound HTTP request
- WHEN evaluating conformance against this capability
- THEN a `SpanRole::Client` span for that request is expected (in scope), not
  excluded as it was under the v1 propagation-only rule
