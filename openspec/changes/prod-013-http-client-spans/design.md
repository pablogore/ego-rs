# Design: PROD-013 — HTTP Client Spans

## Technical Approach

`ego-domain` owns the *contract and neutral values*; `infrastructure` owns the
*OpenTelemetry mapping*; `transport` owns the *outbound call site wiring*. The
domain gains a vendor-neutral `SpanRole { Server, Client }`, neutral
`OutboundRequestInfo` / `OutboundResponseInfo` value types, and an object-safe
`OutboundHttpInstrumentation` port that wraps an outbound call: start a client
span, inject `traceparent` from the current `TraceContext`, record the
response/error and the elapsed duration, end the span. The port receives the
outbound call as a **neutral abstraction** (a future producing an
`OutboundResponseInfo`), so no concrete client type (reqwest/hyper/axum) ever
enters the contract. The `infrastructure` OTLP adapter — today
`SpanKind::Server`-only (`crates/infrastructure/src/tracing_otlp.rs:301`) and
attribute-poor (`to_otel_attributes` maps only `tenant.present` + `duration_ms`,
`tracing_otlp.rs:209-217`) — is extended to emit `SpanKind::Client` and to map
the neutral fields to the pinned-version semantic-convention attribute keys. The
client span honors the sampling decision `TraceContext` carries after PROD-012
(it re-samples nothing). Layering stays acyclic and matches the existing
hexagonal boundary: `domain` (ports + neutral values) ← `infrastructure` (sole
`opentelemetry` consumer) and `transport` (outbound call site).

## Architecture Decisions

### ADR-1 (DECISION 1): Client-span contract is a hexagonal port → **object-safe `OutboundHttpInstrumentation` in `ego-domain`, neutral of any concrete client**

**Choice**: a domain port that wraps an outbound call given as a neutral future,
mirroring the existing `#[async_trait]` object-safe ports already in
`ego-domain`. Domain carries `SpanRole`, `OutboundRequestInfo`,
`OutboundResponseInfo` as plain values; the OTLP client-span emission lives in
`infrastructure`.
**Rejected**: (a) taking a concrete `reqwest::Request`/`http::Request` in the
contract — leaks a client into the domain and couples the framework to one HTTP
stack; (b) a `tower::Layer`/middleware-typed contract in domain — pulls a
transport/middleware type into the vendor-neutral core.

| Option | Tradeoff | Verdict |
|---|---|---|
| Neutral port + neutral value types in domain | Matches the existing hexagonal boundary — `Tracer`/`SpanAttributes` are already vendor-neutral in `ego-domain`, OTel confined to infra. The concrete client stays behind the port; domain never names reqwest/hyper/axum. | **Chosen** |
| Concrete `http::Request`/reqwest in the contract | Couples the domain to one HTTP stack; contradicts frozen decision 2 and the existing `Tracer` port precedent. | Rejected |
| `tower::Layer` middleware contract in domain | Middleware is a transport concern; belongs in an adapter, not the vendor-neutral core. | Rejected |

The concrete client is wired in an adapter (transport/infra); the domain
contract only knows "an outbound call producing an `OutboundResponseInfo`".

### ADR-2 (DECISION 4): Pinned OTel Semantic Conventions version → **semconv `v1.37.0`, via `opentelemetry-semantic-conventions` 0.32 (aligned to the 0.32 stack); HTTP client attributes stable since semconv v1.23.0**

**Choice**: pin attribute naming to the OTel Semantic Conventions release
**vendored by `opentelemetry-semantic-conventions` 0.32**, the version aligned
with the already-pinned `opentelemetry`/`opentelemetry_sdk`/`opentelemetry-otlp`
`0.32` stack (`crates/infrastructure/Cargo.toml`). That crate release vendors
OTel Semantic Conventions **v1.37.0**; the HTTP **client** span attribute set
below has been *stable* since semconv **v1.23.0** (the release that stabilized
HTTP semantic conventions and deprecated the old `http.method`/`http.url`/
`http.status_code` names). Attribute keys are consumed from the crate's typed
constants, **not** hand-typed string literals, and the exact vendored version is
re-verified at apply time against the crate's `SCHEMA_URL` constant (TASK below)
so the number is sourced, never guessed.

**Rejected**: hardcoding attribute names with no stated version (the exact guard
frozen decision 4 forbids — it invites silent drift when the crate bumps); using
the deprecated pre-1.23.0 names (`http.method`, `http.url`, `http.status_code`),
which the stable conventions removed.

**Pinned HTTP client attribute set (stable, semconv v1.23.0+; vendored by
`opentelemetry-semantic-conventions` 0.32 = semconv v1.37.0)** — the design names
attributes ONLY from this set:

