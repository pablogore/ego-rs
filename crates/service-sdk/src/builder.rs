//! Service builder for constructing service instances.
//!
//! This module provides a builder pattern for constructing service instances
//! with configurable options.

use crate::contract::ServiceDescriptor;
use crate::error::{Result as ServiceResult, ServiceError};
use crate::implementation::Service;
use async_trait::async_trait;
use std::collections::HashMap;

/// Service builder for constructing service instances.
///
/// The builder pattern allows for configurable construction of service instances
/// with various options and dependencies.
pub struct ServiceBuilder {
    /// Service descriptor
    #[allow(dead_code)]
    descriptor: ServiceDescriptor,
    /// Configuration options
    config: HashMap<String, String>,
}

impl ServiceBuilder {
    /// Creates a new service builder.
    pub fn new(descriptor: ServiceDescriptor) -> Self {
        Self {
            descriptor,
            config: HashMap::new(),
        }
    }

    /// Sets a configuration option.
    pub fn with_config(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.insert(key.into(), value.into());
        self
    }

    /// Builds the service instance.
    pub async fn build(&self) -> ServiceResult<Box<dyn Service>> {
        // In a real implementation, this would create and configure the service
        // For now, we'll return an error to indicate this is a placeholder
        Err(ServiceError::internal("Service builder not implemented"))
    }
}

/// Service builder trait.
///
/// Allows services to provide their own builder implementations.
#[async_trait]
pub trait ServiceBuilderTrait: Send + Sync {
    /// Creates a service builder for this service.
    fn builder(&self) -> ServiceBuilder;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ContractVersion;

    #[test]
    fn test_service_builder_creation() {
        let descriptor = ServiceDescriptor {
            name: "TestService".to_string(),
            version: ContractVersion::new(1, 0, 0),
            operations: vec![],
            description: None,
            metadata: std::collections::HashMap::new(),
        };

        let builder = ServiceBuilder::new(descriptor);
        assert_eq!(builder.descriptor.name, "TestService");
    }
}
