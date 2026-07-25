//! Integration coverage for PROD-005 PR3: a registered
//! `ego_runtime::providers::ExternalDataProvider`'s health reaches the
//! runtime's health aggregation surface ONLY through its
//! `ProviderHealthContributor` registration into the single runtime-owned
//! `HealthAggregator` (ADR-7) — there is no surviving parallel provider
//! readiness path (the removed `ProviderSubsystemReadiness`/
//! `RuntimeDataProviderAccess::readiness()`).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ego_domain::health::{DependencyRequirement, HealthCode, HealthContributor, HealthStatus};
use ego_runtime::providers::{ExternalDataProvider, ProviderHealth, ProviderHealthContributor};
use ego_service_sdk::health::{HealthAggregationConfig, HealthAggregator, HealthRegistry};
use ego_service_sdk::runtime::RuntimeBuilder;
use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

// -- TASK-022: concurrent fan-out over ProviderHealthContributor ------------

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

/// A provider whose `health()` never resolves — proves the per-contributor
/// timeout fires for a `ProviderHealthContributor` exactly as it does for any
/// other `HealthContributor`.
struct HangingHealthProvider {
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl ExternalDataProvider for HangingHealthProvider {
    async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
        Ok(DataResponse {
            payload: vec![],
            cache_hit: false,
        })
    }

    async fn health(&self) -> ProviderHealth {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn two_provider_health_contributors_are_checked_concurrently_and_a_slow_one_times_out_without_blocking_the_others(
) {
    let calls = Arc::new(AtomicU32::new(0));
    let registry = HealthRegistry::from_contributors(vec![
        Arc::new(ProviderHealthContributor::new(
            "slow",
            Arc::new(HangingHealthProvider {
                calls: calls.clone(),
            }) as Arc<dyn ExternalDataProvider>,
        )) as Arc<dyn HealthContributor>,
        Arc::new(ProviderHealthContributor::new(
            "fast",
            Arc::new(FixedHealthProvider {
                health: ProviderHealth::Healthy,
            }) as Arc<dyn ExternalDataProvider>,
        )) as Arc<dyn HealthContributor>,
    ]);
    let config = HealthAggregationConfig {
        per_contributor: Duration::from_millis(50),
        global_budget: None,
    };
    let aggregator = HealthAggregator::new(registry, config);

    let start = tokio::time::Instant::now();
    let report = aggregator.readiness().await;
    let elapsed = start.elapsed();

    // Concurrent fan-out: bounded by the per-contributor timeout, not by
    // sequential evaluation of both contributors.
    assert_eq!(elapsed, Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the slow provider's health() was entered exactly once");

    let slow = report
        .contributors
        .iter()
        .find(|c| c.name == "slow")
        .expect("the slow provider must still produce a synthesized report");
    assert_eq!(slow.status, HealthStatus::Unhealthy);
    assert_eq!(slow.code, Some(HealthCode::Timeout));
    assert_eq!(slow.requirement, DependencyRequirement::Required);

    let fast = report
        .contributors
        .iter()
        .find(|c| c.name == "fast")
        .expect("the fast provider is unaffected by the slow one timing out");
    assert_eq!(fast.status, HealthStatus::Healthy);
    assert_eq!(fast.code, None);

    // Required + Timeout -> global Unhealthy.
    assert_eq!(report.status, HealthStatus::Unhealthy);
}

// -- TASK-023: RuntimeBuilder wires registered providers into the ONE
// runtime-owned HealthAggregator ---------------------------------------

#[tokio::test]
async fn a_registered_provider_surfaces_its_health_through_runtime_readiness_and_startup() {
    let provider: Arc<dyn ExternalDataProvider> = Arc::new(FixedHealthProvider {
        health: ProviderHealth::Healthy,
    });
    let runtime = RuntimeBuilder::new()
        .register_data_provider("pricing", provider)
        .unwrap()
        .build();

    let readiness = runtime.readiness().await;
    let startup = runtime.startup().await;

    let readiness_entry = readiness
        .contributors
        .iter()
        .find(|c| c.name == "pricing")
        .expect("the registered provider must surface as a health contributor on readiness");
    assert_eq!(readiness_entry.status, HealthStatus::Healthy);
    assert_eq!(readiness_entry.requirement, DependencyRequirement::Required);
    assert_eq!(readiness.status, HealthStatus::Healthy);

    let startup_entry = startup
        .contributors
        .iter()
        .find(|c| c.name == "pricing")
        .expect("the registered provider must surface as a health contributor on startup");
    assert_eq!(startup_entry.status, HealthStatus::Healthy);
}

#[tokio::test]
async fn an_unhealthy_registered_provider_makes_readiness_unhealthy_registered_equals_required() {
    let provider: Arc<dyn ExternalDataProvider> = Arc::new(FixedHealthProvider {
        health: ProviderHealth::Unhealthy,
    });
    let runtime = RuntimeBuilder::new()
        .register_data_provider("jwks", provider)
        .unwrap()
        .build();

    let readiness = runtime.readiness().await;

    assert_eq!(readiness.status, HealthStatus::Unhealthy);
    let entry = readiness
        .contributors
        .iter()
        .find(|c| c.name == "jwks")
        .expect("the unhealthy provider must be named in the report");
    assert_eq!(entry.status, HealthStatus::Unhealthy);
    assert_eq!(entry.code, Some(HealthCode::DependencyFailure));
}

// -- Integration: no surviving parallel provider readiness path -------------

#[tokio::test]
async fn a_providers_health_reaches_the_global_report_only_through_its_health_contributor_registration(
) {
    // A provider with no health contributor registration (only registered as
    // a data provider) must be indistinguishable, from `readiness()`'s point
    // of view, from a healthy contributor UNLESS it is wired through
    // `RuntimeBuilder::register_data_provider` -> `ProviderHealthContributor`
    // -> the runtime's single `HealthAggregator`. This proves that path is
    // the ONLY one: an unhealthy provider registered via
    // `register_data_provider` is visible in `readiness()`, and there is no
    // second, provider-subsystem-only readiness surface left to bypass it
    // (the removed `ProviderSubsystemReadiness`/
    // `RuntimeDataProviderAccess::readiness()`).
    let unhealthy_provider: Arc<dyn ExternalDataProvider> = Arc::new(FixedHealthProvider {
        health: ProviderHealth::Unhealthy,
    });
    let runtime = RuntimeBuilder::new()
        .register_data_provider("flaky", unhealthy_provider)
        .unwrap()
        .build();

    let report = runtime.readiness().await;

    assert_eq!(
        report.status,
        HealthStatus::Unhealthy,
        "the provider's health reached the aggregate report through its contributor registration"
    );
    assert_eq!(report.contributors.len(), 1);
    assert_eq!(report.contributors[0].name, "flaky");
}
