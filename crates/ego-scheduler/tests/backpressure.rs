use ego_scheduler::event_bus::{
    event_bus_channel_with_config, DropPolicy, EntityTriple, EventBusConfig, SchedulerEvent,
    SchedulerEventEnvelope,
};

#[test]
fn test_backpressure_block() {
    let config = EventBusConfig {
        capacity: 2,
        drop_policy: DropPolicy::Block,
    };
    let (tx, mut rx) = event_bus_channel_with_config(config);

    let entity = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    let ev = SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 1,
    };
    let env1 = SchedulerEventEnvelope::new(ev.clone(), entity.clone(), 1);
    let env2 = SchedulerEventEnvelope::new(ev.clone(), entity.clone(), 2);
    let env3 = SchedulerEventEnvelope::new(ev.clone(), entity.clone(), 3);

    tx.try_send(env1).unwrap();
    tx.try_send(env2).unwrap();
    assert!(tx.try_send(env3).is_err());

    let items = rx.drain_all();
    assert_eq!(items.len(), 2);
}

#[test]
fn test_backpressure_drop_newest() {
    let config = EventBusConfig {
        capacity: 2,
        drop_policy: DropPolicy::DropNewest,
    };
    let (tx, mut rx) = event_bus_channel_with_config(config);

    let entity = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
    let ev = SchedulerEvent::ExecutionCompleted {
        entity: entity.clone(),
        state_version: 1,
    };
    let env1 = SchedulerEventEnvelope::new(ev.clone(), entity.clone(), 1);
    let env2 = SchedulerEventEnvelope::new(ev.clone(), entity.clone(), 2);
    let env3 = SchedulerEventEnvelope::new(ev.clone(), entity.clone(), 3);

    tx.try_send(env1).unwrap();
    tx.try_send(env2).unwrap();
    tx.try_send(env3).unwrap();

    let items = rx.drain_all();
    assert_eq!(items.len(), 2);
}
