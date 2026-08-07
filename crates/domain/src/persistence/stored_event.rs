use crate::operation::OperationKey;

/// Wrapper for events stored in an event store.
///
/// Adds optional metadata to the raw event payload without constraining the event
/// type itself.
///
/// # Which of these fields storage actually carries
///
/// `operation_key` round-trips: an adapter that persists a stored event must write
/// it and return it on load, and the shared conformance harness requires that of
/// every implementation.
///
/// `correlation_id` does **not**. No adapter has ever read or written it — the
/// field exists on the type and is discarded at the boundary. That is stated here
/// rather than left for a caller to discover after trusting it, and it is recorded
/// as debt rather than fixed alongside the key: making it round-trip changes what
/// an existing setter does, which needs its own slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent<E> {
    /// The domain event payload.
    pub event: E,
    /// Optional correlation identifier for tracing across service boundaries.
    ///
    /// **Not persisted.** See the type-level note.
    pub correlation_id: Option<String>,
    /// The client-supplied operation this event was produced by, when there was
    /// one.
    ///
    /// `Option` because not every event comes from an externally-keyed operation:
    /// an event replayed by a projection, or one produced by an internal timer, has
    /// no client operation behind it. Storing the key alongside the event is what
    /// lets a later reader tell which operation wrote which history — the question
    /// a duplicate-suppression decision has to answer about events that already
    /// exist.
    pub operation_key: Option<OperationKey>,
}

impl<E> StoredEvent<E> {
    /// Create a new stored event with the given payload and optional correlation id.
    pub fn new(event: E, correlation_id: Option<String>) -> Self {
        StoredEvent {
            event,
            correlation_id,
            operation_key: None,
        }
    }

    /// Create a stored event without a correlation id.
    pub fn without_correlation(event: E) -> Self {
        StoredEvent {
            event,
            correlation_id: None,
            operation_key: None,
        }
    }

    /// Attaches the operation this event was produced by.
    ///
    /// A builder step rather than another constructor parameter: every existing
    /// call site keeps compiling, and a caller that has no operation key does not
    /// have to say so by passing `None` to a parameter it does not use.
    pub fn with_operation_key(mut self, operation_key: OperationKey) -> Self {
        self.operation_key = Some(operation_key);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored event carries no operation key unless one is attached.
    ///
    /// The default matters: an event with a key it never had would claim an
    /// operation produced it, which is the fact a duplicate-suppression decision
    /// reads.
    #[test]
    fn both_constructors_start_without_an_operation_key() {
        assert_eq!(StoredEvent::without_correlation("e").operation_key, None);
        assert_eq!(
            StoredEvent::new("e", Some("corr".to_string())).operation_key,
            None
        );
    }

    #[test]
    fn attaching_an_operation_key_leaves_the_other_fields_alone() {
        let key = OperationKey::parse("op-1").expect("valid");
        let stored =
            StoredEvent::new("e", Some("corr".to_string())).with_operation_key(key.clone());

        assert_eq!(stored.event, "e");
        assert_eq!(stored.correlation_id, Some("corr".to_string()));
        assert_eq!(stored.operation_key, Some(key));
    }

    /// Attaching twice keeps the last key rather than refusing or accumulating.
    ///
    /// Pinning the choice rather than leaving it to be discovered. A builder step
    /// that silently ignored the second call would make a caller's later, more
    /// specific attachment vanish.
    #[test]
    fn attaching_twice_keeps_the_last_key() {
        let first = OperationKey::parse("op-1").expect("valid");
        let second = OperationKey::parse("op-2").expect("valid");
        let stored = StoredEvent::without_correlation("e")
            .with_operation_key(first)
            .with_operation_key(second.clone());

        assert_eq!(stored.operation_key, Some(second));
    }
}
