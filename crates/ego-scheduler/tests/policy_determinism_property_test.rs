//! Tests for policy determinism property.

use ego_scheduler::scheduler::Scheduler;
use ego_scheduler::state::SchedulerState;
use ego_scheduler::policy::RoundRobin;
use ego_scheduler::event_bus::{event_bus_channel_with_config, EventBusConfig, DropPolicy};
use ego_scheduler::types::EntityTriple;
use ego_scheduler::event::SchedulerEvent;
use ego_scheduler::event::SchedulerEventEnvelope;
use std::collections::HashSet;

#[tokio::test]
async fn policy_determinism_property_test() {
    // Create event bus
    let config = EventBusConfig { capacity: 4096, drop_policy: DropPolicy::Block };
    let (sender, receiver) = event_bus_channel_with_config(config.clone());
    
    // Create scheduler with round-robin policy
    let policy = RoundRobin;
    let mut scheduler = Scheduler::new(SchedulerState::new(), receiver, Box::new(policy.clone()));

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
    let entity3 = EntityTriple {
        tenant: "tenant3".to_string(),
        entity_type: "actor".to_string(),
        entity_id: "actor3".to_string(),
    };

    // Send events in different orders to test determinism
    let event1 = SchedulerEventEnvelope::new(
        SchedulerEvent::ExecutionCompleted {
            entity: entity1.clone(),
            state_version: 1,
        },
        entity1.clone(),
        1,
    );
    
    let event2 = SchedulerEventEnvelope::new(
        SchedulerEvent::ExecutionCompleted {
            entity: entity2.clone(),
            state_version: 1,
        },
        entity2.clone(),
        2,
    );
    
    let event3 = SchedulerEventEnvelope::new(
        SchedulerEvent::ExecutionCompleted {
            entity: entity3.clone(),
            state_version: 1,
        },
        entity3.clone(),
        3,
    );

    // Send events in order 1, 2, 3
    sender.send(event1.clone()).await.unwrap();
    sender.send(event2.clone()).await.unwrap();
    sender.send(event3.clone()).await.unwrap();
    // Drop sender to signal no more events
    drop(sender);

    // Process events
    scheduler.drain_and_apply().await;
    
    // Get suggestion
    let mut pending = HashSet::new();
    pending.insert(entity1.clone());
    pending.insert(entity2.clone());
    pending.insert(entity3.clone());
    
    let suggestion1 = scheduler.suggest_activation(&pending).await;
    
    // Reset and send same events in different order
    let (sender2, receiver2) = event_bus_channel_with_config(config);
    let mut scheduler2 = Scheduler::new(SchedulerState::new(), receiver2, Box::new(policy));
    
    // Send events in order 3, 2, 1
    sender2.send(event3.clone()).await.unwrap();
    sender2.send(event2.clone()).await.unwrap();
    sender2.send(event1.clone()).await.unwrap();
    // Drop sender to signal no more events
    drop(sender2);
    
    scheduler2.drain_and_apply().await;
    
    let suggestion2 = scheduler2.suggest_activation(&pending).await;
    
    // Suggestions should be the same (deterministic)
    assert_eq!(suggestion1, suggestion2);
}