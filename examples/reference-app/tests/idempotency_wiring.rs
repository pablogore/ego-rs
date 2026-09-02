//! The posture the host chose is the posture the host gets.
//!
//! **Layers traversed:** `build_runtime_with` → `AppBuilder` → the built
//! `Runtime`, read back through `App::resolver()`.
//!
//! # What this covers, and what covers the rest
//!
//! `build_runtime_with` now takes an [`IdempotencyWiring`] instead of fixing the
//! posture itself, and this file is about that selection: whichever variant the
//! host names is the one in force, and neither is reachable by omission.
//!
//! It deliberately does **not** re-assert that the four pieces of an enforced
//! configuration arrive. `RuntimeResolver` exposes the enforcement mode and
//! nothing about the reservation, so that claim cannot be made honestly from out
//! here — it is proven inside `ego-service-sdk`, against
//! `AppBuilder::enforced_idempotency` itself, where the config is readable
//! (`app::enforced_idempotency_wiring`). Two halves, each asserted where its
//! evidence actually is.
//!
//! # No infrastructure
//!
//! Nothing here reserves anything. The subject is what the composition root
//! selected, which is observable the moment it is built; a container would add a
//! dependency without adding evidence.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use ego_domain::operation::{
    OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome,
    ReserveRequest, StoredServiceResponse,
};
use ego_domain::time::Clock;
use ego_service_sdk::runtime::IdempotencyEnforcementMode;
use reference_app::{
    build_runtime_with, AppConfig, EntityEventStores, ExternalEffectsWiring, IdempotencyWiring,
};

/// Frozen, and years in the past. Only its existence matters here.
struct FrozenClock;

impl Clock for FrozenClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2021, 3, 14, 15, 9, 26).unwrap()
    }
}

/// Registered, never driven — this file provokes no reservation.
struct InertStore;

#[async_trait]
impl OperationReservationStore for InertStore {
    async fn reserve(&self, _req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        unreachable!("this fixture is registered, never driven")
    }
    async fn renew(&self, _f: &OwnerFence, _until: DateTime<Utc>) -> Result<(), ReservationError> {
        unreachable!()
    }
    async fn complete(
        &self,
        _f: &OwnerFence,
        _r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        unreachable!()
    }
    async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
        unreachable!()
    }
    async fn purge_completed_before(
        &self,
        _cutoff: DateTime<Utc>,
        _batch: usize,
    ) -> Result<u64, ReservationError> {
        unreachable!()
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
    }
}

fn mode_under(idempotency: IdempotencyWiring) -> IdempotencyEnforcementMode {
    build_runtime_with(
        &AppConfig::default(),
        EntityEventStores::in_memory(),
        idempotency,
        None,
        ExternalEffectsWiring::None,
        None,
    )
    .expect("the reference app builds")
    .app
    .resolver()
    .idempotency_enforcement_mode()
}

/// The compatibility posture keeps admitting requests with no operation key.
///
/// The negative control for the one below: without it, a change that made every
/// host enforce would leave nothing failing, and the honest "not adopted yet"
/// declaration would quietly stop existing.
#[test]
fn the_compatibility_posture_is_carried_through() {
    assert_eq!(
        mode_under(IdempotencyWiring::Compatibility),
        IdempotencyEnforcementMode::Compatibility,
        "a deployment still in transition must not be silently upgraded"
    );
}

/// The enforced posture reaches the built runtime.
///
/// This is what `build_runtime_with` could not express before: the function fixed
/// `Compatibility` itself, so no host could adopt enforcement through it at all.
#[test]
fn the_enforced_posture_is_carried_through() {
    assert_eq!(
        mode_under(IdempotencyWiring::Enforced {
            store: Arc::new(InertStore),
            owner_id: OwnerId::new("replica-under-test"),
            lease_duration: Duration::from_secs(97),
            clock: Arc::new(FrozenClock),
        }),
        IdempotencyEnforcementMode::MandatoryKey,
        "the host asked for enforcement, so the runtime must require a key — this \
         function used to hardcode the weaker posture and ignore the caller"
    );
}

/// An enforced host is never downgraded to compatibility.
///
/// Stated separately from the assertion above because it is the mutation that
/// matters: a composition root that accepted the wiring and then matched to the
/// compatibility arm anyway would look correct at the call site and be wrong in
/// force.
#[test]
fn an_enforced_host_is_not_downgraded() {
    let mode = mode_under(IdempotencyWiring::Enforced {
        store: Arc::new(InertStore),
        owner_id: OwnerId::new("replica-under-test"),
        lease_duration: Duration::from_secs(97),
        clock: Arc::new(FrozenClock),
    });

    assert_ne!(
        mode,
        IdempotencyEnforcementMode::Compatibility,
        "a host that named an enforced posture must not end up in the permissive \
         one, whatever else the composition root does"
    );
}
