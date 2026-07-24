//! Runtime health model (PROD-005).
//!
//! Domain contracts for expressing dependency health without coupling to
//! any transport, probe framework, or infrastructure concern. This module
//! owns:
//!
//! - The semantic vocabulary — [`ProbeKind`], [`HealthStatus`], [`HealthCode`],
//!   [`DependencyRequirement`].
//! - The per-contributor result shape — [`HealthCheck`], [`ContributorReport`].
//! - The aggregate result shape — [`HealthReport`].
//! - The object-safe contributor contract — [`HealthContributor`].
//! - The deterministic, probe-independent aggregation — [`fold`].
//!
//! ## Determinism Axiom
//!
//! [`fold`] is pure: the same multiset of `(HealthStatus, DependencyRequirement)`
//! contributions always yields the same [`HealthStatus`], independent of
//! evaluation order. A [`HealthCode`] never influences the fold — it rides
//! along on the [`ContributorReport`] purely as diagnostic detail.

use async_trait::async_trait;

/// The probe under which a health evaluation is being performed.
///
/// Kubernetes-style probe taxonomy. Purely semantic — contributors never
/// receive or branch on this value (see [`HealthContributor::check`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// Is the process alive and able to make progress?
    Liveness,
    /// Is the process ready to receive traffic?
    Readiness,
    /// Has the process completed its startup sequence?
    Startup,
}

/// Severity of a health evaluation.
///
/// Ordered by severity for aggregation purposes: `Healthy < Degraded < Unhealthy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Fully operational.
    Healthy,
    /// Operating with reduced capability, but still serving.
    Degraded,
    /// Not operational.
    Unhealthy,
}

/// Whether a dependency is required for correct operation or merely optional.
///
/// Drives how an unhealthy contributor is clamped when folded into the
/// aggregate [`HealthStatus`] — see [`fold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyRequirement {
    /// The dependency must be healthy for the system to be healthy.
    Required,
    /// The dependency's unhealthiness degrades, but never fails, the system.
    Optional,
}

/// A closed, non-string-carrying classification of why a health check failed.
///
/// FROZEN INVARIANT: this set MUST NEVER gain a string-carrying variant
/// (e.g. `Other(String)`). The only public failure surface for free-form
/// detail is the absence of a code (`None`) — never a payload-carrying code.
/// [`fold`] never consults this value; it rides along on [`ContributorReport`]
/// purely for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthCode {
    /// The check did not complete within its allotted time.
    Timeout,
    /// The dependency is currently unreachable.
    Unavailable,
    /// The dependency has not finished initializing yet.
    InitializationPending,
    /// A downstream dependency this contributor relies on has failed.
    DependencyFailure,
    /// An unclassified internal failure occurred while checking.
    InternalFailure,
}

/// The result of evaluating a single [`HealthContributor`].
///
/// Probe-independent: a contributor produces the same `HealthCheck`
/// regardless of which [`ProbeKind`] triggered the evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthCheck {
    /// The evaluated status.
    pub status: HealthStatus,
    /// The closed-set reason code, if the status is not [`HealthStatus::Healthy`].
    pub code: Option<HealthCode>,
}

/// A named contributor's health, annotated with its dependency requirement.
///
/// This is the unit [`fold`] aggregates over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributorReport {
    /// The contributor's name.
    pub name: String,
    /// The contributor's evaluated status.
    pub status: HealthStatus,
    /// Whether this contributor is required or optional.
    pub requirement: DependencyRequirement,
    /// The closed-set reason code, if any.
    pub code: Option<HealthCode>,
}

/// The aggregate health result for a given probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// The probe this report was produced for.
    pub probe: ProbeKind,
    /// The aggregate status, computed by [`fold`] over `contributors`.
    pub status: HealthStatus,
    /// The per-contributor reports that were folded into `status`.
    pub contributors: Vec<ContributorReport>,
}

/// Maps a single contributor's `(status, requirement)` to its contribution
/// on the severity lattice `Unhealthy > Degraded > Healthy`.
///
/// `Unhealthy` + `Optional` is clamped to `Degraded` — an optional
/// dependency's failure never drives the aggregate to `Unhealthy`.
fn contribution(status: HealthStatus, requirement: DependencyRequirement) -> HealthStatus {
    match (status, requirement) {
        (HealthStatus::Healthy, _) => HealthStatus::Healthy,
        (HealthStatus::Degraded, _) => HealthStatus::Degraded,
        (HealthStatus::Unhealthy, DependencyRequirement::Required) => HealthStatus::Unhealthy,
        (HealthStatus::Unhealthy, DependencyRequirement::Optional) => HealthStatus::Degraded,
    }
}

