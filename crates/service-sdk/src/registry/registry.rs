use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A service registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRegistry {
    /// The services in the registry.
    pub services: HashMap<String, ServiceDescriptor>,
}

/// An error that can occur in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistryError {
    /// A service with the same name and version already exists.
    DuplicateService,
    /// A service was not found.
    ServiceNotFound,
    /// A service dependency was not found.
    DependencyNotFound,
    /// A service dependency cycle was detected.
    DependencyCycle,
}