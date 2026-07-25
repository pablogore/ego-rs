//! Health aggregation (PROD-005 PR2).
//!
//! Consumes the PR1 domain health contract (`ego_domain::health`) to build a
//! concurrent, timeout-safe aggregator over registered
//! [`HealthContributor`]s. This module owns:
//!
//! - [`HealthRegistry`] — the set of registered contributors.
//! - [`HealthAggregationConfig`] — per-contributor timeout + optional global budget.
//! - [`HealthAggregator`] — the concurrent fan-out that folds contributor
//!   reports into a [`HealthReport`] for exactly two aggregatable probes:
//!   [`HealthAggregator::readiness`] and [`HealthAggregator::startup`].
//!
//! `ProbeKind::Liveness` is deliberately NOT expressible through this
//! aggregator — liveness is a process-internal check that consults no
//! contributor (see `Runtime::liveness` in `crate::runtime`).

use std::sync::Arc;
use std::time::Duration;

use ego_domain::health::{
    fold, ContributorReport, DependencyRequirement, HealthCode, HealthContributor, HealthReport,
    HealthStatus, ProbeKind,
};
use futures::stream::FuturesUnordered;
use futures::StreamExt;

/// The set of registered health contributors (TASK-006).
///
/// Cheap to clone (`Vec<Arc<dyn HealthContributor>>`) — a `HealthAggregator`
/// owns its own copy rather than borrowing, so callers don't need to keep the
/// registry alive independently.
#[derive(Clone, Default)]
pub struct HealthRegistry {
    contributors: Vec<Arc<dyn HealthContributor>>,
}

impl HealthRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a single contributor.
    pub fn register(&mut self, contributor: Arc<dyn HealthContributor>) {
        self.contributors.push(contributor);
    }

    /// Builds a registry from an already-collected list of contributors —
    /// the shape `RuntimeBuilder::build()` uses to fold every registered
    /// lifecycle component's `health_contributors()` into one registry.
    pub fn from_contributors(contributors: Vec<Arc<dyn HealthContributor>>) -> Self {
        Self { contributors }
    }

    /// Returns the registered contributors.
    pub fn contributors(&self) -> &[Arc<dyn HealthContributor>] {
        &self.contributors
    }
}

/// Per-contributor timeout + optional global aggregation budget (TASK-006).
///
/// `per_contributor` bounds each individual `HealthContributor::check()`
/// call (TASK-010/011). `global_budget`, if set, additionally bounds the
/// whole `readiness()`/`startup()` call — contributors still pending when it
/// elapses are synthesized as `Unhealthy` + `HealthCode::Timeout` without
/// losing their `name`/`requirement` identity (TASK-012/013).
#[derive(Debug, Clone, Copy)]
pub struct HealthAggregationConfig {
    /// Per-contributor `check()` timeout.
    pub per_contributor: Duration,
    /// Optional whole-aggregation deadline.
    pub global_budget: Option<Duration>,
}

impl Default for HealthAggregationConfig {
    fn default() -> Self {
        Self {
            per_contributor: Duration::from_secs(5),
            global_budget: None,
        }
    }
}

/// Concurrent, timeout-safe aggregator over a [`HealthRegistry`] (TASK-007).
///
/// Exposes ONLY [`HealthAggregator::readiness`] and
/// [`HealthAggregator::startup`] — there is deliberately no public
/// `aggregate(ProbeKind)` entry point, so `ProbeKind::Liveness` cannot be
/// expressed as an aggregation call (liveness is process-internal only, see
/// `crate::runtime::Runtime::liveness`).
#[derive(Clone)]
pub struct HealthAggregator {
    registry: HealthRegistry,
    config: HealthAggregationConfig,
}

impl HealthAggregator {
    /// Builds an aggregator over `registry`, using `config` for every
    /// `readiness()`/`startup()` call.
    pub fn new(registry: HealthRegistry, config: HealthAggregationConfig) -> Self {
        Self { registry, config }
    }

