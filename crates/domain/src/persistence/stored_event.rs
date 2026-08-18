use crate::operation::OperationKey;

/// Wrapper for events stored in an event store.
///
/// Adds optional metadata to the raw event payload without constraining the event
/// type itself.
///
/// # Every field here round-trips, by obligation
///
/// `operation_key` is written and returned by every adapter, and the shared
/// conformance harness requires that of each one — so the guarantee holds because
/// something enforces it, not because the implementations happen to agree.
///
/// That obligation now covers the whole type. The harness destructures a loaded
/// event **without `..`**, so adding a field here breaks its compilation until
/// someone states how the new field must be verified. A field whose persistence
/// nobody specified is exactly how `correlation_id` came to mean two different
/// things in two stores, until it was withdrawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent<E> {
    /// The domain event payload.
    pub event: E,
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
    /// Wraps an event for storage.
    ///
    /// The only constructor. It previously took an `Option<String>` correlation id
    /// alongside a `without_correlation` shorthand; with that field withdrawn,
    /// the two collapsed into this one. The shorthand's name went with it
    /// rather than being kept for the smaller diff: a name defined by a capability
    /// that no longer exists sends a reader looking for a "with correlation"
    /// variant there is nothing to find.
    pub fn new(event: E) -> Self {
        StoredEvent {
            event,
            operation_key: None,
        }
    }

    /// Attaches the operation this event was produced by.
    ///
    /// A builder step rather than another constructor parameter: a caller that has
    /// no operation key does not have to say so by passing `None` to a parameter it
    /// does not use.
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
    fn a_new_stored_event_starts_without_an_operation_key() {
        assert_eq!(StoredEvent::new("e").operation_key, None);
    }

    #[test]
    fn attaching_an_operation_key_leaves_the_payload_alone() {
        let key = OperationKey::parse("op-1").expect("valid");
        let stored = StoredEvent::new("e").with_operation_key(key.clone());

        assert_eq!(stored.event, "e");
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
        let stored = StoredEvent::new("e")
            .with_operation_key(first)
            .with_operation_key(second.clone());

        assert_eq!(stored.operation_key, Some(second));
    }
}
