# Distributed Tracing Specification

## Purpose

Distributed tracing (spans) over OTLP for `ego-rs`, v1 scope: request/response
span lifecycle only. Provides a transport-agnostic `Tracer` port in
`ego-domain`, a `NoopTracer` default, and an OTLP-backed adapter in
`infrastructure` configurable for gRPC or HTTP export.

## Requirements

### Requirement: Tracer Port Starts Spans With Context

`ego-domain` MUST define a `Tracer` port, distinct from `Observability`, that
starts a span given a span name, a span kind, and an optional parent
trace-context (trace-id, span-id, and any parent linkage needed to build a
child span). The method MUST return a handle/value representing the started
span, usable to end it later.

#### Scenario: Starting a root span with no parent
- GIVEN no parent trace-context is supplied
- WHEN a span is started with a name and kind
- THEN the port returns a span handle representing a new root span

#### Scenario: Starting a child span with a parent trace-context
- GIVEN a parent trace-context (trace-id, span-id) is supplied
- WHEN a span is started with that parent
- THEN the port returns a span handle whose lineage links to the given parent

### Requirement: Tracer Port Ends Spans on Success

The `Tracer` port MUST provide a method to end a previously started span
indicating successful completion, with no error information attached.

#### Scenario: Ending a span successfully
- GIVEN a span handle returned by starting a span
- WHEN the span is ended without an error
- THEN the port records the span as completed successfully with no error status

### Requirement: Tracer Port Ends Spans With Error

The `Tracer` port MUST provide a method to end a previously started span
while recording that the operation it represents failed, capturing
error information distinct from the success path.

#### Scenario: Ending a span with an error
- GIVEN a span handle returned by starting a span
- WHEN the span is ended with error information
- THEN the port records the span as completed with an error status and the
  given error information

### Requirement: Tracer Port Is Transport-Agnostic

The `Tracer` trait signature (methods, parameters, return types) MUST NOT
reference `opentelemetry` or any other vendor/transport-specific type. All
`opentelemetry`/`opentelemetry-otlp` types MUST be confined to the
`infrastructure` crate; `ego-domain` MUST NOT depend on them.

#### Scenario: Domain crate has no opentelemetry symbols
- GIVEN the `ego-domain` crate source, including the `Tracer` port
- WHEN its public signatures are inspected
- THEN no `opentelemetry` or `opentelemetry-otlp` type appears anywhere in them

### Requirement: Tracer Port Is Non-Blocking

Implementations of `Tracer` MUST NOT perform blocking operations (synchronous
I/O, network calls, lock contention under load) inside any trait method,
mirroring the `Observability` port's non-blocking contract. Expensive work
MUST be enqueued and handed off to background processing by the implementor.

#### Scenario: Span start/end calls do not block the caller
- GIVEN a `Tracer` implementor with expensive span-export work to perform
- WHEN a span is started or ended on a request-critical path
- THEN the call returns without waiting on network I/O or export completion

### Requirement: NoopTracer Is a Zero-Effect Default

`ego-domain` MUST provide a `NoopTracer` implementation of `Tracer` that
performs no observable action for any method: starting a span returns an
inert handle, and ending a span (success or error) has no side effect.
`NoopTracer` MUST be the tracer used when no tracer implementor is wired,
mirroring `NoopObservability`'s role for `Observability`.

#### Scenario: NoopTracer produces no observable side effects
- GIVEN a `NoopTracer` instance
- WHEN spans are started and ended (with and without error) through it
- THEN no span data is exported, recorded, or otherwise observable outside
  the call

#### Scenario: No tracer wired defaults to NoopTracer
- GIVEN a runtime or component that has not been explicitly configured with
  a `Tracer` implementor
- WHEN it needs to start or end spans
- THEN it uses `NoopTracer`, producing zero behavioral effect

### Requirement: OTLP Adapter Exports Spans Over a Configurable Transport

The `infrastructure` crate MUST provide a `Tracer` implementor that exports
completed spans over OTLP. The wire transport (gRPC or HTTP) MUST be
selectable via configuration, not hardcoded, and MUST NOT change the
`Tracer` port's signature.

#### Scenario: OTLP adapter configured for gRPC export
- GIVEN the OTLP adapter is configured with a gRPC endpoint
- WHEN spans are started and ended through it
- THEN completed spans are exported to the collector over gRPC

#### Scenario: OTLP adapter configured for HTTP export
- GIVEN the OTLP adapter is configured with an HTTP endpoint
- WHEN spans are started and ended through it
- THEN completed spans are exported to the collector over HTTP

### Requirement: Span Attributes Crossing the Network Boundary Are Redacted

Span attributes produced by the OTLP adapter and sent over the network MUST
NOT contain unredacted tenant-identifying values or credential data,
following the existing redaction convention used elsewhere in the codebase
(`Display`/`Debug` split; recorded/exported form omits raw sensitive values).

#### Scenario: Exported span omits raw tenant identifier
- GIVEN a span whose context includes a tenant identifier
- WHEN the span is exported over OTLP
- THEN the exported attributes do not contain the raw, unredacted tenant
  identifier

#### Scenario: Exported span omits credential data
- GIVEN a span whose context could carry credential-bearing data (e.g. a
  token or secret)
- WHEN the span is exported over OTLP
- THEN no credential value appears in the exported attributes

### Requirement: Out of Scope for v1

The following are explicitly NOT covered by this capability in v1 and MUST
NOT be assumed present: OTLP-exported metrics, OTLP-exported logs,
actor/effect-runner tracing (`persistent-entity`, `ego-scheduler`
non-request/response origination), and spans for CORE-012A macro-guard
denials (guard denials short-circuit before span origination; "guard denied
→ no span" is an accepted, documented v1 limitation). The `Observability`
port and the CORE-012A denial-recording contract remain unchanged by this
capability.

#### Scenario: A macro-guard denial produces no span
- GIVEN an operation denied by `#[authorize]` or `#[tenant_scoped]` before
  reaching the interceptor chain
- WHEN the denial occurs
- THEN no span is started or ended for that invocation, and the existing
  `Observability` denial-recording contract (CORE-012A) still records the
  denial event exactly as before this change
