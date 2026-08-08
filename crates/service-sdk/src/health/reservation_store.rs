//! Readiness for the registered `OperationReservationStore`.
//!
//! # The two failure modes are not the same failure
//!
//! A runtime that declares `IdempotencyEnforcementMode::MandatoryKey` and has
//! **no store registered at all** never starts: `RuntimeBuilder::build` refuses
//! the configuration outright, because a process that promises idempotent
//! dispatch with nowhere to reserve a key cannot keep that promise for a single
//! request. That is a misconfiguration, it is discovered at bootstrap, and it is
//! not this module's subject.
//!
//! This module covers the other one: the store **is** registered, the runtime
//! **did** start, and the backing store has since become unreachable. Nothing
//! about the configuration is wrong; the dependency is down. That cannot be
//! decided at startup, only observed, so it belongs to readiness.
//!
//! # Readiness, deliberately not liveness
//!
//! An unreachable store makes this contributor `Unhealthy`, which makes the
//! readiness probe fail, which takes the instance out of rotation until the
//! store comes back. It does **not** touch liveness — `Runtime::liveness`
//! consults no contributor at all and keeps answering `Healthy`.
//!
//! That separation is the whole point. Losing Postgres is not a reason to kill
//! and restart the process: the new process would come up against the same
//! unreachable database, fail the same way, and — under a restart-on-failure
//! supervisor — do it in a loop, replacing a recoverable outage with a crash
//! loop that clears no state and fixes nothing. Stopping traffic is the correct
//! response; stopping the process is not.

use std::sync::Arc;

use async_trait::async_trait;
use ego_domain::health::{
    DependencyRequirement, HealthCheck, HealthCode, HealthContributor, HealthStatus,
};
use ego_domain::operation::{OperationReservationStore, ReservationError};

/// The name this contributor reports under.
///
/// Public and named, because it is the string an operator greps for in a
/// readiness payload and the one the wiring tests assert on — a literal spelled
/// in both places would let the two drift apart silently.
pub const OPERATION_RESERVATION_STORE_CONTRIBUTOR: &str = "operation-reservation-store";

/// Reports the reachability of the runtime's registered
/// [`OperationReservationStore`].
///
/// Holds the store as an `Arc` and probes **that** instance. It is constructed
/// by `RuntimeBuilder::build` from the very `Arc` that is then handed to
/// `RuntimeInner`, so the thing the readiness probe reports on is the thing
/// dispatch reserves through — not a second connection, a second pool, or a
/// second reading of the same configuration, any of which could be healthy
/// while the store actually in use is not.
pub struct OperationReservationStoreHealthContributor {
    store: Arc<dyn OperationReservationStore>,
}

impl OperationReservationStoreHealthContributor {
    /// Builds a contributor over `store`.
    pub fn new(store: Arc<dyn OperationReservationStore>) -> Self {
        Self { store }
    }

    /// The store this contributor probes.
    ///
    /// Exposed so a caller can establish, by `Arc::ptr_eq`, that this is the
    /// same instance the runtime dispatches through.
    pub fn store(&self) -> &Arc<dyn OperationReservationStore> {
        &self.store
    }
}

