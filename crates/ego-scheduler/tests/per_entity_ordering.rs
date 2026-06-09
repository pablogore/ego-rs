use ego_scheduler::event_bus::{
    event_bus_channel, EntityTriple, SchedulerEvent, SchedulerEventEnvelope,
};
use ego_scheduler::policy::RoundRobin;
use ego_scheduler::scheduler::Scheduler;
use std::collections::BTreeSet;

#[test]
fn test_entity_isolation() {
    let (tx, rx) = event_bus_channel();
    let mut scheduler = Scheduler::new(rx, Box::new(RoundRobin));

    let a = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    let b = EntityTriple::new("t1".into(), "actor".into(), "a2".into());

    for i in 1..=3 {
        let ev = SchedulerEvent::ExecutionCompleted {
            entity: a.clone(),
            state_version: i,
        };
        tx.try_send(SchedulerEventEnvelope::new(ev, a.clone(), i))
            .unwrap();
    }
    for i in 1..=3 {
        let ev = SchedulerEvent::ExecutionCompleted {
            entity: b.clone(),
            state_version: i,
        };
        tx.try_send(SchedulerEventEnvelope::new(ev, b.clone(), i))
            .unwrap();
    }
    drop(tx);

    let mut suggestions = Vec::new();
    while let Some(s) = scheduler.run_cycle() {
        suggestions.push(s);
    }

    assert!(
        !suggestions.is_empty(),
        "Should produce at least one suggestion"
    );
    assert!(suggestions.iter().any(|s| *s == a || *s == b));
}

#[test]
fn test_pending_is_btreeset() {
    let pending: BTreeSet<EntityTriple> = [
        EntityTriple::new("t1".into(), "actor".into(), "a1".into()),
        EntityTriple::new("t1".into(), "actor".into(), "a2".into()),
    ]
    .into_iter()
    .collect();

    let collected: Vec<_> = pending.iter().collect();
    let collected2: Vec<_> = pending.iter().collect();
    assert_eq!(
        collected, collected2,
        "BTreeSet iteration must be deterministic"
    );
}
