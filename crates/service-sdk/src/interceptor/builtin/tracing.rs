//! `TracingInterceptor` — drives the domain `Tracer` span lifecycle from the
//! explicit `TraceContext` carried on `ServiceContext` (PROD-003).
//!
//! Exactly one span is owned per request boundary: `on_request` starts it,
//! `on_response`/`on_error` end it. `start_span` returns nothing (ADR-3): the
//! authoritative span identity is `ctx.trace_context().span_id()`. Nothing is
//! stored between the separate `on_request`/`on_response`/`on_error` calls;
//! each re-derives the same `SpanId` from `&ctx`, and `end_span` is called
//! with that id. `TraceContext::child()` is never invoked here (v1 has no
//! manual/nested spans).

use std::sync::Arc;

use async_trait::async_trait;
use ego_domain::{SpanAttributes, SpanOutcome, Tracer};

use crate::context::ServiceContext;
use crate::error::{ServiceError, ServiceErrorTrait};
use crate::interceptor::chain::Interceptor;

/// The fixed span name used for the single request-boundary span (v1 has no
/// per-operation name threaded through the `Interceptor` call sites — see
/// `service-sdk-macros/src/lib.rs:478-486`).
const REQUEST_SPAN_NAME: &str = "request";

/// Built-in interceptor that owns exactly one span per request boundary,
/// driven entirely from the explicit `TraceContext` on `ServiceContext`
/// (PROD-003). Stateless: `on_request`/`on_response`/`on_error` each
/// re-derive the span identity from `&ctx` — nothing is stored locally or
/// ambiently between calls (ADR-1/ADR-3).
///
/// A `None` `trace_context()` (no trace originated for this request) makes
/// every hook a no-op.
pub struct TracingInterceptor {
    tracer: Arc<dyn Tracer>,
}

impl TracingInterceptor {
    /// Creates a new `TracingInterceptor` backed by the given `Tracer`.
    pub fn new(tracer: Arc<dyn Tracer>) -> Self {
        Self { tracer }
    }
}

#[async_trait]
impl Interceptor for TracingInterceptor {
    async fn on_request(&self, context: &ServiceContext) -> Result<(), ServiceError> {
        if let Some(trace_context) = context.trace_context() {
            let attrs = SpanAttributes::new(REQUEST_SPAN_NAME)
                .with_tenant_hint_present(context.has_tenant_hint());
            self.tracer
                .start_span(trace_context, REQUEST_SPAN_NAME, attrs);
        }
        Ok(())
    }

    async fn on_response(&self, context: &ServiceContext) -> Result<(), ServiceError> {
        if let Some(trace_context) = context.trace_context() {
            self.tracer
                .end_span(trace_context.span_id(), SpanOutcome::Ok);
        }
        Ok(())
    }