/// Severity rank used to take the MAX contribution. Higher is more severe.
fn severity(status: HealthStatus) -> u8 {
    match status {
        HealthStatus::Healthy => 0,
        HealthStatus::Degraded => 1,
        HealthStatus::Unhealthy => 2,
    }
}

/// Deterministically aggregates contributor reports into a single [`HealthStatus`].
///
/// Probe-independent — takes no [`ProbeKind`]. Each report's `(status,
/// requirement)` is mapped to a contribution via [`contribution`], and the
/// global result is the MAX contribution over the severity lattice
/// `Unhealthy > Degraded > Healthy`. This operation is commutative and
/// associative, so the result is independent of input order. An empty input
/// yields [`HealthStatus::Healthy`].
///
/// `code` on each [`ContributorReport`] is never consulted — it is purely
/// diagnostic and does not participate in the fold.
pub fn fold(reports: &[ContributorReport]) -> HealthStatus {
    reports
        .iter()
        .map(|report| contribution(report.status, report.requirement))
        .max_by_key(|status| severity(*status))
        .unwrap_or(HealthStatus::Healthy)
}

/// A single dependency's health check contract.
///
/// Object-safe by construction: `Vec<std::sync::Arc<dyn HealthContributor>>`
/// must compile, allowing an aggregator (PR2) to hold a heterogeneous
/// collection of contributors.
///
/// # Probe-independence
///
/// [`check`](HealthContributor::check) takes no [`ProbeKind`] and MUST
/// return the same [`HealthCheck`] regardless of which probe triggered the
/// evaluation. Contributors MUST NOT receive or branch on `ProbeKind` — any
/// probe-specific policy (e.g. "readiness only" contributors) is an
/// aggregation concern, not a contributor concern.
#[async_trait]
pub trait HealthContributor: Send + Sync {
    /// The contributor's name, used as the key in [`ContributorReport`].
    fn name(&self) -> &str;

    /// Whether this contributor is required or optional for the aggregate
    /// [`HealthStatus`] — see [`fold`].
    fn requirement(&self) -> DependencyRequirement;

    /// Evaluates this contributor's current health.
    ///
    /// Probe-independent: the same [`HealthCheck`] is returned regardless of
    /// probe.
    async fn check(&self) -> HealthCheck;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_unhealthy_yields_global_unhealthy() {
        let reports = vec![ContributorReport {
            name: "db".to_string(),
            status: HealthStatus::Unhealthy,
            requirement: DependencyRequirement::Required,
            code: Some(HealthCode::DependencyFailure),
        }];

        assert_eq!(fold(&reports), HealthStatus::Unhealthy);
    }

    #[test]
    fn optional_unhealthy_is_clamped_to_degraded_never_global_unhealthy() {
        let reports = vec![ContributorReport {
            name: "cache".to_string(),
            status: HealthStatus::Unhealthy,
            requirement: DependencyRequirement::Optional,
            code: Some(HealthCode::Timeout),
        }];

        assert_eq!(fold(&reports), HealthStatus::Degraded);
    }

    #[test]
    fn empty_contributors_yield_healthy() {
        let reports: Vec<ContributorReport> = vec![];

        assert_eq!(fold(&reports), HealthStatus::Healthy);
    }

    #[test]
    fn fold_is_order_independent_over_the_same_multiset() {
        let a = ContributorReport {
            name: "a".to_string(),
            status: HealthStatus::Healthy,
            requirement: DependencyRequirement::Required,
            code: None,
        };
        let b = ContributorReport {
            name: "b".to_string(),
            status: HealthStatus::Degraded,
            requirement: DependencyRequirement::Optional,
            code: Some(HealthCode::Timeout),
        };
        let c = ContributorReport {
            name: "c".to_string(),
            status: HealthStatus::Unhealthy,
            requirement: DependencyRequirement::Optional,
            code: Some(HealthCode::Unavailable),
        };

        let forward = vec![a.clone(), b.clone(), c.clone()];
        let reversed = vec![c.clone(), b.clone(), a.clone()];
        let shuffled = vec![b, c, a];

        let expected = fold(&forward);
        assert_eq!(fold(&reversed), expected);
        assert_eq!(fold(&shuffled), expected);
    }

