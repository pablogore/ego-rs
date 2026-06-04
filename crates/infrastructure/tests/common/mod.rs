use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError, Repository, Snapshot, StoredEvent};
use serde_json::json;

// ---------------------------------------------------------------------------
// EventStore contract tests
// ---------------------------------------------------------------------------

pub fn event_store_contract_tests<E, S, F>(mut store: S, make_event: F)
where
    E: DomainEvent + Clone + std::fmt::Debug,
    S: EventStore<E>,
    F: Fn(&str) -> E,
{
    // --- Basic append and load ---
    let events = vec![
        StoredEvent::without_correlation(make_event("e1")),
        StoredEvent::without_correlation(make_event("e2")),
    ];
    let version = store
        .append("agg-1", None, 0, events)
        .expect("append should succeed");
    assert_eq!(version, 2, "version should be 2 after 2 events");

    let loaded = store.load("agg-1", None).expect("load should succeed");
    assert_eq!(loaded.len(), 2, "should load 2 events");

    // --- correlation_id preservation ---
    let events = vec![StoredEvent::new(
        make_event("e3"),
        Some("corr-1".to_string()),
    )];
    store
        .append("agg-corr", None, 0, events)
        .expect("append with correlation_id should succeed");
    let loaded = store.load("agg-corr", None).expect("load should succeed");
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].correlation_id,
        Some("corr-1".to_string()),
        "correlation_id should be preserved"
    );

    // --- append without correlation_id returns None ---
    let events = vec![StoredEvent::without_correlation(make_event("e4"))];
    store
        .append("agg-nocorr", None, 0, events)
        .expect("append without correlation_id should succeed");
    let loaded = store.load("agg-nocorr", None).expect("load should succeed");
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].correlation_id, None,
        "correlation_id should be None"
    );

    // --- batch append with mixed correlation_ids ---
    let events = vec![
        StoredEvent::new(make_event("e5a"), Some("corr-a".to_string())),
        StoredEvent::without_correlation(make_event("e5b")),
        StoredEvent::new(make_event("e5c"), Some("corr-c".to_string())),
    ];
    store
        .append("agg-mixed", None, 0, events)
        .expect("batch append should succeed");
    let loaded = store.load("agg-mixed", None).expect("load should succeed");
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].correlation_id, Some("corr-a".to_string()));
    assert_eq!(loaded[1].correlation_id, None);
    assert_eq!(loaded[2].correlation_id, Some("corr-c".to_string()));

    // --- NotFound on missing aggregate ---
    let err = store.load("missing", None).unwrap_err();
    assert!(
        matches!(&err, PersistenceError::NotFound { .. }),
        "expected NotFound, got {:?}",
        err
    );

    // --- Optimistic concurrency: conflict ---
    let err = store
        .append(
            "agg-1",
            None,
            99,
            vec![StoredEvent::without_correlation(make_event("e6"))],
        )
        .unwrap_err();
    assert!(
        matches!(&err, PersistenceError::Conflict { .. }),
        "expected Conflict, got {:?}",
        err
    );

    // --- Concurrency: correct version works ---
    let version = store
        .append(
            "agg-1",
            None,
            2,
            vec![StoredEvent::without_correlation(make_event("e7"))],
        )
        .expect("append with correct version should succeed");
    assert_eq!(version, 3, "version should be 3 after third event");

    // --- Tenant isolation ---
    store
        .append(
            "agg-1",
            Some("tenant-a"),
            0,
            vec![StoredEvent::without_correlation(make_event("t1"))],
        )
        .expect("append for tenant-a should succeed");

    let tenant_events = store
        .load("agg-1", Some("tenant-a"))
        .expect("load for tenant-a should succeed");
    assert_eq!(tenant_events.len(), 1, "tenant-a should see 1 event");

    let no_tenant_events = store
        .load("agg-1", None)
        .expect("load without tenant should succeed");
    assert_eq!(
        no_tenant_events.len(),
        3,
        "non-tenant stream should have 3 events"
    );

    // --- MissingTenant ---
    let err = store
        .append(
            "agg-1",
            Some(""),
            0,
            vec![StoredEvent::without_correlation(make_event("bad"))],
        )
        .unwrap_err();
    assert!(
        matches!(&err, PersistenceError::MissingTenant),
        "expected MissingTenant, got {:?}",
        err
    );

    // --- list_aggregate_ids ---
    let ids = store.list_aggregate_ids(None).expect("list should succeed");
    assert!(ids.contains(&"agg-1".to_string()), "agg-1 should be listed");
}

