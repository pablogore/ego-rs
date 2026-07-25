//! Adapts a registered [`ExternalDataProvider`] into a
//! [`ego_domain::health::HealthContributor`] (PROD-005 PR3 TASK-020/021).
//!
//! This is the SOLE channel through which a provider's self-reported
//! [`ProviderHealth`] reaches the runtime's health aggregation surface
//! (`Runtime::readiness`/`Runtime::startup`) — the previous parallel
//! `ProviderSubsystemReadiness`/`RuntimeDataProviderAccess::readiness()`
//! surface is removed in the same change (TASK-024), so there is exactly one
//! registration authority (ADR-7).

use std::sync::Arc;

use async_trait::async_trait;
use ego_domain::health::{DependencyRequirement, HealthCheck, HealthContributor, HealthCode, HealthStatus};

use super::provider::{ExternalDataProvider, ProviderHealth};

/// Wraps a registered `Arc<dyn ExternalDataProvider>` so it participates in
/// health aggregation as a [`HealthContributor`].
///
/// `requirement()` is always [`DependencyRequirement::Required`] — issue
/// #234's "registered = required" rule, preserved verbatim from the removed
/// `ProviderSubsystemReadiness::is_ready` semantics: every registered
/// provider must be healthy for the aggregate to be healthy, with no
/// separate required/optional concept at the provider layer.
pub struct ProviderHealthContributor {
    provider_id: String,
    provider: Arc<dyn ExternalDataProvider>,
}

impl ProviderHealthContributor {
    /// Wraps `provider`, identified by `provider_id` — the same identity
    /// `ExternalDataProviderRegistry` registers it under, and the same value
    /// the removed `ProviderSubsystemReadiness` used to sort/report by.
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

    struct FixedHealthProvider {
        health: ProviderHealth,
    }

    #[async_trait]
    impl ExternalDataProvider for FixedHealthProvider {
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

    #[tokio::test]
    async fn name_returns_the_provider_id_it_was_constructed_with() {
        let contributor = ProviderHealthContributor::new(
            "pricing",
            Arc::new(FixedHealthProvider {
                health: ProviderHealth::Healthy,
            }),
        );

        assert_eq!(contributor.name(), "pricing");
    }

    #[tokio::test]
    async fn requirement_defaults_to_required_preserving_registered_equals_required() {
        let contributor = ProviderHealthContributor::new(
            "pricing",
            Arc::new(FixedHealthProvider {
                health: ProviderHealth::Healthy,
            }),
        );

        assert_eq!(contributor.requirement(), DependencyRequirement::Required);
    }

    #[tokio::test]
    async fn healthy_provider_maps_to_healthy_check_with_no_code() {
        let contributor = ProviderHealthContributor::new(
            "pricing",
            Arc::new(FixedHealthProvider {
                health: ProviderHealth::Healthy,
            }),
        );

        let check = contributor.check().await;

        assert_eq!(check.status, HealthStatus::Healthy);
        assert_eq!(check.code, None);
    }

    #[tokio::test]
    async fn unhealthy_provider_maps_to_unhealthy_check_with_dependency_failure_code() {
        let contributor = ProviderHealthContributor::new(
            "jwks",
            Arc::new(FixedHealthProvider {
                health: ProviderHealth::Unhealthy,
            }),
        );

        let check = contributor.check().await;

        assert_eq!(check.status, HealthStatus::Unhealthy);
        assert_eq!(check.code, Some(HealthCode::DependencyFailure));
    }
}
