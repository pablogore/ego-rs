//! OTLP-backed [`Observability`] metrics adapter — the second and last
//! `opentelemetry` consumer in `ego-rs`, alongside
//! [`crate::tracing_otlp`].
//!
//! # What it does, and deliberately nothing more
//!
//! It translates one [`MetricObservation`] into one OpenTelemetry instrument
//! recording. The kind selects the instrument, the name becomes the instrument
//! name, the value is recorded verbatim, and every [`MetricAttribute`] becomes
//! one `KeyValue`. It does not rename, prefix, bucket, filter, sample, or
//! aggregate — the emitters decided all of that under AD-10, and an adapter that
//! second-guessed them would make the table describe something other than what
//! ships.
//!
//! In particular it applies **no redaction of its own**. Cardinality and
//! redaction are settled where a signal is emitted: values are drawn from closed
//! sets, and the client-supplied operation key is never handed to a metric at
//! all. An adapter-side filter would suggest those guarantees live here, and a
//! deployment swapping adapters would silently lose them.
//!
//! # Instruments are created once per name
//!
//! OpenTelemetry expects an instrument to be created once and recorded to many
//! times. Creating one per observation would allocate on every request and, on
//! some backends, register duplicate instruments. The adapter therefore caches
//! them by `(kind, name)` — `&'static str` names make that cache bounded by the
//! program's own contract rather than by traffic.
//!
//! # Lifecycle belongs to the host
//!
//! Constructing the `MeterProvider`, choosing an exporter, and flushing or
//! shutting it down are the host's, through [`OtlpMetrics::shutdown`]. Nothing
//! in the domain or the runtime knows a provider exists, and there is no global
//! installed as a side effect: an adopter that configures no exporter keeps
//! [`crate::observability::NoopObservability`], explicitly.

use std::collections::HashMap;
use std::sync::Mutex;

