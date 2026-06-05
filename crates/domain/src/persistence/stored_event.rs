/// Wrapper for events stored in an event store.
///
/// Adds optional metadata (e.g., `correlation_id`) to the raw event payload
/// without constraining the event type itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent<E> {
    /// The domain event payload.
    pub event: E,
    /// Optional correlation identifier for tracing across service boundaries.
    pub correlation_id: Option<String>,
}

impl<E> StoredEvent<E> {
    /// Create a new stored event with the given payload and optional correlation id.
    pub fn new(event: E, correlation_id: Option<String>) -> Self {
        StoredEvent {
            event,
            correlation_id,
        }
    }

    /// Create a stored event without a correlation id.
    pub fn without_correlation(event: E) -> Self {
        StoredEvent {
            event,
            correlation_id: None,
        }
    }
}
