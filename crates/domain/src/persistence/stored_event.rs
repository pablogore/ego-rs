use crate::operation::OperationKey;

/// Wrapper for events stored in an event store.
///
/// Adds optional metadata to the raw event payload without constraining the event
/// type itself.
///
/// # Which of these fields storage actually carries
///
/// `operation_key` round-trips everywhere. An adapter that persists a stored event
/// must write it and return it on load, and the shared conformance harness
/// requires that of every implementation, so the guarantee holds by obligation
/// rather than by coincidence.
///
/// `correlation_id` is **not** guaranteed, and what it does depends on the store.
/// The in-memory implementations keep whole `StoredEvent` values, so it survives
/// there. The PostgreSQL implementation neither writes it nor reconstructs it, so
/// it is dropped there. Nothing in the shared contract requires either behaviour,
/// which is why the two differ — a caller cannot rely on the field, and cannot
/// rely on losing it.
///
/// That divergence is recorded as debt rather than fixed alongside the key: closing
/// it means deciding what the contract should require and then making the durable
/// store meet it, which changes what an existing setter observably does and needs
/// its own verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent<E> {
    /// The domain event payload.
    pub event: E,
    /// Optional correlation identifier for tracing across service boundaries.
    ///
    /// **Persistence is store-dependent and unspecified** — kept by the in-memory
    /// stores, dropped by PostgreSQL. See the type-level note.
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
