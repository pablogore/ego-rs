//! Event stream element — a tagged event from the event store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::event_tag::EventTag;

/// A tagged event from the event store.
///
/// Immutable snapshot of a stored event plus precomputed metadata.
/// The universal consumption unit for the read side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventStreamElement<E> {
    /// Globally unique event identifier.
    pub event_id: String,
    /// The aggregate that produced this event.
    pub aggregate_id: String,
    /// Multi-tenant scope.
    pub tenant_id: String,
    /// Discriminant for routing (e.g., "OrderPlaced").
    pub event_type: String,
    /// The event data (generic).
    pub payload: E,
    /// Monotonic version within tag stream (>= 1, may have gaps).
    pub event_version: i64,
    /// Wall-clock timestamp in UTC.
    pub occurred_at: DateTime<Utc>,
    /// Precomputed partition keys.
    pub tags: Vec<EventTag>,
}

impl<E> EventStreamElement<E> {
    /// Creates a new `EventStreamElement`.
    ///
    /// # Panics
    /// Panics if `event_id`, `aggregate_id`, `tenant_id`, or `event_type` is empty,
    /// if `event_version < 1`, or if `tags` is empty.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: impl Into<String>,
        aggregate_id: impl Into<String>,
        tenant_id: impl Into<String>,
        event_type: impl Into<String>,
        payload: E,
        event_version: i64,
        occurred_at: DateTime<Utc>,
        tags: Vec<EventTag>,
    ) -> Self {
        let event_id = event_id.into();
        let aggregate_id = aggregate_id.into();
        let tenant_id = tenant_id.into();
        let event_type = event_type.into();

        assert!(!event_id.is_empty(), "event_id must not be empty");
        assert!(!aggregate_id.is_empty(), "aggregate_id must not be empty");
        assert!(!tenant_id.is_empty(), "tenant_id must not be empty");
        assert!(!event_type.is_empty(), "event_type must not be empty");
        assert!(event_version >= 1, "event_version must be >= 1");
        assert!(!tags.is_empty(), "tags must contain at least one tag");

        Self {
            event_id,
            aggregate_id,
            tenant_id,
            event_type,
            payload,
            event_version,
            occurred_at,
            tags,
        }
    }

    /// Returns the event ID.
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    /// Returns the aggregate ID.
    pub fn aggregate_id(&self) -> &str {
        &self.aggregate_id
    }

    /// Returns the tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Returns the event type.
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    /// Returns the event version.
    pub fn event_version(&self) -> i64 {
        self.event_version
    }

    /// Returns the occurred_at timestamp.
    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }

    /// Returns the tags.
    pub fn tags(&self) -> &[EventTag] {
        &self.tags
    }

    /// Returns true if the element has the given tag value.
    pub fn has_tag(&self, tag_value: &str) -> bool {
        self.tags.iter().any(|t| t.value() == tag_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tag(value: &str) -> EventTag {
        EventTag::new(value)
    }

    #[test]
    fn test_new_element_success() {
        let elem = EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-1",
            "TestEvent",
            (),
            1,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            vec![make_tag("test")],
        );
        assert_eq!(elem.event_id(), "evt-1");
        assert_eq!(elem.aggregate_id(), "agg-1");
        assert_eq!(elem.tenant_id(), "tenant-1");
        assert_eq!(elem.event_type(), "TestEvent");
        assert_eq!(elem.event_version(), 1);
        assert_eq!(elem.tags().len(), 1);
    }

    #[test]
    #[should_panic(expected = "event_id must not be empty")]
    fn test_new_element_empty_event_id() {
        EventStreamElement::new(
            "",
            "agg-1",
            "tenant-1",
            "TestEvent",
            (),
            1,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            vec![make_tag("test")],
        );
    }

    #[test]
    #[should_panic(expected = "event_version must be >= 1")]
    fn test_new_element_zero_version() {
        EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-1",
            "TestEvent",
            (),
            0,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            vec![make_tag("test")],
        );
    }

    #[test]
    #[should_panic(expected = "tags must contain at least one tag")]
    fn test_new_element_empty_tags() {
        EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-1",
            "TestEvent",
            (),
            1,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            vec![],
        );
    }

    #[test]
    fn test_has_tag() {
        let elem = EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-1",
            "TestEvent",
            (),
            1,
            DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            vec![make_tag("order"), make_tag("payment")],
        );
        assert!(elem.has_tag("order"));
        assert!(elem.has_tag("payment"));
        assert!(!elem.has_tag("shipping"));
    }

    #[test]
    fn test_clone_preserves_all_fields() {
        let elem = EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-1",
            "TestEvent",
            "payload-data",
            5,
            DateTime::<Utc>::from_timestamp(1000, 0).unwrap(),
            vec![make_tag("test")],
        );
        let cloned = elem.clone();
        assert_eq!(elem, cloned);
    }

    #[test]
    fn test_equality() {
        let t = DateTime::<Utc>::from_timestamp(1000, 0).unwrap();
        let e1 = EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-1",
            "TestEvent",
            "payload",
            1,
            t,
            vec![make_tag("test")],
        );
        let e2 = EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-1",
            "TestEvent",
            "payload",
            1,
            t,
            vec![make_tag("test")],
        );
        assert_eq!(e1, e2);
    }
}
