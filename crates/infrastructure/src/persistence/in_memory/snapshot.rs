use std::collections::HashMap;

use ego_domain::persistence::resolve_tenant;
use ego_domain::persistence::{PersistenceError, Snapshot};
use serde_json::Value;

type SnapshotKey = (String, Option<String>);

/// In-memory snapshot store.
///
/// Stores the latest snapshot per aggregate per tenant.
pub struct InMemorySnapshotStore {
    snapshots: HashMap<SnapshotKey, (i64, Value)>,
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
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_id.to_string(), tenant);
        self.snapshots.insert(key, (version, payload));
        Ok(())
    }

    fn load_snapshot(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<(i64, Value)>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_id.to_string(), tenant);

        match self.snapshots.get(&key) {
            Some((v, p)) => Ok(Some((*v, p.clone()))),
            None => Ok(None),
        }
    }
}
