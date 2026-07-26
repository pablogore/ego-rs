//! Service implementation traits and utilities.
//!
//! This module contains traits and utilities for implementing services.

use crate::contract::{ContractVersion, ServiceDescriptor};
use crate::error::Result as ServiceResult;
use async_trait::async_trait;
use ego_domain::health::HealthContributor;
use std::collections::HashMap;
use std::sync::Arc;

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
///
/// The runtime currently consumes only [`health_contributors()`](Self::health_contributors)
/// (folded into the runtime-owned health aggregator by the builder).
/// [`initialize()`](Self::initialize) and [`shutdown()`](Self::shutdown) are
/// opt-in hooks that the runtime does not yet drive — implementing them has no
/// effect on startup or teardown today.
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

    /// Returns this component's health contributors, if any (PROD-005 PR2
    /// TASK-016/017). The default implementation returns an empty `Vec` —
    /// non-breaking: every existing implementor compiles unchanged and
    /// contributes nothing to health aggregation unless it explicitly
    /// overrides this method.
    fn health_contributors(&self) -> Vec<Arc<dyn HealthContributor>> {
        Vec::new()
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

    // -- TASK-016/017: LifecycleManaged::health_contributors default --------

    use ego_domain::health::{DependencyRequirement, HealthCheck, HealthContributor, HealthStatus};
    use std::sync::Arc;

    struct DefaultLifecycle;

    #[async_trait]
    impl LifecycleManaged for DefaultLifecycle {
        // Uses the default `health_contributors()` — no override.
    }

    struct StubHealthContributor;

    #[async_trait]
    impl HealthContributor for StubHealthContributor {
        fn name(&self) -> &str {
            "stub"
        }

        fn requirement(&self) -> DependencyRequirement {
            DependencyRequirement::Required
        }

        async fn check(&self) -> HealthCheck {
            HealthCheck {
                status: HealthStatus::Healthy,
                code: None,
            }
        }
    }

    struct ContributingLifecycle;

    #[async_trait]
    impl LifecycleManaged for ContributingLifecycle {
        fn health_contributors(&self) -> Vec<Arc<dyn HealthContributor>> {
            vec![Arc::new(StubHealthContributor)]
        }
    }

    #[test]
    fn default_health_contributors_is_empty() {
        let component = DefaultLifecycle;
        assert!(component.health_contributors().is_empty());
    }

    #[test]
    fn overridden_health_contributors_returns_non_empty() {
        let component = ContributingLifecycle;
        let contributors = component.health_contributors();
        assert_eq!(contributors.len(), 1);
        assert_eq!(contributors[0].name(), "stub");
    }
}