impl std::fmt::Debug for OperationReservationStoreHealthContributor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationReservationStoreHealthContributor")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl HealthContributor for OperationReservationStoreHealthContributor {
    fn name(&self) -> &str {
        OPERATION_RESERVATION_STORE_CONTRIBUTOR
    }

    fn requirement(&self) -> DependencyRequirement {
        // `Required`, not `Optional`. A registered reservation store is the
        // only place a client-supplied operation key can be reserved; with it
        // unreachable, a retried request cannot be recognised as a retry and
        // would be executed a second time. Serving in that state is exactly the
        // duplicate execution the whole mechanism exists to prevent, so this
        // must fail readiness rather than merely degrade it.
        DependencyRequirement::Required
    }

    async fn check(&self) -> HealthCheck {
        match self.store.probe().await {
            Ok(()) => HealthCheck {
                status: HealthStatus::Healthy,
                code: None,
            },
            // The error's message is read for nothing and carried nowhere.
            // `ReservationError::Backend` wraps whatever the driver said, and a
            // driver's connection error can name a host, a user, a database, or
            // a connection string that includes a password. `HealthCode` is a
            // closed set that structurally cannot carry a payload, so the
            // readiness response says "unreachable" and stops — the detail
            // belongs in the process's own logs, not in a response an unauthed
            // probe endpoint hands out.
            Err(ReservationError::Backend(_)) => HealthCheck {
                status: HealthStatus::Unhealthy,
                code: Some(HealthCode::Unavailable),
            },
            // A read-only probe has no owner, no fence and no token, so it
            // cannot legitimately produce either of these. Reaching here means
            // the implementation is doing something other than what `probe` is
            // specified to do, which is a defect in the adapter rather than an
            // outage in the dependency — and it is reported as one instead of
            // being folded into `Unavailable`, so the two do not look alike
            // while a store is being written or replaced.
            Err(ReservationError::StaleOwner | ReservationError::FencingExhausted) => HealthCheck {
                status: HealthStatus::Unhealthy,
                code: Some(HealthCode::InternalFailure),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use ego_domain::operation::{OwnerFence, ReservationOutcome, ReserveRequest, StoredResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A store whose `probe` answers with a fixed outcome and counts the
    /// calls, and whose five real methods are unreachable — a readiness check
    /// that invoked one of them panics here rather than passing quietly while
    /// mutating the table it is supposed to be observing.
    struct ProbeStore {
        outcome: Result<(), ReservationError>,
        probes: AtomicUsize,
    }

    impl ProbeStore {
        fn always(outcome: Result<(), ReservationError>) -> Arc<Self> {
            Arc::new(Self {
                outcome,
                probes: AtomicUsize::new(0),
            })
        }

        fn probes(&self) -> usize {
            self.probes.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl OperationReservationStore for ProbeStore {
        async fn reserve(
            &self,
            _req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            panic!("a readiness probe must never reserve an operation");
        }

        async fn renew(
            &self,
            _fence: &OwnerFence,
            _until: DateTime<Utc>,
        ) -> Result<(), ReservationError> {
            panic!("a readiness probe must never renew a lease");
        }

        async fn complete(
            &self,
            _fence: &OwnerFence,
            _response: StoredResponse,
        ) -> Result<(), ReservationError> {
            panic!("a readiness probe must never complete a reservation");
        }

        async fn abandon(&self, _fence: &OwnerFence) -> Result<(), ReservationError> {
            panic!("a readiness probe must never abandon a reservation");
        }

        async fn purge_completed_before(
            &self,
            _cutoff: DateTime<Utc>,
            _batch: usize,
        ) -> Result<u64, ReservationError> {
            panic!("a readiness probe must never purge reservations");
        }

        async fn probe(&self) -> Result<(), ReservationError> {
            self.probes.fetch_add(1, Ordering::SeqCst);
            self.outcome.clone()
        }
    }

    #[tokio::test]
    async fn a_reachable_store_reports_healthy_with_no_code() {
        let contributor =
            OperationReservationStoreHealthContributor::new(ProbeStore::always(Ok(())));

        let check = contributor.check().await;

        assert_eq!(check.status, HealthStatus::Healthy);
        assert_eq!(check.code, None);
    }

    #[tokio::test]
    async fn a_backend_failure_reports_unhealthy_and_unavailable() {
        let contributor = OperationReservationStoreHealthContributor::new(ProbeStore::always(Err(
            ReservationError::Backend("connection refused".to_string()),
        )));

        let check = contributor.check().await;

        assert_eq!(
            check.status,
            HealthStatus::Unhealthy,
            "an unreachable store must never be folded in as healthy — that is the \
             fail-open this contributor exists to close"
        );
        assert_eq!(check.code, Some(HealthCode::Unavailable));
    }

    /// The probe's error text never reaches the report.
    ///
    /// A driver's connection error routinely quotes the DSN it failed on, and a
    /// DSN routinely carries a password. `HealthCode` cannot hold a string at
    /// all, which is what makes this structural rather than a discipline
    /// requirement — this test is here so a future change that adds a
    /// message-carrying field has to break it deliberately.
    #[tokio::test]
    async fn the_probe_error_text_never_reaches_the_report() {
        let secret = "postgres://ego:sup3r-s3cret@db.internal:5432/ego";
        let contributor = OperationReservationStoreHealthContributor::new(ProbeStore::always(Err(
            ReservationError::Backend(format!("failed to connect to {secret}")),
        )));

        let check = contributor.check().await;
        let rendered = format!("{check:?}");

        assert!(
            !rendered.contains("sup3r-s3cret"),
            "the readiness result must not carry the store's error text: {rendered}"
        );
        assert!(
            !rendered.contains("db.internal"),
            "the readiness result must not carry the store's connection detail: {rendered}"
        );
    }

    /// A fence verdict from a read-only probe is an adapter defect, and is
    /// reported as one rather than as an outage.
    #[tokio::test]
    async fn a_fence_verdict_from_a_probe_reports_an_internal_failure() {
        for error in [
            ReservationError::StaleOwner,
            ReservationError::FencingExhausted,
        ] {
            let contributor = OperationReservationStoreHealthContributor::new(ProbeStore::always(
                Err(error.clone()),
            ));

            let check = contributor.check().await;

            assert_eq!(check.status, HealthStatus::Unhealthy);
            assert_eq!(
                check.code,
                Some(HealthCode::InternalFailure),
                "{error:?} from a probe means the adapter is broken, not that the \
                 dependency is down"
            );
        }
    }

    /// The check reaches the store, and reaches it exactly once per call.
    #[tokio::test]
    async fn each_check_probes_the_held_store_once() {
        let store = ProbeStore::always(Ok(()));
        let contributor = OperationReservationStoreHealthContributor::new(store.clone());

        contributor.check().await;
        contributor.check().await;

        assert_eq!(store.probes(), 2);
        assert!(
            Arc::ptr_eq(
                contributor.store(),
                &(store as Arc<dyn OperationReservationStore>)
            ),
            "the contributor must hold the instance it was given, not a copy"
        );
    }

    #[tokio::test]
    async fn the_store_is_a_required_dependency() {
        let contributor =
            OperationReservationStoreHealthContributor::new(ProbeStore::always(Ok(())));

        assert_eq!(
            contributor.requirement(),
            DependencyRequirement::Required,
            "an unreachable reservation store must fail readiness, not merely degrade it: \
             serving without it means a retry cannot be recognised as one"
        );
        assert_eq!(contributor.name(), OPERATION_RESERVATION_STORE_CONTRIBUTOR);
    }
}
