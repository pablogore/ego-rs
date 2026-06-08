use ego_scheduler::event_bus::{
    EntityTriple, SchedulerEvent, SchedulerEventEnvelope, event_bus_channel,
};
use ego_scheduler::policy::RoundRobin;
use ego_scheduler::scheduler::Scheduler;

#[test]
fn test_gap_detection() {
    let (tx, rx) = event_bus_channel();
    let mut scheduler = Scheduler::new(rx, Box::new(RoundRobin));

    let entity = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    let ev = SchedulerEvent::ExecutionCompleted { entity: entity.clone(), state_version: 1 };
    tx.try_send(SchedulerEventEnvelope::new(ev, entity.clone(), 1)).unwrap();

    let ev = SchedulerEvent::ExecutionCompleted { entity: entity.clone(), state_version: 3 };
    tx.try_send(SchedulerEventEnvelope::new(ev, entity.clone(), 3)).unwrap();

    let ev = SchedulerEvent::ExecutionCompleted { entity: entity.clone(), state_version: 5 };
    tx.try_send(SchedulerEventEnvelope::new(ev, entity.clone(), 5)).unwrap();

    drop(tx);

    while scheduler.run_cycle().is_some() {}

    assert!(scheduler.state().detected_gaps > 0);
}

#[test]
fn test_no_gaps_with_contiguous_stream() {
    let (tx, rx) = event_bus_channel();
    let mut scheduler = Scheduler::new(rx, Box::new(RoundRobin));

    let entity = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    for i in 1..=10 {
        let ev = SchedulerEvent::ExecutionCompleted {
            entity: entity.clone(),
            state_version: i,
        };
        tx.try_send(SchedulerEventEnvelope::new(ev, entity.clone(), i)).unwrap();
    }
    drop(tx);

    while scheduler.run_cycle().is_some() {}

    assert_eq!(scheduler.state().detected_gaps, 0);
}
