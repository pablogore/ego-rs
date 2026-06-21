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
/// with the service registry. Lifecycle hooks (`initialize`/`shutdown`) are
/// intentionally absent — use [`LifecycleManaged`] for components that need them.
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
}

/// Lifecycle management trait for runtime-managed components (entities, projections, adapters).
///
/// This trait is separate from [`Service`] so that the `Service` trait remains minimal.
/// Only components that explicitly need startup/teardown hooks implement this.
/// The runtime drives `initialize()` on startup and `shutdown()` on teardown in reverse order.
#[async_trait]
pub trait LifecycleManaged: Send + Sync {
    /// Called once by the runtime when the component is starting up.
    /// The default implementation is a no-op.
    async fn initialize(&self) -> ServiceResult<()> {
        Ok(())
    }

    /// Called once by the runtime when the component is shutting down.
    /// The default implementation is a no-op.
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
