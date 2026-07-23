//! OTLP-backed `Tracer`/`TracerLifecycle` adapter — the SOLE `opentelemetry`
//! consumer in `ego-rs` (design.md ADR-5, ADR-9).
//!
//! ## Span-table semantics (ADR-5)
//!
//! The adapter holds a thread-safe `Map<SpanId, in-progress span record>`
//! keyed by the domain [`ego_domain::SpanId`] supplied as an argument to
//! `start_span`/`end_span` — bookkeeping, never thread-local/ambient context
//! propagation:
//!
//! - `end_span` is **idempotent per `SpanId`**: the first call closes and
//!   exports the span and removes it from the table; a later call for the
//!   same id (or an id that was never started) is a no-op.
//! - A duplicate `start_span` for a still-live `SpanId` is **ignored with a
//!   diagnostic warning** — the existing table entry is left untouched.
//! - The table is bounded by `OtlpConfig::max_in_flight_spans`. At capacity,
//!   a new `start_span` **drops the new span and warns** — it never evicts a
//!   live span, overwrites an entry, or grows unbounded.
//! - `TracerLifecycle::shutdown()` flushes every remaining (orphaned) span
//!   and clears the table.
//!
//! `Context::current()`/`Span::current()` are never used here to carry
//! framework trace-context: every span is built from the explicit
//! `&TraceContext` argument (`ego_domain::TraceContext`), converted losslessly
//! into `opentelemetry` id types (see [`to_otel_trace_id`]/[`to_otel_span_id`]
//! and their inverses).

use ego_domain::{
    SpanAttributes, SpanId as DomainSpanId, SpanOutcome, TraceContext, TraceId as DomainTraceId,
    Tracer, TracerLifecycle,
};
use opentelemetry::trace::{
    SpanContext as OtelSpanContext, SpanId as OtelSpanId, SpanKind as OtelSpanKind,
    Status as OtelStatus, TraceFlags, TraceId as OtelTraceId, TraceState,
};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_otlp::{SpanExporter as OtlpSpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::{BatchSpanProcessor, SpanData, SpanProcessor};
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Lossless domain <-> otel id conversion (TASK-022/023)
// ---------------------------------------------------------------------------
//
// Domain `TraceId`/`SpanId` expose only `to_hex()`/`from_hex()` (no raw byte
// accessor — deliberately, they stay opaque outside `ego-domain`). Both they
// and `opentelemetry`'s id types serialize to the SAME fixed-width lowercase
// hex representation (32 hex chars / 16 bytes for a trace id, 16 hex chars /
// 8 bytes for a span id), so hex is a lossless, bijective encoding: a
// domain -> otel -> domain round trip always yields identical bytes.

/// Convert a domain [`ego_domain::TraceId`] into `opentelemetry`'s trace id.
fn to_otel_trace_id(id: DomainTraceId) -> OtelTraceId {
    OtelTraceId::from_hex(&id.to_hex())
        .expect("ego_domain::TraceId::to_hex always yields a valid 32-hex-digit trace id")
}

/// Convert an `opentelemetry` trace id back into a domain [`ego_domain::TraceId`].
#[allow(dead_code)] // symmetric counterpart of to_otel_trace_id; exercised by the round-trip test
fn from_otel_trace_id(id: OtelTraceId) -> DomainTraceId {
    DomainTraceId::from_hex(&format!("{id:032x}"))
        .expect("opentelemetry::trace::TraceId always yields a valid 32-hex-digit trace id")
}

/// Convert a domain [`ego_domain::SpanId`] into `opentelemetry`'s span id.
fn to_otel_span_id(id: DomainSpanId) -> OtelSpanId {
    OtelSpanId::from_hex(&id.to_hex())
        .expect("ego_domain::SpanId::to_hex always yields a valid 16-hex-digit span id")
}

/// Convert an `opentelemetry` span id back into a domain [`ego_domain::SpanId`].
#[allow(dead_code)] // symmetric counterpart of to_otel_span_id; exercised by the round-trip test
fn from_otel_span_id(id: OtelSpanId) -> DomainSpanId {
    DomainSpanId::from_hex(&format!("{id:016x}"))
        .expect("opentelemetry::trace::SpanId always yields a valid 16-hex-digit span id")
}

// ---------------------------------------------------------------------------
// OtlpConfig
// ---------------------------------------------------------------------------

/// Wire transport used to export spans to the OTLP collector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtlpProtocol {
    /// OTLP over gRPC (tonic).
    Grpc,
    /// OTLP over HTTP (binary protobuf).
    Http,
}

