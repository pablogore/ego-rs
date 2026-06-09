//! Service implementation traits and utilities.
//!
//! This module contains traits and utilities for implementing services.

use crate::contract::{ContractVersion, ServiceDescriptor};
use crate::error::Result as ServiceResult;
use async_trait::async_trait;
use std::collections::HashMap;

/// Service implementation trait.
///
/// All service implementations must implement this trait to be registered
/// with the service registry.
#[async_trait]
pub trait Service: Send + Sync {
    /// Returns the service descriptor for this service.
    fn descriptor(&self) -> &ServiceDescriptor;

    /// Returns the service name.
    fn name(&self) -> &str {
        &self.descriptor().name
    }

    /// Returns the service version.
    fn version(&self) -> &ContractVersion {
        &self.descriptor().version
    }

    /// Returns the service metadata.
    fn metadata(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Initializes the service.
    async fn initialize(&self) -> ServiceResult<()> {
        Ok(())
    }

    /// Shuts down the service.
    async fn shutdown(&self) -> ServiceResult<()> {
        Ok(())
    }
}

/// Service factory trait.
///
/// Used to create new instances of services.
#[async_trait]
pub trait ServiceFactory: Send + Sync {
    /// Creates a new instance of the service.
    async fn create(&self) -> ServiceResult<Box<dyn Service>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_service_descriptor() {
        let descriptor = ServiceDescriptor {
            name: "TestService".to_string(),
            version: ContractVersion::new(1, 0, 0),
            operations: vec![],
            description: None,
            metadata: std::collections::HashMap::new(),
        };

        let service = TestService { descriptor };
        assert_eq!(service.name(), "TestService");
        assert_eq!(service.version(), &ContractVersion::new(1, 0, 0));
    }

    struct TestService {
        descriptor: ServiceDescriptor,
    }

    #[async_trait]
    impl Service for TestService {
        fn descriptor(&self) -> &ServiceDescriptor {
            &self.descriptor
        }
    }
}
