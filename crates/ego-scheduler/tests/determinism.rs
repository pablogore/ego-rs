use ego_scheduler::event_bus::{
    event_bus_channel, EntityTriple, SchedulerEvent, SchedulerEventEnvelope,
};
use ego_scheduler::policy::RoundRobin;
use ego_scheduler::scheduler::Scheduler;

#[test]
fn determinism_identical_streams() {
    let entity1 = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    let entity2 = EntityTriple::new("t1".into(), "actor".into(), "a2".into());

    let (tx1, rx1) = event_bus_channel();
    let (tx2, rx2) = event_bus_channel();

    let mut s1 = Scheduler::new(rx1, Box::new(RoundRobin));
    let mut s2 = Scheduler::new(rx2, Box::new(RoundRobin));

    for i in 1..=10 {
        let ev = SchedulerEvent::ExecutionCompleted {
            entity: entity1.clone(),
            state_version: i,
        };
        let env = SchedulerEventEnvelope::new(ev, entity1.clone(), i);
        tx1.try_send(env.clone()).unwrap();
        tx2.try_send(env.clone()).unwrap();
    }

    for i in 1..=5 {
        let ev = SchedulerEvent::ExecutionCompleted {
            entity: entity2.clone(),
            state_version: i,
        };
        let env = SchedulerEventEnvelope::new(ev, entity2.clone(), i);
        tx1.try_send(env.clone()).unwrap();
        tx2.try_send(env.clone()).unwrap();
    }

    drop(tx1);
    drop(tx2);

    while s1.run_cycle().is_some() {}
    while s2.run_cycle().is_some() {}

    let st1 = s1.state();
    let st2 = s2.state();

    assert_eq!(st1.total_events_consumed, st2.total_events_consumed);
    assert_eq!(st1.detected_gaps, st2.detected_gaps);
    assert_eq!(st1.last_suggestion, st2.last_suggestion);
    assert_eq!(st1, st2);
}
