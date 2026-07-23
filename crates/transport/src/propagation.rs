//! HTTP `traceparent` propagation (ADR-7): the outbound injection helper and
//! the paired inbound origination helper used at HTTP ingress
//! (service-sdk spec: "Trace-Context Originates At HTTP Ingress").
//!
//! Both directions obtain their `TraceContext` EXPLICITLY — never via
//! ambient/task-local lookup — and neither starts a span. The span stays
//! owned by the request-boundary interceptor (ADR-7); this module is
//! propagation-only.

use axum::http::HeaderMap;
use ego_domain::TraceContext;
use ego_service_sdk::context::ServiceContext;

/// The W3C `traceparent` header name.
pub const TRACEPARENT_HEADER: &str = "traceparent";

/// Build the outbound `traceparent` header value from an explicitly-passed
/// `ServiceContext` (ADR-7). Obtains the context's `TraceContext` EXPLICITLY
/// — no ambient/task-local lookup — and starts no span; the outbound call
/// remains propagation-only. Returns `None` when no `TraceContext` is
/// attached (nothing to propagate).
pub fn traceparent_header(ctx: &ServiceContext) -> Option<String> {
    ctx.trace_context().map(TraceContext::to_traceparent)
}

/// Originate the `TraceContext` for an inbound HTTP request (service-sdk
/// spec: "Trace-Context Originates At HTTP Ingress"). Continues an inbound
/// `traceparent` header when present and well-formed
/// (`TraceContext::from_inbound`); otherwise starts a fresh root trace
/// (`TraceContext::root`). A malformed header MUST NOT fail the request —
/// W3C treats an invalid `traceparent` as absent.
pub fn originate_trace_context(headers: &HeaderMap) -> TraceContext {
    headers
        .get(TRACEPARENT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| TraceContext::from_inbound(header).ok())
        .unwrap_or_else(TraceContext::root)
}

#[cfg(test)]
mod tests {
    use super::*;

    // TASK-015 (RED): builds the outbound `traceparent` header value from an
    // explicitly-passed `ServiceContext` — no ambient lookup, no span.
    #[test]
    fn traceparent_header_is_some_when_trace_context_is_set() {
        let tc = TraceContext::root();
        let ctx = ServiceContext::new().with_trace_context(tc);

        assert_eq!(traceparent_header(&ctx), Some(tc.to_traceparent()));
    }

    #[test]
    fn traceparent_header_is_none_when_no_trace_context_is_set() {
        let ctx = ServiceContext::new();

        assert_eq!(traceparent_header(&ctx), None);
    }

    // TASK-014a (RED): ingress origination — valid inbound traceparent
    // continues the trace (parent linkage).
    #[test]
    fn originate_trace_context_continues_a_valid_inbound_traceparent() {
        let remote = TraceContext::root();
        let header_value = remote.to_traceparent();
        let mut headers = HeaderMap::new();
        headers.insert(TRACEPARENT_HEADER, header_value.parse().unwrap());

        let originated = originate_trace_context(&headers);

        assert_eq!(originated.trace_id(), remote.trace_id());
        assert_eq!(originated.parent_span_id(), Some(remote.span_id()));
        assert_ne!(originated.span_id(), remote.span_id());
    }

    // TASK-014a (RED): no inbound traceparent header -> a fresh root trace.
    #[test]
    fn originate_trace_context_starts_a_root_trace_when_header_is_absent() {
        let headers = HeaderMap::new();

        let originated = originate_trace_context(&headers);

        assert_eq!(originated.parent_span_id(), None);
    }

    // TASK-014a (RED): a malformed traceparent MUST NOT fail the request —
    // it falls back to root(), per W3C (invalid traceparent treated as absent).
    #[test]
    fn originate_trace_context_falls_back_to_root_on_malformed_header() {
        let mut headers = HeaderMap::new();
        headers.insert(TRACEPARENT_HEADER, "not-a-traceparent".parse().unwrap());

        let originated = originate_trace_context(&headers);

        assert_eq!(originated.parent_span_id(), None);
    }
}
