use blake3::Hasher;
use serde::Serialize;
use std::fmt;
use std::hash::{Hash, Hasher as StdHasher};

pub type TenantId = String;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct EntityTriple {
    pub tenant: TenantId,
    pub entity_type: &'static str,
    pub entity_id: String,
}

impl EntityTriple {
    pub fn new(tenant: TenantId, entity_type: &'static str, entity_id: impl Into<String>) -> Self {
        Self {
            tenant,
            entity_type,
            entity_id: entity_id.into(),
        }
    }
}

impl fmt::Display for EntityTriple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            self.tenant, self.entity_type, self.entity_id
        )
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct EntityId {
    pub tenant_id: TenantId,
    pub entity_type: &'static str,
    pub entity_id: String,
}

impl EntityId {
    pub fn new(tenant_id: TenantId, entity_type: &'static str, entity_id: impl Into<String>) -> Self {
        Self {
            tenant_id,
            entity_type,
            entity_id: entity_id.into(),
        }
    }

    pub fn to_triple(&self) -> EntityTriple {
        EntityTriple {
            tenant: self.tenant_id.clone(),
            entity_type: self.entity_type,
            entity_id: self.entity_id.clone(),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct ExecutionKey([u8; 32]);

impl ExecutionKey {
    pub fn compute(
        entity_id: &EntityId,
        command_payload: &impl Serialize,
        state_version: u64,
    ) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(entity_id.tenant_id.as_bytes());
        hasher.update(b"|");
        hasher.update(entity_id.entity_type.as_bytes());
        hasher.update(b"|");
        hasher.update(entity_id.entity_id.as_bytes());
        hasher.update(b"|");
        hasher.update(&state_version.to_le_bytes());

        if let Ok(json) = serde_json::to_vec(command_payload) {
            hasher.update(&json);
        }

        Self(hasher.finalize().into())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Hash for ExecutionKey {
    fn hash<H: StdHasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
