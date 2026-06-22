//! In-memory implementation of [`RoleStore`].

use std::collections::HashMap;

use async_trait::async_trait;

use crate::{
    error::SecurityError,
    principal::Role,
};

use super::{Permission, RoleStore};

/// An in-memory [`RoleStore`] backed by a `HashMap<Role, Vec<Permission>>`.
///
/// Intended for tests and local development. Not suitable for production
/// deployments that require persistent storage.
pub struct InMemoryRoleStore {
    grants: HashMap<Role, Vec<Permission>>,
}

impl InMemoryRoleStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self { grants: HashMap::new() }
    }

    /// Builder: grants `permissions` to `role`.
    pub fn with_role(mut self, role: Role, permissions: Vec<Permission>) -> Self {
        self.grants.insert(role, permissions);
        self
    }
}

impl Default for InMemoryRoleStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RoleStore for InMemoryRoleStore {
    async fn permissions_for_role(
        &self,
        role: &Role,
    ) -> Result<Vec<Permission>, SecurityError> {
        Ok(self.grants.get(role).cloned().unwrap_or_default())
    }
}