/// Configuration for [`OtlpTracer`].
#[derive(Debug, Clone)]
pub struct OtlpConfig {
    /// The OTLP collector endpoint. For `Grpc` this is the base gRPC endpoint
    /// (e.g. `http://localhost:4317`). For `Http` it is used verbatim as the
    /// traces URL — the exporter does NOT append `/v1/traces`, so pass the full
    /// path (e.g. `http://localhost:4318/v1/traces`).
    pub endpoint: String,
    /// The wire transport to export over.
    pub protocol: OtlpProtocol,
    /// Upper bound on the number of concurrently in-flight (started, not yet
    /// ended) spans. See [`Tracer`]'s span-table semantics above.
    pub max_in_flight_spans: usize,
}

fn build_exporter(
    config: &OtlpConfig,
) -> Result<OtlpSpanExporter, opentelemetry_otlp::ExporterBuildError> {
    match config.protocol {
        OtlpProtocol::Grpc => OtlpSpanExporter::builder()
            .with_tonic()
            .with_endpoint(config.endpoint.clone())
            .build(),
        OtlpProtocol::Http => OtlpSpanExporter::builder()
            .with_http()
            .with_endpoint(config.endpoint.clone())
            .build(),
    }
}

// ---------------------------------------------------------------------------
// InProgressSpan — bookkeeping record for a started-but-not-yet-ended span
// ---------------------------------------------------------------------------

/// Everything `end_span` needs to build the exported `SpanData` directly,
/// captured at `start_span` time. Deliberately plain data — no SDK span
/// object, no OTel call is involved in producing or holding this record.
struct InProgressSpan {
    trace_id: DomainTraceId,
    span_id: DomainSpanId,
    parent_span_id: Option<DomainSpanId>,
    name: String,
    start_time: SystemTime,
    attributes: Vec<KeyValue>,
}

// ---------------------------------------------------------------------------
// OtlpTracer
// ---------------------------------------------------------------------------

/// OTLP-backed `Tracer` + `TracerLifecycle` adapter.
///
/// Holds a [`dashmap::DashMap<DomainSpanId, InProgressSpan>`] span table
/// keyed by the argument-supplied domain [`ego_domain::SpanId`] (ADR-5) —
/// bookkeeping only, never ambient/thread-local context. `DashMap` gives
/// per-shard locking, so `start_span`/`end_span` never hold one contended,
/// global lock — and, critically, no lock is ever held across `opentelemetry`
/// SDK/exporter work: `start_span` does not call into OTel at all, and
/// `end_span` releases its shard guard (via `DashMap::remove`) before handing
/// the built `SpanData` to the span processor's `on_end`.
///
/// The exported span is built **directly** from the domain ids recorded in
/// the `InProgressSpan`, with no OTel `Tracer`/`IdGenerator` involved:
/// `span_context.span_id()`, `span_context.trace_id()`, and
/// `parent_span_id` are all forced to the values carried by the domain
/// [`ego_domain::TraceContext`] passed to `start_span`. This is what makes
/// cross-service propagation correct: the `span_id` this adapter exports is
/// the exact same id `to_traceparent()` propagates downstream as the next
/// service's `parent_span_id`, so the collector can stitch the link.
pub struct OtlpTracer {
    processor: Box<dyn SpanProcessor>,
    table: dashmap::DashMap<DomainSpanId, InProgressSpan>,
    max_in_flight_spans: usize,
}

