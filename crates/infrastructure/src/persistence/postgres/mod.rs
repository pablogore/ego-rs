//! PostgreSQL store implementations.
//!
//! Concrete backends for the domain persistence SPI traits.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use chrono::Utc;

use ego_domain::read_side::{
    event_stream::EventStreamElement,
    event_tag::EventTag,
    offset::Offset,
    store::ReadSideStore as ReadSideStoreTrait,
    dedup::DedupStore as DedupStoreTrait,
    projection_state_store::ProjectionStateStore as ProjectionStateStoreTrait,
    offset::OffsetStore as OffsetStoreTrait,
    state::ProjectionState,
};

use ego_infrastructure::persistence::postgres::{
    PostgresReadSideStore,
    PostgreSQLOffsetStore,
    PostgresDedupStore,
    PostgresProjectionStateStore,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_side_store_fetch() {
        // Setup
        let pool = setup_test_db().await;
        let store = PostgresReadSideStore::new(pool.clone());
        store.create_tables().await.unwrap();

        // Create test events
        let tag = EventTag::new("user_created");
        let event1 = create_test_event("event1", "user1", "tenant1", &tag, 1);
        let event2 = create_test_event("event2", "user2", "tenant1", &tag, 2);
        let event3 = create_test_event("event3", "user3", "tenant1", &tag, 3);

        // Insert events into the database directly
        insert_event(&pool, &event1).await;
        insert_event(&pool, &event2).await;
        insert_event(&pool, &event3).await;

        // Test fetch with no offset (should return all events)
        let fetched_events = store.fetch(&tag, None, 10).await.unwrap();
        assert_eq!(fetched_events.len(), 3);
        assert_eq!(fetched_events[0].id(), "event1");
        assert_eq!(fetched_events[1].id(), "event2");
        assert_eq!(fetched_events[2].id(), "event3");

        // Test fetch with offset (should return events after offset)
        let offset = Offset::sequence(1);
        let fetched_events = store.fetch(&tag, Some(&offset), 10).await.unwrap();
        assert_eq!(fetched_events.len(), 2);
        assert_eq!(fetched_events[0].id(), "event2");
        assert_eq!(fetched_events[1].id(), "event3");

        // Test fetch with batch size limit
        let fetched_events = store.fetch(&tag, None, 2).await.unwrap();
        assert_eq!(fetched_events.len(), 2);
        assert_eq!(fetched_events[0].id(), "event1");
        assert_eq!(fetched_events[1].id(), "event2");

        // Test fetch with empty results
        let tag2 = EventTag::new("nonexistent");
        let fetched_events = store.fetch(&tag2, None, 10).await.unwrap();
        assert_eq!(fetched_events.len(), 0);
    }

    #[tokio::test]
    async fn test_dedup_store_seen_and_mark_seen() {
        // Setup
        let pool = setup_test_db().await;
        let store = PostgresDedupStore::new(pool.clone());
        store.create_tables().await.unwrap();

        let projection_id = "test_projection";
        let tag = EventTag::new("user_created");
        let event_id = "test_event_id";

        // Test that unseen events return false
        let seen = store.seen(projection_id, &tag, event_id).await.unwrap();
        assert_eq!(seen, false);

        // Mark event as seen
        store.mark_seen(projection_id, &tag, event_id).await.unwrap();

        // Test that seen events return true
        let seen = store.seen(projection_id, &tag, event_id).await.unwrap();
        assert_eq!(seen, true);

        // Test that different event IDs are still unseen
        let seen = store.seen(projection_id, &tag, "different_event_id").await.unwrap();
        assert_eq!(seen, false);

        // Test that same event with different projection is unseen
        let seen = store.seen("different_projection", &tag, event_id).await.unwrap();
        assert_eq!(seen, false);
    }

    #[tokio::test]
    async fn test_projection_state_store_save_and_load() {
        // Setup
        let pool = setup_test_db().await;
        let store = PostgresProjectionStateStore::new(pool.clone());
        store.create_tables().await.unwrap();

        let projection_id = "test_projection";
        let tag = EventTag::new("user_created");
        let state = ProjectionState::Running;

        // Save state
        store.write_state(projection_id, &tag, &state).await.unwrap();

        // Load state
        let loaded_state = store.read_state(projection_id, &tag).await.unwrap();
        assert_eq!(loaded_state, Some(state));

        // Test loading non-existent state
        let loaded_state = store.read_state("non_existent", &tag).await.unwrap();
        assert_eq!(loaded_state, None);
    }

    #[tokio::test]
    async fn test_offset_store_save_and_load() {
        // Setup
        let pool = setup_test_db().await;
        let store = PostgreSQLOffsetStore::new(pool.clone());
        store.create_tables().await.unwrap();

        let projection_id = "test_projection";
        let tag = EventTag::new("user_created");
        let tenant = "tenant1";
        let offset = Offset::sequence(42);

        // Save offset
        store.write_offset(projection_id, &tag, tenant, &offset).await.unwrap();

        // Load offset
        let loaded_offset = store.read_offset(projection_id, &tag, tenant).await.unwrap();
        assert_eq!(loaded_offset, Some(offset));

        // Test loading non-existent offset
        let loaded_offset = store.read_offset("non_existent", &tag, tenant).await.unwrap();
        assert_eq!(loaded_offset, None);
    }

    // Helper functions
    async fn setup_test_db() -> PgPool {
        // Use the test database URL from environment or default to a test DB
        let db_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/test".to_string());
        
        PgPool::connect(&db_url).await.unwrap()
    }

    #[derive(Serialize, Deserialize, Clone)]
    struct TestEvent {
        id: String,
        name: String,
    }

    fn create_test_event(
        id: &str,
        aggregate_id: &str,
        tenant_id: &str,
        tag: &EventTag,
        version: i64,
    ) -> EventStreamElement<TestEvent> {
        let event = TestEvent {
            id: id.to_string(),
            name: format!("event_{}", id),
        };

        EventStreamElement::new(
            id.to_string(),
            aggregate_id.to_string(),
            tenant_id.to_string(),
            "TestEvent".to_string(),
            event,
            version,
            Utc::now(),
            vec![tag.clone()],
        )
    }

    async fn insert_event(pool: &PgPool, event: &EventStreamElement<TestEvent>) {
        let query = r#"
            INSERT INTO event_stream_elements (
                id, aggregate_id, tenant_id, event_type, payload, event_version, occurred_at, tags
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#;

        sqlx::query(query)
            .bind(event.id())
            .bind(event.aggregate_id())
            .bind(event.tenant_id())
            .bind(event.event_type())
            .bind(serde_json::to_value(event.payload()).unwrap())
            .bind(event.event_version())
            .bind(event.occurred_at())
            .bind(event.tags().iter().map(|t| t.value()).collect::<Vec<_>>())
            .execute(pool)
            .await
            .unwrap();
    }
}