// ---------------------------------------------------------------------------
// Repository contract tests
// ---------------------------------------------------------------------------

pub fn repository_contract_tests<R>(mut repo: R)
where
    R: Repository<String>,
{
    // --- Save and load ---
    let version = repo
        .save("agg-1", "state-1".to_string(), None, 0)
        .expect("save should succeed");
    assert_eq!(version, 1, "version should be 1 after first save");

    let loaded = repo.load("agg-1", None).expect("load should succeed");
    assert_eq!(loaded, "state-1");

    // --- NotFound on missing aggregate ---
    let err = repo.load("missing", None).unwrap_err();
    assert!(
        matches!(&err, PersistenceError::NotFound { .. }),
        "expected NotFound, got {:?}",
        err
    );

    // --- Optimistic concurrency: conflict ---
    let err = repo
        .save("agg-1", "state-2".to_string(), None, 99)
        .unwrap_err();
    assert!(
        matches!(&err, PersistenceError::Conflict { .. }),
        "expected Conflict, got {:?}",
        err
    );

    // --- Concurrency: correct version works ---
    let version = repo
        .save("agg-1", "state-2".to_string(), None, 1)
        .expect("save with correct version should succeed");
    assert_eq!(version, 2, "version should be 2 after update");

    // --- Delete ---
    repo.delete("agg-1", None).expect("delete should succeed");
    let err = repo.load("agg-1", None).unwrap_err();
    assert!(
        matches!(&err, PersistenceError::NotFound { .. }),
        "expected NotFound after delete"
    );

    // --- Tenant isolation ---
    repo.save("agg-1", "tenant-state".to_string(), Some("t1"), 0)
        .expect("save for tenant should succeed");
    let tenant_agg = repo
        .load("agg-1", Some("t1"))
        .expect("load for tenant should succeed");
    assert_eq!(tenant_agg, "tenant-state");

    // --- MissingTenant ---
    let err = repo
        .save("agg-1", "bad".to_string(), Some(""), 0)
        .unwrap_err();
    assert!(
        matches!(&err, PersistenceError::MissingTenant),
        "expected MissingTenant"
    );
}

// ---------------------------------------------------------------------------
// Snapshot contract tests
// ---------------------------------------------------------------------------

pub fn snapshot_contract_tests<S>(mut store: S)
where
    S: Snapshot,
{
    // --- Save and load latest ---
    store
        .save_snapshot("agg-1", None, 1, json!({"state": "v1"}))
        .expect("save snapshot should succeed");

    let loaded = store
        .load_snapshot("agg-1", None)
        .expect("load should succeed");
    assert_eq!(loaded, Some((1, json!({"state": "v1"}))));

    // --- Overwrite with higher version ---
    store
        .save_snapshot("agg-1", None, 3, json!({"state": "v3"}))
        .expect("save v3 should succeed");
    let loaded = store
        .load_snapshot("agg-1", None)
        .expect("load v3 should succeed");
    assert_eq!(loaded, Some((3, json!({"state": "v3"}))));

    // --- No snapshot returns None ---
    let none = store
        .load_snapshot("unknown", None)
        .expect("load unknown should succeed");
    assert_eq!(none, None, "unknown aggregate should return None");

    // --- Tenant isolation ---
    store
        .save_snapshot("agg-1", Some("t1"), 1, json!({"tenant": "t1"}))
        .expect("save tenant snapshot should succeed");
    let tenant = store
        .load_snapshot("agg-1", Some("t1"))
        .expect("load tenant snapshot should succeed");
    assert_eq!(tenant, Some((1, json!({"tenant": "t1"}))));

    let no_tenant = store
        .load_snapshot("agg-1", None)
        .expect("load without tenant should succeed");
    assert_eq!(no_tenant, Some((3, json!({"state": "v3"}))));
}
