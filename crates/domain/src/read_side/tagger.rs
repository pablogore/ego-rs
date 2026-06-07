//! Event tagger — computes tags for events at creation time.

use super::event_tag::EventTag;

/// Computes tags for events at creation time.
///
/// The tagger precomputes tags at event creation time.
/// The runtime never recalculates tags.
pub trait EventTagger<E> {
    /// Computes the tags for an event.
    ///
    /// # Arguments
    /// * `event` - The event to tag
    /// * `aggregate_id` - The aggregate that produced the event
    ///
    /// # Returns
    /// A vector of tags for the event. May be empty.
    fn tags(&self, event: &E, aggregate_id: &str) -> Vec<EventTag>;
}
