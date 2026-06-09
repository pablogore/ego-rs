use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A tenant ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantId {
    /// The ID of the tenant.
    pub id: String,
}

impl TenantId {
    /// Creates a new tenant ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

/// An error that can occur with tenants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TenantError {
    /// A tenant was not found.
    TenantNotFound,
}

/// A tenant context.
#[derive(Debug, Clone)]
pub struct TenantContext {
    /// The current tenant ID.
    pub tenant_id: Option<TenantId>,
    /// Whether cross-tenant access is allowed.
    pub allow_cross_tenant: bool,
}

impl TenantContext {
    /// Creates a new tenant context.
    pub fn new() -> Self {
        Self {
            tenant_id: None,
            allow_cross_tenant: false,
        }
    }

    /// Sets the tenant ID.
    pub fn with_tenant_id(mut self, tenant_id: TenantId) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Allows cross-tenant access.
    pub fn allow_cross_tenant(mut self) -> Self {
        self.allow_cross_tenant = true;
        self
    }
}

impl Default for TenantContext {
    fn default() -> Self {
        Self::new()
    }
}