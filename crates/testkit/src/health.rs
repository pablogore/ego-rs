//! `StaticHealthContributor` test double (PROD-005 PR3 TASK-027/028).
//!
//! Same-contract principle as `providers.rs`'s `StaticDataProvider`: a real
//! implementation of the real production `ego_domain::health::HealthContributor`
//! trait, not a look-alike. Fixed to a canned `(status, requirement)` and an
//! optional artificial `delay`, so a test can drive a REAL
//! `ego_service_sdk::health::HealthAggregator` deterministically without
//! faking the aggregator itself.

use std::time::Duration;

use async_trait::async_trait;
use ego_domain::health::{DependencyRequirement, HealthCheck, HealthCode, HealthContributor, HealthStatus};

/// A contributor fixed to a canned `status`/`requirement`, optionally
/// delaying its `check()` response by `delay` (via `tokio::time::sleep`).
///
/// When `status` is not [`HealthStatus::Healthy`], `check()` reports a
/// [`HealthCode`] appropriate to the fixed status: [`HealthCode::Unavailable`]
/// for [`HealthStatus::Degraded`], [`HealthCode::DependencyFailure`] for
/// [`HealthStatus::Unhealthy`]. [`HealthStatus::Healthy`] always carries no
/// code, matching production contributors' contract.
pub struct StaticHealthContributor {
    name: String,
    status: HealthStatus,
    requirement: DependencyRequirement,
    delay: Option<Duration>,
}

impl StaticHealthContributor {
    /// A contributor named `name`, fixed to `status`/`requirement`, with no
    /// artificial delay.
    pub fn new(name: impl Into<String>, status: HealthStatus, requirement: DependencyRequirement) -> Self {
        Self {
            name: name.into(),
            status,
            requirement,
            delay: None,
        }
    }

    /// Adds an artificial `delay` before `check()` resolves — useful for
    /// exercising a real `HealthAggregator`'s per-contributor/global timeout
    /// behavior against a contributor that eventually completes (as opposed
    /// to one that never does).
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

#[async_trait]
impl HealthContributor for StaticHealthContributor {
    fn name(&self) -> &str {
        &self.name
    }

    fn requirement(&self) -> DependencyRequirement {
        self.requirement
    }

    async fn check(&self) -> HealthCheck {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        let code = match self.status {
            HealthStatus::Healthy => None,
            HealthStatus::Degraded => Some(HealthCode::Unavailable),
            HealthStatus::Unhealthy => Some(HealthCode::DependencyFailure),
        };
        HealthCheck {
            status: self.status,
            code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_service_sdk::health::{HealthAggregationConfig, HealthAggregator, HealthRegistry};
    use std::sync::Arc;

    #[tokio::test]
    async fn healthy_static_contributor_reports_healthy_with_no_code() {
        let contributor =
            StaticHealthContributor::new("db", HealthStatus::Healthy, DependencyRequirement::Required);

        let check = contributor.check().await;

        assert_eq!(check.status, HealthStatus::Healthy);
        assert_eq!(check.code, None);
    }

    /// TASK-027/028's core scenario: an `Optional` + `Unhealthy` contributor,
    /// registered into a REAL `HealthAggregator` (no aggregator faking),
    /// deterministically drives the aggregate to `Degraded` — matching
    /// production semantics exactly (`fold`'s "Optional + Unhealthy clamps to
    /// Degraded" contract).
    #[tokio::test]
    async fn optional_unhealthy_static_contributor_drives_a_real_aggregator_to_degraded() {
        let contributor = Arc::new(StaticHealthContributor::new(
            "cache",
            HealthStatus::Unhealthy,
            DependencyRequirement::Optional,
        ));
        let registry = HealthRegistry::from_contributors(vec![contributor]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let report = aggregator.readiness().await;

        assert_eq!(report.status, HealthStatus::Degraded);
        assert_eq!(report.contributors.len(), 1);
        assert_eq!(report.contributors[0].name, "cache");
        assert_eq!(report.contributors[0].status, HealthStatus::Unhealthy);
        assert_eq!(
            report.contributors[0].code,
            Some(HealthCode::DependencyFailure)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn with_delay_defers_the_check_by_the_configured_duration() {
        let contributor = StaticHealthContributor::new(
            "warmer",
            HealthStatus::Healthy,
            DependencyRequirement::Required,
        )
        .with_delay(Duration::from_millis(200));

        let start = tokio::time::Instant::now();
        let check = contributor.check().await;
        let elapsed = start.elapsed();

        assert_eq!(elapsed, Duration::from_millis(200));
        assert_eq!(check.status, HealthStatus::Healthy);
    }
}
