//! F2 (PROD-003 PR5 review) — real OTLP wire round-trip.
//!
//! `OtlpConfig`'s `Grpc`/`Http` selection is proven end-to-end by standing up
//! a MINIMAL in-process collector per protocol, pointing a real `OtlpTracer`
//! at it, exporting one span, and asserting the RECEIVED span's
//! trace_id/span_id/parent_span_id equal the domain ids — not merely that
//! "something arrived". No sleeps: a channel plus a bounded timeout.

use std::time::Duration;

use ego_domain::{SpanAttributes, SpanOutcome, TraceContext, Tracer, TracerLifecycle};
use ego_infrastructure::{OtlpConfig, OtlpProtocol, OtlpTracer};
use opentelemetry_proto::tonic::collector::trace::v1::{
    trace_service_server::{TraceService, TraceServiceServer},
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use prost::Message;
use tokio::sync::mpsc;
use tonic::{Request, Response, Status};

// A known inbound `traceparent`: trace-id T and remote parent R are fixed;
// `from_inbound` mints a NEW local span S with parent = R — so all three ids
// are non-trivial to assert.
const INBOUND: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The single exported span's (trace_id, span_id, parent_span_id) as hex.
fn single_span_ids(req: &ExportTraceServiceRequest) -> (String, String, String) {
    let spans: Vec<_> = req
        .resource_spans
        .iter()
        .flat_map(|rs| rs.scope_spans.iter())
        .flat_map(|ss| ss.spans.iter())
        .collect();
    assert_eq!(
        spans.len(),
        1,
        "expected exactly one exported span, got {}",
        spans.len()
    );
    let s = spans[0];
    (hex(&s.trace_id), hex(&s.span_id), hex(&s.parent_span_id))
}

fn export_one_span(config: OtlpConfig) -> TraceContext {
    let tc = TraceContext::from_inbound(INBOUND).expect("valid inbound traceparent");
    let tracer = OtlpTracer::new(config).expect("OtlpTracer builds");
    tracer.start_span(&tc, "request", SpanAttributes::new());
    tracer.end_span(tc.span_id(), SpanOutcome::Ok);
    tracer.shutdown(); // force_flush pushes the batch over the wire
    tc
}

fn assert_ids_match(req: &ExportTraceServiceRequest, tc: &TraceContext) {
    let (trace_id, span_id, parent_span_id) = single_span_ids(req);
    assert_eq!(
        trace_id,
        tc.trace_id().to_hex(),
        "exported trace_id must equal the domain trace_id"
    );
    assert_eq!(
        span_id,
        tc.span_id().to_hex(),
        "exported span_id must equal the domain span_id"
    );
    assert_eq!(
        parent_span_id,
        tc.parent_span_id()
            .expect("from_inbound sets a parent")
            .to_hex(),
        "exported parent_span_id must equal the inbound remote span"
    );
}

// ---------------------------------------------------------------------------
// gRPC
// ---------------------------------------------------------------------------

struct GrpcStub {
    tx: mpsc::Sender<ExportTraceServiceRequest>,
}

#[tonic::async_trait]
impl TraceService for GrpcStub {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let _ = self.tx.send(request.into_inner()).await;
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

#[tokio::test]
async fn otlp_grpc_export_preserves_domain_ids_over_the_wire() {
    let (tx, mut rx) = mpsc::channel(4);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // Pre-bound listener → the server is listening before the exporter connects
    // (deterministic, no sleep/race).
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(TraceServiceServer::new(GrpcStub { tx }))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    let tc = export_one_span(OtlpConfig {
        endpoint: format!("http://{addr}"),
        protocol: OtlpProtocol::Grpc,
        max_in_flight_spans: 128,
    });

    let req = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("collector received an export within the timeout")
        .expect("stub channel stayed open");
    assert_ids_match(&req, &tc);
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

#[tokio::test]
async fn otlp_http_export_preserves_domain_ids_over_the_wire() {
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::Uri;
    use axum::Router;

    let (tx, mut rx) = mpsc::channel::<ExportTraceServiceRequest>(4);
    // Capture ANY request the exporter makes (robust to the exact OTLP/HTTP
    // path), decode the protobuf body, and forward it.
    let app =
        Router::new()
            .fallback(
                |State(tx): State<mpsc::Sender<ExportTraceServiceRequest>>,
                 _uri: Uri,
                 body: Bytes| async move {
                    if let Ok(req) = ExportTraceServiceRequest::decode(body) {
                        let _ = tx.send(req).await;
                    }
                    axum::http::StatusCode::OK
                },
            )
            .with_state(tx);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let tc = export_one_span(OtlpConfig {
        // A programmatic `.with_endpoint(..)` is used VERBATIM by
        // opentelemetry-otlp 0.32 — it does NOT append `/v1/traces` (only the
        // `OTEL_EXPORTER_OTLP_ENDPOINT` env var / default does). The stub's
        // `fallback` captures whatever path the exporter POSTs to, so this
        // test is robust to that; a real collector needs the full traces URL.
        endpoint: format!("http://{addr}"),
        protocol: OtlpProtocol::Http,
        max_in_flight_spans: 128,
    });

    let req = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("collector received an export within the timeout")
        .expect("stub channel stayed open");
    assert_ids_match(&req, &tc);
}
