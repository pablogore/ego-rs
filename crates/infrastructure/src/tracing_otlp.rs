//! OTLP-backed `Tracer`/`TracerLifecycle` adapter — the SOLE `opentelemetry`
//! consumer in `ego-rs` (design.md ADR-5, ADR-9).
//!
//! ## Span-table semantics (ADR-5)
//!
//! The adapter holds a thread-safe `Map<SpanId, otel span>` keyed by the
//! domain [`ego_domain::SpanId`] supplied as an argument to `start_span`/
//! `end_span` — bookkeeping, never thread-local/ambient context propagation:
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
    Span as OtelSpanTrait, SpanBuilder, SpanContext as OtelSpanContext, SpanId as OtelSpanId,
    Status as OtelStatus, TraceContextExt, TraceFlags, TraceId as OtelTraceId,
    Tracer as OtelTracerTrait, TracerProvider as OtelTracerProviderTrait, TraceState,
};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_otlp::{SpanExporter as OtlpSpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::{IdGenerator, RandomIdGenerator, SdkTracer, SdkTracerProvider, Span as SdkSpan};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

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
// DomainIdGenerator — forces the exported span's OWN span_id to the domain's
// (PROD-003 PR5 correctness fix)
// ---------------------------------------------------------------------------
//
// `opentelemetry_sdk::trace::SdkTracer::build_with_context` never accepts a
// caller-supplied span id: it always calls the provider's configured
// `IdGenerator::new_span_id()` exactly once, synchronously, to mint the new
// span's own id (see `opentelemetry_sdk` 0.32's `Tracer::build_with_context`).
// `parent_context()` below only forces the PARENT link (trace_id +
// parent_span_id) via a remote `SpanContext` — it cannot influence the new
// span's own id at all, since `opentelemetry`'s public `SpanBuilder` has no
// `span_id`/`trace_id` field or setter in this version.
//
// This adapter-internal `IdGenerator` closes that gap: `start_span` pushes
// the domain-derived id onto a queue immediately before calling
// `build_with_context`, and `new_span_id()` pops it. This is bookkeeping
// confined to the adapter (ADR-5) — it does NOT read
// `Context::current()`/`Span::current()` or any other ambient/thread-local
// framework state; the only input is the explicit id pushed by `start_span`.
#[derive(Debug, Default)]
struct DomainIdGenerator {
    next_span_ids: Arc<Mutex<VecDeque<OtelSpanId>>>,
}

impl DomainIdGenerator {
    fn new() -> (Self, Arc<Mutex<VecDeque<OtelSpanId>>>) {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        (
            Self {
                next_span_ids: queue.clone(),
            },
            queue,
        )
    }
}

impl IdGenerator for DomainIdGenerator {
    fn new_trace_id(&self) -> OtelTraceId {
        // Never used to mint a child span's trace id in practice: every
        // span built here goes through `parent_context()`, which supplies
        // an active remote span context, so `build_with_context` takes the
        // trace id from the parent context instead of calling this method.
        // Falls back to the default random generator for defense in depth
        // (e.g. if `opentelemetry_sdk` internals ever change).
        RandomIdGenerator::default().new_trace_id()
    }

    fn new_span_id(&self) -> OtelSpanId {
        let mut queue = self
            .next_span_ids
            .lock()
            .expect("DomainIdGenerator queue mutex poisoned");
        queue
            .pop_front()
            .unwrap_or_else(|| RandomIdGenerator::default().new_span_id())
    }
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
    /// The OTLP collector endpoint (e.g. `http://localhost:4317`).
    pub endpoint: String,
    /// The wire transport to export over.
    pub protocol: OtlpProtocol,
    /// Upper bound on the number of concurrently in-flight (started, not yet
    /// ended) spans. See [`Tracer`]'s span-table semantics above.
    pub max_in_flight_spans: usize,
}

fn build_exporter(config: &OtlpConfig) -> Result<OtlpSpanExporter, opentelemetry_otlp::ExporterBuildError> {
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
// OtlpTracer
// ---------------------------------------------------------------------------

/// OTLP-backed `Tracer` + `TracerLifecycle` adapter.
///
/// Holds a `Mutex<HashMap<DomainSpanId, SdkSpan>>` span table keyed by the
/// argument-supplied domain [`ego_domain::SpanId`] (ADR-5) — bookkeeping
/// only, never ambient/thread-local context.
///
/// The exported span is **fully domain-identified**: `trace_id`, its OWN
/// `span_id`, and `parent_span_id` are all forced to the values carried by
/// the domain [`ego_domain::TraceContext`] passed to `start_span` — none of
/// the three is left to the SDK's default random `IdGenerator`. `trace_id`/
/// `parent_span_id` are forced via the remote `SpanContext` built by
/// `parent_context()`; the span's own `span_id` is forced via the adapter's
/// [`DomainIdGenerator`], since `opentelemetry`'s `SpanBuilder` has no
/// `span_id` field or setter to set it directly. This is what makes
/// cross-service propagation correct: the `span_id` this adapter exports is
/// the exact same id `to_traceparent()` propagates downstream as the next
/// service's `parent_span_id`, so the collector can stitch the link.
pub struct OtlpTracer {
    tracer: SdkTracer,
    provider: SdkTracerProvider,
    table: Mutex<HashMap<DomainSpanId, SdkSpan>>,
    /// Shared with the `DomainIdGenerator` wired into `provider`'s config —
    /// `start_span` pushes the next span's forced id here (see
    /// `DomainIdGenerator` doc comment for the single-consumer guarantee).
    next_span_ids: Arc<Mutex<VecDeque<OtelSpanId>>>,
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
    /// `config`. Building the exporter/provider is lazy: this MUST NOT panic
    /// or block even when no collector is reachable at `config.endpoint` —
    /// the underlying gRPC channel/HTTP client connect lazily on first
    /// export, and export itself happens on a background batch processor
    /// (never on the `start_span`/`end_span` call path).
    pub fn new(config: OtlpConfig) -> Result<Self, opentelemetry_otlp::ExporterBuildError> {
        let exporter = build_exporter(&config)?;
        let (id_generator, next_span_ids) = DomainIdGenerator::new();
        let provider = SdkTracerProvider::builder()
            .with_id_generator(id_generator)
            .with_batch_exporter(exporter)
            .build();
        Ok(Self::from_provider(
            provider,
            next_span_ids,
            config.max_in_flight_spans,
        ))
    }

    /// Build directly from an already-configured `SdkTracerProvider` (e.g.
    /// one wired to an in-memory exporter for tests) plus the queue handle
    /// shared with the `DomainIdGenerator` that provider was built with. Not
    /// part of the public port — an infra-internal test/construction seam.
    ///
    /// Callers MUST build `provider` with `.with_id_generator(id_generator)`
    /// using the `DomainIdGenerator` paired with `next_span_ids` (i.e. the
    /// two halves returned together by `DomainIdGenerator::new()`) — passing
    /// mismatched halves silently falls back to random span ids.
    fn from_provider(
        provider: SdkTracerProvider,
        next_span_ids: Arc<Mutex<VecDeque<OtelSpanId>>>,
        max_in_flight_spans: usize,
    ) -> Self {
        let tracer = provider.tracer("ego-rs");
        Self {
            tracer,
            provider,
            table: Mutex::new(HashMap::new()),
            next_span_ids,
            max_in_flight_spans,
        }
    }

    /// Number of spans currently live in the table (started, not yet ended).
    /// Test/diagnostic seam.
    #[cfg(test)]
    fn in_flight_count(&self) -> usize {
        self.table.lock().expect("span table mutex poisoned").len()
    }
}

/// Build the otel parent `Context` for a new span from the domain
/// `TraceContext`: `trace_id` is forced to the domain's `trace_id` (so the
/// exported span always carries EGO's own trace identity, root or not), and
/// `parent_span_id` is the domain's `parent_span_id` when present (else
/// `SpanId::INVALID`, correctly representing a root span to the SDK).
fn parent_context(ctx: &TraceContext) -> OtelContext {
    let parent_span_id = ctx
        .parent_span_id()
        .map(to_otel_span_id)
        .unwrap_or(OtelSpanId::INVALID);
    let span_context = OtelSpanContext::new(
        to_otel_trace_id(ctx.trace_id()),
        parent_span_id,
        TraceFlags::SAMPLED,
        true,
        TraceState::NONE,
    );
    OtelContext::new().with_remote_span_context(span_context)
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
        let mut table = self.table.lock().expect("span table mutex poisoned");

        if table.contains_key(&key) {
            tracing::warn!(
                span_id = %key.to_hex(),
                "OtlpTracer::start_span: duplicate start for a still-live span id; ignoring"
            );
            return;
        }

        if table.len() >= self.max_in_flight_spans {
            tracing::warn!(
                span_id = %key.to_hex(),
                max_in_flight_spans = self.max_in_flight_spans,
                "OtlpTracer::start_span: in-flight span table at capacity; dropping new span"
            );
            return;
        }

        let parent_cx = parent_context(ctx);
        let builder =
            SpanBuilder::from_name(name.to_string()).with_attributes(to_otel_attributes(&attrs));

        // Force the exported span's OWN id to the domain span id (PR5 fix):
        // push it onto the queue the `DomainIdGenerator` drains from. Safe
        // because `table`'s lock (held for this whole function) serializes
        // every `start_span` call across threads, so no other thread's push
        // can land between this push and the pop that
        // `build_with_context` performs synchronously, on this thread,
        // exactly once, immediately below.
        self.next_span_ids
            .lock()
            .expect("DomainIdGenerator queue mutex poisoned")
            .push_back(to_otel_span_id(key));

        let span = self.tracer.build_with_context(builder, &parent_cx);
        table.insert(key, span);
    }

    fn end_span(&self, span: DomainSpanId, outcome: SpanOutcome) {
        let mut table = self.table.lock().expect("span table mutex poisoned");
        let Some(mut live) = table.remove(&span) else {
            // Idempotent: already ended, or never started. No-op either way.
            return;
        };
        drop(table);

        match outcome {
            SpanOutcome::Ok => {}
            SpanOutcome::Error { status_message } => {
                live.set_status(OtelStatus::error(status_message));
            }
        }
        live.end();
    }
}

impl TracerLifecycle for OtlpTracer {
    /// Flush all pending (unended) spans and clear the span table (ADR-9).
    ///
    /// This ends every orphaned span still in the table (so its data reaches
    /// the configured `SpanExporter`) and force-flushes the underlying
    /// provider so the export is not left sitting in the batch processor's
    /// queue. It deliberately does NOT tear down the `SdkTracerProvider`
    /// itself (no exporter/processor shutdown): the domain contract for
    /// `shutdown` is "flush pending + clear the table", not "the process is
    /// exiting and connections must close" — and leaving the provider live
    /// keeps a caller-visible re-flush idempotent rather than turning any
    /// later export into a silent post-shutdown no-op.
    fn shutdown(&self) {
        let mut table = self.table.lock().expect("span table mutex poisoned");
        for (_, mut span) in table.drain() {
            span.end();
        }
        drop(table);

        if let Err(err) = self.provider.force_flush() {
            tracing::warn!(error = %err, "OtlpTracer::shutdown: force-flushing pending spans reported an error");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::TraceContext;
    use opentelemetry_sdk::trace::{BatchSpanProcessor, InMemorySpanExporter, InMemorySpanExporterBuilder};
    use std::time::Duration;

    /// Build an `OtlpTracer` wired to an in-memory exporter (no live
    /// collector involved) plus a handle to read back exported spans.
    ///
    /// NOTE (explicit scope disclosure): TASK-024/025 ask for export
    /// verification against a "stub collector". Standing up a real gRPC/HTTP
    /// stub server is heavyweight for a unit-test suite, so this uses
    /// `opentelemetry_sdk`'s in-memory `SpanExporter` test double instead —
    /// it proves the OTLP adapter's span table hands completed spans to
    /// WHATEVER `SpanExporter` the provider is configured with, independent
    /// of the wire transport (which is exercised separately, without a live
    /// endpoint, by `otlp_tracer_can_be_constructed_for_each_protocol`).
    fn otlp_tracer_with_in_memory_exporter(max_in_flight_spans: usize) -> (OtlpTracer, InMemorySpanExporter) {
        let exporter = InMemorySpanExporterBuilder::new().build();
        let (id_generator, next_span_ids) = DomainIdGenerator::new();
        let provider = SdkTracerProvider::builder()
            .with_id_generator(id_generator)
            .with_span_processor(BatchSpanProcessor::builder(exporter.clone()).build())
            .build();
        (
            OtlpTracer::from_provider(provider, next_span_ids, max_in_flight_spans),
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
            .provider
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
            .provider
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
            .provider
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

        assert_eq!(tracer.in_flight_count(), 0, "table must be empty after shutdown");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 2, "both orphaned spans must have been flushed/exported");
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
            .provider
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
            .provider
            .force_flush()
            .expect("in-memory exporter flush must not fail");
        let finished = exporter.get_finished_spans().unwrap();
        assert_eq!(finished.len(), 1);
        let attrs = &finished[0].attributes;
        assert!(attrs
            .iter()
            .any(|kv| kv.key.as_str() == "tenant.present" && kv.value == opentelemetry::Value::Bool(true)));
        assert!(attrs
            .iter()
            .any(|kv| kv.key.as_str() == "duration_ms" && kv.value == opentelemetry::Value::I64(42)));
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
        // Building the exporter/provider must not panic or block even though
        // nothing is listening at these endpoints — gRPC channels and the
        // HTTP client connect lazily; the batch processor exports in the
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
            .provider
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
            .provider
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
        // Triangulation: proves `DomainIdGenerator` is not just returning a
        // fixed/hardcoded id — two DIFFERENT domain span ids, started and
        // ended one after another on the queue, must each come back out on
        // their OWN exported span, never swapped or reused.
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
            .provider
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
