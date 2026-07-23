//! HTTP `traceparent` propagation (ADR-7): the outbound injection helper and
//! the paired inbound origination helper used at HTTP ingress
//! (service-sdk spec: "Trace-Context Originates At HTTP Ingress").
//!
//! Both directions obtain their `TraceContext` EXPLICITLY — never via
//! ambient/task-local lookup — and neither starts a span. The span stays
//! owned by the request-boundary interceptor (ADR-7); this module is
//! propagation-only.

use std::convert::Infallible;

use async_trait::async_trait;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use ego_domain::TraceContext;
use ego_service_sdk::context::ServiceContext;

use crate::state::AppState;

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

/// The `TraceContext` originated for this request, extracted before the
/// handler runs (service-sdk spec: "Trace-Context Originates At HTTP
/// Ingress"). Mirrors [`crate::security::AuthenticatedContext`]: origination
/// lives at the transport boundary, not hand-repeated in every handler that
/// needs it.
pub struct TraceContextExtractor(pub TraceContext);

/// Generic over any axum state `S` an `AppState` can be extracted from via
/// `FromRef` (same bound as `AuthenticatedContext`), even though this
/// extractor does not read `AppState` today — it keeps every ingress
/// extractor uniformly usable on any route regardless of its concrete state
/// type.
#[async_trait]
impl<S> FromRequestParts<S> for TraceContextExtractor
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    // Infallible: a missing or malformed inbound `traceparent` falls back to
    // `TraceContext::root()` inside `originate_trace_context` — this
    // extractor never rejects a request.
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(TraceContextExtractor(originate_trace_context(&parts.headers)))
    }
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

    // G1 review fix (RED): `TraceContextExtractor` mirrors
    // `AuthenticatedContext` (security.rs) — a `FromRequestParts` extractor
    // so ingress origination happens at the transport boundary, not
    // hand-repeated in every handler. Valid inbound traceparent -> parent
    // linkage.
    use ego_domain::auth::AuthenticationError;
    use ego_security_sdk::{AuthenticationProvider, Credential, SecurityContext};
    use ego_service_sdk::runtime::RuntimeBuilder;

    struct StubAuthn;

    impl AuthenticationProvider for StubAuthn {
        fn authenticate(&self, _credential: &Credential) -> Result<SecurityContext, AuthenticationError> {
            unimplemented!("not exercised by these tests")
        }
    }

    fn make_state() -> AppState {
        AppState::new(RuntimeBuilder::new().build().resolver(), std::sync::Arc::new(StubAuthn))
    }

    fn parts_with_traceparent(value: Option<&str>) -> Parts {
        let mut builder = axum::http::Request::builder().method("POST").uri("/register");
        if let Some(v) = value {
            builder = builder.header(TRACEPARENT_HEADER, v);
        }
        let (parts, ()) = builder.body(()).unwrap().into_parts();
        parts
    }

    #[tokio::test]
    async fn extractor_continues_a_valid_inbound_traceparent() {
        let remote = TraceContext::root();
        let header_value = remote.to_traceparent();
        let mut parts = parts_with_traceparent(Some(&header_value));
        let state = make_state();

        let TraceContextExtractor(originated) =
            TraceContextExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("infallible");

        assert_eq!(originated.trace_id(), remote.trace_id());
        assert_eq!(originated.parent_span_id(), Some(remote.span_id()));
        assert_ne!(originated.span_id(), remote.span_id());
    }

    #[tokio::test]
    async fn extractor_starts_a_root_trace_when_header_is_absent() {
        let mut parts = parts_with_traceparent(None);
        let state = make_state();

        let TraceContextExtractor(originated) =
            TraceContextExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("infallible");

        assert_eq!(originated.parent_span_id(), None);
    }

    #[tokio::test]
    async fn extractor_falls_back_to_root_on_malformed_header_and_never_errors() {
        let mut parts = parts_with_traceparent(Some("not-a-traceparent"));
        let state = make_state();

        let result = TraceContextExtractor::from_request_parts(&mut parts, &state).await;

        let TraceContextExtractor(originated) = result.expect("infallible — never rejects");
        assert_eq!(originated.parent_span_id(), None);
    }
}
