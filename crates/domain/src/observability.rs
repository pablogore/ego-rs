//! Observability port - the domain contract for observing runtime behavior.
//!
//! Observability is a **semantic visibility contract**: observing runtime
//! behavior without owning execution. It is runtime-neutral, transport-neutral,
//! and vendor-neutral.
//!
//! ## Responsibility
//!
//! - Capture semantic events (execution, lifecycle, message, failure)
//! - Provide deterministic correlation via `correlation_id`
//! - Support replay-safe observation (identical inputs produce identical events)
//!
//! ## Non-responsibility
//!
//! - Runtime execution or scheduling
//! - Transport or telemetry infrastructure
//! - Persistence lifecycle
//! - Cluster coordination
//!
//! ## Determinism Axiom
//!
//! Given identical inputs, replay produces identical observable semantic
//! events. The trait itself is stateless - all state lives in adapters.
//!
//! ## Fail-closed
//!
//! Invalid event construction (empty event name) is rejected.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A semantic event captured by the observability system.
///
/// Contains the minimal metadata needed for deterministic, replay-safe
/// observation. All fields are immutable once constructed.
///
/// # Deterministic
///
/// `event_name`, `correlation_id`, `actor_id`, and `lifecycle_state`
/// are all deterministic values. `timestamp` is set at construction
/// time and never mutated.
///
/// # Fail-closed
///
/// `SemanticEvent::new()` returns `Err` if `event_name` is empty
/// or whitespace-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticEvent {
    /// The name of the event (e.g. "execution.started", "message.sent").
    pub event_name: String,
    /// Correlation identifier linking related events across the system.
    pub correlation_id: String,
    /// The actor that produced this event.
    pub actor_id: String,
    /// The lifecycle state at the time of the event.
    pub lifecycle_state: String,
    /// When the event occurred (ISO 8601 / RFC 3339).
    pub timestamp: String,
    /// Arbitrary key-value metadata attached to the event.
    pub metadata: HashMap<String, String>,
}

impl SemanticEvent {
    /// Constructors for `SemanticEvent`.
    /// Create a new `SemanticEvent`.
    ///
    /// Returns `Err` if `event_name` is empty or whitespace-only.
    pub fn new(
        event_name: impl Into<String>,
        correlation_id: impl Into<String>,
        actor_id: impl Into<String>,
        lifecycle_state: impl Into<String>,
        timestamp: impl Into<String>,
        metadata: HashMap<String, String>,
    ) -> Result<Self, SemanticEventError> {
        let event_name = event_name.into();
        if event_name.trim().is_empty() {
            return Err(SemanticEventError::EmptyName);
        }
        Ok(Self {
            event_name,
            correlation_id: correlation_id.into(),
            actor_id: actor_id.into(),
            lifecycle_state: lifecycle_state.into(),
            timestamp: timestamp.into(),
            metadata,
        })
    }

    /// Create a new `SemanticEvent` with no metadata.
    pub fn without_metadata(
        event_name: impl Into<String>,
        correlation_id: impl Into<String>,
        actor_id: impl Into<String>,
        lifecycle_state: impl Into<String>,
        timestamp: impl Into<String>,
    ) -> Result<Self, SemanticEventError> {
        Self::new(
            event_name,
            correlation_id,
            actor_id,
            lifecycle_state,
            timestamp,
            HashMap::new(),
        )
    }
}

/// Errors that can occur when constructing a [`SemanticEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticEventError {
    /// The event name was empty or whitespace-only.
    EmptyName,
}

impl std::fmt::Display for SemanticEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => write!(f, "event name must not be empty"),
        }
    }
}

impl std::error::Error for SemanticEventError {}

/// Log level for observability log entries.
///
/// Ordered from least severe to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    /// Debug-level detail.
    Debug,
    /// Informational.
    Info,
    /// Warning - something unexpected but recoverable.
    Warn,
    /// Error - something failed.
    Error,
}

impl Level {
    /// Severity methods for `Level`.
    /// Returns the numeric severity of this level (higher = more severe).
    pub fn severity(&self) -> u8 {
        match self {
            Self::Debug => 0,
            Self::Info => 1,
            Self::Warn => 2,
            Self::Error => 3,
        }
    }
}

