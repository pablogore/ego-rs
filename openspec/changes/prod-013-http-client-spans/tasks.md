# Tasks: PROD-013 — HTTP Client Spans

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~450-600 (new domain outbound types + port, infra OTLP client-span mapping + semconv dep, transport helper change, reference call-site change, incl. tests) |
| 400-line budget risk | Med-High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 domain outbound contract (`SpanRole`, value types, port) → PR2 infra OTLP client-span + pinned semconv mapping + sampling honoring → PR3 transport helper + reference call site + double-instrumentation/retry integration |
| Delivery strategy | auto-forecast (no recognized ask-on-risk/auto-chain/single-pr/exception-ok label — treated conservatively) |
| Chain strategy | feature-branch-chain (PR1→PR2→PR3); only the tracker merged to develop |

Decision needed before apply: No (retry-model fork resolved in ADR-4 = Option B)
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain (PR1→PR2→PR3)
400-line budget risk: Med-High
Dependency gate: MUST land after PROD-012 (Inbound Sampling Propagation).

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 1 | `ego-domain` outbound contract: `SpanRole`, `OutboundRequestInfo`, `OutboundResponseInfo`, `OutboundHttpInstrumentation` port | PR1 | `cargo test -p ego-domain tracer::` | Revert `tracer.rs`/`lib.rs` additions |
| 2 | Infra OTLP client-span emission + pinned-semconv attribute mapping + honored sampling decision | PR2 | `cargo test -p ego-infrastructure tracing_otlp::` | Revert `tracing_otlp.rs` + `Cargo.toml` semconv dep |
| 3 | Transport helper (client-span-aware) + reference call site + double-instrumentation/retry integration | PR3 | `cargo test -p ego-transport propagation:: && cargo test -p ego-reference-app outbound::` | Revert `propagation.rs` + `outbound.rs` to propagation-only |

## Phase 1: Domain — Neutral Outbound Client-Span Contract

- [ ] TASK-001 RED: failing test in `crates/domain/src/tracer.rs` asserting `SpanRole { Server, Client }` exists as a vendor-neutral enum and that `OutboundHttpInstrumentation` is object-safe — build `Vec<Arc<dyn OutboundHttpInstrumentation>>` from a local trivial stub. Asserts the "wrapped in a client span" contract lives in domain (spec: "Outbound HTTP Request Is Wrapped In A Client Span").
- [ ] TASK-002 GREEN: implement `SpanRole { Server, Client }`, `OutboundRequestInfo { method, url, server_address, server_port, resend_count }`, `OutboundResponseInfo { status_code, error_type }`, and object-safe `#[async_trait] OutboundHttpInstrumentation` with `fn emits_own_client_span(&self) -> bool { false }` and `async fn instrument(&self, ctx: &TraceContext, request: OutboundRequestInfo, call: BoxFuture<'_, OutboundResponseInfo>) -> OutboundResponseInfo`. AC: TASK-001 green.
- [ ] TASK-003 RED: failing test proving the outbound contract is OTel-free AND client-free — a type/API-contract check that `OutboundResponseInfo.error_type` is a closed redaction-safe kind with NO free-text backend-message field, and that neither value type nor the port names any `opentelemetry` or reqwest/hyper/axum type (spec: "Client Span Carries HTTP Semantic-Convention Attributes...", redaction scenario; "Outbound Instrumentation Contract Is Decoupled...").
- [ ] TASK-004 GREEN: finalize the neutral value types so failure detail is only a closed `error_type` kind (no message field); confirm zero vendor/client symbols. AC: TASK-003 green.
- [ ] TASK-005: wire re-exports (`SpanRole`, `OutboundRequestInfo`, `OutboundResponseInfo`, `OutboundHttpInstrumentation`) in `crates/domain/src/lib.rs`. AC: `ego_domain::{...}` importable; `cargo build -p ego-domain` succeeds.

## Phase 2: Infrastructure — OTLP Client Span, Pinned Semconv, Honored Sampling

