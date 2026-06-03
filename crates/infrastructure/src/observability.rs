//! Infrastructure implementations of the observability port.
//!
//! Provides `NoopObservability` for production when observability is disabled.

use ego_domain::{Level, Observability, SemanticEvent};

/// A no-op implementation of [`Observability`] that discards all events.
///
/// Use this when observability is disabled in production.
#[derive(Debug, Clone, Default)]
pub struct NoopObservability;

impl Observability for NoopObservability {
    fn trace(&self, _event: SemanticEvent) {}
    fn metric(&self, _name: &str, _value: f64) {}
    fn log(&self, _level: Level, _message: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn noop_observation_discards_all() {
        let noop = NoopObservability::default();
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
        noop.trace(event);
        noop.metric("test.metric", 42.0);
        noop.log(Level::Info, "test message");
    }

    #[test]
    fn noop_clone_default() {
        let noop1 = NoopObservability::default();
        let noop2 = noop1.clone();
        noop1.trace(SemanticEvent::without_metadata(
            "test",
            "c1",
            "a1",
            "Running",
            "2025-01-01T00:00:00Z",
        )
        .unwrap());
        noop2.trace(SemanticEvent::without_metadata(
            "test",
            "c2",
            "a2",
            "Running",
            "2025-01-01T00:00:00Z",
        )
        .unwrap());
    }
}