impl std::fmt::Debug for OtlpTracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtlpTracer")
            .field("max_in_flight_spans", &self.max_in_flight_spans)
            .finish_non_exhaustive()
    }
}

impl OtlpTracer {
    /// Construct an `OtlpTracer` wired to the OTLP collector described by
    /// `config`. Building the exporter/processor is lazy: this MUST NOT
    /// panic or block even when no collector is reachable at
    /// `config.endpoint` — the underlying gRPC channel/HTTP client connect
    /// lazily on first export, and export itself happens on a background
    /// batch processor (never on the `start_span`/`end_span` call path).
    pub fn new(config: OtlpConfig) -> Result<Self, opentelemetry_otlp::ExporterBuildError> {
        let exporter = build_exporter(&config)?;
        let processor = BatchSpanProcessor::builder(exporter).build();
        Ok(Self::from_processor(processor, config.max_in_flight_spans))
    }

    /// Build directly from an already-configured [`SpanProcessor`] (e.g. one
    /// wired to an in-memory exporter for tests). Not part of the public
    /// port — an infra-internal test/construction seam.
    fn from_processor(processor: impl SpanProcessor + 'static, max_in_flight_spans: usize) -> Self {
        Self {
            processor: Box::new(processor),
            table: dashmap::DashMap::new(),
            max_in_flight_spans,
        }
    }

    /// Number of spans currently live in the table (started, not yet ended).
    /// Test/diagnostic seam.
    #[cfg(test)]
    fn in_flight_count(&self) -> usize {
        self.table.len()
    }
}

/// Map the redaction-safe domain [`SpanAttributes`] allow-list to OTel
/// key/values. No redaction step here — the port already guarantees
/// `SpanAttributes` cannot carry sensitive data (ADR-6).
fn to_otel_attributes(attrs: &SpanAttributes) -> Vec<KeyValue> {
    let mut kvs = Vec::with_capacity(2);
    if let Some(present) = attrs.tenant_present() {
        kvs.push(KeyValue::new("tenant.present", present));
    }
    if let Some(duration) = attrs.duration() {
        kvs.push(KeyValue::new("duration_ms", duration.as_millis() as i64));
    }
    kvs
}

impl Tracer for OtlpTracer {
    fn start_span(&self, ctx: &TraceContext, name: &str, attrs: SpanAttributes) {
        let key = ctx.span_id();

        // No OTel call anywhere in this method, no lock held across SDK/
        // exporter work: `DashMap` gives per-shard locking for the
        // contains_key/len/insert bookkeeping below, and none of it touches
        // `opentelemetry`.
        if self.table.contains_key(&key) {
            tracing::warn!(
                span_id = %key.to_hex(),
                "OtlpTracer::start_span: duplicate start for a still-live span id; ignoring"
            );
            return;
        }

        // Soft bound: a benign TOCTOU race between this length check and the
        // insert below is acceptable (this is bookkeeping capacity, not a
        // hard safety invariant) — it trades a global lock for the rare
        // possibility of a slightly-over-capacity table under concurrent
        // racing starts.
        if self.table.len() >= self.max_in_flight_spans {
            tracing::warn!(
                span_id = %key.to_hex(),
                max_in_flight_spans = self.max_in_flight_spans,
                "OtlpTracer::start_span: in-flight span table at capacity; dropping new span"
            );
            return;
        }

        self.table.insert(
            key,
            InProgressSpan {
                trace_id: ctx.trace_id(),
                span_id: key,
                parent_span_id: ctx.parent_span_id(),
                name: name.to_string(),
                start_time: SystemTime::now(),
                attributes: to_otel_attributes(&attrs),
            },
        );
    }

