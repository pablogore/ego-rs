use std::collections::HashMap;

use async_trait::async_trait;
use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, Snapshot, PersistenceError, StoredEvent};
use serde_json::Value;

use crate::publisher::EventPublisher;

pub struct InMemoryEventStore<E> {
    streams: HashMap<(String, Option<String>), Vec<StoredEvent<E>>>,
}

impl<E> InMemoryEventStore<E> {
    pub fn new() -> Self {
        InMemoryEventStore {
            streams: HashMap::new(),
        }
    }
}

impl<E: DomainEvent> Default for InMemoryEventStore<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: DomainEvent + Clone> EventStore<E> for InMemoryEventStore<E> {
    fn append(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let key = (aggregate_id.to_string(), tenant_id.map(String::from));
        let stream = self.streams.entry(key.clone()).or_insert_with(Vec::new);
        let current_version = stream.len() as i64;

        if expected_version != current_version {
            return Err(PersistenceError::Conflict {
                aggregate_id: aggregate_id.to_string(),
                expected: expected_version,
                actual: current_version,
            });
        }

        stream.extend(events);
        Ok(stream.len() as i64)
    }

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let key = (aggregate_id.to_string(), tenant_id.map(String::from));
        self.streams.get(&key).cloned().ok_or_else(|| {
            PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            }
        })
    }

    fn list_aggregate_ids(&self, _tenant_id: Option<&str>) -> Result<Vec<String>, PersistenceError> {
        Ok(self.streams.keys().map(|(id, _)| id.clone()).collect())
    }
}

pub struct InMemorySnapshotStore {
    snapshots: HashMap<(String, Option<String>), (i64, Value)>,
}

impl InMemorySnapshotStore {
    pub fn new() -> Self {
        InMemorySnapshotStore {
            snapshots: HashMap::new(),
        }
    }
}

impl Default for InMemorySnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot for InMemorySnapshotStore {
    fn save_snapshot(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: i64,
        payload: Value,
    ) -> Result<(), PersistenceError> {
        let key = (aggregate_id.to_string(), tenant_id.map(String::from));
        self.snapshots.insert(key, (version, payload));
        Ok(())
    }

    fn load_snapshot(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<(i64, Value)>, PersistenceError> {
        let key = (aggregate_id.to_string(), tenant_id.map(String::from));
        Ok(self.snapshots.get(&key).cloned())
    }
}

pub struct NoopPublisher<E> {
    _phantom: std::marker::PhantomData<E>,
}

impl<E> NoopPublisher<E> {
    pub fn new() -> Self {
        NoopPublisher {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<E: Send + Sync + 'static> Default for NoopPublisher<E> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<E: Send + Sync + 'static> EventPublisher<E> for NoopPublisher<E> {
    async fn publish(&self, _events: &[E]) -> Result<(), ()> {
        Ok(())
    }
}
