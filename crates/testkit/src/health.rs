//! Static health contributor test double (PROD-005 PR3 TASK-027/028).
//!
//! Same-contract principle as `providers.rs`'s `StaticDataProvider`: a real
//! implementation of the real production
//! [`ego_domain::health::HealthContributor`] trait, not a look-alike.
//! `StaticHealthContributor` reports one canned [`HealthStatus`]/
//! [`DependencyRequirement`], with an optional artificial delay before
//! answering — useful for exercising a real
//! `ego_service_sdk::health::HealthAggregator`'s concurrency/timeout
//! behavior in a test without a real dependency.

use std::time::Duration;

use async_trait::async_trait;
use ego_domain::health::{
    DependencyRequirement, HealthCheck, HealthCode, HealthContributor, HealthStatus,
};

/// A contributor whose `check()` always reports the same canned
/// `(status, requirement)`, optionally after sleeping for a fixed `delay`.
pub struct StaticHealthContributor {
    status: HealthStatus,
    requirement: DependencyRequirement,
    delay: Option<Duration>,
}

impl StaticHealthContributor {
    /// A contributor that always reports `status`/`requirement`, with no
    /// artificial delay.
    pub fn new(status: HealthStatus, requirement: DependencyRequirement) -> Self {
        Self {
            status,
            requirement,
            delay: None,
        }
    }

    /// Sleeps for `delay` before answering `check()` — used to exercise a
    /// real `HealthAggregator`'s per-contributor/global timeout behavior
    /// deterministically under `#[tokio::test(start_paused = true)]`.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

#[async_trait]
impl HealthContributor for StaticHealthContributor {
    fn name(&self) -> &str {
        "static-health"
    }

    fn requirement(&self) -> DependencyRequirement {
        self.requirement
    }

    async fn check(&self) -> HealthCheck {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        match self.status {
            HealthStatus::Healthy => HealthCheck {
                status: HealthStatus::Healthy,
                code: None,
            },
            HealthStatus::Degraded => HealthCheck {
                status: HealthStatus::Degraded,
                code: Some(HealthCode::Unavailable),
            },
            HealthStatus::Unhealthy => HealthCheck {
                status: HealthStatus::Unhealthy,
                code: Some(HealthCode::DependencyFailure),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_service_sdk::health::{HealthAggregationConfig, HealthAggregator, HealthRegistry};
    use std::sync::Arc;

    #[test]
    fn name_is_stable() {
        let contributor =
            StaticHealthContributor::new(HealthStatus::Healthy, DependencyRequirement::Required);
        assert_eq!(contributor.name(), "static-health");
    }

    #[tokio::test]
    async fn healthy_has_no_code() {
        let contributor =
            StaticHealthContributor::new(HealthStatus::Healthy, DependencyRequirement::Required);
        let check = contributor.check().await;
        assert_eq!(check.status, HealthStatus::Healthy);
        assert_eq!(check.code, None);
    }

    #[tokio::test]
    async fn unhealthy_reports_dependency_failure_code() {
        let contributor =
            StaticHealthContributor::new(HealthStatus::Unhealthy, DependencyRequirement::Required);
        let check = contributor.check().await;
        assert_eq!(check.status, HealthStatus::Unhealthy);
        assert_eq!(check.code, Some(HealthCode::DependencyFailure));
    }

    /// RED (TASK-027): registered in a REAL `HealthRegistry`, evaluated via a
    /// REAL `HealthAggregator::readiness()` — no fake, no fold duplication.
    /// An `Optional` + `Unhealthy` contributor is clamped to global
    /// `Degraded` by `ego_domain::health::fold`'s own frozen contract.
    #[tokio::test]
    async fn an_optional_unhealthy_contributor_yields_global_degraded_through_the_real_aggregator()
    {
        let registry = HealthRegistry::from_contributors(vec![Arc::new(
            StaticHealthContributor::new(HealthStatus::Unhealthy, DependencyRequirement::Optional),
        )]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let report = aggregator.readiness().await;

        assert_eq!(report.status, HealthStatus::Degraded);
        assert_eq!(report.contributors.len(), 1);
        assert_eq!(report.contributors[0].name, "static-health");
        assert_eq!(
            report.contributors[0].code,
            Some(HealthCode::DependencyFailure)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn with_delay_sleeps_before_answering() {
        let contributor =
            StaticHealthContributor::new(HealthStatus::Healthy, DependencyRequirement::Required)
                .with_delay(Duration::from_millis(200));

        let start = tokio::time::Instant::now();
        let check = contributor.check().await;
        let elapsed = start.elapsed();

        assert_eq!(elapsed, Duration::from_millis(200));
        assert_eq!(check.status, HealthStatus::Healthy);
    }
}