    #[test]
    fn health_code_never_alters_the_fold_for_required_unhealthy() {
        let with_initialization_pending = vec![ContributorReport {
            name: "queue".to_string(),
            status: HealthStatus::Unhealthy,
            requirement: DependencyRequirement::Required,
            code: Some(HealthCode::InitializationPending),
        }];
        let with_dependency_failure = vec![ContributorReport {
            name: "queue".to_string(),
            status: HealthStatus::Unhealthy,
            requirement: DependencyRequirement::Required,
            code: Some(HealthCode::DependencyFailure),
        }];

        assert_eq!(fold(&with_initialization_pending), HealthStatus::Unhealthy);
        assert_eq!(fold(&with_dependency_failure), HealthStatus::Unhealthy);
    }

    #[test]
    fn health_code_never_alters_the_fold_for_optional_unhealthy() {
        let with_initialization_pending = vec![ContributorReport {
            name: "warmer".to_string(),
            status: HealthStatus::Unhealthy,
            requirement: DependencyRequirement::Optional,
            code: Some(HealthCode::InitializationPending),
        }];
        let with_dependency_failure = vec![ContributorReport {
            name: "warmer".to_string(),
            status: HealthStatus::Unhealthy,
            requirement: DependencyRequirement::Optional,
            code: Some(HealthCode::DependencyFailure),
        }];

        assert_eq!(fold(&with_initialization_pending), HealthStatus::Degraded);
        assert_eq!(fold(&with_dependency_failure), HealthStatus::Degraded);
    }

    /// A trivial stub contributor used only to prove `HealthContributor` is
    /// object-safe.
    struct StubContributor {
        name: String,
        requirement: DependencyRequirement,
        check: HealthCheck,
    }

    #[async_trait]
    impl HealthContributor for StubContributor {
        fn name(&self) -> &str {
            &self.name
        }

        fn requirement(&self) -> DependencyRequirement {
            self.requirement
        }

        async fn check(&self) -> HealthCheck {
            self.check
        }
    }

    #[tokio::test]
    async fn health_contributor_is_object_safe_as_vec_of_arc_dyn() {
        let contributors: Vec<std::sync::Arc<dyn HealthContributor>> = vec![
            std::sync::Arc::new(StubContributor {
                name: "db".to_string(),
                requirement: DependencyRequirement::Required,
                check: HealthCheck {
                    status: HealthStatus::Healthy,
                    code: None,
                },
            }),
            std::sync::Arc::new(StubContributor {
                name: "cache".to_string(),
                requirement: DependencyRequirement::Optional,
                check: HealthCheck {
                    status: HealthStatus::Unhealthy,
                    code: Some(HealthCode::Unavailable),
                },
            }),
        ];

        assert_eq!(contributors[0].name(), "db");
        assert_eq!(
            contributors[1].requirement(),
            DependencyRequirement::Optional
        );
        assert_eq!(
            contributors[1].check().await,
            HealthCheck {
                status: HealthStatus::Unhealthy,
                code: Some(HealthCode::Unavailable),
            }
        );
    }

    /// Exhaustively matches every `HealthCode` variant. If a string-carrying
    /// variant (e.g. `Other(String)`) is ever added, this match becomes
    /// non-exhaustive and fails to compile — the closed set is enforced at
    /// compile time, not by a runtime reflection test.
    fn describe(code: HealthCode) -> &'static str {
        match code {
            HealthCode::Timeout => "timeout",
            HealthCode::Unavailable => "unavailable",
            HealthCode::InitializationPending => "initialization_pending",
            HealthCode::DependencyFailure => "dependency_failure",
            HealthCode::InternalFailure => "internal_failure",
        }
    }

    #[test]
    fn health_code_is_a_closed_set_with_no_string_carrying_variant() {
        assert_eq!(describe(HealthCode::Timeout), "timeout");
        assert_eq!(describe(HealthCode::Unavailable), "unavailable");
        assert_eq!(
            describe(HealthCode::InitializationPending),
            "initialization_pending"
        );
        assert_eq!(
            describe(HealthCode::DependencyFailure),
            "dependency_failure"
        );
        assert_eq!(describe(HealthCode::InternalFailure), "internal_failure");
    }
}
