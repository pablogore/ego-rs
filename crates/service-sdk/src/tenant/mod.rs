use serde::{Deserialize, Serialize};

/// A tenant ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantId {
    /// The ID of the tenant.
    pub id: String,
}

/// An error that can occur with tenants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TenantError {
    /// A tenant was not found.
    TenantNotFound,
}
