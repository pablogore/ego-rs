mod common;

use ego_domain::event::DomainEvent;
use ego_infrastructure::persistence::in_memory::InMemoryEventStore;
use serde_json::json;

#[derive(Clone, Debug)]
struct FakeEvent {
    event_type: String,
    payload: serde_json::Value,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

impl FakeEvent {
    fn new(label: &str) -> Self {
        FakeEvent {
            event_type: format!("FakeEvent::{}", label),
            payload: json!({"label": label}),
            occurred_at: chrono::Utc::now(),
        }
    }
}

impl DomainEvent for FakeEvent {
    fn aggregate_id(&self) -> &str {
        "fake"
    }
    fn event_type(&self) -> &str {
        &self.event_type
    }
    fn payload(&self) -> &serde_json::Value {
        &self.payload
    }
    fn occurred_at(&self) -> &chrono::DateTime<chrono::Utc> {
        &self.occurred_at
    }
}

#[test]
fn in_memory_event_store_passes_contract_tests() {
    let store = InMemoryEventStore::<FakeEvent>::new();
    common::event_store_contract_tests(store, FakeEvent::new);
}
