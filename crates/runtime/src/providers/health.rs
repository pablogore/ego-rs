//! Provider health contributor (PROD-005 PR3, TASK-020/021).
//!
//! Bridges the CORE-019A [`ExternalDataProvider`] SPI's `health()` method to
//! the PROD-005 [`ego_domain::health::HealthContributor`] contract, so a
//! registered provider participates in the runtime-owned
//! [`ego_service_sdk::health::HealthAggregator`] the same way any other
//! `LifecycleManaged` component does — see
//! `ego_service_sdk::runtime::builder::RuntimeBuilder::register_data_provider`.

use std::sync::Arc;

use async_trait::async_trait;
use ego_domain::health::{
    DependencyRequirement, HealthCheck, HealthCode, HealthContributor, HealthStatus,
};

use super::provider::{ExternalDataProvider, ProviderHealth};

/// Adapts a single registered [`ExternalDataProvider`] into a
/// [`HealthContributor`], keyed by the `provider_id` it was registered
/// under (not any name the provider itself might expose — providers don't
/// expose one).
///
/// ALWAYS [`DependencyRequirement::Required`]: a registered data provider is,
/// by construction, a dependency the runtime relies on — there is no
/// "optional provider" concept in the CORE-019A registration contract.
pub struct ProviderHealthContributor {
    provider_id: String,
    provider: Arc<dyn ExternalDataProvider>,
}

impl ProviderHealthContributor {
    /// Wraps `provider`, reporting as `provider_id` under the aggregator.
    pub fn new(provider_id: impl Into<String>, provider: Arc<dyn ExternalDataProvider>) -> Self {
        Self {
            provider_id: provider_id.into(),
            provider,
        }
    }
}

#[async_trait]
impl HealthContributor for ProviderHealthContributor {
    fn name(&self) -> &str {
        &self.provider_id
    }

    fn requirement(&self) -> DependencyRequirement {
        DependencyRequirement::Required
    }

    async fn check(&self) -> HealthCheck {
        match self.provider.health().await {
            ProviderHealth::Healthy => HealthCheck {
                status: HealthStatus::Healthy,
                code: None,
            },
            ProviderHealth::Unhealthy => HealthCheck {
                status: HealthStatus::Unhealthy,
                code: Some(HealthCode::DependencyFailure),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

    struct StubProvider {
        health: ProviderHealth,
    }

    #[async_trait]
    impl ExternalDataProvider for StubProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(DataResponse {
                payload: vec![],
                cache_hit: false,
            })
        }

        async fn health(&self) -> ProviderHealth {
            self.health
        }
    }

    fn contributor(provider_id: &str, health: ProviderHealth) -> ProviderHealthContributor {
        ProviderHealthContributor::new(provider_id, Arc::new(StubProvider { health }))
    }

    #[tokio::test]
    async fn healthy_provider_maps_to_healthy_check_with_no_code() {
        let contributor = contributor("pricing", ProviderHealth::Healthy);

        let check = contributor.check().await;

        assert_eq!(check.status, HealthStatus::Healthy);
        assert_eq!(check.code, None);
    }

    #[tokio::test]
    async fn unhealthy_provider_maps_to_unhealthy_check_with_dependency_failure_code() {
        let contributor = contributor("pricing", ProviderHealth::Unhealthy);

        let check = contributor.check().await;

        assert_eq!(check.status, HealthStatus::Unhealthy);
        assert_eq!(check.code, Some(HealthCode::DependencyFailure));
    }

    #[test]
    fn requirement_is_always_required() {
        let contributor = contributor("pricing", ProviderHealth::Healthy);

        assert_eq!(contributor.requirement(), DependencyRequirement::Required);
    }

    #[test]
    fn name_returns_the_registered_provider_id() {
        let contributor = contributor("pricing-v2", ProviderHealth::Healthy);

        assert_eq!(contributor.name(), "pricing-v2");
    }
}
