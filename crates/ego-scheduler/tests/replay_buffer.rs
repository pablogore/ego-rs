use ego_scheduler::event_bus::{
    EntityTriple, SchedulerEvent, SchedulerEventEnvelope, event_bus_channel,
};
use ego_scheduler::policy::RoundRobin;
use ego_scheduler::scheduler::Scheduler;

#[test]
fn test_replay_buffer_bounded() {
    let (tx, rx) = event_bus_channel();
    let mut scheduler = Scheduler::new(rx, Box::new(RoundRobin));

    let entity = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    for i in 1..=1100 {
        let ev = SchedulerEvent::ExecutionCompleted {
            entity: entity.clone(),
            state_version: i,
        };
        let env = SchedulerEventEnvelope::new(ev, entity.clone(), i);
        tx.try_send(env).unwrap();
    }
    drop(tx);

    while scheduler.run_cycle().is_some() {}

    let buf_len = scheduler.state().replay_buffer.len();
    assert!(buf_len <= 1024, "ReplayBuffer should be bounded at 1024, got {}", buf_len);
}

#[test]
fn test_replay_buffer_non_semantic_equivalence() {
    let (tx1, rx1) = event_bus_channel();
    let (tx2, rx2) = event_bus_channel();

    let mut s1 = Scheduler::new(rx1, Box::new(RoundRobin));
    let mut s2 = Scheduler::new(rx2, Box::new(RoundRobin));

    let entity = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    for i in 1..=10 {
        let ev = SchedulerEvent::ExecutionCompleted {
            entity: entity.clone(),
            state_version: i,
        };
        let env = SchedulerEventEnvelope::new(ev, entity.clone(), i);
        tx1.try_send(env.clone()).unwrap();
        tx2.try_send(env.clone()).unwrap();
    }
    drop(tx1);
    drop(tx2);

    while s1.run_cycle().is_some() {}
    while s2.run_cycle().is_some() {}

    assert_eq!(s1.state(), s2.state());
}