/// One dimension of a metric observation: a contract-owned key, a runtime value.
///
/// # Why the key is `&'static str` and the value is not
///
/// The two sides have different owners. A key names a dimension the metric
/// contract declares, so it is known when the code is written and there is no
/// legitimate source for one computed at runtime; typing it `&'static str` makes
/// a key built from a request unrepresentable rather than merely discouraged.
///
/// A value is the opposite: it comes from a registered descriptor or a closed
/// enum, and is only known once something is running. It borrows, so a caller
/// can pass a value it already holds without allocating on a path that may run
/// per request.
///
/// # What a value must not be
///
/// Values are bounded by something the deployment registers — an entity type
/// from the registry, a variant of a closed enum. Raw request input is never a
/// value: it is attacker-influenced and unbounded, and every distinct one it
/// produces becomes a distinct series that never stops accumulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricAttribute<'a> {
    /// The dimension's name, owned by the metric contract.
    pub key: &'static str,
    /// The dimension's value for this observation.
    pub value: &'a str,
}

impl<'a> MetricAttribute<'a> {
    /// Builds one dimension of a metric observation.
    pub fn new(key: &'static str, value: &'a str) -> Self {
        Self { key, value }
    }
}

/// Observability port - the domain contract for capturing runtime behavior.
///
/// This trait defines **what** can be observed, not **how** it is stored
/// or transmitted. Implementations (adapters) live in the infrastructure
/// layer.
///
/// # Non-mutating
///
/// Observability calls MUST NOT alter runtime state or behavior. They are
/// side-effect observers only.
///
/// # Non-blocking
///
/// Implementations MUST NOT perform blocking operations (synchronous I/O,
/// network calls, lock contention under load) inside any method on this
/// trait. Callers on security- and request-critical paths (CORE-012A) invoke
/// these methods synchronously and do not isolate or bound their execution
/// time. Every method is expected to execute in bounded, effectively O(1)
/// time; an implementation with expensive work to do MUST enqueue it and
/// hand off to its own asynchronous/background processing — the same
/// discipline `tracing`, `slog`, and `log4rs` subscribers follow. An
/// implementor that blocks here is a contract violation by that
/// implementor, not a gap the caller is responsible for working around.
///
/// # Deterministic
///
/// The trait itself is stateless. Determinism is ensured by the data
/// carried in `SemanticEvent` (correlation_id, actor_id, lifecycle_state).
///
/// # Compatibility
///
/// Two source-breaking changes were made to the metric surface, and both are
/// deliberate rather than incidental:
///
/// - [`metric_with_attributes`] is **required**, and [`metric`] became a default
///   that delegates to it. An implementor that previously defined only `metric`
///   must now define `metric_with_attributes` instead. The default was pointed
///   this way on purpose: with it reversed, every implementor written before
///   dimensions existed would keep compiling and keep discarding them.
/// - [`metric`]'s `name` narrowed from `&str` to `&'static str`. A metric name is
///   a stable part of the contract, known when the code is written; a name
///   computed at runtime is what folding a dimension into it looks like, and this
///   makes that unrepresentable. Dimensional values live in [`MetricAttribute`],
///   which borrows precisely because they are not static.
///
/// Both break only out-of-tree implementors and callers; every in-tree call site
/// already passed a `'static` name. Anything depending on this crate across a
/// release boundary needs the two edits above.
///
/// [`metric`]: Observability::metric
/// [`metric_with_attributes`]: Observability::metric_with_attributes
pub trait Observability: Send + Sync {
    /// Record a semantic event.
    ///
    /// Semantic events carry structured metadata (correlation_id, actor_id,
    /// lifecycle_state) enabling deterministic tracing and replay.
    ///
    /// Called synchronously on security-denial paths (CORE-012A) with no
    /// blocking isolation — see the trait's "Non-blocking" contract above.
    fn trace(&self, event: SemanticEvent);

    /// Record a metric value carrying its dimensions.
    ///
    /// This is the method implementors must provide. [`metric`] delegates here
    /// with no attributes, so an implementor cannot claim to support metrics
    /// while silently discarding the dimensions a caller supplied — which is the
    /// failure this direction of defaulting exists to prevent. Had the default
    /// gone the other way, every implementor written before dimensions existed
    /// would keep compiling and keep dropping them.
    ///
    /// # Why dimensions are not folded into the name
    ///
    /// A closed set of values can be encoded in the name without much harm; an
    /// open one cannot. Folding an application-defined value moves cardinality
    /// out of the attributes and into the name space, producing one series name
    /// per value, breaking aggregation across them and leaving a dashboard that
    /// has to enumerate names it cannot know in advance.
    ///
    /// # Contract for implementors
    ///
    /// Attributes are borrowed for the duration of the call only. An
    /// implementor MUST consume or copy what it needs before returning and MUST
    /// NOT retain the references — the caller is free to build them on its own
    /// stack frame, and typically does.
    ///
    /// [`metric`]: Observability::metric
    fn metric_with_attributes(
        &self,
        name: &'static str,
        value: f64,
        attributes: &[MetricAttribute<'_>],
    );

    /// Record a metric value with no dimensions.
    ///
    /// A convenience over [`metric_with_attributes`] for the signals that carry
    /// none, so those call sites do not have to spell out an empty slice.
    ///
    /// Metrics are named, numeric observations (e.g. "request.duration",
    /// "queue.depth").
    ///
    /// [`metric_with_attributes`]: Observability::metric_with_attributes
    fn metric(&self, name: &'static str, value: f64) {
        self.metric_with_attributes(name, value, &[]);
    }

    /// Record a log entry.
    ///
    /// Log entries carry a severity level and a human-readable message.
    fn log(&self, level: Level, message: &str);
}

#[cfg(test)]
mod metric_attribute_tests {
    use super::*;
    use std::sync::Mutex;

