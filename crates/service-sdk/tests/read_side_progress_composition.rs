//! `AppBuilder::read_side_progress` (PROD-014A task 3.5): the refusal a
//! volatile registered progress pair produces under `Profile::Production`
//! must surface through the full `build()` path as
//! `CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_))`
//! — mirroring `effect_store_composition.rs`'s section 8
//! (`app_builder_surfaces_the_missing_effect_store_refusal_as_composition_validation_error`).
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
