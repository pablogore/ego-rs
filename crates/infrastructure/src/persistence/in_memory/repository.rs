use std::collections::HashMap;

use ego_domain::persistence::{PersistenceError, Repository};

type RepoKey = (String, Option<String>);

/// In-memory aggregate repository.
///
/// Stores aggregates per ID per tenant. Enforces optimistic concurrency.
pub struct InMemoryRepository<A> {
    aggregates: HashMap<RepoKey, (A, i64)>,
}

impl<A> InMemoryRepository<A> {
    pub fn new() -> Self {
        InMemoryRepository {
            aggregates: HashMap::new(),
        }
    }
}

impl<A> Default for InMemoryRepository<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Clone> Repository<A> for InMemoryRepository<A> {
    fn save(
        &mut self,
        aggregate_id: &str,
        aggregate: A,
        tenant_id: Option<&str>,
        expected_version: i64,
    ) -> Result<i64, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_id.to_string(), tenant);

        let current = self.aggregates.get(&key).map(|(_, v)| *v).unwrap_or(0);

        if current != expected_version {
            return Err(PersistenceError::Conflict {
                aggregate_id: aggregate_id.to_string(),
                expected: expected_version,
                actual: current,
            });
        }

        let new_version = current + 1;
        self.aggregates.insert(key, (aggregate, new_version));
        Ok(new_version)
    }

    fn load(&self, aggregate_id: &str, tenant_id: Option<&str>) -> Result<A, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_id.to_string(), tenant);

        match self.aggregates.get(&key) {
            Some((agg, _)) => Ok(agg.clone()),
            None => Err(PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            }),
        }
    }

    fn delete(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let key = (aggregate_id.to_string(), tenant);

        match self.aggregates.remove(&key) {
            Some(_) => Ok(()),
            None => Err(PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            }),
        }
    }
}

fn resolve_tenant(tenant_id: Option<&str>) -> Result<Option<String>, PersistenceError> {
    match tenant_id {
        Some("") => Err(PersistenceError::MissingTenant),
        Some(t) => Ok(Some(t.to_string())),
        None => Ok(None),
    }
}
