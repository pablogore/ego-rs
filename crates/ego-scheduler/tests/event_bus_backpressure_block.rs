//! Tests for event bus backpressure with Block policy.

use ego_scheduler::event_bus::{event_bus_channel_with_config, EventBusConfig, DropPolicy};
use ego_scheduler::event::SchedulerEventEnvelope;
use ego_scheduler::types::EntityTriple;
use ego_scheduler::event::SchedulerEvent;

#[tokio::test]
async fn event_bus_backpressure_block() {
    // Create event bus with small capacity and Block policy
    let config = EventBusConfig { 
        capacity: 2, 
        drop_policy: DropPolicy::Block 
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

    // Send events - first two should succeed, third should block
    sender.send(envelope1).await.unwrap();
    sender.send(envelope2).await.unwrap();
    
    // This should block until receiver consumes one
    let sender_clone = sender.clone();
    let handle = tokio::spawn(async move {
        sender_clone.send(envelope3).await.unwrap();
    });
    
    // Give some time for the blocking to happen
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Receive one event to unblock the sender
    let received1 = receiver.recv().await.unwrap();
    
    // Wait for the send to complete
    handle.await.unwrap();
    
    // Read the remaining two events
    let received2 = receiver.recv().await.unwrap();
    let received3 = receiver.recv().await.unwrap();
    
    assert_eq!(received1.sequence_id, 1);
    assert_eq!(received2.sequence_id, 2);
    assert_eq!(received3.sequence_id, 3);
}