| Neutral domain field | Semconv attribute key | Type | When |
|---|---|---|---|
| method | `http.request.method` | string | always |
| url | `url.full` | string | always (redaction-safe; no credentials/secret query) |
| server address | `server.address` | string | always |
| server port | `server.port` | int | when known |
| response status | `http.response.status_code` | int | on response |
| error kind | `error.type` | string | on error/unset status |
| resend index | `http.request.resend_count` | int | retries only, when > 0 (ADR-4) |

Span name for a client span is `{http.request.method}` (semconv HTTP client
naming), NOT a hand-written literal. The deprecated `http.method` / `http.url` /
`http.status_code` keys MUST NOT be emitted.

### ADR-3 (DECISION 1): Double-instrumentation avoidance → **span creation is opt-in per client binding; a self-instrumented client disables the framework decorator**

**Choice**: exactly one `SpanKind::Client` span per outbound request. Client-span
creation is **opt-in per client binding**: the framework creates the client span
only for clients bound as *not self-instrumented*. A client wired as
*self-instrumented* (its own OTel middleware/layer already emits a client span —
e.g. an instrumented `reqwest` middleware or `tower-http` trace layer) is bound
WITHOUT the framework's client-span decorator, so only that client's span exists.
The rule is expressed on the port: an implementation declares whether it emits
its own client span (`emits_own_client_span()`), and the framework decorator is a
structural no-op when it does — it still injects propagation but starts no
second span.
**Rejected**: always creating a framework client span (double spans when the
client is instrumented); span de-duplication after the fact by inspecting the
active span (fragile, ambient-state-dependent, and forbidden by the
"no ambient lookup" propagation precedent in
`crates/transport/src/propagation.rs:1-9`).

**Rationale**: making creation opt-in at bind time is deterministic and needs no
ambient inspection — the wiring decides once which layer owns the span, matching
the existing "obtain context explicitly, never ambiently" transport rule.

### ADR-4 (OPEN FORK): Retry-span model → **one `SpanKind::Client` span per attempt (Option B)**

**Choice**: each outbound **attempt** (initial send + each resend) gets its own
`SpanKind::Client` span, tagged with `http.request.resend_count` (0 for the first
attempt is omitted; each resend carries its index). The parent of each attempt
span is the ambient request span (the interceptor's server span / the operation
span), so attempts nest correctly without inventing a synthetic wrapper.
**Rejected**: (A) one logical span for the whole retried operation; (C) both a
logical parent plus a child per attempt.

| Option | Tradeoff | Verdict |
|---|---|---|
| B — one client span per attempt | Directly matches OTel HTTP semantic conventions: each resend is a distinct HTTP request and the conventions define `http.request.resend_count` precisely for this. Every attempt gets its own status, duration, and `error.type`, so a failing attempt is individually attributable. No synthetic wrapper span is invented (ego-rs has no retry machinery today — nothing to hang a logical span on). | **Chosen** |
| A — one logical span for the whole operation | A single span cannot carry per-attempt status/duration/error; a mid-retry failure is lost. Contradicts the semconv resend model; forces inventing a non-standard "attempts" attribute. | Rejected |
| C — both (logical parent + child per attempt) | Richest, but doubles span volume and requires a logical wrapper span the framework does not otherwise create; the ambient request span already serves as the parent, so the extra wrapper is redundant cardinality. | Rejected |

**Verdict**: Option B. One client span per attempt with
`http.request.resend_count`; the ambient request span is the parent. If a future
change wants an operation-level aggregate it can add it as metrics, not an extra
span.

### ADR-5: Client span honors the completed inbound sampling decision (depends on PROD-012)

The client span MUST inherit the sampling decision `TraceContext` carries after
PROD-012 (`trace_flags`), and MUST NOT re-sample or override it. Because
`to_traceparent()` already serializes the real decision after PROD-012, the
injected outbound header and the exported client span both reflect that decision
automatically — PROD-013 adds no sampler and makes no independent decision. This
is why PROD-013 MUST land after PROD-012: before it, `TraceContext` carries no
faithful decision to inherit (v1 hardcodes `-01`), so a client span would export
a fabricated sampled bit. The `SpanRole → SpanKind` and decision → `TraceFlags`
mapping both live in `infrastructure`, never in domain.

### ADR-6: `SpanRole` is a domain-neutral concept mapped to `SpanKind` in infra