    async fn on_error(
        &self,
        context: &ServiceContext,
        error: &dyn ServiceErrorTrait,
    ) -> Result<(), ServiceError> {
        if let Some(trace_context) = context.trace_context() {
            // Redaction-safe by construction: `code()` is a fixed
            // machine-readable string drawn from `ServiceError`'s variants
            // (ADR-6 analogue) — never the free-form `message()`, which may
            // carry caller-supplied (and therefore sensitive) content.
            self.tracer.end_span(
                trace_context.span_id(),
                SpanOutcome::Error {
                    status_message: error.code().to_string(),
                },
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ego_domain::{SpanAttributes, SpanId, SpanOutcome, TraceContext, Tracer};

    use crate::context::ServiceContext;
    use crate::error::{ServiceError, ServiceErrorTrait};
    use crate::interceptor::Interceptor;

    use super::TracingInterceptor;

    // -----------------------------------------------------------------
    // Spy Tracer: records start/end calls for assertion.
    // -----------------------------------------------------------------

    struct SpyTracer {
        started: Mutex<Vec<SpanId>>,
        ended: Mutex<Vec<(SpanId, SpanOutcome)>>,
    }

    impl SpyTracer {
        fn new() -> Self {
            Self {
                started: Mutex::new(Vec::new()),
                ended: Mutex::new(Vec::new()),
            }
        }
    }

    impl Tracer for SpyTracer {
        fn start_span(&self, ctx: &TraceContext, _name: &str, _attrs: SpanAttributes) {
            self.started.lock().unwrap().push(ctx.span_id());
        }

        fn end_span(&self, span: SpanId, outcome: SpanOutcome) {
            self.ended.lock().unwrap().push((span, outcome));
        }
    }

    // -----------------------------------------------------------------
    // TASK-010/011 tests
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn on_request_starts_span_equal_to_context_span_id() {
        let tracer = std::sync::Arc::new(SpyTracer::new());
        let interceptor = TracingInterceptor::new(tracer.clone());

        let tc = TraceContext::root();
        let ctx = ServiceContext::new().with_trace_context(tc);

        interceptor.on_request(&ctx).await.unwrap();

        let started = tracer.started.lock().unwrap();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0], ctx.trace_context().unwrap().span_id());
    }

    #[tokio::test]
    async fn on_response_ends_span_with_ok_outcome() {
        let tracer = std::sync::Arc::new(SpyTracer::new());
        let interceptor = TracingInterceptor::new(tracer.clone());

        let tc = TraceContext::root();
        let ctx = ServiceContext::new().with_trace_context(tc);

        interceptor.on_request(&ctx).await.unwrap();
        interceptor.on_response(&ctx).await.unwrap();

        let ended = tracer.ended.lock().unwrap();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].0, ctx.trace_context().unwrap().span_id());
        assert_eq!(ended[0].1, SpanOutcome::Ok);
    }

    #[tokio::test]
    async fn on_error_ends_span_with_redaction_safe_status_message() {
        let tracer = std::sync::Arc::new(SpyTracer::new());
        let interceptor = TracingInterceptor::new(tracer.clone());

        let tc = TraceContext::root();
        let ctx = ServiceContext::new().with_trace_context(tc);
        let err = ServiceError::validation("tenant-id=acme-secret credential=shh");

        interceptor.on_request(&ctx).await.unwrap();
        interceptor
            .on_error(&ctx, &err as &dyn ServiceErrorTrait)
            .await
            .unwrap();

        let ended = tracer.ended.lock().unwrap();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].0, ctx.trace_context().unwrap().span_id());
        match &ended[0].1 {
            SpanOutcome::Error { status_message } => {
                assert!(!status_message.is_empty());
                // Redaction-safe: must never carry the raw error message's
                // sensitive markers, only the fixed machine-readable code.
                assert!(!status_message.contains("tenant-id"));
                assert!(!status_message.contains("credential"));
                assert!(!status_message.contains("secret"));
                assert_eq!(status_message, "VALIDATION");
            }
            SpanOutcome::Ok => panic!("expected Error outcome"),
        }
    }

    #[tokio::test]
    async fn on_response_then_on_error_race_both_call_end_span() {
        let tracer = std::sync::Arc::new(SpyTracer::new());
        let interceptor = TracingInterceptor::new(tracer.clone());

        let tc = TraceContext::root();
        let ctx = ServiceContext::new().with_trace_context(tc);
        let err = ServiceError::validation("bad input");

        interceptor.on_request(&ctx).await.unwrap();
        interceptor.on_response(&ctx).await.unwrap();
        interceptor
            .on_error(&ctx, &err as &dyn ServiceErrorTrait)
            .await
            .unwrap();

        // The interceptor is stateless and re-derives the SpanId from &ctx
        // each time — it calls end_span on both paths; idempotency (first
        // wins) is the adapter's contract (ADR-5), not asserted here.
        let ended = tracer.ended.lock().unwrap();
        assert_eq!(ended.len(), 2);
        assert_eq!(ended[0].0, ctx.trace_context().unwrap().span_id());
        assert_eq!(ended[1].0, ctx.trace_context().unwrap().span_id());
    }

    #[tokio::test]
    async fn no_trace_context_is_a_noop() {
        let tracer = std::sync::Arc::new(SpyTracer::new());
        let interceptor = TracingInterceptor::new(tracer.clone());

        let ctx = ServiceContext::new();
        let err = ServiceError::validation("bad input");

        interceptor.on_request(&ctx).await.unwrap();
        interceptor.on_response(&ctx).await.unwrap();
        interceptor
            .on_error(&ctx, &err as &dyn ServiceErrorTrait)
            .await
            .unwrap();

        assert!(tracer.started.lock().unwrap().is_empty());
        assert!(tracer.ended.lock().unwrap().is_empty());
    }
}
