//! One shared proof that an `Observability` implementor preserves the whole
//! observation it is handed.
//!
//! The port's required method takes a single record, which keeps the four fields
//! associated but does nothing to make an implementor honour them: one can read
//! three and ignore the fourth and still satisfy the compiler. No signature can
//! close that, which is why the check is a test rather than a type. The failure is
//! also invisible from the outside — names and values still arrive, assertions on
//! them still pass, and only the kind, or only the dimensions, are gone.
//!
//! Every implementor is therefore checked against the same probe rather than each
//! writing its own — one definition of "preserved" that cannot drift between
//! doubles, and a single place to strengthen when the contract grows.

use ego_domain::{MetricKind, MetricObservation, Observability};

/// One emission as it arrived: how it aggregates, its name, its value, and the
/// dimensions it carried.
///
/// Shared rather than redefined per fixture, because the reason for its shape is a
/// single invariant that must not drift. The four fields live in one record so a
/// recorder can append them inside one critical section; parallel collections can
/// only stay aligned if every writer holds every lock at once, and two concurrent
/// emissions would otherwise leave one call's name paired with another call's
/// dimensions — a fixture that reports a false result in either direction.
///
/// Values are owned rather than borrowed: copying them on arrival is what proves
/// the recorder did not retain the caller's references, which the port forbids.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedMetric {
    /// How this emission aggregates.
    pub kind: MetricKind,
    /// The metric name this emission carried.
    pub name: String,
    /// The observed value.
    pub value: f64,
    /// The dimensions, as `(key, value)` pairs, in the order supplied.
    pub attributes: Vec<(String, String)>,
}

impl RecordedMetric {
    /// Captures one emission, copying the observation out of the borrowed record.
    ///
    /// Every recorder builds its record through this, so "what counts as recording
    /// an emission" is defined once and cannot drift between fixtures.
    ///
    /// # Why even a trace-focused double records these
    ///
    /// Several doubles assert on semantic events, not counters, and have no
    /// reason of their own to keep metrics. They keep them anyway, because a
    /// double that ignores a field is indistinguishable from one that supports it
    /// until something reads the field back — which is what this probe exists to
    /// do, since no signature can rule the case out. Recording
    /// is also append-only: keeping just the most recent emission would silently
    /// drop every earlier one, so a flow emitting more than one metric would
    /// answer for the last and nothing could detect the loss.
    pub fn capture(observation: &MetricObservation<'_>) -> Self {
        Self {
            kind: observation.kind,
            name: observation.name.to_string(),
            value: observation.value,
            attributes: observation
                .attributes
                .iter()
                .map(|a| (a.key.to_string(), a.value.to_string()))
                .collect(),
        }
    }
}

/// The metric names the probe emits under, one per kind.
///
/// Deliberately not any of the real signal names, so a fixture that filters or
/// asserts on production metrics is unaffected by having been probed.
pub const PROBE_COUNTER_METRIC: &str = "testkit.observability.conformance.probe.counter";
/// See [`PROBE_COUNTER_METRIC`].
pub const PROBE_HISTOGRAM_METRIC: &str = "testkit.observability.conformance.probe.histogram";
/// See [`PROBE_COUNTER_METRIC`].
pub const PROBE_GAUGE_METRIC: &str = "testkit.observability.conformance.probe.gauge";

