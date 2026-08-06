use std::collections::HashMap;

use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};

type StreamKey = (String, String, Option<String>);

/// In-memory event store.
///
/// Stores events per `(aggregate_type, aggregate_id)` per tenant. Enforces
/// optimistic concurrency.
pub struct InMemoryEventStore<E> {
    streams: HashMap<StreamKey, Vec<StoredEvent<E>>>,
}

impl<E> InMemoryEventStore<E> {
    pub fn new() -> Self {
        InMemoryEventStore {
            streams: HashMap::new(),
        }
    }
}

impl<E> Default for InMemoryEventStore<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: DomainEvent + Clone> EventStore<E> for InMemoryEventStore<E> {
    fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (
            aggregate_type.to_string(),
            aggregate_id.to_string(),
            tenant.clone(),
        );

        let stream = self.streams.entry(key).or_default();
        let current = stream.len() as i64;

        if current != expected_version {
            return Err(PersistenceError::Conflict {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                expected: expected_version,
                actual: current,
            });
        }

        let count = events.len() as i64;
        stream.extend(events);
        Ok(current + count)
    }

    fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_type.to_string(), aggregate_id.to_string(), tenant);

        match self.streams.get(&key) {
            Some(events) => Ok(events.clone()),
            None => Err(PersistenceError::NotFound {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
            }),
        }
    }

    fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let mut ids: Vec<(String, String)> = self
            .streams
            .keys()
            .filter(|(_, _, t)| *t == tenant)
            .map(|(atype, aid, _)| (atype.clone(), aid.clone()))
            .collect();
        ids.sort();
        Ok(ids)
    }
}

fn resolve_tenant(tenant_id: Option<&str>) -> Result<Option<String>, PersistenceError> {
    match tenant_id {
        Some("") => Err(PersistenceError::MissingTenant),
        Some(t) => Ok(Some(t.to_string())),
        None => Ok(None),
    }
}
