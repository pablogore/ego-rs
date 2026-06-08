//! Tests for replay buffer bounded to 1024.

use ego_scheduler::state::SchedulerState;
use ego_scheduler::event::SchedulerEventEnvelope;
use ego_scheduler::types::EntityTriple;
use ego_scheduler::event::SchedulerEvent;

#[tokio::test]
async fn replay_buffer_bound_1024() {
    let mut state = SchedulerState::new();
    
    // Create test events
    let entity = EntityTriple {
        tenant: "tenant".to_string(),
        entity_type: "actor".to_string(),
        entity_id: "actor1".to_string(),
    };
    
    // Add 1025 events to test buffer overflow
    for i in 1..=1025 {
        let event = SchedulerEvent::ExecutionCompleted {
            entity: entity.clone(),
            state_version: i,
        };
        let envelope = SchedulerEventEnvelope::new(event, entity.clone(), i);
        state.apply_event(&envelope);
    }
    
    // Buffer should be bounded to 1024
    assert_eq!(state.replay_buffer.len(), 1024);
    
    // First event should be dropped (oldest)
    assert_eq!(state.replay_buffer.front().unwrap().sequence_id, 2);
    
    // Last event should be kept
    assert_eq!(state.replay_buffer.back().unwrap().sequence_id, 1025);
}