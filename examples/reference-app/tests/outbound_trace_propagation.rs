//! PROD-003 Phase 5 (TASK-017/018) — outbound HTTP `traceparent`
//! propagation at a reference-app call site (distributed-tracing spec:
//! "Outbound HTTP Propagation Injects TraceContext Without Creating A
//! Span").
//!
//! `ego-rs` ships no real outbound HTTP client (design.md ADR-7 —
//! `PricingLookupProvider` is deliberately in-memory-only, see
//! `providers/pricing_lookup.rs`), so this exercises a small, minimal,
//! representative outbound-request builder (`reference_app::outbound`)
//! that applies `ego-transport`'s propagation helper — proving the wiring
//! a real outbound call site would use, without inventing a fake network
//! call.

use ego_domain::TraceContext;
use ego_service_sdk::context::ServiceContext;
use reference_app::outbound::build_outbound_request;

#[test]
fn outbound_request_carries_the_traceparent_header_from_the_context() {
    let tc = TraceContext::root();
    let ctx = ServiceContext::new().with_trace_context(tc);

    let request = build_outbound_request(&ctx, "https://example.invalid/pricing");

    assert_eq!(
        request.headers().get("traceparent").and_then(|v| v.to_str().ok()),
        Some(tc.to_traceparent()).as_deref(),
    );
}

#[test]
fn outbound_request_has_no_traceparent_header_when_context_has_no_trace_context() {
    let ctx = ServiceContext::new();

    let request = build_outbound_request(&ctx, "https://example.invalid/pricing");

    assert!(request.headers().get("traceparent").is_none());
}