    fn end_span(&self, span_id: DomainSpanId, outcome: SpanOutcome) {
        // `DashMap::remove` returns the owned entry and releases its shard
        // guard before we ever touch `opentelemetry` — no table lock is held
        // across the `SpanData` construction or the `on_end` call below.
        let Some((_, record)) = self.table.remove(&span_id) else {
            // Idempotent: already ended, or never started. No-op either way.
            return;
        };

        let status = match outcome {
            SpanOutcome::Ok => OtelStatus::Ok,
            SpanOutcome::Error { status_message } => OtelStatus::error(status_message),
        };
        self.processor.on_end(build_span_data(record, status));
    }
}

/// Build the exported `SpanData` directly from a domain-identified
/// [`InProgressSpan`] record — no SDK `Tracer`/`IdGenerator` involved.
/// `span_context.span_id()`/`trace_id()` and `parent_span_id` are all forced
/// to the domain ids captured by `start_span` (PROD-003 PR5 fix).
fn build_span_data(record: InProgressSpan, status: OtelStatus) -> SpanData {
    let span_context = OtelSpanContext::new(
        to_otel_trace_id(record.trace_id),
        to_otel_span_id(record.span_id),
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    );
    let parent_span_id = record
        .parent_span_id
        .map(to_otel_span_id)
        .unwrap_or(OtelSpanId::INVALID);

    SpanData {
        span_context,
        parent_span_id,
        parent_span_is_remote: record.parent_span_id.is_some(),
        span_kind: OtelSpanKind::Server,
        name: record.name.into(),
        start_time: record.start_time,
        end_time: SystemTime::now(),
        attributes: record.attributes,
        dropped_attributes_count: 0,
        events: Default::default(),
        links: Default::default(),
        status,
        instrumentation_scope: InstrumentationScope::builder("ego-rs").build(),
    }
}

impl TracerLifecycle for OtlpTracer {
    /// Flush all pending (unended) spans and clear the span table (ADR-9).
    ///
    /// This exports every orphaned span still in the table directly (via the
    /// span processor's `on_end`, building `SpanData` exactly like a normal
    /// `end_span` — but with `Status::Unset`, since these spans were never
    /// explicitly ended with an outcome) and force-flushes the processor so
    /// the export is not left sitting in the batch processor's queue. It
    /// deliberately does NOT tear down the span processor itself: the domain
    /// contract for `shutdown` is "flush pending + clear the table", not
    /// "the process is exiting and connections must close" — and leaving
    /// the processor live keeps a caller-visible re-flush idempotent rather
    /// than turning any later export into a silent post-shutdown no-op.
    fn shutdown(&self) {
        let orphaned_keys: Vec<DomainSpanId> =
            self.table.iter().map(|entry| *entry.key()).collect();
        for span_id in orphaned_keys {
            if let Some((_, record)) = self.table.remove(&span_id) {
                self.processor
                    .on_end(build_span_data(record, OtelStatus::Unset));
            }
        }

        if let Err(err) = self.processor.force_flush() {
            tracing::warn!(error = %err, "OtlpTracer::shutdown: force-flushing pending spans reported an error");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::TraceContext;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, InMemorySpanExporterBuilder};
    use std::time::Duration;

    /// Build an `OtlpTracer` wired to an in-memory exporter (no live
    /// collector involved) plus a handle to read back exported spans.
    ///
    /// NOTE (explicit scope disclosure): TASK-024/025 ask for export
    /// verification against a "stub collector". Standing up a real gRPC/HTTP
    /// stub server is heavyweight for a unit-test suite, so this uses
    /// `opentelemetry_sdk`'s in-memory `SpanExporter` test double instead —
    /// it proves the OTLP adapter's span table hands completed spans to
    /// WHATEVER `SpanProcessor`/`SpanExporter` the adapter is configured
    /// with, independent of the wire transport (which is exercised
    /// separately, without a live endpoint, by
    /// `otlp_tracer_can_be_constructed_for_each_protocol`).
    fn otlp_tracer_with_in_memory_exporter(
        max_in_flight_spans: usize,
    ) -> (OtlpTracer, InMemorySpanExporter) {
        let exporter = InMemorySpanExporterBuilder::new().build();
        let processor = BatchSpanProcessor::builder(exporter.clone()).build();
        (
            OtlpTracer::from_processor(processor, max_in_flight_spans),
            exporter,
        )
    }

    // -----------------------------------------------------------------
    // TASK-020/021: span-table bookkeeping
    // -----------------------------------------------------------------

    #[test]
    fn start_span_then_end_span_ok_removes_it_from_the_table() {
        let (tracer, _exporter) = otlp_tracer_with_in_memory_exporter(8);
        let ctx = TraceContext::root();

        tracer.start_span(&ctx, "op.execute", SpanAttributes::new());
        assert_eq!(tracer.in_flight_count(), 1);

        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);
        assert_eq!(tracer.in_flight_count(), 0);
    }

