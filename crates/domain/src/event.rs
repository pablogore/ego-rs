use chrono::{DateTime, Utc};

/// Trait for domain events.
pub trait DomainEvent: Send + Sync {
    fn aggregate_id(&self) -> &str;
    fn event_type(&self) -> &str;
    fn payload(&self) -> &serde_json::Value;
    fn occurred_at(&self) -> &DateTime<Utc>;
}
