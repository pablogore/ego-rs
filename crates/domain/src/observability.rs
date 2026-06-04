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
/// # Deterministic
///
/// The trait itself is stateless. Determinism is ensured by the data
/// carried in `SemanticEvent` (correlation_id, actor_id, lifecycle_state).
pub trait Observability: Send + Sync {
    /// Record a semantic event.
    ///
    /// Semantic events carry structured metadata (correlation_id, actor_id,
    /// lifecycle_state) enabling deterministic tracing and replay.
    fn trace(&self, event: SemanticEvent);

    /// Record a metric value.
    ///
    /// Metrics are named, numeric observations (e.g. "request.duration",
    /// "queue.depth").
    fn metric(&self, name: &str, value: f64);

    /// Record a log entry.
    ///
    /// Log entries carry a severity level and a human-readable message.
    fn log(&self, level: Level, message: &str);
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
