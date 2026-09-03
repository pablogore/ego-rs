//! PROD-013 AD-10: regression guard for the `Profile::Dev` half of IS-12.
//!
//! Two behaviors, not one: `EntityEventStores::in_memory()` must keep
//! declaring [`Profile::Dev`], AND a composition built over it must keep
//! building. A profile-only assertion would not catch a future change that
//! flips the declared profile to `Production` while leaving the stores
//! themselves in memory — the exact drift AD-8 exists to make unreachable.
//! No pool, no Docker: this runs under plain `cargo test -p reference-app`.
//! The `Profile::Production` half lives in the existing
//! `integration-tests/tests/infrastructure/durable_entity_progress_postgres.rs`,
//! which already opens a real pool `EntityEventStores::open` needs.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ego_persistence_api::read_side::claim::{ClaimError, ClaimFence, ClaimId, ReadSideClaimStore};
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, Profile};
use ego_service_sdk::App;
use reference_app::read_side::{ReadSideProgressStores, PROJECTION_ID};
use reference_app::{build_runtime_with, AppConfig, EntityEventStores, ExternalEffectsWiring};

/// Minimal `ReadSideClaimStore` stub for the `Profile::Production` accept
/// path (PROD-014C): only `is_durable()` is read by the gate this test
/// exercises via `.build()`, so `try_claim`/`renew`/`release` are never
/// called — mirrors `StubClaimStore` in `service-sdk`'s own gate tests.
struct StubDurableClaimStore;

#[async_trait::async_trait]
impl ReadSideClaimStore for StubDurableClaimStore {
    fn is_durable(&self) -> bool {
        true
    }

    async fn try_claim(
        &self,
        _claim_id: &ClaimId,
        _owner_id: &ego_domain::operation::OwnerId,
        _lease_until: DateTime<Utc>,
    ) -> Result<Option<ClaimFence>, ClaimError> {
        unreachable!("this guard only calls .build(), never spawns the poll loop")
    }

    async fn renew(
        &self,
        _fence: &ClaimFence,
        _lease_until: DateTime<Utc>,
    ) -> Result<(), ClaimError> {
        unreachable!("this guard only calls .build(), never spawns the poll loop")
    }

    async fn release(&self, _fence: &ClaimFence) -> Result<(), ClaimError> {
        unreachable!("this guard only calls .build(), never spawns the poll loop")
    }
}

#[test]
fn in_memory_stores_declare_profile_dev() {
    assert_eq!(
        EntityEventStores::in_memory().profile(),
        Profile::Dev,
        "in_memory() must never look like a production declaration"
    );
}

#[test]
fn dev_profile_still_builds_at_the_composition_root() {
    build_runtime_with(
        &AppConfig::default(),
        EntityEventStores::in_memory(),
        reference_app::IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
        None,
    )
    .expect("Profile::Dev over in-memory stores must still build (SC-5), with no read-side progress registered");
}

// PROD-014A SC-2/SC-10: `Profile::Production` with a registered pair whose
// `OffsetStore`/`DedupStore` are both durable builds successfully. Exercised
// directly through `App::builder()` (not `build_runtime_with`, which can
// only reach `Profile::Production` via `EntityEventStores::open` and a real
// Postgres pool — see this file's Production-over-real-stores counterpart,
// `integration-tests/tests/infrastructure/durable_entity_progress_postgres.rs`)
// so this stays a plain `cargo test -p reference-app`, no Docker required.
#[test]
fn production_profile_with_durable_read_side_progress_registers_and_builds() {
    let pair = ReadSideProgressStores::fake_durable();
    App::builder()
        .profile(Profile::Production)
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .read_side_progress(PROJECTION_ID, pair.offset, pair.dedup)
        // PROD-014C AD-9: a registered progress pair now also requires a
        // durable claim store, or `Profile::Production` refuses.
        .read_side_claims(Arc::new(StubDurableClaimStore))
        .build()
        .expect(
            "a durable read-side progress pair plus a durable claim store \
             must be accepted under Profile::Production",
        );
}

// PROD-014A SC-3: `Profile::Production` with either store of a registered
// pair volatile is refused at `AppBuilder::build()` — never deferred.
#[test]
fn production_profile_with_volatile_read_side_progress_is_refused() {
    let pair = ReadSideProgressStores::in_memory();
    let result = App::builder()
        .profile(Profile::Production)
        .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .read_side_progress(PROJECTION_ID, pair.offset, pair.dedup)
        .build();
    let Err(err) = result else {
        panic!(
            "a volatile registered read-side progress pair must be refused under Profile::Production"
        );
    };

    let message = err.to_string();
    assert!(
        message.contains("read-side progress"),
        "the refusal must name the read-side progress capability, got: {message}"
    );
}
