//! `AppBuilder::read_side_progress` (PROD-014A task 3.5): the refusal a
//! volatile registered progress pair produces under `Profile::Production`
//! must surface through the full `build()` path as
//! `CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_))`
//! — mirroring `effect_store_composition.rs`'s section 8
//! (`app_builder_surfaces_the_missing_effect_store_refusal_as_composition_validation_error`).
//!
//! Extended by STOOLAP-RS-01 PR5 (AD-13) with a real `Profile::Production`
//! composition over three real Stoolap-backed stores and its volatile
//! negative control — the spec's "A Real Profile::Production Composition
//! Exercises The Gate" requirement, which the two tests above (built on
//! synthetic volatile stubs only) do not cover on their own.
//!
//! Run with: cargo test -p ego-service-sdk --test read_side_progress_composition

use std::sync::Arc;

use async_trait::async_trait;
use ego_domain::read_side::dedup::{DedupStore, DedupStoreError};
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::offset::{Offset, OffsetStore, OffsetStoreError};
use ego_service_sdk::app::App;
use ego_service_sdk::app::CompositionError;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, Profile, RuntimeError};

/// Never overrides `is_durable()` — classified volatile by default
/// (PROD-014A AD-4), matching every real implementation in this workspace
/// today.
struct VolatileOffsetStore;

#[async_trait]
impl OffsetStore for VolatileOffsetStore {
    async fn read_offset(
        &self,
        _projection_id: &str,
        _tag: &EventTag,
        _tenant: &str,
    ) -> Result<Option<Offset>, OffsetStoreError> {
        unreachable!("this test never reaches the store")
    }

    async fn write_offset(
        &self,
        _projection_id: &str,
        _tag: &EventTag,
        _tenant: &str,
        _offset: &Offset,
    ) -> Result<(), OffsetStoreError> {
        unreachable!("this test never reaches the store")
    }
}

struct VolatileDedupStore;

#[async_trait]
impl DedupStore for VolatileDedupStore {
    async fn seen(
        &self,
        _projection_id: &str,
        _tag: &EventTag,
        _event_id: &str,
    ) -> Result<bool, DedupStoreError> {
        unreachable!("this test never reaches the store")
    }

    async fn mark_seen(
        &self,
        _projection_id: &str,
        _tag: &EventTag,
        _event_id: &str,
    ) -> Result<(), DedupStoreError> {
        unreachable!("this test never reaches the store")
    }
}

#[test]
fn app_builder_surfaces_a_volatile_read_side_progress_pair_as_composition_validation_error() {
    let err = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .profile(Profile::Production)
        .read_side_progress(
            "users-by-tenant",
            Arc::new(VolatileOffsetStore),
            Arc::new(VolatileDedupStore),
        )
        .build()
        .err()
        .expect("Production with a volatile registered progress pair must refuse");

    match err {
        CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_)) => {}
        other => panic!("expected Validation(PersistenceNotConfigured), got {other:?}"),
    }
}

/// Regression: the same volatile pair still builds cleanly under
/// `Profile::Dev` (IS-6) — the gate is Production-only.
#[test]
fn app_builder_accepts_a_volatile_read_side_progress_pair_under_dev_profile() {
    let result = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .profile(Profile::Dev)
        .read_side_progress(
            "users-by-tenant",
            Arc::new(VolatileOffsetStore),
            Arc::new(VolatileDedupStore),
        )
        .build();

    assert!(
        result.is_ok(),
        "Dev profile must accept a volatile read-side progress pair"
    );
}

// ---------------------------------------------------------------------------
// STOOLAP-RS-01 PR5 (AD-13): a real Profile::Production composition over
// three real Stoolap-backed stores, and the identical composition with one
// store swapped for volatile — spec `persistence-stoolap-read-side`'s "A Real
// Profile::Production Composition Exercises The Gate, With A Negative
// Control". This is not the isolated `is_durable()` check the spec calls
// insufficient: it drives registration through the real `App::builder()` /
// `.build()` path, exercising the SAME unmodified gate
// (`validate_read_side_progress_profile` / `validate_read_side_claim_profile`)
// the two tests above already exercise against volatile stubs — mirroring
// `operation_reservation_gate_composition.rs`'s Stoolap case.
// ---------------------------------------------------------------------------

/// A real durable Stoolap-backed offset, dedup, and claim store, all opened
/// at the same tempdir path (Stoolap's process-global-engine-per-DSN
/// property, re-proved for this crate by PR4's
/// `three_stores_at_one_path_share_one_engine`), registered through
/// `App::builder()` under `Profile::Production`. The unmodified gate must
/// accept it.
#[tokio::test]
async fn a_real_durable_stoolap_composition_builds_under_production_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let offset = ego_persistence_stoolap::StoolapOffsetStore::open(dir.path())
        .await
        .expect("open a real embedded StoolapOffsetStore");
    let dedup = ego_persistence_stoolap::StoolapDedupStore::open(dir.path())
        .await
        .expect("open a real embedded StoolapDedupStore");
    let claim = ego_persistence_stoolap::StoolapReadSideClaimStore::open(
        dir.path(),
        Arc::new(ego_testkit::TestClock::new(chrono::Utc::now())),
    )
    .await
    .expect("open a real embedded StoolapReadSideClaimStore");

    let app = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .profile(Profile::Production)
        .read_side_progress("users-by-tenant", Arc::new(offset), Arc::new(dedup))
        .read_side_claims(Arc::new(claim))
        .build();

    assert!(
        app.is_ok(),
        "Production with three real, durable Stoolap-backed read-side stores must succeed: {:?}",
        app.err()
    );
}

/// Negative control: the identical composition, with exactly one store
/// swapped for this file's existing `VolatileOffsetStore` — design.md's exact
/// interface-contract example. The unmodified gate must still refuse,
/// proving the acceptance above is not a gate that has quietly gone
/// permissive.
#[tokio::test]
async fn a_stoolap_composition_with_one_volatile_store_is_rejected_under_production_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dedup = ego_persistence_stoolap::StoolapDedupStore::open(dir.path())
        .await
        .expect("open a real embedded StoolapDedupStore");
    let claim = ego_persistence_stoolap::StoolapReadSideClaimStore::open(
        dir.path(),
        Arc::new(ego_testkit::TestClock::new(chrono::Utc::now())),
    )
    .await
    .expect("open a real embedded StoolapReadSideClaimStore");

    let err = App::builder()
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .profile(Profile::Production)
        .read_side_progress(
            "users-by-tenant",
            Arc::new(VolatileOffsetStore),
            Arc::new(dedup),
        )
        .read_side_claims(Arc::new(claim))
        .build()
        .err()
        .expect("Production with one volatile store among three real Stoolap stores must refuse");

    match err {
        CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_)) => {}
        other => panic!("expected Validation(PersistenceNotConfigured), got {other:?}"),
    }
}
