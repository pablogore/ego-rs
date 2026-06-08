//! Tests for gap detection with sequence loss.

use ego_scheduler::state::SchedulerState;
use ego_scheduler::event::SchedulerEventEnvelope;
use ego_scheduler::types::EntityTriple;
use ego_scheduler::event::SchedulerEvent;
#[tokio::test]
async fn gap_detection_sequence_loss() {
    let mut state = SchedulerState::new();
    
    // Create test events with gaps
    let entity = EntityTriple {
        tenant: "tenant".to_string(),
        entity_type: "actor".to_string(),
        entity_id: "actor1".to_string(),
    };
    
    // Send event with sequence 1
    let event1 = SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 1,
    };
    let envelope1 = SchedulerEventEnvelope::new(event1, entity.clone(), 1);
    state.apply_event(&envelope1);
    
    // Send event with sequence 3 (gap of 1)
    let event3 = SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 3,
    };
    let envelope3 = SchedulerEventEnvelope::new(event3, entity.clone(), 3);
    state.apply_event(&envelope3);
    
    // Should detect one gap
    assert_eq!(state.detected_gaps.len(), 1);
    
    // Check gap info
    let gap = &state.detected_gaps[0];
    assert_eq!(gap.start_seq, 1);
    assert_eq!(gap.end_seq, 3);
    assert_eq!(gap.source_actor, entity);
}