use ego_domain::{Level, MetricKind, MetricObservation, Observability, SemanticEvent};
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter, MeterProvider as _};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{MetricExporter as OtlpMetricExporter, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

use crate::tracing_otlp::OtlpProtocol;

/// The instrumentation scope every instrument this adapter creates is attributed
/// to, so a backend can tell framework metrics from an application's own.
const SCOPE: &str = "ego-rs";

/// The instruments created so far, keyed by name within each kind.
///
/// Three maps rather than one keyed by `(kind, name)`: the instrument types are
/// distinct, so a single map would need an enum wrapper that exists only to be
/// unwrapped at every call site. A name recorded under two different kinds gets
/// two instruments, which is what an operator would see anyway — and is a
/// contract error at the emitter, not something for the adapter to reconcile.
#[derive(Default)]
struct Instruments {
    counters: HashMap<&'static str, Counter<f64>>,
    gauges: HashMap<&'static str, Gauge<f64>>,
    histograms: HashMap<&'static str, Histogram<f64>>,
}

/// Where and how to export metrics.
///
/// # Why this is not [`crate::tracing_otlp::OtlpConfig`]
///
/// That type's `endpoint` is documented as the **full traces URL** under
/// `Http` — the exporter appends no path, so a host passes
/// `http://collector:4318/v1/traces` verbatim. Handing the same string to a
/// metrics exporter would POST metrics to `/v1/traces`: accepted by the
/// transport, rejected or misfiled by the collector, and invisible until
/// someone wonders where the dashboards went.
///
/// Under `Grpc` the endpoint genuinely is shared — the path is part of the
/// service definition, not the URL — so the two configs would agree there and
/// disagree only over HTTP. A type that is safe to share for one transport and
/// silently wrong for the other is worse than two types.
///
/// The alternative, redefining `OtlpConfig::endpoint` as a *root* and appending
/// per-signal paths, is a change to a shipped contract every existing host was
/// configured against. It is a reasonable future migration and deliberately not
/// smuggled in here.
#[derive(Debug, Clone)]
pub struct OtlpMetricsConfig {
    /// The OTLP collector endpoint. For `Grpc` this is the base gRPC endpoint
    /// (e.g. `http://localhost:4317`) and is the same value tracing uses. For
    /// `Http` it is used verbatim as the **metrics** URL — the exporter appends
    /// nothing — so pass the full path (e.g.
    /// `http://localhost:4318/v1/metrics`).
    pub endpoint: String,
    /// The wire transport to export over.
    pub protocol: OtlpProtocol,
}

/// An [`Observability`] that records metrics through OpenTelemetry.
///
/// Holds the provider so the host can flush and shut it down; see the module
/// docs for why that lifecycle is not the runtime's.
pub struct OtlpMetrics {
    meter: Meter,
    provider: SdkMeterProvider,
    instruments: Mutex<Instruments>,
}

impl OtlpMetrics {
    /// Builds the whole OTLP metrics pipeline from configuration.
    ///
    /// This is the constructor a deployment uses, and it is the reason this type
    /// is an *OTLP exporter* rather than a generic OpenTelemetry adapter: it
    /// creates the exporter for the configured endpoint and protocol, drives it
    /// from a `PeriodicReader`, and owns the resulting provider. An adopter
    /// supplies an endpoint, not a hand-assembled SDK pipeline.
    ///
    /// Deliberately the same shape as [`crate::tracing_otlp::OtlpTracer::new`],
    /// over its **own** config — see [`OtlpMetricsConfig`] for why sharing the
    /// tracing one would misroute every HTTP export.
    pub fn new(config: OtlpMetricsConfig) -> Result<Self, opentelemetry_otlp::ExporterBuildError> {
        let exporter = match config.protocol {
            OtlpProtocol::Grpc => OtlpMetricExporter::builder()
                .with_tonic()
                .with_endpoint(config.endpoint.clone())
                .build()?,
            OtlpProtocol::Http => OtlpMetricExporter::builder()
                .with_http()
                .with_endpoint(config.endpoint.clone())
                .build()?,
        };
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter).build())
            .build();
        Ok(Self::from_provider(provider))
    }

    /// Build directly from an already-configured provider — for example one
    /// wired to an in-memory reader for tests, or a host that genuinely needs to
    /// own resource attributes and reader cadence itself.
    ///
    /// Public because the second case is real: the acceptance test uses it to
    /// observe what a deployment exports, over the same recording path
    /// [`Self::new`] produces.
    pub fn from_provider(provider: SdkMeterProvider) -> Self {
        // Deliberately not `global::meter(...)`: taking the meter from the
        // provider we were handed binds this adapter to it, so two adapters in
        // one process cannot silently share a backend and nothing depends on a
        // global having been installed as a side effect.
        let meter = provider.meter(SCOPE);
        Self {
            meter,
            provider,
            instruments: Mutex::new(Instruments::default()),
        }
    }

    /// Exports anything pending, without ending the provider's life.
    ///
    /// For a host that wants a checkpoint — before a deploy, or on a signal —
    /// and expects to keep recording afterwards.
    pub fn flush(&self) -> Result<(), opentelemetry_sdk::error::OTelSdkError> {
        self.provider.force_flush()
    }

    /// Shuts the provider down, exporting whatever is pending as it goes.
    ///
    /// Deliberately **not** a flush followed by a shutdown: `shutdown` already
    /// exports, so doing both sends every pending series twice. Harmless for a
    /// cumulative sum, wrong for anything a backend counts per export, and it
    /// made the adapter's own tests read double — which is how it was found.
    ///
    /// The host calls this; the runtime does not. A metric recorded afterwards is
    /// dropped by the SDK, which is what happens to one recorded before any
    /// provider existed too.
    pub fn shutdown(&self) -> Result<(), opentelemetry_sdk::error::OTelSdkError> {
        self.provider.shutdown()
    }

    /// The observation's dimensions, as the SDK's key-value pairs.
    ///
    /// Handed over in the order the emitter wrote them. The SDK does **not**
    /// preserve that order: an attribute set is identified by its members, not
    /// their sequence, and the exported points come back canonically ordered.
    /// That is correct — two observations differing only in attribute order are
    /// the same series — and it is worth stating because a test comparing
    /// exported dimensions positionally will disagree with the emitter's source
    /// order and be right to.
    fn lock_instruments(&self) -> std::sync::MutexGuard<'_, Instruments> {
        self.instruments
            .lock()
            .expect("the instrument cache mutex is never held across a panic")
    }

    fn key_values(observation: &MetricObservation<'_>) -> Vec<KeyValue> {
        observation
            .attributes
            .iter()
            .map(|a| KeyValue::new(a.key, a.value.to_string()))
            .collect()
    }
}

