//! One shared proof that an `Observability` implementor preserves the dimensions
//! it is handed.
//!
//! The port's required method takes attributes by borrow, which means an
//! implementor can satisfy the compiler while binding them to `_` and throwing
//! them away. That is precisely the failure the port's shape was changed to
//! prevent, and it is invisible from the outside: names and values still arrive,
//! assertions on them still pass, and only the dimensions are gone.
//!
//! Every implementor is therefore checked against the same probe rather than each
//! writing its own — one definition of "preserved" that cannot drift between
//! doubles, and a single place to strengthen when the contract grows.

use ego_domain::{MetricAttribute, Observability};

/// One emission as it arrived: its name, its value, and the dimensions it carried.
///
/// Shared rather than redefined per fixture, because the reason for its shape is a
/// single invariant that must not drift. The three fields live in one record so a
/// recorder can append them inside one critical section; parallel collections can
/// only stay aligned if every writer holds every lock at once, and two concurrent
/// emissions would otherwise leave one call's name paired with another call's
/// dimensions — a fixture that reports a false result in either direction.
///
/// Values are owned rather than borrowed: copying them on arrival is what proves
/// the recorder did not retain the caller's references, which the port forbids.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedMetric {
    /// The metric name this emission carried.
    pub name: String,
    /// The observed value.
    pub value: f64,
    /// The dimensions, as `(key, value)` pairs, in the order supplied.
    pub attributes: Vec<(String, String)>,
}

impl RecordedMetric {
    /// Captures one emission, copying the dimensions out of the borrowed slice.
    ///
    /// Every recorder builds its record through this, so "what counts as recording
    /// an emission" is defined once and cannot drift between fixtures.
    ///
    /// # Why even a trace-focused double records these
    ///
    /// Several doubles assert on semantic events, not counters, and have no
    /// reason of their own to keep metrics. They keep them anyway, because a
    /// double that binds the dimensions to `_` is indistinguishable from one that
    /// supports them — exactly the confusion the port's shape was changed to
    /// prevent. Recording is also append-only: keeping just the most recent
    /// emission would silently drop every earlier one, so a flow emitting more
    /// than one metric would answer for the last and nothing could detect the
    /// loss.
    pub fn capture(name: &str, value: f64, attributes: &[MetricAttribute<'_>]) -> Self {
        Self {
            name: name.to_string(),
            value,
            attributes: attributes
                .iter()
                .map(|a| (a.key.to_string(), a.value.to_string()))
                .collect(),
        }
    }
}

/// The metric name the probe emits under.
///
/// Deliberately not one of the real signal names, so a fixture that filters or
/// asserts on production metrics is unaffected by having been probed.
pub const PROBE_METRIC: &str = "testkit.observability.conformance.probe";

/// Asserts that `observability` kept both dimensions of one emission, in order,
/// with each key still paired to its own value.
///
/// `read_back` must return the attributes the implementor recorded for the most
/// recent emission, as `(key, value)` pairs.
///
/// # What the probe is shaped to catch
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
pub fn assert_metric_attributes_are_preserved<O, F>(observability: &O, read_back: F)
where
    O: Observability + ?Sized,
    F: FnOnce() -> Vec<(String, String)>,
{
    observability.metric_with_attributes(
        PROBE_METRIC,
        1.0,
        &[
            MetricAttribute::new("probe_first", "alpha"),
            MetricAttribute::new("probe_second", "beta"),
        ],
    );

    assert_eq!(
        read_back(),
        vec![
            ("probe_first".to_string(), "alpha".to_string()),
            ("probe_second".to_string(), "beta".to_string()),
        ],
        "the implementor must record both dimensions, in order, each key with its own value — \
         binding the attributes slice to `_` lets a future emitter's dimensions vanish silently"
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
        attributes: Mutex<Vec<(String, String)>>,
    }

    impl Observability for Preserving {
        fn trace(&self, _event: SemanticEvent) {}
        fn metric_with_attributes(
            &self,
            _name: &'static str,
            _value: f64,
            attributes: &[MetricAttribute<'_>],
        ) {
            *self.attributes.lock().unwrap() = attributes
                .iter()
                .map(|a| (a.key.to_string(), a.value.to_string()))
                .collect();
        }
        fn log(&self, _level: Level, _message: &str) {}
    }

    /// Throws them away, which is the shape the probe exists to reject.
    #[derive(Default)]
    struct Discarding;

    impl Observability for Discarding {
        fn trace(&self, _event: SemanticEvent) {}
        fn metric_with_attributes(
            &self,
            _name: &'static str,
            _value: f64,
            _attributes: &[MetricAttribute<'_>],
        ) {
        }
        fn log(&self, _level: Level, _message: &str) {}
    }

    #[test]
    fn a_preserving_implementor_passes() {
        let obs = Preserving::default();
        assert_metric_attributes_are_preserved(&obs, || obs.attributes.lock().unwrap().clone());
    }

    /// The probe has teeth: an implementor that discards fails it.
    ///
    /// Without this, the helper could be vacuous — asserting something every
    /// implementor satisfies — and every call site would be decoration.
    #[test]
    #[should_panic(expected = "must record both dimensions")]
    fn a_discarding_implementor_fails() {
        let obs = Discarding;
        assert_metric_attributes_are_preserved(&obs, Vec::new);
    }
}
