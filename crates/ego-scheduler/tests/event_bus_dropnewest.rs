//! Tests for event bus with DropNewest policy.

use ego_scheduler::event_bus::{event_bus_channel_with_config, EventBusConfig, DropPolicy};
use ego_scheduler::event::SchedulerEventEnvelope;
use ego_scheduler::types::EntityTriple;
use ego_scheduler::event::SchedulerEvent;

#[tokio::test]
async fn event_bus_dropnewest() {
    // Create event bus with small capacity and DropNewest policy
    let config = EventBusConfig { 
        capacity: 2, 
        drop_policy: DropPolicy::DropNewest 
    };
    let (sender, mut receiver) = event_bus_channel_with_config(config);

    // Create test events
    let entity = EntityTriple {
        tenant: "tenant".to_string(),
        entity_type: "actor".to_string(),
        entity_id: "actor1".to_string(),
    };
    let event = SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 1,
    };
    let envelope1 = SchedulerEventEnvelope::new(event, entity.clone(), 1);
    
    let event2 = SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 2,
    };
    let envelope2 = SchedulerEventEnvelope::new(event2, entity.clone(), 2);
    
    let event3 = SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 3,
    };
    let envelope3 = SchedulerEventEnvelope::new(event3, entity.clone(), 3);

    // Send events - first two should succeed, third should be dropped
    sender.send(envelope1).await.unwrap();
    sender.send(envelope2).await.unwrap();
    sender.send(envelope3).await.unwrap(); // This should be dropped
    
    // Receive events - only first two should be received
    let received1 = receiver.recv().await.unwrap();
    let received2 = receiver.recv().await.unwrap();
    
    // Third event should not be received
    let timeout = tokio::time::timeout(tokio::time::Duration::from_millis(100), receiver.recv()).await;
    assert!(timeout.is_err());
    
    assert_eq!(received1.sequence_id, 1);
    assert_eq!(received2.sequence_id, 2);
}