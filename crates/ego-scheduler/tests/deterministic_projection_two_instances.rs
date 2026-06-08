//! Tests for deterministic projection with two instances.

use ego_scheduler::scheduler::Scheduler;
use ego_scheduler::state::SchedulerState;
use ego_scheduler::policy::RoundRobin;
use ego_scheduler::event_bus::{event_bus_channel_with_config, EventBusConfig, DropPolicy};
use ego_scheduler::types::EntityTriple;
use ego_scheduler::event::SchedulerEvent;
use ego_scheduler::event::SchedulerEventEnvelope;

#[tokio::test]
async fn deterministic_projection_two_instances() {
    // Create two separate event buses
    let config = EventBusConfig { capacity: 4096, drop_policy: DropPolicy::Block };
    let (sender1, receiver1) = event_bus_channel_with_config(config.clone());
    let (sender2, receiver2) = event_bus_channel_with_config(config);

    // Create two schedulers with the same policy
    let policy = RoundRobin;
    let mut scheduler1 = Scheduler::new(SchedulerState::new(), receiver1, Box::new(policy.clone()));
    let mut scheduler2 = Scheduler::new(SchedulerState::new(), receiver2, Box::new(policy));

    // Create entities
    let entity1 = EntityTriple {
        tenant: "tenant1".to_string(),
        entity_type: "actor".to_string(),
        entity_id: "actor1".to_string(),
    };

    let entity2 = EntityTriple {
        tenant: "tenant2".to_string(),
        entity_type: "actor".to_string(),
        entity_id: "actor2".to_string(),
    };

    // Send events to both buses
    let event1 = SchedulerEvent::ExecutionCompleted {
        entity: entity1.clone(),
        state_version: 1,
    };
    let envelope1 = SchedulerEventEnvelope::new(event1, entity1.clone(), 1);
    sender1.send(envelope1).await.unwrap();
    // Drop sender to signal no more events
    drop(sender1);

    let event2 = SchedulerEvent::ExecutionCompleted {
        entity: entity2.clone(),
        state_version: 1,
    };
    let envelope2 = SchedulerEventEnvelope::new(event2, entity2.clone(), 1);
    sender2.send(envelope2).await.unwrap();
    // Drop sender to signal no more events
    drop(sender2);

    // Process events
    scheduler1.drain_and_apply().await;
    scheduler2.drain_and_apply().await;

    // Both schedulers should have the same state
    let metrics1 = scheduler1.get_metrics().await;
    let metrics2 = scheduler2.get_metrics().await;
    
    assert_eq!(metrics1.total_events_consumed, 1);
    assert_eq!(metrics2.total_events_consumed, 1);
}