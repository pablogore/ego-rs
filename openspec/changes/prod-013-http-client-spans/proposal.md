# Proposal: PROD-013 — HTTP Client Spans

## Intent

`ego-rs` v1 distributed tracing makes outbound HTTP **propagation-only**: the
transport injects a `traceparent` header
(`crates/transport/src/propagation.rs:27-29`) and the reference call site builds
an `axum::http::Request<()>` decoupled from any real client while starting **no
span** (`examples/reference-app/src/outbound.rs:1-12` — "ADR-7 propagation-only,
ego-rs ships no real outbound HTTP client"). The OTLP adapter only ever sets
`SpanKind::Server` (`crates/infrastructure/src/tracing_otlp.rs:301`) and maps no
`http.*` attributes at all — `to_otel_attributes` emits only `tenant.present`
and `duration_ms` (`tracing_otlp.rs:209-217`). No outbound retry/backoff model
exists anywhere. The v1 spec deliberately forbids a client span today
(`openspec/specs/distributed-tracing/spec.md:140-149` — "outbound MUST NOT
create its own client span in v1"; reinforced by the out-of-scope requirement at
`:283-290`). PROD-013 defines the **instrumentation contract for outbound HTTP
requests**: a `SpanKind::Client` span carrying HTTP semantic-convention
attributes, status/error/duration, continued context propagation, an explicit
double-instrumentation rule, and an explicit retry-span model — decoupled from
any concrete HTTP client via a hexagonal port, with OpenTelemetry confined to
`infrastructure`.

## Scope

### In Scope
- A `SpanKind::Client` span wrapping each outbound HTTP request.
- HTTP semantic-convention attributes (method, url/server address, response
  status, error) named per an **explicitly pinned** OTel Semantic Conventions
  version derived from the pinned `0.32` OpenTelemetry crate stack.
- Span status reflecting success/error; recorded span duration.
- Continued W3C `traceparent` propagation, now parented by the client span.
- Honoring the completed inbound sampling decision established by PROD-012 — the
  client span MUST NOT make an independent sampling decision.
- An explicit double-instrumentation avoidance rule (exactly one client span per
  outbound request even when the underlying client/framework is itself
  instrumented).
- An explicit retry-span model (one client span per attempt, ADR-4).
- A hexagonal port abstracting outbound-HTTP instrumentation from any concrete
  client (reqwest/hyper/axum); `ego-domain` stays OpenTelemetry-free.

### Out of Scope (Non-Goals / Follow-ups)
- Shipping a real outbound HTTP client — `ego-rs` still ships none; this fixes
  the contract a future client adapter consumes.
- gRPC and messaging client spans (no gRPC client transport; in-process
  messaging has no wire-header model — already out of scope in v1).
- Configurable/ratio sampling and any change to the inbound sampling policy
  itself (owned by PROD-012).
- Retry/backoff **policy** (when/how many times to retry) — PROD-013 only fixes
  how each attempt is instrumented, not the retry strategy.
- Metrics (`http.client.request.duration` histogram) and OTLP-exported logs.

## Frozen Decisions (decided constraints, not open questions)

1. **Exactly one client span per outbound HTTP request.** An outbound request
   MUST be wrapped in exactly one `SpanKind::Client` span. When the underlying
   client/framework already emits its own OTel client span, the framework MUST
   NOT add a second (ADR-3).
2. **Domain stays OpenTelemetry-free.** The client-span contract, its neutral
   request/response value types, and the `SpanRole` concept live in `ego-domain`
   as vendor-neutral values; NO `opentelemetry` type and NO concrete HTTP client
   type (reqwest/hyper/axum) may appear in `crates/domain`. The
   `SpanRole → SpanKind` mapping and the semantic-convention **attribute naming**
   live ONLY in the `infrastructure` adapter (the sole `opentelemetry` consumer).
3. **Honor the inbound sampling decision (depends on PROD-012).** The client span
   MUST inherit the sampling decision already carried by `TraceContext`
   (PROD-012). It MUST NOT re-sample or override that decision.
4. **Semantic-convention version is pinned and stated explicitly.** The design
   MUST pin the exact OTel Semantic Conventions version (from the `0.32` crate
   stack) and name every `http.*`/`url.*`/`server.*`/`error.*` attribute per that
   version. Attribute names MUST NOT be hardcoded without stating the version
   they come from (ADR-2).

## Open Fork for DESIGN (do not resolve here)

The retry-span model: **(A)** one logical client span for the whole
retried operation, **(B)** one `SpanKind::Client` span per attempt, or **(C)**
both (a logical parent plus a child span per attempt). Design MUST decide
consciously and justify the OTel-conformance / cardinality / attributability
tradeoff (resolved in ADR-4).

## Capabilities

### New Capabilities
- None. This extends an existing capability.

### Modified Capabilities
- `distributed-tracing`: adds the outbound client-span instrumentation contract
  (client span, pinned-semconv attributes, status/error/duration, continued
  propagation, honored inbound sampling, double-instrumentation rule, retry
  model, hexagonal port). The v1 "outbound MUST NOT create its own client span"
  requirement and the "client/server transport spans out of scope" clause are
  superseded via explicit MODIFIED deltas rather than silent contradiction.

## Approach

Add a vendor-neutral `SpanRole { Server, Client }` and neutral
`OutboundRequestInfo` / `OutboundResponseInfo` value types to
`ego-domain::tracer`, plus an object-safe `OutboundHttpInstrumentation` port that
wraps an outbound call: start a client span, inject propagation from the current
`TraceContext`, record the response status/error and the elapsed duration, end
the span. The port takes the outbound call as an abstraction (a closure/future
producing the neutral response info), so no concrete HTTP client type enters the
contract. In `infrastructure`, extend the OTLP adapter to set
`SpanKind::Client`, to map the neutral fields to the pinned-version semantic
convention attribute keys, and to honor `TraceContext`'s PROD-012 sampling
decision. The double-instrumentation rule is a contract property: span creation
is opt-in per client binding, and a client declared self-instrumented disables
the framework's client-span decorator. Retries are one client span per attempt
(ADR-4), each tagged with the resend-count attribute.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/domain/src/tracer.rs` (or new `tracer/` submodule) | New/Modified | `SpanRole`, `OutboundRequestInfo`, `OutboundResponseInfo`, `OutboundHttpInstrumentation` port; neutral, OTel-free |
| `crates/infrastructure/src/tracing_otlp.rs` | Modified | Set `SpanKind::Client` for client spans; map neutral fields to pinned-semconv attribute keys; honor PROD-012 sampling decision |
| `crates/transport/src/propagation.rs` | Modified | Client-span-aware outbound helper: still injects `traceparent`, now parented by the client span |
| `examples/reference-app/src/outbound.rs` | Modified (future) | Representative call site now goes through the instrumentation port (one client span) instead of propagation-only |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| OTel/semconv leaks into `ego-domain` via attribute names or `SpanKind` | Med | Domain carries neutral fields only; semconv naming + `SpanRole→SpanKind` confined to infra; grep gate in tasks |
| Double instrumentation (two client spans per request) | Med | Frozen decision 1 + ADR-3 opt-in rule; contract test asserts exactly one client span |
| Hardcoding attribute names without a version → silent drift | High (by design guard) | ADR-2 pins the semconv version explicitly; apply-time task verifies the crate's `SCHEMA_URL` |
| Retry model explodes span cardinality or hides per-attempt failures | Med | ADR-4 fixes one span per attempt with a resend-count attribute; no unbounded fan-out |
| Client span ignores/overrides the inbound sampling decision | Med | Frozen decision 3 depends on PROD-012; client span inherits `TraceContext` decision, never re-samples |
| Concrete HTTP client type creeps into the domain contract | Low | Hexagonal port takes the call as a neutral abstraction; no reqwest/hyper/axum type in domain |

## Rollback Plan

Additive at the contract layer; `ego-rs` still ships no real outbound client, so
no live traffic depends on it. To revert: drop `SpanRole` /
`OutboundHttpInstrumentation` / the neutral outbound value types, restore
`SpanKind::Server`-only export and the propagation-only outbound helper, and
return `examples/reference-app/src/outbound.rs` to its propagation-only form. No
schema/migration impact; the span lifecycle and inbound sampling policy
(PROD-012) are untouched, so rollback is behavior-neutral except for removing the
client span.

## Dependencies

- **DEPENDS ON PROD-012 (Inbound Sampling Propagation)** — MUST land after it. A
  client span MUST honor the completed inbound sampling decision that PROD-012
  makes `TraceContext` carry; without PROD-012 the client span has no faithful
  decision to inherit. PROD-012's own proposal names PROD-013 as its dependent.
- Builds on PROD-003 distributed tracing (`Tracer` port, `TraceContext`,
  `to_traceparent`, the OTLP adapter). No dedicated open issue — this is a
  PROD-003 follow-up in the outbound-tracing line.

## Success Criteria

- [ ] Each outbound HTTP request is wrapped in exactly one `SpanKind::Client`
      span; a self-instrumented underlying client adds no second span.
- [ ] The client span carries HTTP semantic-convention attributes named per an
      explicitly pinned OTel Semantic Conventions version (stated in the design),
      not hardcoded without a version.
- [ ] The client span's status reflects success/error and its duration is
      recorded.
- [ ] `traceparent` propagation continues, now parented by the client span, and
      the client span honors the PROD-012 inbound sampling decision (no
      re-sampling).
- [ ] Retries produce one client span per attempt, each tagged with a
      resend-count attribute (ADR-4).
- [ ] The instrumentation contract is decoupled from any concrete HTTP client;
      `crates/domain` carries zero `opentelemetry` and zero reqwest/hyper/axum
      types. `cargo test --workspace` green.
