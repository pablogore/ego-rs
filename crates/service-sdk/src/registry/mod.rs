use std::collections::HashMap;

use crate::contract::ServiceDescriptor;
use serde::{Deserialize, Serialize};

/// A service registry for managing service implementations and their contracts.
///
/// The service registry is responsible for:
/// - Registering service implementations
/// - Resolving services by name and version
/// - Managing service dependencies
/// - Ensuring service contract compliance
pub struct ServiceRegistry {
    pub services: HashMap<String, ServiceDescriptor>,
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    /// Creates a new service registry.
    ///
    /// # Returns
    /// A new, empty `ServiceRegistry` instance
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    /// Returns the services in the registry.
    ///
    /// # Returns
    /// An immutable reference to the registry's services map
    pub fn services(&self) -> &HashMap<String, ServiceDescriptor> {
        &self.services
    }

    /// Checks if the registry is empty.
    ///
    /// # Returns
    /// `true` if the registry contains no services, `false` otherwise
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }
}

/// An error that can occur in the registry.
///
/// These errors represent various failure conditions that can occur when
/// working with the service registry, such as attempting to register duplicate
/// services or resolving non-existent services.
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
