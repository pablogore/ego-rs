//! Domain event trait.
//!
//! Defines `DomainEvent` — the contract for events in a CQRS / Event
//! Sourcing system. Events are immutable, append-only records of
//! state transitions.

use chrono::{DateTime, Utc};

/// Trait for domain events in a CQRS / Event Sourcing system.
///
/// Domain events represent **facts that happened**. They are immutable,
/// append-only records of state transitions. Events are published by
/// command handlers after successful mutation and consumed by projections,
/// event stores, and other subscribers.
///
/// # Required methods
///
/// | Method | Purpose |
/// |--------|---------|
/// | `aggregate_id()` | Identifies the aggregate this event belongs to |
/// | `event_type()` | Discriminant for routing/dispatch (e.g. `"OrderPlaced"`) |
/// | `payload()` | The event data as a JSON value |
/// | `occurred_at()` | The wall-clock or logical time when the event occurred |
///
/// # Example
///
/// ```rust
/// use chrono::Utc;
/// use ego_domain::DomainEvent;
/// use serde_json::json;
///
/// struct OrderPlaced {
///     order_id: String,
///     occurred_at: chrono::DateTime<Utc>,
/// }
///
/// impl DomainEvent for OrderPlaced {
///     fn aggregate_id(&self) -> &str { &self.order_id }
///     fn event_type(&self) -> &str { "OrderPlaced" }
///     fn payload(&self) -> &serde_json::Value {
///         // Return a reference to a static or owned value
///         unimplemented!("use a stored serde_json::Value")
///     }
///     fn occurred_at(&self) -> &chrono::DateTime<Utc> { &self.occurred_at }
/// }
/// ```
pub trait DomainEvent: Send + Sync {
    /// The aggregate this event belongs to (e.g. order id, user id).
    fn aggregate_id(&self) -> &str;

    /// The event type discriminant (e.g. `"OrderPlaced"`).
    /// Used for routing and dispatch.
    fn event_type(&self) -> &str;

    /// The event payload as a JSON value.
    /// Serialization format is intentionally JSON for contract compatibility.
    fn payload(&self) -> &serde_json::Value;

    /// When the event occurred (wall-clock or logical time).
    fn occurred_at(&self) -> &DateTime<Utc>;
}