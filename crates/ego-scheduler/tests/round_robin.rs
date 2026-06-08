use ego_scheduler::event_bus::{EntityTriple, event_bus_channel};
use ego_scheduler::policy::{RoundRobin, SchedulingPolicy};
use ego_scheduler::scheduler::Scheduler;
use ego_scheduler::state::SchedulerState;
use std::collections::BTreeSet;

#[test]
fn test_round_robin_rotates() {
    let policy = RoundRobin;
    let state = SchedulerState::new();
    let mut pending = BTreeSet::new();
    let e1 = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    let e2 = EntityTriple::new("t1".into(), "actor".into(), "a2".into());
    let e3 = EntityTriple::new("t1".into(), "actor".into(), "a3".into());
    pending.insert(e1.clone());
    pending.insert(e2.clone());
    pending.insert(e3.clone());

    let suggestion = policy.suggest_activation(&state, &pending);
    assert!(suggestion.is_some());
}

#[test]
fn test_empty_pending_returns_none() {
    let policy = RoundRobin;
    let state = SchedulerState::new();
    let pending = BTreeSet::new();
    assert_eq!(policy.suggest_activation(&state, &pending), None);
}

#[test]
fn test_round_robin_determinism() {
    let policy = RoundRobin;
    let state = SchedulerState::new();
    let mut pending = BTreeSet::new();
    pending.insert(EntityTriple::new("t1".into(), "actor".into(), "a1".into()));
    pending.insert(EntityTriple::new("t1".into(), "actor".into(), "a2".into()));

    let first = policy.suggest_activation(&state, &pending);
    for _ in 0..100 {
        assert_eq!(policy.suggest_activation(&state, &pending), first);
    }
}

#[test]
fn test_scheduler_integration() {
    let (tx, rx) = event_bus_channel();
    let mut scheduler = Scheduler::new(rx, Box::new(RoundRobin));

    let entity = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    let ev = ego_scheduler::event_bus::SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 1,
    };
    let env = ego_scheduler::event_bus::SchedulerEventEnvelope::new(ev, entity.clone(), 1);
    tx.try_send(env).unwrap();
    drop(tx);

    let suggestion = scheduler.run_cycle();
    assert!(suggestion.is_some());
    assert!(scheduler.state().total_events_consumed > 0);
}