impl Observability for OtlpMetrics {
    /// Not this adapter's concern.
    ///
    /// Semantic events are the tracing adapter's surface, and an implementor
    /// that quietly forwarded them here would give a deployment two paths for
    /// one signal. A host that wants both composes them.
    fn trace(&self, _event: SemanticEvent) {}

    /// # The lock is held to look up an instrument, never to record through one
    ///
    /// Instrument handles are cheap clones over shared state, so the cache guard
    /// is dropped before the SDK call. Holding it across `add`/`record` would
    /// serialise every metric in the process behind one mutex on a path that
    /// runs per request — a contention point invisible in tests and obvious
    /// under load.
    fn record_metric(&self, observation: MetricObservation<'_>) {
        let attributes = Self::key_values(&observation);

        match observation.kind {
            MetricKind::Counter => {
                let instrument = {
                    let mut cache = self.lock_instruments();
                    cache
                        .counters
                        .entry(observation.name)
                        .or_insert_with(|| self.meter.f64_counter(observation.name).build())
                        .clone()
                };
                instrument.add(observation.value, &attributes);
            }
            MetricKind::Gauge => {
                let instrument = {
                    let mut cache = self.lock_instruments();
                    cache
                        .gauges
                        .entry(observation.name)
                        .or_insert_with(|| self.meter.f64_gauge(observation.name).build())
                        .clone()
                };
                instrument.record(observation.value, &attributes);
            }
            MetricKind::Histogram => {
                let instrument = {
                    let mut cache = self.lock_instruments();
                    cache
                        .histograms
                        .entry(observation.name)
                        .or_insert_with(|| self.meter.f64_histogram(observation.name).build())
                        .clone()
                };
                instrument.record(observation.value, &attributes);
            }
        }
    }

    /// Not this adapter's concern; see [`Self::trace`].
    fn log(&self, _level: Level, _message: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::MetricAttribute;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider as Provider,
    };

    /// The adapter under test, plus the exporter it will have written to.
    fn adapter() -> (OtlpMetrics, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = Provider::builder().with_reader(reader).build();
        (OtlpMetrics::from_provider(provider), exporter)
    }