    /// One observation as it arrived: name, value, and the dimensions it carried.
    ///
    /// The attribute values are owned rather than borrowed on purpose — copying
    /// them here is what proves the implementor did not need to retain the
    /// caller's references.
    type RecordedCall = (&'static str, f64, Vec<(&'static str, String)>);

    /// Records what actually reached the required method.
    ///
    /// It deliberately implements **only** `metric_with_attributes`, because that
    /// is the whole claim under test: an implementor written this way must still
    /// serve callers that go through `metric`.
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<RecordedCall>>,
    }

    impl Observability for Recorder {
        fn trace(&self, _event: SemanticEvent) {}
        fn metric_with_attributes(
            &self,
            name: &'static str,
            value: f64,
            attributes: &[MetricAttribute<'_>],
        ) {
            self.calls.lock().unwrap().push((
                name,
                value,
                attributes
                    .iter()
                    .map(|a| (a.key, a.value.to_string()))
                    .collect(),
            ));
        }
        fn log(&self, _level: Level, _message: &str) {}
    }

    /// An attribute keeps the contract's key and the caller's value apart.
    ///
    /// Asserted as two separate fields rather than as one formatted string,
    /// because a swap of the two is exactly the mistake this type exists to make
    /// impossible to express silently.
    #[test]
    fn an_attribute_carries_a_contract_key_and_a_runtime_value() {
        let aggregate_type = String::from("User");
        let attribute = MetricAttribute::new("aggregate_type", &aggregate_type);

        assert_eq!(
            attribute.key, "aggregate_type",
            "the key names the dimension"
        );
        assert_eq!(attribute.value, "User", "the value is this observation's");
    }

    /// `metric` reaches the required method carrying no dimensions.
    ///
    /// This is the direction of the default, and it is the load-bearing part: an
    /// implementor cannot satisfy the trait by writing `metric` alone, so it
    /// cannot accept dimensions and quietly drop them.
    #[test]
    fn metric_delegates_to_the_attributed_method_with_none() {
        let recorder = Recorder::default();

        recorder.metric("idempotency.purge.rows", 7.0);

        let calls = recorder.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "one call reached the required method");
        assert_eq!(
            calls[0].0, "idempotency.purge.rows",
            "the name is unchanged"
        );
        assert_eq!(calls[0].1, 7.0, "the value is unchanged");
        assert!(
            calls[0].2.is_empty(),
            "a metric with no dimensions arrives with none, not with a placeholder: {:?}",
            calls[0].2
        );
    }

    /// Dimensions arrive whole, in the order given, keys and values unswapped.
    ///
    /// Two attributes rather than one, and neither value equal to the other's
    /// key, so a transposition anywhere in the path fails rather than producing a
    /// coincidentally identical record.
    #[test]
    fn attributes_arrive_in_order_with_keys_and_values_unswapped() {
        let recorder = Recorder::default();
        let aggregate_type = String::from("TenantOrganization");

        recorder.metric_with_attributes(
            "idempotency.receipt.outcome",
            1.0,
            &[
                MetricAttribute::new("outcome", "confirmed"),
                MetricAttribute::new("aggregate_type", &aggregate_type),
            ],
        );

        let calls = recorder.calls.lock().unwrap();
        assert_eq!(
            calls[0].2,
            vec![
                ("outcome", "confirmed".to_string()),
                ("aggregate_type", "TenantOrganization".to_string()),
            ],
            "both dimensions arrive, in order, each key still paired with its own value"
        );
    }

    /// The name stays one stable series regardless of what the dimensions carry.
    ///
    /// This is the property folding a value into the name would destroy: two
    /// observations differing only by dimension must remain the same metric, or
    /// nothing downstream can aggregate across them.
    #[test]
    fn the_name_is_one_stable_series_across_differing_dimensions() {
        let recorder = Recorder::default();

        for aggregate in ["User", "TenantOrganization"] {
            recorder.metric_with_attributes(
                "idempotency.receipt.outcome",
                1.0,
                &[MetricAttribute::new("aggregate_type", aggregate)],
            );
        }

        let calls = recorder.calls.lock().unwrap();
        let names: Vec<_> = calls.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(
            names,
            vec!["idempotency.receipt.outcome", "idempotency.receipt.outcome"],
            "the dimension varies, the series name does not"
        );
    }

    /// A borrowed value outliving nothing: the implementor copies during the call.
    ///
    /// The attribute borrows from a local that is dropped immediately after, which
    /// only compiles and only holds if the recorder took what it needed while the
    /// call was running rather than retaining the reference.
    #[test]
    fn a_value_borrowed_from_a_temporary_survives_as_recorded_data() {
        let recorder = Recorder::default();
        {
            let scoped = String::from("Order");
            recorder.metric_with_attributes(
                "idempotency.receipt.outcome",
                1.0,
                &[MetricAttribute::new("aggregate_type", &scoped)],
            );
        }

        let calls = recorder.calls.lock().unwrap();
        assert_eq!(
            calls[0].2,
            vec![("aggregate_type", "Order".to_string())],
            "the implementor copied the value rather than retaining the borrow"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_event_valid() {
        let event = SemanticEvent::without_metadata(
            "execution.started",
            "corr-1",
            "actor-1",
            "Running",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(event.event_name, "execution.started");
        assert_eq!(event.correlation_id, "corr-1");
        assert_eq!(event.actor_id, "actor-1");
        assert_eq!(event.lifecycle_state, "Running");
        assert_eq!(event.timestamp, "2025-01-01T00:00:00Z");
        assert!(event.metadata.is_empty());
    }

    #[test]
    fn semantic_event_with_metadata() {
        let mut meta = HashMap::new();
        meta.insert("key".to_string(), "value".to_string());
        let event = SemanticEvent::new(
            "message.sent",
            "corr-2",
            "actor-2",
            "Running",
            "2025-06-01T12:00:00Z",
            meta,
        )
        .unwrap();
        assert_eq!(event.event_name, "message.sent");
        assert_eq!(event.metadata.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn semantic_event_empty_name_rejected() {
        let result = SemanticEvent::without_metadata(
            "",
            "corr-1",
            "actor-1",
            "Running",
            "2025-01-01T00:00:00Z",
        );
        assert_eq!(result, Err(SemanticEventError::EmptyName));
    }

    #[test]
    fn semantic_event_whitespace_name_rejected() {
        let result = SemanticEvent::without_metadata(
            "   ",
            "corr-1",
            "actor-1",
            "Running",
            "2025-01-01T00:00:00Z",
        );
        assert_eq!(result, Err(SemanticEventError::EmptyName));
    }

    #[test]
    fn semantic_event_error_display() {
        let err = SemanticEventError::EmptyName;
        assert_eq!(format!("{}", err), "event name must not be empty");
    }

    #[test]
    fn level_severity_ordering() {
        assert_eq!(Level::Debug.severity(), 0);
        assert_eq!(Level::Info.severity(), 1);
        assert_eq!(Level::Warn.severity(), 2);
        assert_eq!(Level::Error.severity(), 3);
    }

    #[test]
    fn level_equality() {
        assert_eq!(Level::Debug, Level::Debug);
        assert_ne!(Level::Debug, Level::Info);
        assert_ne!(Level::Warn, Level::Error);
    }

    #[test]
    fn level_clone_copy() {
        let level = Level::Warn;
        let cloned = level;
        let copied = level;
        assert_eq!(level, cloned);
        assert_eq!(level, copied);
    }

    #[test]
    fn semantic_event_serialization() {
        let mut meta = HashMap::new();
        meta.insert("key".to_string(), "value".to_string());
        let event = SemanticEvent::new(
            "test.event",
            "corr-1",
            "actor-1",
            "Running",
            "2025-01-01T00:00:00Z",
            meta,
        )
        .unwrap();
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: SemanticEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn semantic_event_deterministic_serialization() {
        let mut meta1 = HashMap::new();
        meta1.insert("a".to_string(), "1".to_string());
        let mut meta2 = HashMap::new();
        meta2.insert("a".to_string(), "1".to_string());
        let event1 = SemanticEvent::new(
            "test.event",
            "corr-1",
            "actor-1",
            "Running",
            "2025-01-01T00:00:00Z",
            meta1,
        )
        .unwrap();
        let event2 = SemanticEvent::new(
            "test.event",
            "corr-1",
            "actor-1",
            "Running",
            "2025-01-01T00:00:00Z",
            meta2,
        )
        .unwrap();
        assert_eq!(event1, event2);
        assert_eq!(
            serde_json::to_string(&event1).unwrap(),
            serde_json::to_string(&event2).unwrap()
        );
    }
}