`SpanKind` is an OpenTelemetry type and MUST NOT appear in `ego-domain`. The
domain carries a neutral `SpanRole { Server, Client }`; the `infrastructure`
adapter maps `SpanRole::Server → OtelSpanKind::Server` and `SpanRole::Client →
OtelSpanKind::Client` when building `SpanData`. This preserves the existing
boundary where `crates/infrastructure/src/tracing_otlp.rs` is the sole
`opentelemetry` consumer, and keeps the client-span concept expressible without
leaking a vendor enum into the vendor-neutral core.

## Data Flow

    Outbound call site (transport adapter)
      │  OutboundHttpInstrumentation::instrument(ctx, req_info, call)
      ├─ start client span  (SpanRole::Client; parent = ctx request span; honors ctx sampling decision — PROD-012)
      ├─ inject traceparent  (from ctx.trace_context().to_traceparent(); parented by the client span)
      ├─ await call ──▶ concrete client (reqwest/hyper/axum, behind the port)  ─▶ OutboundResponseInfo
      ├─ record status (Ok / Error{error.type}), http.response.status_code, elapsed duration
      └─ end client span
                                   │
    infrastructure (OTLP adapter)  ▼
      SpanRole::Client ─▶ OtelSpanKind::Client
      neutral fields   ─▶ semconv keys (http.request.method, url.full, server.address, server.port,
                          http.response.status_code, error.type, http.request.resend_count)  [pinned v1.37.0]
      ctx.trace_flags  ─▶ TraceFlags (honored, not re-sampled)

### Sequence: instrumented outbound request with one retry

    CallSite   Instrumentation      Client(attempt 0)   Client(attempt 1)
      │─instrument(ctx,req)─▶│
      │                      ├─start client span (resend_count omitted, parent=ctx span, sampling=ctx)
      │                      ├─inject traceparent ─▶│ (fails / 503)
      │                      │◀── OutboundResponseInfo{ error.type } ──│
      │                      ├─record status=Error, end span (attempt 0)
      │                      ├─start client span (http.request.resend_count=1, parent=ctx span)
      │                      ├─inject traceparent ─────────────────────▶│ (200)
      │                      │◀── OutboundResponseInfo{ status_code=200 } ─│
      │                      ├─record status=Ok, http.response.status_code=200, end span (attempt 1)
      │◀── result ───────────┤
    (retry POLICY — when/how many — is out of scope; only per-attempt instrumentation is fixed)

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/tracer.rs` (or new `tracer/outbound.rs`) | Modify/Create | `SpanRole { Server, Client }`; `OutboundRequestInfo { method, url, server_address, server_port }`; `OutboundResponseInfo { status_code, error_type, resend_count }`; object-safe `OutboundHttpInstrumentation` port with `emits_own_client_span()` — neutral, OTel-free, client-free |
| `crates/domain/src/lib.rs` | Modify | Re-export the new outbound types + port |
| `crates/infrastructure/src/tracing_otlp.rs` | Modify | Set `OtelSpanKind::Client` for client spans (was `Server`-only, `:301`); extend attribute mapping (was `tenant.present`+`duration_ms` only, `:209-217`) to the pinned-semconv HTTP client keys via crate constants; honor `TraceContext` sampling decision (PROD-012) |
| `crates/infrastructure/Cargo.toml` | Modify | Add `opentelemetry-semantic-conventions = "0.32"` (aligned to the 0.32 stack) for typed attribute-key constants |
| `crates/transport/src/propagation.rs` | Modify | Client-span-aware outbound helper: still injects `traceparent`, now parented by the client span; still obtains `TraceContext` explicitly (no ambient lookup) |
| `examples/reference-app/src/outbound.rs` | Modify | Representative call site goes through `OutboundHttpInstrumentation` (one client span) instead of propagation-only |

(All production files above are FUTURE work planned by this change, not edited by
this planning artifact.)

## Interfaces / Contracts

```rust
// ego-domain::tracer — zero opentelemetry, zero concrete-HTTP-client deps
pub enum SpanRole { Server, Client } // maps to OTel SpanKind ONLY in infrastructure

/// Neutral, redaction-safe description of an outbound HTTP request.
/// `url` MUST be redaction-safe (no credentials / secret query values).
pub struct OutboundRequestInfo {
    pub method: String,           // -> http.request.method (infra)
    pub url: String,              // -> url.full (infra)
    pub server_address: String,   // -> server.address (infra)
    pub server_port: Option<u16>, // -> server.port (infra)
    pub resend_count: u32,        // -> http.request.resend_count when > 0 (ADR-4)
}

/// Neutral outcome of an outbound HTTP request.
pub struct OutboundResponseInfo {
    pub status_code: Option<u16>, // -> http.response.status_code (infra)
    pub error_type: Option<String>, // -> error.type (infra); redaction-safe kind, not a message
}

