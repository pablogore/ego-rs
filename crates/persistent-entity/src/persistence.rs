use std::sync::Mutex;

use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, Snapshot, PersistenceError, StoredEvent};

use crate::error::EntityError;

pub struct SnapshotData {
    pub version: u64,
    pub data: Vec<u8>,
}

pub struct PersistenceFacade<E: DomainEvent> {
    event_store: Mutex<Box<dyn EventStore<E> + Send>>,
    snapshot_store: Mutex<Box<dyn Snapshot + Send>>,
}

impl<E: DomainEvent + Clone> PersistenceFacade<E> {
    pub fn new(
        event_store: Box<dyn EventStore<E> + Send>,
        snapshot_store: Box<dyn Snapshot + Send>,
    ) -> Self {
        PersistenceFacade {
            event_store: Mutex::new(event_store),
            snapshot_store: Mutex::new(snapshot_store),
        }
    }

    pub fn load_for_recovery(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(Option<SnapshotData>, Vec<StoredEvent<E>>), EntityError> {
        let snapshot = {
            let store = self.snapshot_store.lock().unwrap();
            store.load_snapshot(aggregate_id, tenant_id)
                .map_err(|e| EntityError::Runtime(e.to_string()))?
                .map(|(v, payload)| SnapshotData {
                    version: v as u64,
                    data: serde_json::to_vec(&payload).unwrap_or_default(),
                })
        };

        let events = {
            let store = self.event_store.lock().unwrap();
            store.load(aggregate_id, tenant_id)
                .map_err(|e| EntityError::Runtime(e.to_string()))?
        };

        Ok((snapshot, events))
    }

    pub fn persist_events(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: u64,
        events: Vec<E>,
    ) -> Result<u64, EntityError> {
        let current_version = {
            let store = self.event_store.lock().unwrap();
            match store.load(aggregate_id, tenant_id) {
                Ok(loaded) => loaded.len() as i64,
                Err(PersistenceError::NotFound { .. }) => 0,
                Err(e) => return Err(EntityError::Runtime(e.to_string())),
            }
        };

        if current_version as u64 != expected_version {
            return Err(EntityError::VersionConflict {
                expected: expected_version,
                current: current_version as u64,
            });
        }

        let stored: Vec<StoredEvent<E>> = events.into_iter()
            .map(|event| StoredEvent::without_correlation(event))
            .collect();

        let new_version = {
            let mut store = self.event_store.lock().unwrap();
            store.append(aggregate_id, tenant_id, current_version, stored)
                .map_err(|e| EntityError::Runtime(e.to_string()))?
        };

        Ok(new_version as u64)
    }

    pub fn store_snapshot(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: u64,
        payload: &serde_json::Value,
    ) -> Result<(), EntityError> {
        let mut store = self.snapshot_store.lock().unwrap();
        store.save_snapshot(aggregate_id, tenant_id, version as i64, payload.clone())
            .map_err(|e| EntityError::Runtime(e.to_string()))
    }
}