/// Asserts that `observability` kept every field of every observation it was
/// handed: kind, name, value, and both dimensions in order.
///
/// `read_back` must return every emission the implementor recorded, oldest first.
/// The probe compares against the **last three**, so a double that also records
/// production signals passes without having to be cleared first.
///
/// # What the probe is shaped to catch
///
/// One observation per kind, so an implementor that collapses all three to a
/// single variant — or hardcodes one — fails rather than passing on the one kind
/// it happens to handle. The three carry different values as well as different
/// names, so a record assembled from the wrong call is not coincidentally equal.
///
/// Two attributes, not one, so dropping only the tail is caught. No value equals
/// any key, and the two pairs share no text, so a transposition of key and value
/// — or of one attribute with the other — cannot produce a coincidentally equal
/// record.
///
/// # Panics
///
/// Panics with the difference if the implementor dropped, reordered, or
/// transposed anything.
pub fn assert_metric_observations_are_preserved<O, F>(observability: &O, read_back: F)
where
    O: Observability + ?Sized,
    F: FnOnce() -> Vec<RecordedMetric>,
{
    let attributes = [
        ego_domain::MetricAttribute::new("probe_first", "alpha"),
        ego_domain::MetricAttribute::new("probe_second", "beta"),
    ];

    observability.counter(PROBE_COUNTER_METRIC, 1.0, &attributes);
    observability.histogram(PROBE_HISTOGRAM_METRIC, 2.5, &attributes);
    observability.gauge(PROBE_GAUGE_METRIC, 3.0, &attributes);

    let expected_attributes = vec![
        ("probe_first".to_string(), "alpha".to_string()),
        ("probe_second".to_string(), "beta".to_string()),
    ];
    let expected = vec![
        RecordedMetric {
            kind: MetricKind::Counter,
            name: PROBE_COUNTER_METRIC.to_string(),
            value: 1.0,
            attributes: expected_attributes.clone(),
        },
        RecordedMetric {
            kind: MetricKind::Histogram,
            name: PROBE_HISTOGRAM_METRIC.to_string(),
            value: 2.5,
            attributes: expected_attributes.clone(),
        },
        RecordedMetric {
            kind: MetricKind::Gauge,
            name: PROBE_GAUGE_METRIC.to_string(),
            value: 3.0,
            attributes: expected_attributes,
        },
    ];

    let recorded = read_back();
    let observed: Vec<_> = recorded
        .iter()
        .rev()
        .take(expected.len())
        .rev()
        .cloned()
        .collect();

    assert_eq!(
        observed, expected,
        "the implementor must record every field of every observation — its kind, its name, \
         its value, and both dimensions in order, each key with its own value. Ignoring one \
         field lets a future emitter's kind or dimensions vanish silently"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::{Level, SemanticEvent};
    use std::sync::Mutex;

    /// Keeps what it is given.
    #[derive(Default)]
    struct Preserving {
        metrics: Mutex<Vec<RecordedMetric>>,
    }

    impl Observability for Preserving {
        fn trace(&self, _event: SemanticEvent) {}
        fn record_metric(&self, observation: MetricObservation<'_>) {
            self.metrics
                .lock()
                .unwrap()
                .push(RecordedMetric::capture(&observation));
        }
        fn log(&self, _level: Level, _message: &str) {}
    }

    /// Throws the dimensions away, which is one shape the probe exists to reject.
    #[derive(Default)]
    struct DiscardingAttributes {
        metrics: Mutex<Vec<RecordedMetric>>,
    }

    impl Observability for DiscardingAttributes {
        fn trace(&self, _event: SemanticEvent) {}
        fn record_metric(&self, observation: MetricObservation<'_>) {
            self.metrics.lock().unwrap().push(RecordedMetric {
                kind: observation.kind,
                name: observation.name.to_string(),
                value: observation.value,
                attributes: Vec::new(),
            });
        }
        fn log(&self, _level: Level, _message: &str) {}
    }

    /// Records everything except which kind it was, which is the failure the
    /// typed surface exists to make detectable.
    ///
    /// It reads three of four fields and substitutes a plausible default for the
    /// fourth — the exact shape that would compile, look complete at every call
    /// site, and export a histogram as a counter.
    #[derive(Default)]
    struct FlatteningKind {
        metrics: Mutex<Vec<RecordedMetric>>,
    }

    impl Observability for FlatteningKind {
        fn trace(&self, _event: SemanticEvent) {}
        fn record_metric(&self, observation: MetricObservation<'_>) {
            self.metrics.lock().unwrap().push(RecordedMetric {
                kind: MetricKind::Counter,
                name: observation.name.to_string(),
                value: observation.value,
                attributes: observation
                    .attributes
                    .iter()
                    .map(|a| (a.key.to_string(), a.value.to_string()))
                    .collect(),
            });
        }
        fn log(&self, _level: Level, _message: &str) {}
    }

    #[test]
    fn a_preserving_implementor_passes() {
        let obs = Preserving::default();
        assert_metric_observations_are_preserved(&obs, || obs.metrics.lock().unwrap().clone());
    }

    /// A double that already recorded other emissions still passes.
    ///
    /// The probe compares the tail rather than the whole log, so a fixture that
    /// captures production signals does not have to be drained before it can be
    /// checked for conformance.
    #[test]
    fn earlier_unrelated_emissions_do_not_disturb_the_probe() {
        let obs = Preserving::default();
        obs.counter("some.earlier.signal", 41.0, &[]);

        assert_metric_observations_are_preserved(&obs, || obs.metrics.lock().unwrap().clone());
    }

    /// The probe has teeth: an implementor that discards dimensions fails it.
    ///
    /// Without this, the helper could be vacuous — asserting something every
    /// implementor satisfies — and every call site would be decoration.
    #[test]
    #[should_panic(expected = "must record every field")]
    fn an_implementor_discarding_attributes_fails() {
        let obs = DiscardingAttributes::default();
        assert_metric_observations_are_preserved(&obs, || obs.metrics.lock().unwrap().clone());
    }

    /// And teeth on the kind specifically: preserving the other three fields is
    /// not enough.
    ///
    /// This is the case the older attribute-only probe could not see. It passed
    /// every assertion that existed before the kind did.
    #[test]
    #[should_panic(expected = "must record every field")]
    fn an_implementor_flattening_the_kind_fails() {
        let obs = FlatteningKind::default();
        assert_metric_observations_are_preserved(&obs, || obs.metrics.lock().unwrap().clone());
    }
}