- [ ] TASK-006: add `opentelemetry-semantic-conventions = "0.32"` to `crates/infrastructure/Cargo.toml` (aligned to the pinned 0.32 opentelemetry stack). AC: `cargo build -p ego-infrastructure` succeeds with the new dependency resolved.
- [ ] TASK-007 RED: failing test in `crates/infrastructure/src/tracing_otlp.rs` (in-memory OTLP exporter) — a client span is exported with `OtelSpanKind::Client` (not `Server`) and carries the pinned-semconv keys `http.request.method`, `url.full`, `server.address`, `http.response.status_code` (and `server.port` when known), sourced from `opentelemetry_semantic_conventions` constants, with a recorded duration (spec: "Client Span Carries HTTP Semantic-Convention Attributes At A Pinned Version"; "Client Span Status Reflects Success Or Error And Records Duration").
- [ ] TASK-008 GREEN: map `SpanRole::Client → OtelSpanKind::Client` when building `SpanData` (extends the `Server`-only path at `tracing_otlp.rs:301`) and extend attribute mapping (beyond `tenant.present`/`duration_ms` at `tracing_otlp.rs:209-217`) to the pinned-semconv HTTP client keys via crate constants; record duration. AC: TASK-007 green.
- [ ] TASK-009 RED: failing test asserting the deprecated keys `http.method`, `http.url`, `http.status_code` are NEVER emitted, and that the pinned semconv version is sourced (assert `opentelemetry_semantic_conventions::SCHEMA_URL` matches the version pinned in design ADR-2 — v1.37.0 via the 0.32 crate), not hand-typed literals (spec: "Deprecated attribute keys are never emitted").
- [ ] TASK-010 GREEN: ensure all attribute keys come from `opentelemetry_semantic_conventions` constants and no deprecated key is emitted; assert the `SCHEMA_URL` constant equals the design-pinned version. AC: TASK-009 green.
- [ ] TASK-011 RED: failing test — an error outcome ends the client span with Error status and records `error.type` as a closed kind (no free-text backend message), still with a recorded duration (spec: "A failed outbound request records an error status"; "The recorded URL and error carry no sensitive free text").
- [ ] TASK-012 GREEN: map `OutboundResponseInfo.error_type → error.type` and error status; ensure `url.full` is recorded redaction-safe (no credentials/secret query). AC: TASK-011 green.
- [ ] TASK-013 RED: failing test (DEPENDS ON PROD-012) — a `TraceContext` carrying a not-sampled decision yields a client span and outbound `traceparent` that reflect not-sampled; the client span applies no sampler of its own (spec: "Client Span Honors The Completed Inbound Sampling Decision").
- [ ] TASK-014 GREEN: honor `TraceContext`'s PROD-012 sampling decision when building the client `SpanData` (no independent sampler); the decision flows from the domain value, never re-computed. AC: TASK-013 green. NOTE: requires PROD-012 merged first.

## Phase 3: Transport & Reference Call Site — One Span, Double-Instrumentation, Retries

- [ ] TASK-015 RED: failing test in `crates/transport/src/propagation.rs` — the client-span-aware outbound path still injects `traceparent` obtained EXPLICITLY from `ServiceContext` (no ambient lookup) AND starts exactly one `SpanRole::Client` span parented by the current request span; the injected header reflects the client span as parent (spec: MODIFIED "Outbound HTTP Propagation Injects TraceContext Without Creating A Span" — canonical heading retained for the archive merge; its body is what PROD-013 supersedes).
- [ ] TASK-016 GREEN: implement the client-span-aware outbound helper wiring `OutboundHttpInstrumentation` so it starts one client span and injects the client-span-parented `traceparent`, preserving explicit (non-ambient) context sourcing. AC: TASK-015 green.
- [ ] TASK-017 RED: failing test — a client bound as self-instrumented (`emits_own_client_span() == true`) causes the framework to start NO second span (exactly one client span total) while `traceparent` is still injected; a client bound as not self-instrumented gets exactly one framework client span (spec: "Double Instrumentation Is Avoided").
- [ ] TASK-018 GREEN: implement opt-in-per-binding client-span creation — the framework decorator is a structural no-op (propagation only, no span) when `emits_own_client_span()` is true. AC: TASK-017 green; no ambient active-span inspection used.
- [ ] TASK-019 RED: failing test — a retried outbound request produces one `SpanRole::Client` span per attempt; the resend attempt carries `http.request.resend_count = 1` (from `opentelemetry_semantic_conventions` constant), each with its own status/duration; a single-attempt request produces one span with no resend count (spec: "Retries Produce One Client Span Per Attempt"; ADR-4 Option B).
- [ ] TASK-020 GREEN: implement per-attempt client spans, tagging each resend with `http.request.resend_count` (omitted/0 on the first attempt); no merged multi-attempt span. AC: TASK-019 green.
- [ ] TASK-021 RED: failing test in `examples/reference-app/src/outbound.rs` — the representative call site goes through `OutboundHttpInstrumentation` and produces exactly one client span (replacing the propagation-only builder), still injecting `traceparent`.
- [ ] TASK-022 GREEN: rewire `build_outbound_request` (or its successor) to run through the instrumentation port (one client span) instead of propagation-only. AC: TASK-021 green; existing propagation behavior preserved.

## Phase 4: Cross-Cutting Guarantees & Verification

- [ ] TASK-023: grep-verify the domain outbound contract is vendor/client-free — no `opentelemetry` and no reqwest/hyper/axum symbol in `crates/domain/src/tracer.rs` (or the new outbound submodule). AC: grep clean (spec: "The domain instrumentation contract has no OTel or client symbols").
- [ ] TASK-024: confirm the semconv version is sourced, not hardcoded — grep confirms attribute keys come from `opentelemetry_semantic_conventions` constants and that no literal `"http.method"`/`"http.url"`/`"http.status_code"` string appears in `crates/infrastructure/src/tracing_otlp.rs`. AC: grep clean; `SCHEMA_URL` assertion (TASK-010) present.
- [ ] TASK-025: confirm the PROD-012 dependency is satisfied on the integration branch — `TraceContext` exposes the sampling decision the client span honors (the sampling tests TASK-013/014 are green against merged PROD-012). AC: PROD-012 merged; sampling-honoring tests pass.
- [ ] TASK-026: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions.