    /// Every exported metric as `(name, kind-label, value, attributes)`.
    ///
    /// The kind is projected as a label rather than compared structurally
    /// because the SDK's exported shape *is* the evidence: a counter arrives as
    /// a `Sum`, a gauge as a `Gauge`, a histogram as a `Histogram`. Reading which
    /// arm it landed in is what proves the translation, and it is exactly the
    /// distinction a folded or mistranslated kind would lose.
    /// One exported series: its name, the arm it landed in, its value, and its
    /// dimensions.
    type Series = (String, &'static str, f64, Vec<(String, String)>);

    fn exported(exporter: &InMemoryMetricExporter) -> Vec<Series> {
        let mut out = Vec::new();
        for resource_metric in exporter
            .get_finished_metrics()
            .expect("metrics are readable")
        {
            for scope in resource_metric.scope_metrics() {
                for metric in scope.metrics() {
                    let name = metric.name().to_string();
                    match metric.data() {
                        AggregatedMetrics::F64(MetricData::Sum(sum)) => {
                            for point in sum.data_points() {
                                out.push((
                                    name.clone(),
                                    "sum",
                                    point.value(),
                                    attrs(point.attributes()),
                                ));
                            }
                        }
                        AggregatedMetrics::F64(MetricData::Gauge(gauge)) => {
                            for point in gauge.data_points() {
                                out.push((
                                    name.clone(),
                                    "gauge",
                                    point.value(),
                                    attrs(point.attributes()),
                                ));
                            }
                        }
                        AggregatedMetrics::F64(MetricData::Histogram(hist)) => {
                            for point in hist.data_points() {
                                out.push((
                                    name.clone(),
                                    "histogram",
                                    point.sum(),
                                    attrs(point.attributes()),
                                ));
                            }
                        }
                        other => panic!("unexpected exported shape for {name}: {other:?}"),
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn attrs<'a>(kvs: impl Iterator<Item = &'a KeyValue>) -> Vec<(String, String)> {
        kvs.map(|kv| (kv.key.to_string(), kv.value.to_string()))
            .collect()
    }

    /// Each kind reaches the backend as its own instrument, carrying its
    /// dimensions.
    ///
    /// Three kinds in one pass rather than three tests, because the failure this
    /// is shaped to catch is a `match` arm pointing at the wrong instrument —
    /// which a single-kind test would pass while the other two were wrong.
    #[test]
    fn each_kind_is_translated_to_its_own_instrument() {
        let (adapter, exporter) = adapter();

        adapter.counter(
            "test.counter",
            2.0,
            &[MetricAttribute::new("outcome", "fresh")],
        );
        adapter.gauge(
            "test.gauge",
            90.0,
            &[MetricAttribute::new("scope", "backlog")],
        );
        adapter.histogram("test.histogram", 12.5, &[]);
        adapter.shutdown().expect("the provider shuts down");

        assert_eq!(
            exported(&exporter),
            vec![
                (
                    "test.counter".to_string(),
                    "sum",
                    2.0,
                    vec![("outcome".to_string(), "fresh".to_string())]
                ),
                (
                    "test.gauge".to_string(),
                    "gauge",
                    90.0,
                    vec![("scope".to_string(), "backlog".to_string())]
                ),
                ("test.histogram".to_string(), "histogram", 12.5, vec![]),
            ],
            "each kind selects its own instrument and carries its own dimensions"
        );
    }

    /// Two observations of one name under different dimensions stay one series
    /// with two attribute sets — never two names.
    ///
    /// This is the property the whole migration away from folded names exists to
    /// produce, and it is the adapter's job not to undo it.
    #[test]
    fn one_name_with_differing_dimensions_stays_one_instrument() {
        let (adapter, exporter) = adapter();

        for outcome in ["fresh", "conflict"] {
            adapter.counter(
                "idempotency.reservation.outcome",
                1.0,
                &[MetricAttribute::new("outcome", outcome)],
            );
        }
        adapter.shutdown().expect("the provider shuts down");

        let rows = exported(&exporter);
        let names: std::collections::BTreeSet<_> = rows.iter().map(|(n, ..)| n.clone()).collect();
        assert_eq!(
            names.len(),
            1,
            "the dimension varies; the instrument name does not: {rows:?}"
        );
        let mut observed: Vec<_> = rows.iter().map(|(_, _, v, a)| (*v, a.clone())).collect();
        observed.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            observed,
            vec![
                (1.0, vec![("outcome".to_string(), "conflict".to_string())]),
                (1.0, vec![("outcome".to_string(), "fresh".to_string())]),
            ],
            "each attribute set is its own data point under the one name"
        );
    }

    /// Recording the same name repeatedly creates one instrument, not one per call.
    ///
    /// A cache miss on every observation would allocate per request and register
    /// duplicates on some backends. The evidence is that the counter accumulates
    /// into a single data point rather than arriving as several.
    #[test]
    fn repeated_observations_reuse_one_instrument() {
        let (adapter, exporter) = adapter();

        for _ in 0..5 {
            adapter.counter("test.repeated", 1.0, &[]);
        }
        adapter.shutdown().expect("the provider shuts down");

        assert_eq!(
            exported(&exporter),
            vec![("test.repeated".to_string(), "sum", 5.0, vec![])],
            "five increments accumulate in one instrument"
        );
    }

    /// An observation with no dimensions arrives with none.
    #[test]
    fn an_observation_with_no_dimensions_carries_none() {
        let (adapter, exporter) = adapter();

        adapter.counter("test.bare", 1.0, &[]);
        adapter.shutdown().expect("the provider shuts down");

        let rows = exported(&exporter);
        assert_eq!(
            rows.first().map(|(_, _, _, a)| a.clone()),
            Some(vec![]),
            "no dimensions were supplied, so none may be synthesised: {rows:?}"
        );
    }
}