#[async_trait]
pub trait OutboundHttpInstrumentation: Send + Sync { // object-safe; client-agnostic
    /// True when the underlying client already emits its own OTel client span;
    /// the framework decorator then starts NO second span (ADR-3), still
    /// injecting propagation. Default false.
    fn emits_own_client_span(&self) -> bool { false }

    /// Wrap one outbound attempt: start a client span (honoring the ctx
    /// sampling decision — PROD-012), inject propagation, run `call`, record
    /// status/error/duration from the returned info, end the span. The
    /// concrete client is hidden inside `call` — no client type enters here.
    async fn instrument(
        &self,
        ctx: &TraceContext,
        request: OutboundRequestInfo,
        call: BoxFuture<'_, OutboundResponseInfo>,
    ) -> OutboundResponseInfo;
}

// infrastructure (sole opentelemetry consumer): SpanRole::Client -> OtelSpanKind::Client;
// neutral fields -> pinned-semconv keys via opentelemetry-semantic-conventions 0.32 constants;
// TraceContext sampling decision honored (never re-sampled).
```

## Error Model

`OutboundResponseInfo.error_type` carries a redaction-safe error **kind** (never
a backend message or payload), mapped to the semconv `error.type` attribute. A
call that fails (transport error, or an HTTP response with an error/unset status)
ends the client span with error status; the neutral `error_type` is the only
failure detail crossing the boundary, consistent with the existing
redaction-safe `SpanOutcome::Error { status_message }` contract. No raw URL
credentials, headers, or payloads are recorded.

## Observability

The client span itself is the observable output: `SpanKind::Client`, named
`{http.request.method}`, carrying the pinned-semconv HTTP attributes, an
Ok/Error status, a recorded duration, and — for retries — one span per attempt
with `http.request.resend_count`. Propagation continues via the injected
`traceparent`, now parented by the client span. No metrics are added here
(follow-up).

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | `SpanRole`, `OutboundRequestInfo`/`OutboundResponseInfo` are OTel-free and client-free; `OutboundHttpInstrumentation` is object-safe (`Arc<dyn _>` from a local stub) | domain tests |
| Unit | `error_type` is a closed redaction-safe kind (no free-text message field leaks) | domain tests |
| Integration | Instrumenting an outbound call produces exactly ONE `SpanKind::Client` span with the pinned-semconv attribute keys and a recorded duration | `infrastructure` test w/ in-memory OTLP exporter |
| Integration | `emits_own_client_span() == true` ⇒ framework starts NO second span, still injects `traceparent` (double-instrumentation avoided) | `infrastructure`/transport test |
| Integration | Client span honors `TraceContext` sampling decision (PROD-012): not-sampled inbound ⇒ client span reflects not-sampled; no independent re-sampling | `infrastructure` test (depends on PROD-012) |
| Integration | A retried call produces one client span per attempt; the resend carries `http.request.resend_count = 1` | `infrastructure`/transport test |
| Unit | Semconv version is sourced from `opentelemetry-semantic-conventions` constants, deprecated `http.method`/`http.url`/`http.status_code` never emitted; `SCHEMA_URL` matches the pinned version | `infrastructure` test |
| Grep | `crates/domain` names no `opentelemetry` and no reqwest/hyper/axum type | tasks gate |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, or executable-file
classification. The one data-exposure concern (leaking credentials/payloads via
the recorded URL or error) is structurally bounded: `url.full` is required
redaction-safe and `error.type` is a closed kind, not a free-text message —
consistent with the existing redaction-safe span contract.

## Migration / Rollout / Compatibility

Additive at the contract layer; `ego-rs` ships no real outbound client, so no
live traffic changes. The v1 spec's "outbound MUST NOT create its own client
span" and "client/server transport spans out of scope" clauses are superseded via
explicit MODIFIED deltas (not silent contradiction). Existing inbound propagation
(`crates/transport/src/propagation.rs`) is preserved — the header is still
injected explicitly — and now also carries the client span's parentage.
PROD-013 MUST be applied AFTER PROD-012; before it, `TraceContext` carries no
faithful sampling decision to honor. Rollback = drop the outbound types/port,
restore `SpanKind::Server`-only export and the propagation-only helper.

## Open Questions

None blocking. The exact vendored semconv spec version (v1.37.0) is re-verified
at apply time against `opentelemetry-semantic-conventions` 0.32's `SCHEMA_URL`
constant (a TASK), and attribute keys are taken from the crate's typed constants
rather than hand-typed literals — so a version bump surfaces as a compile/const
check, never silent string drift.