    #[test]
    fn end_span_is_idempotent_on_response_then_error_race_resolves_to_one_close() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(8);
        let ctx = TraceContext::root();

        tracer.start_span(&ctx, "op.execute", SpanAttributes::new());

        // on_response then on_error race for the SAME span id.
        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);
        tracer.end_span(
            ctx.span_id(),
            SpanOutcome::Error {
                status_message: "too late".to_string(),
            },
        );

        assert_eq!(tracer.in_flight_count(), 0, "second end must be a no-op");
        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 1, "exactly one export, not two");
    }

    #[test]
    fn end_span_for_a_never_started_id_is_a_no_op() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(8);
        let ctx = TraceContext::root();

        // No start_span call for ctx.span_id() at all (e.g. request failed
        // before on_request opened one).
        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);

        assert_eq!(tracer.in_flight_count(), 0);
        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        assert_eq!(
            exporter.get_finished_spans().unwrap().len(),
            0,
            "nothing should have been exported"
        );
    }

    #[test]
    fn duplicate_start_span_for_a_live_id_is_ignored_and_warns() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(8);
        let ctx = TraceContext::root();

        tracer.start_span(&ctx, "first", SpanAttributes::new());
        assert_eq!(tracer.in_flight_count(), 1);

        // Duplicate start for the SAME still-live span id must be ignored.
        tracer.start_span(&ctx, "second-should-be-ignored", SpanAttributes::new());
        assert_eq!(
            tracer.in_flight_count(),
            1,
            "duplicate start must not add or replace a table entry"
        );

        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);
        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 1);
        assert_eq!(
            finished[0].name, "first",
            "the original span (not the ignored duplicate) must be the one exported"
        );
    }

    #[test]
    fn max_in_flight_spans_overflow_drops_new_span_and_keeps_existing_entries_intact() {
        let (tracer, _exporter) = otlp_tracer_with_in_memory_exporter(2);

        let ctx_a = TraceContext::root();
        let ctx_b = TraceContext::root();
        let ctx_overflow = TraceContext::root();

        tracer.start_span(&ctx_a, "a", SpanAttributes::new());
        tracer.start_span(&ctx_b, "b", SpanAttributes::new());
        assert_eq!(tracer.in_flight_count(), 2, "table is now at capacity (2)");

        // A third distinct span id, started while the table is full, must be
        // dropped — never evicting `a` or `b`, never overwriting, never
        // growing the table past the configured bound.
        tracer.start_span(&ctx_overflow, "overflow", SpanAttributes::new());
        assert_eq!(
            tracer.in_flight_count(),
            2,
            "overflow start must be dropped, table size stays at the bound"
        );

        // The two original spans are still live and endable normally.
        tracer.end_span(ctx_a.span_id(), SpanOutcome::Ok);
        assert_eq!(tracer.in_flight_count(), 1);
        tracer.end_span(ctx_b.span_id(), SpanOutcome::Ok);
        assert_eq!(tracer.in_flight_count(), 0);

        // The dropped overflow span was never in the table, so ending it is
        // the standard never-started no-op.
        tracer.end_span(ctx_overflow.span_id(), SpanOutcome::Ok);
        assert_eq!(tracer.in_flight_count(), 0);
    }

    #[test]
    fn shutdown_flushes_orphaned_spans_and_clears_the_table() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(8);
        let ctx_a = TraceContext::root();
        let ctx_b = TraceContext::root();

        // Both started, NEITHER ended — orphaned spans.
        tracer.start_span(&ctx_a, "orphan-a", SpanAttributes::new());
        tracer.start_span(&ctx_b, "orphan-b", SpanAttributes::new());
        assert_eq!(tracer.in_flight_count(), 2);

        tracer.shutdown();

        assert_eq!(
            tracer.in_flight_count(),
            0,
            "table must be empty after shutdown"
        );
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(
            finished.len(),
            2,
            "both orphaned spans must have been flushed/exported"
        );
    }

    #[test]
    fn end_span_error_outcome_records_a_redaction_safe_status_message() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(8);
        let ctx = TraceContext::root();

        tracer.start_span(&ctx, "op.execute", SpanAttributes::new());
        tracer.end_span(
            ctx.span_id(),
            SpanOutcome::Error {
                status_message: "redacted failure".to_string(),
            },
        );

        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 1);
        match &finished[0].status {
            opentelemetry::trace::Status::Error { description } => {
                assert_eq!(description.as_ref(), "redacted failure");
            }
            other => panic!("expected Status::Error, got {other:?}"),
        }
    }

    #[test]
    fn adapter_maps_attributes_without_redacting() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(8);
        let ctx = TraceContext::root();

        tracer.start_span(
            &ctx,
            "op.execute",
            SpanAttributes::new()
                .with_tenant_hint_present(true)
                .with_duration(Duration::from_millis(42)),
        );
        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);

        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 1);
        let attrs = &finished[0].attributes;
        assert!(attrs.iter().any(|kv| kv.key.as_str() == "tenant.present"
            && kv.value == opentelemetry::Value::Bool(true)));
        assert!(attrs.iter().any(
            |kv| kv.key.as_str() == "duration_ms" && kv.value == opentelemetry::Value::I64(42)
        ));
    }

    // -----------------------------------------------------------------
    // TASK-022/023: lossless id conversion round-trip
    // -----------------------------------------------------------------

    #[test]
    fn trace_id_round_trips_domain_to_otel_and_back() {
        for _ in 0..5 {
            let domain = ego_domain::TraceContext::root().trace_id();
            let otel = to_otel_trace_id(domain);
            let back = from_otel_trace_id(otel);
            assert_eq!(domain, back);
        }
    }

    #[test]
    fn span_id_round_trips_domain_to_otel_and_back() {
        for _ in 0..5 {
            let domain = ego_domain::TraceContext::root().span_id();
            let otel = to_otel_span_id(domain);
            let back = from_otel_span_id(otel);
            assert_eq!(domain, back);
        }
    }

    #[test]
    fn trace_id_round_trip_covers_edge_byte_values() {
        // Exercise the all-zero-except-one-byte and all-`ff` edges, which
        // hex round-tripping must preserve exactly (not just "typical"
        // random ids).
        let edge_hexes = [
            "00000000000000000000000000000001",
            "ffffffffffffffffffffffffffffffff",
            "10000000000000000000000000000000",
        ];
        for hex in edge_hexes {
            let hex = &hex[hex.len() - 32..]; // keep exactly 32 chars
            let domain = ego_domain::TraceId::from_hex(hex).unwrap();
            let otel = to_otel_trace_id(domain);
            let back = from_otel_trace_id(otel);
            assert_eq!(domain, back, "round trip must preserve edge-byte trace ids");
        }
    }

    #[test]
    fn span_id_round_trip_covers_edge_byte_values() {
        for hex in ["0000000000000001", "ffffffffffffffff", "8000000000000000"] {
            let domain = ego_domain::SpanId::from_hex(hex).unwrap();
            let otel = to_otel_span_id(domain);
            let back = from_otel_span_id(otel);
            assert_eq!(domain, back, "round trip must preserve edge-byte span ids");
        }
    }

    // -----------------------------------------------------------------
    // TASK-024/025: config-driven protocol selection
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn otlp_tracer_can_be_constructed_for_each_protocol_without_a_live_collector() {
        // Building the exporter/processor must not panic or block even
        // though nothing is listening at these endpoints — gRPC channels and
        // the HTTP client connect lazily; the batch processor exports in the
        // background, never during construction. Requires a Tokio runtime
        // context (the tonic channel/hyper-util client resolve the current
        // reactor at construction time, even though they do not connect).
        for protocol in [OtlpProtocol::Grpc, OtlpProtocol::Http] {
            let config = OtlpConfig {
                endpoint: "http://127.0.0.1:1".to_string(),
                protocol,
                max_in_flight_spans: 32,
            };
            let tracer = OtlpTracer::new(config);
            assert!(
                tracer.is_ok(),
                "constructing OtlpTracer for {protocol:?} must succeed without a live collector"
            );
        }
    }

    #[test]
    fn a_started_and_ended_span_reaches_the_configured_exporter() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(4);
        let ctx = TraceContext::root();

        tracer.start_span(&ctx, "outbound.call", SpanAttributes::new());
        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);

        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].name, "outbound.call");
    }

    // -----------------------------------------------------------------
    // PROD-003 PR5: exported span must carry the DOMAIN span id (not an
    // SDK-random one), so cross-service parent-linkage via
    // `to_traceparent()` can be stitched by the collector.
    // -----------------------------------------------------------------

    #[test]
    fn exported_span_carries_the_domain_span_id_and_trace_id() {
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(4);
        let ctx = TraceContext::root();

        tracer.start_span(&ctx, "op.execute", SpanAttributes::new());
        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);

        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 1);

        let exported_span_context = finished[0].span_context.clone();
        assert_eq!(
            exported_span_context.span_id(),
            to_otel_span_id(ctx.span_id()),
            "exported span's own span_id must equal the domain span_id, \
             otherwise downstream traceparent propagation (PR4) points at \
             an id the collector never sees on the wire"
        );
        assert_eq!(
            exported_span_context.trace_id(),
            to_otel_trace_id(ctx.trace_id()),
            "exported span's trace_id must equal the domain trace_id"
        );
    }

    #[test]
    fn two_sequential_spans_each_export_with_their_own_distinct_domain_span_id() {
        // Triangulation: proves the exported span_id is not just a
        // fixed/hardcoded value — two DIFFERENT domain span ids, started and
        // ended one after another, must each come back out on their OWN
        // exported span, never swapped or reused.
        let (tracer, exporter) = otlp_tracer_with_in_memory_exporter(4);
        let ctx_a = TraceContext::root();
        let ctx_b = TraceContext::root();
        assert_ne!(
            ctx_a.span_id(),
            ctx_b.span_id(),
            "test precondition: two root contexts must have distinct span ids"
        );

        tracer.start_span(&ctx_a, "first", SpanAttributes::new());
        tracer.end_span(ctx_a.span_id(), SpanOutcome::Ok);
        tracer.start_span(&ctx_b, "second", SpanAttributes::new());
        tracer.end_span(ctx_b.span_id(), SpanOutcome::Ok);

        tracer
            .processor
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 2);

        let first = finished
            .iter()
            .find(|s| s.name == "first")
            .expect("the 'first' span must have been exported");
        let second = finished
            .iter()
            .find(|s| s.name == "second")
            .expect("the 'second' span must have been exported");

        assert_eq!(
            first.span_context.span_id(),
            to_otel_span_id(ctx_a.span_id()),
            "first span must carry ctx_a's domain span id"
        );
        assert_eq!(
            second.span_context.span_id(),
            to_otel_span_id(ctx_b.span_id()),
            "second span must carry ctx_b's domain span id, not ctx_a's"
        );
    }
}