    /// Evaluates every registered contributor and folds the result into a
    /// [`HealthReport`] tagged [`ProbeKind::Readiness`].
    ///
    /// Uses the IDENTICAL fold as [`HealthAggregator::startup`] — the two
    /// methods differ ONLY in the `probe` tag on the returned report
    /// (TASK-025/026).
    pub async fn readiness(&self) -> HealthReport {
        self.aggregate(ProbeKind::Readiness).await
    }

    /// Evaluates every registered contributor and folds the result into a
    /// [`HealthReport`] tagged [`ProbeKind::Startup`].
    ///
    /// Uses the IDENTICAL fold as [`HealthAggregator::readiness`] — see that
    /// method's doc.
    pub async fn startup(&self) -> HealthReport {
        self.aggregate(ProbeKind::Startup).await
    }

    /// Shared aggregation core — probe-independent per
    /// `ego_domain::health::fold`'s own contract. Private: the only public
    /// surface is the pair of probe-tagged methods above.
    async fn aggregate(&self, probe: ProbeKind) -> HealthReport {
        let contributors = collect_reports(&self.registry, &self.config).await;
        let status = fold(&contributors);
        HealthReport {
            probe,
            status,
            contributors,
        }
    }
}

/// Fans every contributor's `check()` out concurrently via
/// [`FuturesUnordered`] (TASK-008/009 — a slow contributor never delays the
/// others), enforcing `config.per_contributor` on each call (TASK-010/011)
/// and, if set, `config.global_budget` on the whole batch (TASK-012/013).
async fn collect_reports(
    registry: &HealthRegistry,
    config: &HealthAggregationConfig,
) -> Vec<ContributorReport> {
    let per_contributor = config.per_contributor;

    // Each in-flight future carries an internal positional `id` alongside its
    // report. `id` — never `name()` — is the bookkeeping key: PR1 does not
    // guarantee `name()` uniqueness (it is public metadata), so keying by name
    // would collapse two same-named contributors and silently drop one.
    let mut in_flight: FuturesUnordered<_> = registry
        .contributors()
        .iter()
        .cloned()
        .enumerate()
        .map(|(id, contributor)| async move {
            let name = contributor.name().to_string();
            let requirement = contributor.requirement();
            let report = match tokio::time::timeout(per_contributor, contributor.check()).await {
                Ok(check) => ContributorReport {
                    name,
                    status: check.status,
                    requirement,
                    code: check.code,
                },
                Err(_elapsed) => ContributorReport {
                    name,
                    status: HealthStatus::Unhealthy,
                    requirement,
                    code: Some(HealthCode::Timeout),
                },
            };
            (id, report)
        })
        .collect();

    let Some(global_budget) = config.global_budget else {
        let mut reports = Vec::with_capacity(in_flight.len());
        while let Some((_id, report)) = in_flight.next().await {
            reports.push(report);
        }
        return reports;
    };

    // Retain (name, requirement) identity for every registered contributor
    // up front, keyed by internal `id`, so a contributor STILL PENDING when
    // the global budget elapses can still be attributed a synthetic report —
    // the global timeout must never collapse into a single, contributor-less
    // error, and same-named contributors must each survive.
    let mut pending: std::collections::HashMap<usize, (String, DependencyRequirement)> = registry
        .contributors()
        .iter()
        .enumerate()
        .map(|(id, c)| (id, (c.name().to_string(), c.requirement())))
        .collect();

    let mut reports = Vec::with_capacity(pending.len());
    let sleep = tokio::time::sleep(global_budget);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            biased;
            next = in_flight.next() => {
                match next {
                    Some((id, report)) => {
                        pending.remove(&id);
                        reports.push(report);
                        if pending.is_empty() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = &mut sleep => {
                for (_id, (name, requirement)) in pending.drain() {
                    reports.push(ContributorReport {
                        name,
                        status: HealthStatus::Unhealthy,
                        requirement,
                        code: Some(HealthCode::Timeout),
                    });
                }
                break;
            }
        }
    }

    reports
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;

    /// A trivial stub contributor, mirroring `ego_domain::health`'s own test
    /// stub, for exercising `HealthRegistry`/`HealthAggregator` in isolation.
    struct StubContributor {
        name: String,
        requirement: DependencyRequirement,
        status: HealthStatus,
        code: Option<HealthCode>,
    }

    #[async_trait]
    impl HealthContributor for StubContributor {
        fn name(&self) -> &str {
            &self.name
        }

        fn requirement(&self) -> DependencyRequirement {
            self.requirement
        }

        async fn check(&self) -> ego_domain::health::HealthCheck {
            ego_domain::health::HealthCheck {
                status: self.status,
                code: self.code,
            }
        }
    }

    fn stub(
        name: &str,
        requirement: DependencyRequirement,
        status: HealthStatus,
        code: Option<HealthCode>,
    ) -> Arc<dyn HealthContributor> {
        Arc::new(StubContributor {
            name: name.to_string(),
            requirement,
            status,
            code,
        })
    }

    // -- TASK-006/007: HealthRegistry + HealthAggregator basic fold ---------

    #[tokio::test]
    async fn readiness_folds_two_healthy_contributors_deterministically() {
        let registry = HealthRegistry::from_contributors(vec![
            stub(
                "db",
                DependencyRequirement::Required,
                HealthStatus::Healthy,
                None,
            ),
            stub(
                "cache",
                DependencyRequirement::Optional,
                HealthStatus::Healthy,
                None,
            ),
        ]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let report = aggregator.readiness().await;

        assert_eq!(report.probe, ProbeKind::Readiness);
        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.contributors.len(), 2);
    }

    #[tokio::test]
    async fn startup_folds_two_contributors_and_tags_startup_probe() {
        let registry = HealthRegistry::from_contributors(vec![
            stub(
                "db",
                DependencyRequirement::Required,
                HealthStatus::Healthy,
                None,
            ),
            stub(
                "warmer",
                DependencyRequirement::Optional,
                HealthStatus::Unhealthy,
                Some(HealthCode::InitializationPending),
            ),
        ]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let report = aggregator.startup().await;

        assert_eq!(report.probe, ProbeKind::Startup);
        // Optional + Unhealthy clamps to Degraded (fold's frozen contract).
        assert_eq!(report.status, HealthStatus::Degraded);
    }

    /// `HealthAggregator` exposes ONLY `readiness()`/`startup()` — there is
    /// no `aggregate(ProbeKind)` public entry point, so `ProbeKind::Liveness`
    /// cannot be expressed as an aggregation call. This is a structural
    /// (compile-time) assertion: if a public `aggregate` method existed, this
    /// test would still compile and pass, so the real enforcement is code
    /// review + the absence of such a method in this file — this test only
    /// pins the two that DO exist.
    #[tokio::test]
    async fn only_readiness_and_startup_are_the_aggregatable_probes() {
        let registry = HealthRegistry::from_contributors(vec![]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let readiness = aggregator.readiness().await;
        let startup = aggregator.startup().await;

        assert_eq!(readiness.probe, ProbeKind::Readiness);
        assert_eq!(startup.probe, ProbeKind::Startup);
    }

    #[tokio::test]
    async fn empty_registry_yields_healthy_for_both_probes() {
        let registry = HealthRegistry::from_contributors(vec![]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        assert_eq!(aggregator.readiness().await.status, HealthStatus::Healthy);
        assert_eq!(aggregator.startup().await.status, HealthStatus::Healthy);
    }

    // -- TASK-025/026: startup vs readiness use the IDENTICAL fold; the
    // frozen 5-row table distinguishing InitializationPending from
    // DependencyFailure by `code`, never by a status remap. ------------------

    #[tokio::test]
    async fn frozen_table_required_initialization_pending_is_global_unhealthy_with_code() {
        let registry = HealthRegistry::from_contributors(vec![stub(
            "queue",
            DependencyRequirement::Required,
            HealthStatus::Unhealthy,
            Some(HealthCode::InitializationPending),
        )]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let readiness = aggregator.readiness().await;
        let startup = aggregator.startup().await;

        for report in [&readiness, &startup] {
            assert_eq!(report.status, HealthStatus::Unhealthy);
            assert_eq!(
                report.contributors[0].code,
                Some(HealthCode::InitializationPending)
            );
        }
        // Probe tag is the ONLY difference between the two calls.
        assert_eq!(readiness.probe, ProbeKind::Readiness);
        assert_eq!(startup.probe, ProbeKind::Startup);
    }

    #[tokio::test]
    async fn frozen_table_required_dependency_failure_is_global_unhealthy_with_code() {
        let registry = HealthRegistry::from_contributors(vec![stub(
            "queue",
            DependencyRequirement::Required,
            HealthStatus::Unhealthy,
            Some(HealthCode::DependencyFailure),
        )]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let report = aggregator.readiness().await;

        assert_eq!(report.status, HealthStatus::Unhealthy);
        assert_eq!(
            report.contributors[0].code,
            Some(HealthCode::DependencyFailure)
        );
    }

    #[tokio::test]
    async fn frozen_table_distinguishes_initialization_pending_from_dependency_failure_by_code_only(
    ) {
        let pending_registry = HealthRegistry::from_contributors(vec![stub(
            "queue",
            DependencyRequirement::Required,
            HealthStatus::Unhealthy,
            Some(HealthCode::InitializationPending),
        )]);
        let failure_registry = HealthRegistry::from_contributors(vec![stub(
            "queue",
            DependencyRequirement::Required,
            HealthStatus::Unhealthy,
            Some(HealthCode::DependencyFailure),
        )]);

        let pending = HealthAggregator::new(pending_registry, HealthAggregationConfig::default())
            .readiness()
            .await;
        let failure = HealthAggregator::new(failure_registry, HealthAggregationConfig::default())
            .readiness()
            .await;

        // Same global status (both Required+Unhealthy)...
        assert_eq!(pending.status, HealthStatus::Unhealthy);
        assert_eq!(failure.status, HealthStatus::Unhealthy);
        // ...but distinguishable by code.
        assert_ne!(pending.contributors[0].code, failure.contributors[0].code);
    }

    #[tokio::test]
    async fn frozen_table_optional_initialization_pending_is_degraded() {
        let registry = HealthRegistry::from_contributors(vec![stub(
            "warmer",
            DependencyRequirement::Optional,
            HealthStatus::Unhealthy,
            Some(HealthCode::InitializationPending),
        )]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let report = aggregator.readiness().await;

        assert_eq!(report.status, HealthStatus::Degraded);
        assert_eq!(
            report.contributors[0].code,
            Some(HealthCode::InitializationPending)
        );
    }

    #[tokio::test]
    async fn frozen_table_healthy_contributor_has_no_code_and_yields_healthy() {
        let registry = HealthRegistry::from_contributors(vec![stub(
            "db",
            DependencyRequirement::Required,
            HealthStatus::Healthy,
            None,
        )]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let report = aggregator.readiness().await;

        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.contributors[0].code, None);
    }

    // -- TASK-008/009: concurrent fan-out via FuturesUnordered --------------

    /// A contributor whose `check()` sleeps for a configurable duration —
    /// used to prove fan-out is concurrent, not sequential.
    struct SlowContributor {
        name: String,
        delay: Duration,
    }

    #[async_trait]
    impl HealthContributor for SlowContributor {
        fn name(&self) -> &str {
            &self.name
        }

        fn requirement(&self) -> DependencyRequirement {
            DependencyRequirement::Optional
        }

        async fn check(&self) -> ego_domain::health::HealthCheck {
            tokio::time::sleep(self.delay).await;
            ego_domain::health::HealthCheck {
                status: HealthStatus::Healthy,
                code: None,
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn one_slow_contributor_does_not_delay_the_others() {
        let registry = HealthRegistry::from_contributors(vec![
            Arc::new(SlowContributor {
                name: "slow".to_string(),
                delay: Duration::from_millis(500),
            }),
            stub(
                "fast-a",
                DependencyRequirement::Optional,
                HealthStatus::Healthy,
                None,
            ),
            stub(
                "fast-b",
                DependencyRequirement::Optional,
                HealthStatus::Healthy,
                None,
            ),
        ]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let start = tokio::time::Instant::now();
        let report = aggregator.readiness().await;
        let elapsed = start.elapsed();

        // Sequential evaluation would take >= 500ms + fast contributors;
        // concurrent fan-out completes in ~the slowest single contributor's
        // time. Under `start_paused`, virtual time advances only as far as
        // needed to resolve every pending timer, so this bound is exact, not
        // a flaky wall-clock race.
        assert_eq!(elapsed, Duration::from_millis(500));
        assert_eq!(report.contributors.len(), 3);
        assert_eq!(report.status, HealthStatus::Healthy);
    }

    // -- TASK-010/011: per-contributor timeout ------------------------------

    /// A contributor whose `check()` never resolves — used to prove the
    /// per-contributor timeout fires and synthesizes a report instead of
    /// hanging the whole aggregation.
    struct HangingContributor {
        name: String,
        requirement: DependencyRequirement,
    }

    #[async_trait]
    impl HealthContributor for HangingContributor {
        fn name(&self) -> &str {
            &self.name
        }

        fn requirement(&self) -> DependencyRequirement {
            self.requirement
        }

        async fn check(&self) -> ego_domain::health::HealthCheck {
            std::future::pending().await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn per_contributor_timeout_synthesizes_timeout_report_without_hanging() {
        let registry = HealthRegistry::from_contributors(vec![
            Arc::new(HangingContributor {
                name: "wedged".to_string(),
                requirement: DependencyRequirement::Required,
            }),
            stub(
                "ok",
                DependencyRequirement::Required,
                HealthStatus::Healthy,
                None,
            ),
        ]);
        let config = HealthAggregationConfig {
            per_contributor: Duration::from_millis(50),
            global_budget: None,
        };
        let aggregator = HealthAggregator::new(registry, config);

        let report = aggregator.readiness().await;

        let wedged = report
            .contributors
            .iter()
            .find(|c| c.name == "wedged")
            .expect("wedged contributor must still produce a report");
        assert_eq!(wedged.status, HealthStatus::Unhealthy);
        assert_eq!(wedged.code, Some(HealthCode::Timeout));

        let ok = report
            .contributors
            .iter()
            .find(|c| c.name == "ok")
            .expect("other contributors are unaffected by one timing out");
        assert_eq!(ok.status, HealthStatus::Healthy);

        // Required + per-contributor Timeout -> global Unhealthy.
        assert_eq!(report.status, HealthStatus::Unhealthy);
    }

    // -- TASK-012/013: optional global budget --------------------------------

    #[tokio::test(start_paused = true)]
    async fn global_budget_synthesizes_reports_for_every_still_pending_contributor() {
        let registry = HealthRegistry::from_contributors(vec![
            Arc::new(HangingContributor {
                name: "wedged-a".to_string(),
                requirement: DependencyRequirement::Required,
            }),
            Arc::new(HangingContributor {
                name: "wedged-b".to_string(),
                requirement: DependencyRequirement::Optional,
            }),
            stub(
                "ok",
                DependencyRequirement::Required,
                HealthStatus::Healthy,
                None,
            ),
        ]);
        let config = HealthAggregationConfig {
            // Per-contributor timeout longer than the global budget, so the
            // global budget — not the per-contributor timeout — is what
            // fires for the two hanging contributors.
            per_contributor: Duration::from_secs(60),
            global_budget: Some(Duration::from_millis(100)),
        };
        let aggregator = HealthAggregator::new(registry, config);

        let report = aggregator.readiness().await;

        // All three contributors are attributable — the global timeout must
        // never collapse into a single, contributor-less error.
        assert_eq!(report.contributors.len(), 3);

        let wedged_a = report
            .contributors
            .iter()
            .find(|c| c.name == "wedged-a")
            .expect("wedged-a must be attributable even though it never completed");
        assert_eq!(wedged_a.status, HealthStatus::Unhealthy);
        assert_eq!(wedged_a.code, Some(HealthCode::Timeout));
        assert_eq!(wedged_a.requirement, DependencyRequirement::Required);

        let wedged_b = report
            .contributors
            .iter()
            .find(|c| c.name == "wedged-b")
            .expect("wedged-b must be attributable even though it never completed");
        assert_eq!(wedged_b.status, HealthStatus::Unhealthy);
        assert_eq!(wedged_b.code, Some(HealthCode::Timeout));
        assert_eq!(wedged_b.requirement, DependencyRequirement::Optional);

        let ok = report.contributors.iter().find(|c| c.name == "ok").expect(
            "a contributor that completes before the global deadline keeps its real report",
        );
        assert_eq!(ok.status, HealthStatus::Healthy);
        assert_eq!(ok.code, None);

        // Required + Timeout -> global Unhealthy.
        assert_eq!(report.status, HealthStatus::Unhealthy);
    }

    #[tokio::test]
    async fn global_budget_never_fires_when_all_contributors_complete_in_time() {
        let registry = HealthRegistry::from_contributors(vec![stub(
            "fast",
            DependencyRequirement::Required,
            HealthStatus::Healthy,
            None,
        )]);
        let config = HealthAggregationConfig {
            per_contributor: Duration::from_secs(5),
            global_budget: Some(Duration::from_secs(5)),
        };
        let aggregator = HealthAggregator::new(registry, config);

        let report = aggregator.readiness().await;

        assert_eq!(report.contributors.len(), 1);
        assert_eq!(report.contributors[0].code, None);
        assert_eq!(report.status, HealthStatus::Healthy);
    }

    /// PR1 does NOT guarantee `HealthContributor::name()` is unique — it is
    /// public metadata, not an identity key. Two contributors sharing a name
    /// must EACH be preserved when the global budget expires while both are
    /// pending; internal bookkeeping keyed by name would collapse them into
    /// one, silently dropping a registered contributor from the report.
    #[tokio::test(start_paused = true)]
    async fn two_contributors_with_same_name_are_preserved_when_global_budget_expires() {
        let registry = HealthRegistry::from_contributors(vec![
            Arc::new(HangingContributor {
                name: "db".to_string(),
                requirement: DependencyRequirement::Required,
            }),
            Arc::new(HangingContributor {
                name: "db".to_string(),
                requirement: DependencyRequirement::Optional,
            }),
        ]);
        let config = HealthAggregationConfig {
            per_contributor: Duration::from_secs(60),
            global_budget: Some(Duration::from_millis(100)),
        };
        let aggregator = HealthAggregator::new(registry, config);

        let report = aggregator.readiness().await;

        // Both same-named contributors must be attributable — not deduped.
        assert_eq!(report.contributors.len(), 2);
        assert_eq!(
            report
                .contributors
                .iter()
                .filter(|c| c.name == "db")
                .count(),
            2
        );
        for c in &report.contributors {
            assert_eq!(c.status, HealthStatus::Unhealthy);
            assert_eq!(c.code, Some(HealthCode::Timeout));
        }
        // Required + Timeout -> global Unhealthy.
        assert_eq!(report.status, HealthStatus::Unhealthy);
    }
}
