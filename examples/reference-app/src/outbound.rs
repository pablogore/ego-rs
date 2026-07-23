//! PROD-003 (ADR-7) — a minimal, representative outbound HTTP call-site
//! builder.
//!
//! `ego-rs` ships no real outbound HTTP client (`providers/pricing_lookup.rs`
//! is deliberately in-memory-only dogfood — never a real HTTP/gRPC/DB call),
//! so this is not wired to any real network call either. It exists only to
//! prove, at a concrete reference-app call site, how a real outbound HTTP
//! call would apply `ego-transport`'s propagation helper
//! (`ego_transport::propagation::traceparent_header`): given an explicit
//! `ServiceContext`, inject its `traceparent` header. No span is started
//! here — the transport remains propagation-only (ADR-7); the span stays
//! owned by the request-boundary interceptor.

use axum::http::Request;
use ego_service_sdk::context::ServiceContext;
use ego_transport::propagation::{traceparent_header, TRACEPARENT_HEADER};

/// Builds a representative outbound `GET` request, injecting the
/// `traceparent` header from `ctx` when a `TraceContext` is attached.
/// Obtains the trace-context EXPLICITLY from `ctx` (no ambient lookup) and
/// starts no span.
///
/// Returns `Err` when `uri` (caller-supplied) is not a valid request URI, or
/// in the (practically unreachable) case `to_traceparent()`'s hex output is
/// somehow not a valid header value — axum's `Builder::header` accepts a
/// plain `String` and carries either failure through to `.body()`'s
/// `Result` rather than panicking.
pub fn build_outbound_request(ctx: &ServiceContext, uri: &str) -> Result<Request<()>, axum::http::Error> {
    let mut builder = Request::get(uri);
    if let Some(header_value) = traceparent_header(ctx) {
        builder = builder.header(TRACEPARENT_HEADER, header_value);
    }
    builder.body(())
}
